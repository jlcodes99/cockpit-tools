"""Synthetic, network-denied recovery/append experiments, never a live repair tool."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import unittest
import uuid

from reproduce import Native, encode, seed


def digest_without_ordinals(data):
    records = [json.loads(line) for line in data.splitlines()]
    for record in records:
        record.pop("ordinal", None)
    return hashlib.sha256(json.dumps(records, sort_keys=True).encode()).hexdigest()


def clone(home, tid, path, label):
    target = home.with_name(home.name + "-" + label)
    shutil.copytree(home, target)
    rollout = target / "sessions" / path.name
    with sqlite3.connect(target / "state_5.sqlite") as db:
        db.execute("UPDATE threads SET rollout_path=?,cwd=? WHERE id=?",
                   (str(rollout), str(target), tid))
    return target, rollout


def append_turn(path, label):
    records = [json.loads(line) for line in path.read_bytes().splitlines()]
    # Reuse native-migrated event shapes, not a guessed app-server wire format.
    items = json.loads(json.dumps(records[1:5]))
    tid = str(uuid.uuid4())
    ordinal = records[-1]["ordinal"] + 1
    for i, item in enumerate(items):
        item["ordinal"] = ordinal + i
        item["payload"]["turn_id"] = tid
        if "item" in item["payload"]:
            item["payload"]["item"]["id"] += "-" + label
    with path.open("ab") as out:
        out.write(b"".join(map(encode, items)))


def snapshot(binary, home, tid, path):
    result = Native(binary, home).read(tid, resume=True)
    if "error" in result:
        raise AssertionError(result["error"])
    with sqlite3.connect((home / "thread_history_1.sqlite").as_uri() + "?mode=ro", uri=True) as db:
        cursor = db.execute("SELECT next_rollout_byte_offset,next_rollout_ordinal "
                            "FROM thread_history_projection_state WHERE thread_id=?", (tid,)).fetchone()
    return {"turns": len(result["result"]["data"]), "cursor": cursor,
            "bytes": path.stat().st_size}


def synthetic_rebuild(home, path):
    # This experiment intentionally refuses all non-fixture directories. It is not
    # exported through Cockpit and must not be used on a real CODEX_HOME.
    if (home / ".synthetic-history-fixture").read_text() != "synthetic-only-v1\n":
        raise ValueError("Not a synthetic fixture")
    before = path.read_bytes()
    records = [json.loads(line) for line in before.splitlines()]
    for i, record in enumerate(records):
        record["ordinal"] = i
    after = b"".join(map(encode, records))
    assert digest_without_ordinals(before) == digest_without_ordinals(after)
    path.write_bytes(after)
    # Keep the original database intact for comparison. All processes have exited.
    database = home / "thread_history_1.sqlite"
    database.rename(home / "projection-before-rebuild.sqlite")
    for suffix in ("-wal", "-shm"):
        sidecar = home / (database.name + suffix)
        if sidecar.exists():
            sidecar.rename(home / ("projection-before-rebuild.sqlite" + suffix))


def main(binary):
    home, tid, path = seed(binary)
    report = {}
    normal, normal_path = clone(home, tid, path, "append-control")
    states = [snapshot(binary, normal, tid, normal_path)]
    for i in range(2):
        append_turn(normal_path, "control-" + str(i))
        states.append(snapshot(binary, normal, tid, normal_path))
    assert [s["turns"] for s in states] == [2, 3, 4], states
    report["normal_append_restart"] = states

    damaged, damaged_path = clone(home, tid, path, "append-duplicate")
    duplicate = json.loads(damaged_path.read_bytes().splitlines()[-1])
    with damaged_path.open("ab") as out:
        out.write(encode(duplicate))
    append_turn(damaged_path, "behind-duplicate")
    states = [snapshot(binary, damaged, tid, damaged_path) for _ in range(2)]
    assert [s["turns"] for s in states] == [2, 2], states
    report["duplicate_stays_stalled_after_restart"] = states

    repaired, repaired_path = clone(damaged, tid, damaged_path, "recovered")
    synthetic_rebuild(repaired, repaired_path)
    states = [snapshot(binary, repaired, tid, repaired_path)]
    for i in range(3):
        append_turn(repaired_path, "recovered-" + str(i))
        states.append(snapshot(binary, repaired, tid, repaired_path))
    assert [s["turns"] for s in states] == [3, 4, 5, 6], states
    assert all(b["cursor"][0] > a["cursor"][0] for a, b in zip(states, states[1:])), states
    report["reindexed_append_restart"] = states
    (home / "incremental-report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    print("PASS: offline append/restart regression; artifacts:", home)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex-bin", required=True, type=Path)
    main(parser.parse_args().codex_bin)
