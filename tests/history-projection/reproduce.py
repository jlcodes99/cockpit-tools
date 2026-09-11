"""Network-denied synthetic Codex history experiment. No real account data."""
import argparse
import json
import os
from pathlib import Path
import queue
import shutil
import sqlite3
import subprocess
import tempfile
import threading
import uuid


def encode(record):
    return (json.dumps(record, separators=(",", ":")) + "\n").encode()


class Native:
    def __init__(self, binary, home):
        self.home = home
        self.env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home),
                    "CODEX_HOME": str(home), "RUST_LOG": "warn"}
        self.command = ["/usr/bin/sandbox-exec", "-p",
                        '(version 1)(allow default)(deny network*)', str(binary)]

    def run(self, *args):
        if args[:3] != ("migrate-rollouts", "--apply", "--json"):
            raise ValueError("Only local rollout migration is permitted")
        return subprocess.run(self.command + list(args), env=self.env, cwd=self.home,
                              capture_output=True, text=True, timeout=40)

    def read(self, tid=None, resume=False):
        messages = queue.Queue()
        with (self.home / "server.stderr").open("a") as err:
            p = subprocess.Popen(self.command + ["app-server", "--stdio"], env=self.env,
                                 cwd=self.home, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=err, text=True)
            def reader():
                for line in p.stdout:
                    try:
                        messages.put(json.loads(line))
                    except ValueError:
                        pass
            threading.Thread(target=reader, daemon=True).start()
            def rpc(i, method, params):
                if method not in {"initialize", "thread/resume", "thread/turns/list"}:
                    raise ValueError("Model task RPCs are forbidden in this harness")
                p.stdin.write(json.dumps({"id": i, "method": method, "params": params}) + "\n")
                p.stdin.flush()
                while True:
                    msg = messages.get(timeout=20)
                    if msg.get("id") == i:
                        return msg
            try:
                result = rpc(1, "initialize", {"clientInfo": {"name": "synthetic-history-test", "version": "1"},
                                               "capabilities": {"experimentalApi": True}})
                p.stdin.write('{"method":"initialized"}\n')
                p.stdin.flush()
                if tid:
                    if resume:
                        resumed = rpc(2, "thread/resume", {"threadId": tid, "cwd": str(self.home),
                                      "modelProvider": "openai"})
                        if "error" in resumed:
                            return resumed
                    result = rpc(3, "thread/turns/list", {"threadId": tid, "limit": 20,
                                 "sortDirection": "asc", "itemsView": "summary"})
                return result
            finally:
                p.terminate()
                try:
                    p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait()


