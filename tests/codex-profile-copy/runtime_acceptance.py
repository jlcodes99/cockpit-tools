#!/usr/bin/env python3
"""Offline macOS acceptance test using an installed Codex app-server and synthetic data."""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import sqlite3
import subprocess
import threading
import time


class Server:
    def __init__(self, codex, home, work, policy, label):
        self.messages = queue.Queue()
        self.counter = 0
        env = {k: v for k, v in os.environ.items() if k in {"PATH", "LANG", "LC_ALL", "TMPDIR"}}
        # Exercise the installed runtime with the same originator as the desktop app.
        env.update(HOME=str(work / "fake-home"), CODEX_HOME=str(home),
                   CODEX_INTERNAL_ORIGINATOR_OVERRIDE="Codex Desktop", CODEX_CI="1")
        self.log = (work / (label + ".stderr.log")).open("w")
        self.process = subprocess.Popen(
            ["sandbox-exec", "-f", str(policy), str(codex), "app-server", "--stdio"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
            text=True, env=env, cwd=work, start_new_session=True,
        )

        def read():
            for line in self.process.stdout:
                try:
                    self.messages.put(json.loads(line))
                except json.JSONDecodeError:
                    pass

        threading.Thread(target=read, daemon=True).start()
        try:
            self.call("initialize", {
                "clientInfo": {"name": "cockpit-profile-copy-test", "version": "0.1"},
                "capabilities": {"experimentalApi": True},
            })
            self.process.stdin.write('{"method":"initialized"}\n')
            self.process.stdin.flush()
        except BaseException:
            self.close()
            raise

    def call(self, method, params):
        self.counter += 1
        request_id = self.counter
        self.process.stdin.write(json.dumps({"id": request_id, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        end = time.monotonic() + 30
        while time.monotonic() < end:
            try:
                message = self.messages.get(timeout=max(0.01, end - time.monotonic()))
            except queue.Empty as error:
                raise TimeoutError(method) from error
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(method + ": " + json.dumps(message["error"]))
                return message.get("result")
        raise TimeoutError(method)

    def close(self):
        try:
            self.process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
        self.log.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def user_texts(thread):
    return [part["text"] for turn in thread.get("turns", [])
            for item in turn.get("items", []) if item.get("type") == "userMessage"
            for part in item.get("content", []) if part.get("type") == "text"]


def cancelled_turn(server, thread_id, text):
    turn = server.call("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": text}]})["turn"]
    try:
        end = time.monotonic() + 10
        while time.monotonic() < end:
            try:
                thread = server.call("thread/read", {"threadId": thread_id, "includeTurns": True})["thread"]
                if text in user_texts(thread):
                    break
            except RuntimeError as error:
                # The first turn initializes its history store asynchronously.
                if "list_turns is not supported yet" not in str(error):
                    raise
            time.sleep(0.1)
        else:
            raise AssertionError("Synthetic user turn was not persisted")
    finally:
        try:
            server.call("turn/interrupt", {"threadId": thread_id, "turnId": turn["id"]})
        except RuntimeError as error:
            # The network-denied provider may have already ended the turn.
            if "no active turn to interrupt" not in str(error):
                raise
    time.sleep(0.1)


def rollout_hashes(home):
    return {str(p.relative_to(home)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in home.glob("sessions/**/*.jsonl")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--copy-helper", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True, help="New or empty disposable directory")
    args = parser.parse_args()
    if not shutil.which("sandbox-exec"):
        parser.error("This acceptance test requires macOS sandbox-exec")
    codex, helper = args.codex.resolve(strict=True), args.copy_helper.resolve(strict=True)
    work = args.work_dir.resolve()
    if work.exists() and any(work.iterdir()):
        parser.error("--work-dir must be empty")
    work.mkdir(parents=True, exist_ok=True)
    (work / "fake-home").mkdir()
    policy = work / "sandbox.sb"
    # No network and no access to the user's home except this disposable test
    # directory and the compiled standalone helper's directory.
    policy.write_text(
        '(version 1)\n(allow default)\n(deny network*)\n'
        '(deny file-read-data file-write* (require-all (subpath ' + json.dumps(str(Path.home())) + ')'
        ' (require-not (subpath ' + json.dumps(str(work)) + '))'
        ' (require-not (subpath ' + json.dumps(str(helper.parent)) + '))))\n'
        '(deny file-read* file-write* (subpath ' + json.dumps(str(work / "forbidden")) + '))\n'
    )
    (work / "forbidden").mkdir()
    (work / "forbidden/probe").write_text("synthetic protected data")
    denied = subprocess.run(["sandbox-exec", "-f", str(policy), "/bin/cat", str(work / "forbidden/probe")], capture_output=True)
    assert denied.returncode != 0 and not denied.stdout
    denied = subprocess.run(["sandbox-exec", "-f", str(policy), "/usr/bin/touch", str(work / "forbidden/new-file")], capture_output=True)
    assert denied.returncode != 0 and not (work / "forbidden/new-file").exists()

    class ProbeHandler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"synthetic-loopback-probe")

        def log_message(self, *_):
            pass

    probe = ThreadingHTTPServer(("127.0.0.1", 0), ProbeHandler)
    threading.Thread(target=probe.serve_forever, daemon=True).start()
    try:
        command = ["/usr/bin/curl", "--silent", "--max-time", "2", f"http://127.0.0.1:{probe.server_port}"]
        control = subprocess.run(command, capture_output=True)
        assert control.returncode == 0 and control.stdout == b"synthetic-loopback-probe"
        denied = subprocess.run(["sandbox-exec", "-f", str(policy), *command], capture_output=True)
        assert denied.returncode != 0 and not denied.stdout
    finally:
        probe.shutdown()
        probe.server_close()
    source, target, baseline = work / "source", work / "target", work / "baseline"
    source.mkdir()
    (source / "config.toml").write_text(
        'model = "fixture-model"\nmodel_provider = "fixture"\n'
        '[model_providers.fixture]\nname = "Offline fixture"\n'
        'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
        'requires_openai_auth = false\nrequest_max_retries = 0\nstream_max_retries = 0\n'
        'stream_idle_timeout_ms = 1000\n[analytics]\nenabled = false\n'
    )

    def server(home, label):
        return Server(codex, home, work, policy, label)

    def copy(src, dst):
        return subprocess.run(["sandbox-exec", "-f", str(policy), str(helper), str(src), str(dst)], capture_output=True, text=True)

    def read(s, tid):
        return s.call("thread/read", {"threadId": tid, "includeTurns": True})["thread"]

    with server(source, "create") as s:
        project = s.call("project/create", {"name": "Synthetic no-folder project", "roots": [], "idempotencyKey": "fixture"})["project"]["id"]
        parent = s.call("thread/start", {"cwd": str(work), "historyMode": "paginated", "ephemeral": False, "projectId": project})["thread"]["id"]
        cancelled_turn(s, parent, "Synthetic parent marker")
        child = s.call("thread/fork", {"threadId": parent, "ephemeral": False})["thread"]["id"]
        cancelled_turn(s, child, "Synthetic child marker")

    state = {
        "local-projects": {"legacy-project": {"name": "Synthetic no-folder project", "rootPaths": []}},
        "thread-project-assignments": {child: {"projectId": "legacy-project", "projectKind": "local"}},
        "app-server-project-id-by-legacy-project-id-by-host": {"local:" + str(source): {"legacy-project": project}},
    }
    (source / ".codex-global-state.json").write_text(json.dumps(state))
    with sqlite3.connect(source / "state_5.sqlite") as db:
        db.execute("UPDATE threads SET project_id=NULL WHERE id=?", (child,))
    original = rollout_hashes(source)
    shutil.copytree(source, baseline)
    result = copy(source, target)
    assert result.returncode == 0, result.stderr
    assert rollout_hashes(source) == original == rollout_hashes(target)
    with server(source, "grow-source") as s:
        s.call("thread/resume", {"threadId": parent})
        cancelled_turn(s, parent, "SOURCE ONLY marker")
    assert rollout_hashes(target) == original

    report = {"sandbox_enabled": True, "sandbox_file_read_write_and_network_probes": "passed",
              "rollout_bytes_preserved": True, "desktop_originator": "Codex Desktop"}
    offline = work / "source-offline"
    source.rename(offline)
    try:
        for label, home in [("patched", target), ("baseline", baseline)]:
            with server(home, label) as s:
                try:
                    thread = read(s, child)
                    assert "SOURCE ONLY marker" not in user_texts(thread)
                    if label == "patched":
                        assert {"Synthetic parent marker", "Synthetic child marker"} <= set(user_texts(thread))
                        assert thread["projectId"] == project
                        assert Path(thread["path"]).is_relative_to(target)
                        pages = s.call("thread/turns/list", {"threadId": child, "itemsView": "full", "limit": 100})
                        assert {"Synthetic parent marker", "Synthetic child marker"} <= set(user_texts({"turns": pages["data"]}))
                        resumed = s.call("thread/resume", {"threadId": child})["thread"]
                        assert Path(resumed["path"]).is_relative_to(target)
                        listed = s.call("project/list", {})
                        assert next(p for p in listed["data"] if p["id"] == project)["roots"] == []
                        (work / "projects.json").write_text(json.dumps(listed, indent=2))
                    report[label] = {"read": True}
                except RuntimeError as error:
                    if label == "patched":
                        raise
                    report[label] = {"read": False, "error": str(error)}
        with server(target, "restart") as s:
            assert "Synthetic child marker" in user_texts(read(s, child))
        report["restart_read"] = True
    finally:
        offline.rename(source)

    broken, rejected = work / "broken", work / "rejected"
    result = copy(source, broken)
    assert result.returncode == 0, result.stderr
    with sqlite3.connect(broken / "state_5.sqlite") as db:
        child_path = Path(db.execute("SELECT rollout_path FROM threads WHERE id=?", (child,)).fetchone()[0])
    with child_path.open() as file:
        base = json.loads(file.readline())["payload"]["history_base"]
    ancestors = [p for p in broken.glob("sessions/**/*.jsonl") if p.stem.endswith(base["thread_id"])]
    assert len(ancestors) == 1
    ancestor = ancestors[0]
    raw = ancestor.read_bytes()
    cutoff = raw.rfind(b"\n", 0, base["end_byte_offset"] - 1) + 1
    assert 0 < cutoff < base["end_byte_offset"]
    ancestor.write_bytes(raw[:cutoff])
    with server(broken, "broken") as s:
        try:
            read(s, child)
            raise AssertionError("Broken lineage unexpectedly readable")
        except RuntimeError as error:
            assert "cutoff byte offset is past the source rollout" in str(error), error
            report["exact_cutoff_error"] = str(error)
    result = copy(broken, rejected)
    assert result.returncode != 0 and not rejected.exists()
    report["invalid_copy_rejected_before_publish"] = True
    (work / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
