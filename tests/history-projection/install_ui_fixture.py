"""Install an allowlisted synthetic fixture into a fresh Codex home, not Cockpit storage."""
import argparse
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile


def install(source: Path, destination: Path):
    source = source.resolve()
    destination = destination.resolve()
    if (source / ".synthetic-history-fixture").read_text() != "synthetic-only-v1\n":
        raise ValueError("Synthetic fixture marker is required")
    if not destination.is_relative_to(Path(tempfile.gettempdir()).resolve()) and not destination.is_relative_to(Path("/private/tmp")):
        raise ValueError("Destination must be temporary")
    destination.mkdir(parents=True, exist_ok=True)
    if list(destination.iterdir()):
        raise ValueError("Destination must be empty; existing data is never overwritten")
    with sqlite3.connect((source / "state_5.sqlite").as_uri() + "?mode=ro", uri=True) as db:
        rows = db.execute("SELECT id,rollout_path FROM threads").fetchall()
        if len(rows) != 1:
            raise ValueError("Expected exactly one synthetic thread")
        tid, old_path = rows[0]
        old_path = Path(old_path).resolve()
        relative = old_path.relative_to(source)
        if relative.parts[0] != "sessions":
            raise ValueError("Source rollout is outside sessions")
        records = [json.loads(line) for line in old_path.read_bytes().splitlines()]
        if records[0]["payload"]["id"] != tid:
            raise ValueError("Fixture identity mismatch")
        rollout = destination / relative
        rollout.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(old_path, rollout)
        with sqlite3.connect(destination / "state_5.sqlite") as out:
            db.backup(out)
            out.execute("UPDATE threads SET rollout_path=?,cwd=?,has_user_event=1,"
                        "first_user_message='Synthetic question 0',thread_source='user' WHERE id=?",
                        (str(rollout), str(destination), tid))
    with sqlite3.connect((source / "thread_history_1.sqlite").as_uri() + "?mode=ro", uri=True) as db:
        with sqlite3.connect(destination / "thread_history_1.sqlite") as out:
            db.backup(out)
    # The legacy session list reads this index independently of the projection DB.
    (destination / "session_index.jsonl").write_text(json.dumps({
        "id": tid, "thread_name": "Synthetic history - provider preview",
        "updated_at": "2026-09-01T00:00:01Z",
    }) + "\n")
    (destination / ".synthetic-history-fixture").write_text("synthetic-only-v1\n")
    return {"codex_home": str(destination), "thread_id": tid, "rollout": str(rollout)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", type=Path)
    args = parser.parse_args()
    dest = args.destination or Path(tempfile.mkdtemp(prefix="cockpit-preview-codex-"))
    print(json.dumps(install(args.source, dest), indent=2))