def seed(binary):
    home = Path(tempfile.mkdtemp(prefix="cockpit-synthetic-history-"))
    (home / ".synthetic-history-fixture").write_text("synthetic-only-v1\n")
    native = Native(binary, home)
    native.read()
    tid = str(uuid.uuid4())
    path = home / "sessions" / ("rollout-2026-09-01T00-00-00-" + tid + ".jsonl")
    path.parent.mkdir(exist_ok=True)
    records = [{"timestamp": "2026-09-01T00:00:00Z", "type": "session_meta", "payload": {
        "id": tid, "session_id": tid, "originator": "codex_cli_rs",
        "timestamp": "2026-09-01T00:00:00Z", "cwd": str(home), "source": "cli",
        "cli_version": "0.153.0", "model_provider": "synthetic-provider", "history_mode": "legacy"}}]
    for i in range(2):
        turn = str(uuid.uuid4())
        for payload in [{"type": "task_started", "turn_id": turn, "model_context_window": 10000,
                         "collaboration_mode_kind": "default"},
                        {"type": "user_message", "message": "Synthetic question " + str(i), "images": []},
                        {"type": "agent_message", "message": "Synthetic answer " + str(i)},
                        {"type": "task_complete", "turn_id": turn, "last_agent_message": ""}]:
            records.append({"timestamp": "2026-09-01T00:00:01Z", "type": "event_msg", "payload": payload})
    path.write_bytes(b"".join(map(encode, records)))
    with sqlite3.connect(home / "state_5.sqlite") as db:
        db.execute("INSERT INTO threads (id,rollout_path,created_at,updated_at,source,model_provider,cwd,title,sandbox_policy,approval_mode) VALUES (?,?,?,?,?,?,?,?,?,?)",
                   (tid, str(path), 1788220800, 1788220801, "cli", "synthetic-provider", str(home), "Synthetic history", '{"type":"read-only"}', "never"))
    migration = native.run("migrate-rollouts", "--apply", "--json", "--thread", tid)
    print("home:", home)
    print("migration:", migration.returncode, migration.stdout, migration.stderr)
    baseline = native.read(tid)
    assert len(baseline["result"]["data"]) == 2, baseline
    return home, tid, path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex-bin", required=True, type=Path)
    parser.add_argument("--rewrite-bin", type=Path)
    args = parser.parse_args()
    home, tid, path = seed(args.codex_bin)
    migrated = list(map(json.loads, path.read_bytes().splitlines()))
    report = {}
    boundary_provider = "synthetic-provider" + "x" * len(encode(migrated[-1]))
    assert len(boundary_provider) <= 200  # Cockpit's provider ID validation limit.
    cases = [("control", "synthetic-provider"), ("shorter", "openai"),
             ("longer", "synthetic-provider-with-longer-name"), ("longer_boundary", boundary_provider),
             ("duplicate_ordinal", "synthetic-provider")]
    if args.rewrite_bin:
        cases += [("guarded_shorter", "openai"), ("guarded_longer", "synthetic-provider-with-longer-name"),
                  ("guarded_boundary", boundary_provider), ("guarded_full", "openai"),
                  ("guarded_roundtrip", "synthetic-provider")]
    for mode, provider in cases:
        trial = home.parent / (home.name + "-" + mode)
        shutil.copytree(home, trial)
        rollout = trial / "sessions" / path.name
        with sqlite3.connect(trial / "state_5.sqlite") as db:
            db.execute("UPDATE threads SET rollout_path=?,cwd=? WHERE id=?", (str(rollout), str(trial), tid))
        before = rollout.read_bytes()
        first, rest = before.split(b"\n", 1)
        refused = False
        if mode.startswith("guarded"):
            if mode == "guarded_roundtrip":
                subprocess.run([str(args.rewrite_bin), "first", str(rollout), "openai"], check=True)
            repaired = subprocess.run([str(args.rewrite_bin), "full" if mode == "guarded_full" else "first",
                                       str(rollout), provider], capture_output=True, text=True, check=False)
            refused = repaired.returncode == 2
            assert repaired.returncode in (0, 2), repaired.stderr
            changed = rollout.read_bytes()
            if refused:
                assert changed == before, "Refused repair changed the rollout"
        else:
            meta = json.loads(first)
            meta["payload"]["model_provider"] = provider
            # Match the old Cockpit write path, without loading any account data.
            changed = encode(meta) + rest
            temporary = rollout.with_suffix(".tmp")
            temporary.write_bytes(changed)
            temporary.replace(rollout)
        os.utime(rollout, ns=(path.stat().st_atime_ns, path.stat().st_mtime_ns))
        added = json.loads(json.dumps(migrated[-4:]))
        new_turn = str(uuid.uuid4())
        for i, record in enumerate(added):
            record["ordinal"] = len(migrated) + i
            record["payload"]["turn_id"] = new_turn
            if "item" in record["payload"]:
                record["payload"]["item"]["id"] += "-new"
        with rollout.open("ab") as f:
            if mode == "duplicate_ordinal":
                # Independent failure: no provider or byte-layout change is needed.
                f.write(encode(migrated[-1]))
            f.write(b"".join(map(encode, added)))
        result = Native(args.codex_bin, trial).read(tid, resume=True)
        with sqlite3.connect(trial / "thread_history_1.sqlite") as db:
            projection = db.execute("SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id=?", (tid,)).fetchone()
            count = db.execute("SELECT count(*) FROM thread_turns WHERE thread_id=?", (tid,)).fetchone()[0]
        warnings = [line for line in (trial / "server.stderr").read_text().splitlines() if "ordinal" in line]
        report[mode] = {"size_delta": len(changed)-len(before), "indexed_turns": count, "refused": refused,
                        "projection": projection, "rpc_error": result.get("error"),
                        "visible_turns": len(result.get("result", {}).get("data", [])),
                        "ordinal_warnings": len(warnings),
                        "stalled_with_ordinal_mismatch": any("expected ordinal 9, got 8" in line for line in warnings)}
        if mode.startswith("guarded") or mode == "control":
            assert len(changed) == len(before)
            assert count == 3 and report[mode]["visible_turns"] == 3 and not warnings, report[mode]
    assert report["longer_boundary"]["stalled_with_ordinal_mismatch"]
    assert report["longer_boundary"]["visible_turns"] == 2
    assert report["duplicate_ordinal"]["size_delta"] == 0
    assert report["duplicate_ordinal"]["stalled_with_ordinal_mismatch"]
    assert report["duplicate_ordinal"]["visible_turns"] == 2
    (home / "summary.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
