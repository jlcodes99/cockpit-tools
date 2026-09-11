"""End-to-end actual Cockpit recovery code followed by network-denied native Codex reads."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess

from reproduce import Native, encode, seed
from test_incremental import append_turn, clone, snapshot


def main(binary, recovery):
    home, tid, path = seed(binary)
    damaged, rollout = clone(home, tid, path, "metadata-stall")
    last = json.loads(rollout.read_bytes().splitlines()[-1])
    duplicate = {"timestamp": last["timestamp"], "ordinal": last["ordinal"], "type": "event_msg",
                 "payload": {"type": "token_count", "info": None, "rate_limits": None}}
    with rollout.open("ab") as out:
        out.write(encode(duplicate))
    append_turn(rollout, "after-stall")
    stalled = snapshot(binary, damaged, tid, rollout)
    assert stalled["turns"] == 2 and stalled["cursor"][0] < stalled["bytes"]
    source_hash = hashlib.sha256(rollout.read_bytes()).hexdigest()
    before_db = (damaged / "thread_history_1.sqlite").read_bytes()
    result = json.loads(subprocess.check_output([str(recovery), str(damaged), tid, str(home / "candidates")]))
    candidate = Path(result["directory"])
    assert (damaged / "thread_history_1.sqlite").read_bytes() == before_db
    assert hashlib.sha256(rollout.read_bytes()).hexdigest() == source_hash
    assert (candidate / "rollout.jsonl").read_bytes() == rollout.read_bytes()
    # Make a new synthetic home; NEVER install the candidate over the source home.
    validated = candidate / "validation-home"
    (validated / "sessions").mkdir(parents=True)
    valid_rollout = validated / "sessions" / path.name
    shutil.copy2(candidate / "rollout.jsonl", valid_rollout)
    shutil.copy2(candidate / "projection.sqlite", validated / "thread_history_1.sqlite")
    with sqlite3.connect((damaged / "state_5.sqlite").as_uri() + "?mode=ro", uri=True) as src:
        with sqlite3.connect(validated / "state_5.sqlite") as out:
            src.backup(out)
            out.execute("UPDATE threads SET rollout_path=?,cwd=? WHERE id=?", (str(valid_rollout), str(validated), tid))
    states = [snapshot(binary, validated, tid, valid_rollout)]
    for i in range(3):
        append_turn(valid_rollout, "cursor-recovered-" + str(i))
        states.append(snapshot(binary, validated, tid, valid_rollout))
    assert [s["turns"] for s in states] == [3, 4, 5, 6], states
    assert all(s["bytes"] == s["cursor"][0] for s in states), states
    assert hashlib.sha256(rollout.read_bytes()).hexdigest() == source_hash
    report = {"stalled": stalled, "recovered": states, "source_unchanged": True, "candidate": str(candidate)}
    (home / "cursor-recovery-report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex-bin", type=Path, required=True)
    parser.add_argument("--recovery-bin", type=Path, required=True)
    args = parser.parse_args()
    main(args.codex_bin, args.recovery_bin)
