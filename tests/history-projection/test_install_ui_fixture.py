import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

from install_ui_fixture import install


class InstallationTests(unittest.TestCase):
    def test_install_relocates_index_and_preserves_body(self):
        with tempfile.TemporaryDirectory(prefix="cockpit-fixture-installer-") as tmp:
            root = Path(tmp)
            source, target = root / "source", root / "codex-home"
            (source / "sessions").mkdir(parents=True)
            (source / ".synthetic-history-fixture").write_text("synthetic-only-v1\n")
            body = b'{"type":"session_meta","payload":{"id":"synthetic"}}\n'
            rollout = source / "sessions/rollout-synthetic.jsonl"
            rollout.write_bytes(body)
            with sqlite3.connect(source / "state_5.sqlite") as db:
                db.execute("CREATE TABLE threads(id TEXT,rollout_path TEXT,cwd TEXT,has_user_event INTEGER,first_user_message TEXT,thread_source TEXT)")
                db.execute("INSERT INTO threads VALUES ('synthetic',?,'old',0,'','')", (str(rollout),))
            with sqlite3.connect(source / "thread_history_1.sqlite") as db:
                db.execute("CREATE TABLE projection_probe(value INTEGER)")
                db.execute("INSERT INTO projection_probe VALUES (1)")
            report = install(source, target)
            self.assertEqual(Path(report["rollout"]).read_bytes(), body)
            with sqlite3.connect(target / "state_5.sqlite") as db:
                record = db.execute("SELECT rollout_path,has_user_event,first_user_message,thread_source FROM threads").fetchone()
            self.assertTrue(Path(record[0]).is_relative_to(target.resolve()))
            self.assertEqual(record[1:], (1, "Synthetic question 0", "user"))
            self.assertEqual(json.loads((target / "session_index.jsonl").read_text())["id"], "synthetic")
            with self.assertRaisesRegex(ValueError, "empty"):
                install(source, target)

    def test_real_source_without_marker_is_refused(self):
        with tempfile.TemporaryDirectory(prefix="cockpit-fixture-installer-") as tmp:
            root = Path(tmp)
            with self.assertRaises(FileNotFoundError):
                install(root, root / "target")
            self.assertFalse((root / "target").exists())


if __name__ == "__main__":
    unittest.main()
