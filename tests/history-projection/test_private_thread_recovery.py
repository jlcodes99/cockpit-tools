"""Explicitly authorized, local-only thread recovery experiment. Never applies to source.

All private artifacts stay in a new mode-0700 temporary directory. No source config,
credentials, other threads, or model RPCs are loaded into the experiment.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import sqlite3
import subprocess
import tempfile
import threading
import uuid

TABLES = ("thread_history_projection_state", "thread_turns", "thread_items", "thread_realtime_items")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def ro(path):
    db = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=5)
    db.execute("PRAGMA query_only=ON")
    return db


def read_stable(path):
    before = path.stat()
    data = path.read_bytes()
    after = path.stat()
    if (before.st_ino, before.st_size, before.st_mtime_ns) != (after.st_ino, after.st_size, after.st_mtime_ns):
        raise RuntimeError("Source changed during snapshot; retry from a fresh snapshot")
    return data


def top_ordinal_span(text):
    decoder = json.JSONDecoder()
    pos = len(text) - len(text.lstrip())
    if text[pos] != "{":
        raise ValueError("Record is not an object")
    pos += 1
    seen, result = set(), None
    while True:
        while text[pos].isspace():
            pos += 1
        if text[pos] == "}":
            break
        key, pos = decoder.raw_decode(text, pos)
        if key in seen:
            raise ValueError("Ambiguous top-level key")
        seen.add(key)
        while text[pos].isspace():
            pos += 1
        if text[pos] != ":":
            raise ValueError("Missing colon")
        pos += 1
        while text[pos].isspace():
            pos += 1
        start = pos
        value, pos = decoder.raw_decode(text, pos)
        if key == "ordinal":
            if type(value) is not int or value < 0:
                raise ValueError("Unsupported ordinal")
            result = (start, pos)
        while text[pos].isspace():
            pos += 1
        if text[pos] == "}":
            break
        if text[pos] != ",":
            raise ValueError("Missing comma")
        pos += 1
    if result is None:
        raise ValueError("No ordinal")
    return result


def normalize(data):
    output, mapping = [], []
    offset, new_offset = 0, 0
    original_payload, candidate_payload = hashlib.sha256(), hashlib.sha256()
    records = data.splitlines(keepends=True)
    first = json.loads(records[0])
    if first["payload"].get("history_mode") != "paginated" or first["ordinal"] != 0:
        raise ValueError("Only non-lineage paginated history starting at zero is supported")
    if first["payload"].get("history_base") or first["payload"].get("subagent_history_start_ordinal") is not None:
        raise ValueError("Inherited/lineage history is outside this experiment")
    previous = -1
    for i, line in enumerate(records):
        if not line.endswith(b"\n"):
            raise ValueError("Partial record")
        text = line.decode()
        start, end = top_ordinal_span(text)
        record = json.loads(text)
        candidate = (text[:start] + str(i) + text[end:]).encode()
        ctext = candidate.decode()
        cstart, cend = top_ordinal_span(ctext)
        # Exact byte preservation outside the top-level ordinal token, not JSON equivalence.
        untouched = (text[:start] + text[end:]).encode()
        assert untouched == (ctext[:cstart] + ctext[cend:]).encode()
        original_payload.update(untouched)
        candidate_payload.update((ctext[:cstart] + ctext[cend:]).encode())
        mapping.append({"line": i + 1, "old_ordinal": record["ordinal"], "new_ordinal": i,
                        "old_offset": offset, "new_offset": new_offset,
                        "duplicate_or_backwards": record["ordinal"] <= previous,
                        "type": record["type"], "payload_type": record.get("payload", {}).get("type")})
        previous = record["ordinal"]
        output.append(candidate)
        offset += len(line)
        new_offset += len(candidate)
    assert original_payload.digest() == candidate_payload.digest()
    return b"".join(output), mapping, original_payload.hexdigest()


class OfflineServer:
    def __init__(self, binary, home):
        self.binary, self.home = binary.resolve(), home.resolve()
        self.home.mkdir(parents=True, exist_ok=True)
        protected = json.dumps(str(Path.home()))
        self.profile = "\n".join([
            "(version 1)(allow default)(deny network*)(deny signal)",
            f"(deny file-read* file-write* (subpath {protected}))",
            "(deny process-exec)", f"(allow process-exec (literal {json.dumps(str(self.binary))}))",
        ])
        self.queue = queue.Queue()
        self.err = (home / "app-server.stderr").open("a")
        env = {"HOME": str(home), "CODEX_HOME": str(home), "PATH": "/usr/bin:/bin",
               "RUST_LOG": "warn", "TMPDIR": str(home)}
        self.proc = subprocess.Popen(["/usr/bin/sandbox-exec", "-p", self.profile,
                                      str(self.binary), "app-server", "--stdio"],
                                     cwd=home, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=self.err, text=True)
        self.counter = 0
        def reader():
            for line in self.proc.stdout:
                try:
                    self.queue.put(json.loads(line))
                except ValueError:
                    pass
            self.queue.put({"closed": True})
        threading.Thread(target=reader, daemon=True).start()

    def rpc(self, method, params):
        if method not in {"initialize", "thread/resume", "thread/turns/list", "thread/items/list"}:
            raise ValueError("Non-history RPC is forbidden")
        self.counter += 1
        self.proc.stdin.write(json.dumps({"id": self.counter, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        while True:
            msg = self.queue.get(timeout=120)
            if msg.get("closed"):
                raise RuntimeError("Isolated app-server exited")
            if msg.get("id") == self.counter:
                if "error" in msg:
                    # Do not surface private model input in error text.
                    raise RuntimeError(f"History RPC {method} failed with code {msg['error'].get('code')}")
                return msg["result"]

    def __enter__(self):
        try:
            self.rpc("initialize", {"clientInfo": {"name": "offline-history-verification", "version": "1"},
                                    "capabilities": {"experimentalApi": True}})
            self.proc.stdin.write('{"method":"initialized"}\n')
            self.proc.stdin.flush()
        except Exception:
            self.__exit__(None, None, None)
            raise
        return self

    def __exit__(self, *args):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.err.close()


def db_rows(home, tid):
    with ro(home / "thread_history_1.sqlite") as db:
        db.execute("BEGIN")
        return {table: {"columns": [r[1] for r in db.execute(f"PRAGMA table_info({table})")],
                        "rows": db.execute(f"SELECT * FROM {table} WHERE thread_id=?", (tid,)).fetchall()}
                for table in TABLES}


def install_rows(home, rows):
    with sqlite3.connect(home / "thread_history_1.sqlite") as db:
        for table, data in rows.items():
            cols = data["columns"]
            for row in data["rows"]:
                db.execute(f"INSERT INTO {table} ({','.join(cols)}) VALUES ({','.join('?' for _ in cols)})", row)


def make_home(root, label, binary, tid, raw, state, schema, projection=None):
    home = root / label
    with OfflineServer(binary, home):
        pass
    (home / "sessions").mkdir(exist_ok=True)
    rollout = home / "sessions" / f"rollout-2026-09-04T23-45-21-{tid}.jsonl"
    rollout.write_bytes(raw)
    with sqlite3.connect(home / "state_5.sqlite") as db:
        cols = {r[1] for r in db.execute("PRAGMA table_info(threads)")}
        values = {k: v for k, v in state.items() if k in cols}
        values.update(rollout_path=str(rollout), cwd=str(home), model_provider="openai", thread_source="user")
        keys = list(values)
        db.execute(f"INSERT INTO threads ({','.join(keys)}) VALUES ({','.join('?' for _ in keys)})", list(values.values()))
    with sqlite3.connect(home / "thread_history_1.sqlite") as db:
        for statement in schema[0]:
            db.execute(statement)
        for row in schema[1]:
            db.execute("INSERT INTO _sqlx_migrations VALUES (" + ",".join("?" for _ in row) + ")", row)
    if projection:
        install_rows(home, projection)
    return home, rollout


def read_history(binary, home, tid):
    with OfflineServer(binary, home) as server:
        server.rpc("thread/resume", {"threadId": tid, "cwd": str(home), "modelProvider": "openai", "excludeTurns": True})
        result, cursor, seen = [], None, set()
        for _ in range(100):
            params = {"threadId": tid, "limit": 100, "sortDirection": "asc", "itemsView": "summary"}
            if cursor:
                params["cursor"] = cursor
            page = server.rpc("thread/turns/list", params)
            result.extend(page["data"])
            cursor = page.get("nextCursor")
            if not cursor:
                break
            if cursor in seen:
                raise RuntimeError("Pagination did not advance")
            seen.add(cursor)
        else:
            raise RuntimeError("Pagination exceeded bound")
    rows = db_rows(home, tid)
    state = rows["thread_history_projection_state"]["rows"]
    return {"visible_turns": len(result), "projected_turns": len(rows["thread_turns"]["rows"]),
            "projected_items": len(rows["thread_items"]["rows"]),
            "cursor": list(state[0][1:]) if state else None}, rows


def compare_history(before, after):
    def extract(table, data):
        cols = data["columns"]
        result = {}
        for row in data["rows"]:
            item = dict(zip(cols, row))
            key = (item.get("turn_id"), item.get("item_id"))
            # Position fields legitimately change after ordinal calibration.
            result[key] = {k: v for k, v in item.items() if "ordinal" not in k and "byte_offset" not in k}
        return result
    summary = {}
    for table in ("thread_turns", "thread_items", "thread_realtime_items"):
        old, new = extract(table, before[table]), extract(table, after[table])
        missing = sum(k not in new for k in old)
        changed = sum(k in new and v != new[k] for k, v in old.items())
        def ordered_keys(data):
            rows = [dict(zip(data["columns"], row)) for row in data["rows"]]
            return [(row.get("turn_id"), row.get("item_id")) for row in sorted(rows, key=lambda r: r["rollout_ordinal"])]
        old_order = ordered_keys(before[table])
        new_order = [key for key in ordered_keys(after[table]) if key in old]
        summary[table] = {"before": len(old), "after": len(new), "missing": missing, "changed": changed,
                          "original_order_preserved": old_order == new_order}
    return summary


def append_local_turn(path, iteration):
    lines = path.read_bytes().splitlines()
    ordinal = json.loads(lines[-1])["ordinal"] + 1
    thread_id = json.loads(lines[0])["payload"]["id"]
    turn_id = str(uuid.uuid4())
    # Shapes verified against native migrate-rollouts output; no captured tool calls replayed.
    payloads = [
        {"type": "task_started", "turn_id": turn_id, "model_context_window": 10000, "collaboration_mode_kind": "default"},
        {"type": "item_completed", "thread_id": thread_id, "turn_id": turn_id, "completed_at_ms": 1788955200000,
         "item": {"type": "UserMessage", "id": f"offline-{iteration}-user", "content": [{"type": "text", "text": "offline append probe", "text_elements": []}]}},
        {"type": "item_completed", "thread_id": thread_id, "turn_id": turn_id, "completed_at_ms": 1788955200000,
         "item": {"type": "AgentMessage", "id": f"offline-{iteration}-agent", "content": [{"type": "Text", "text": "offline append response"}]}},
        {"type": "task_complete", "turn_id": turn_id, "last_agent_message": "offline append response"},
    ]
    added = [(json.dumps({"type": "event_msg", "ordinal": ordinal + i, "timestamp": "2026-09-09T12:00:00Z", "payload": payload}, separators=(",", ":")) + "\n").encode()
             for i, payload in enumerate(payloads)]
    with path.open("ab") as out:
        out.write(b"".join(added))


def main(args):
    root = Path(tempfile.mkdtemp(prefix="cockpit-private-multiboundary-", dir="/private/tmp"))
    report = {"experiment_root": str(root), "original_never_modified": True}
    print("Private experiment directory:", root, flush=True)
    with ro(args.source_home / "state_5.sqlite") as db:
        db.row_factory = sqlite3.Row
        state = dict(db.execute("SELECT * FROM threads WHERE id=?", (args.thread_id,)).fetchone())
    source = Path(state["rollout_path"])
    raw = read_stable(source)
    original = db_rows(args.source_home, args.thread_id)
    with ro(args.source_home / "thread_history_1.sqlite") as db:
        db.execute("BEGIN")
        schema = ([row[0] for row in db.execute("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 ELSE 2 END")],
                  db.execute("SELECT * FROM _sqlx_migrations").fetchall())
    assert sha(read_stable(source)) == sha(raw), "Source changed while capturing projection"
    (root / "original-rollout.jsonl").write_bytes(raw)
    (root / "original-projection.json").write_text(json.dumps(original))
    normalized, mapping, body_hash = normalize(raw)
    (root / "normalized-rollout.jsonl").write_bytes(normalized)
    (root / "ordinal-map.json").write_text(json.dumps(mapping))
    report.update(source_sha256=sha(raw), normalized_sha256=sha(normalized),
                  non_ordinal_bytes_sha256=body_hash, source_bytes=len(raw), normalized_bytes=len(normalized),
                  records=len(mapping), repeated_boundaries=[m for m in mapping if m["duplicate_or_backwards"]])
    report["baseline"] = {"turns": len(original["thread_turns"]["rows"]), "items": len(original["thread_items"]["rows"]),
                          "cursor": original["thread_history_projection_state"]["rows"][0][1:]}
    def save():
        (root / "verification-report.json").write_text(json.dumps(report, indent=2))
    save()
    for label, data, projection in [("original-control", raw, original), ("normalized-stale-index", normalized, original), ("normalized-rebuild", normalized, None)]:
        home, rollout = make_home(root, label, args.codex_bin, args.thread_id, data, state, schema, projection)
        try:
            result, rows = read_history(args.codex_bin, home, args.thread_id)
            result["file_bytes"] = rollout.stat().st_size
            result["history_comparison"] = compare_history(original, rows)
            report[label] = result
            print(label, json.dumps({k: v for k, v in result.items() if k != "history_comparison"}), flush=True)
        except Exception as exc:
            report[label] = {"error": type(exc).__name__ + ": " + str(exc)}
            save()
            if label == "normalized-rebuild":
                raise
            continue
        save()
        if label == "normalized-rebuild":
            assert all(x["missing"] == 0 and x["changed"] == 0 and x["original_order_preserved"] for x in result["history_comparison"].values()), "Rebuilt history differs; do not apply"
            assert result["cursor"][0] == result["file_bytes"], "Rebuilt projection did not reach EOF"
            assert rollout.read_bytes() == normalized, "Native read modified recovered rollout"
            baseline = result["visible_turns"]
            report["append_restart_cycles"] = []
            for iteration in range(5):
                append_local_turn(rollout, iteration)
                result, latest = read_history(args.codex_bin, home, args.thread_id)
                result["file_bytes"] = rollout.stat().st_size
                assert result["visible_turns"] == baseline + iteration + 1, "Appended turn not visible"
                assert result["cursor"][0] == result["file_bytes"], "Append checkpoint behind EOF"
                assert all(x["missing"] == 0 and x["changed"] == 0 and x["original_order_preserved"]
                           for x in compare_history(rows, latest).values()), "Append changed existing history"
                report["append_restart_cycles"].append(result)
                print("append_restart", iteration + 1, json.dumps(result), flush=True)
                save()
            report["reopen_only_cycles"] = []
            for _ in range(3):
                unchanged = sha(rollout.read_bytes())
                reopened, _ = read_history(args.codex_bin, home, args.thread_id)
                assert reopened["visible_turns"] == baseline + 5
                assert reopened["cursor"][0] == rollout.stat().st_size
                assert sha(rollout.read_bytes()) == unchanged
                report["reopen_only_cycles"].append(reopened)
            report["continuous_ordinals"] = all(json.loads(line)["ordinal"] == i for i, line in enumerate(rollout.read_bytes().splitlines()))
            assert report["continuous_ordinals"]
            report["payload_and_identity_preserved"] = True
    report["source_hash_still_matches"] = sha(read_stable(source)) == sha(raw)
    report["verified"] = True
    save()
    print("PASS: private local experiment; report:", root / "verification-report.json")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-home", required=True, type=Path)
    parser.add_argument("--thread-id", required=True)
    parser.add_argument("--codex-bin", required=True, type=Path)
    main(parser.parse_args())
