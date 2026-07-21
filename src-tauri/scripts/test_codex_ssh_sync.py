import hashlib
import importlib.util
import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("codex_ssh_sync.py")
SPEC = importlib.util.spec_from_file_location("codex_ssh_sync", SCRIPT_PATH)
SYNC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SYNC)


class CodexSshSyncTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cps-ssh-sync-")
        self.root = Path(self.temp.name)
        self.staging = self.root / ".cps-codex-sync" / "staging" / "test-run"
        self.staging.mkdir(parents=True)
        self.bundle = {
            "auth.json": b'{"tokens":{"access_token":"new"}}\n',
            "config.toml": b'model_provider = "openai"\n',
            ".cockpit_codex_auth.json": b'{"account_id":"new"}\n',
        }
        manifest = {"version": 1, "run_id": "test-run", "files": []}
        for name, content in self.bundle.items():
            (self.staging / name).write_bytes(content)
            manifest["files"].append({
                "relative_path": name,
                "mode": 0o600,
                "sha256": hashlib.sha256(content).hexdigest(),
            })
        (self.staging / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        for name in self.bundle:
            (self.root / name).write_text("old-" + name, encoding="utf-8")
        rollout_dir = self.root / "sessions" / "2026" / "07" / "21"
        rollout_dir.mkdir(parents=True)
        self.rollout = rollout_dir / "rollout-thread-1.jsonl"
        self.rollout.write_text(
            '{"timestamp":"2026-07-21T01:00:00Z","type":"session_meta","payload":{"id":"thread-1","timestamp":"2026-07-21T01:00:00Z","model_provider":"old","cwd":"/repo","source":"vscode","cli_version":"0.145.0"}}\n'
            '{"type":"event_msg","payload":{"type":"user_message","message":"hello"}}\n',
            encoding="utf-8",
        )
        active_dir = self.root / "sessions" / "2026" / "07" / "22"
        active_dir.mkdir(parents=True)
        self.active_orphan = active_dir / "rollout-active-orphan.jsonl"
        self.active_orphan.write_text(
            '{"timestamp":"2026-07-22T02:00:00Z","type":"session_meta","payload":{"id":"active-orphan","timestamp":"2026-07-22T02:00:00Z","model_provider":"old","cwd":"/active-repo","source":"vscode","cli_version":"0.145.0","git":{"commit_hash":"abc123","branch":"main","repository_url":"https://example.test/repo.git"}}}\n'
            '{"timestamp":"2026-07-22T02:01:00Z","type":"turn_context","payload":{"approval_policy":"on-request","sandbox_policy":{"type":"workspace-write"},"model":"gpt-test","effort":"high"}}\n'
            '{"timestamp":"2026-07-22T02:02:00Z","type":"event_msg","payload":{"type":"user_message","message":"restore active history"}}\n',
            encoding="utf-8",
        )
        archived_dir = self.root / "archived_sessions" / "2026" / "07" / "20"
        archived_dir.mkdir(parents=True)
        self.orphan = archived_dir / "rollout-orphan.jsonl"
        self.orphan.write_text(
            '{"timestamp":"2026-07-20T01:00:00Z","type":"session_meta","payload":{"session_id":"orphan","timestamp":"2026-07-20T01:00:00Z","model_provider":"old","cwd":"/archived-repo","source":"cli"}}\n'
            '{"timestamp":"2026-07-20T01:01:00Z","type":"event_msg","payload":{"type":"user_message","message":"restore archived history"}}\n',
            encoding="utf-8",
        )
        self.db_path = self.root / "state_5.sqlite"
        connection = sqlite3.connect(self.db_path)
        connection.executescript(
            """
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT,
                model_provider TEXT,
                first_user_message TEXT,
                has_user_event INTEGER,
                thread_source TEXT,
                cwd TEXT,
                archived INTEGER
            );
            INSERT INTO threads VALUES
                ('thread-1', '/missing/rollout.jsonl', 'old', '', 0, '', '', 0);
            """
        )
        connection.commit()
        connection.close()

    def tearDown(self):
        os.environ.pop("COCKPIT_SSH_SYNC_TEST_FAIL_STAGE", None)
        os.environ.pop("COCKPIT_SSH_SYNC_TEST_MUTATE_ROLLOUT", None)
        self.temp.cleanup()

    def run_sync(self):
        result = SYNC.empty_result()
        try:
            SYNC.run(self.root.resolve(), self.staging.resolve(), "openai", result)
        except Exception as error:
            result["success"] = False
            result["error"] = str(error)
        return result

    def test_repairs_existing_thread_and_recovers_active_and_archived_orphans(self):
        result = self.run_sync()
        self.assertTrue(result["success"], result)
        self.assertEqual(result["rollout_paths_repaired"], 1)
        self.assertEqual(result["user_events_recovered"], 1)
        self.assertEqual(result["cwd_rows_repaired"], 1)
        self.assertEqual(result["orphan_rollouts_found"], 2)
        self.assertEqual(result["orphan_threads_recovered"], 2)
        self.assertEqual(result["provider_rows_remaining"], 0)
        self.assertEqual(result["visibility_rows_remaining"], 0)
        self.assertEqual(result["rollout_files_remaining"], 0)
        self.assertEqual(result["quick_check"], "ok")
        self.assertIn('"model_provider":"openai"', self.rollout.read_text(encoding="utf-8"))
        connection = sqlite3.connect(self.db_path)
        row = connection.execute(
            "SELECT rollout_path, model_provider, has_user_event, thread_source, cwd, archived, "
            "first_user_message "
            "FROM threads WHERE id = 'thread-1'"
        ).fetchone()
        active = connection.execute(
            "SELECT model_provider, has_user_event, thread_source, cwd, archived, first_user_message "
            "FROM threads WHERE id = 'active-orphan'"
        ).fetchone()
        archived = connection.execute(
            "SELECT model_provider, has_user_event, thread_source, cwd, archived, first_user_message "
            "FROM threads WHERE id = 'orphan'"
        ).fetchone()
        connection.close()
        self.assertEqual(
            row,
            (str(self.rollout.resolve()), "openai", 1, "user", "/repo", 0, "hello"),
        )
        self.assertEqual(
            active,
            ("openai", 1, "user", "/active-repo", 0, "restore active history"),
        )
        self.assertEqual(
            archived,
            ("openai", 1, "user", "/archived-repo", 1, "restore archived history"),
        )
        backup = Path(result["backup_path"])
        backup_manifest = json.loads((backup / "manifest.json").read_text(encoding="utf-8"))
        for record in backup_manifest["files"]:
            if not record["existed"]:
                continue
            if record["kind"] == "bundle":
                path = backup / "bundle" / record["relative_path"]
            elif record["kind"] == "rollout":
                path = backup / "rollouts" / record["relative_path"]
            else:
                path = backup / "state_5.sqlite"
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), record["sha256"])

    def test_bundle_failure_restores_bundle_rollouts_and_database(self):
        originals = {name: (self.root / name).read_bytes() for name in self.bundle}
        original_rollout = self.rollout.read_bytes()
        os.environ["COCKPIT_SSH_SYNC_TEST_FAIL_STAGE"] = "bundle"
        result = self.run_sync()
        self.assertFalse(result["success"])
        self.assertTrue(result["rollback_performed"])
        self.assertTrue(result["rollback_verified"])
        for name, content in originals.items():
            self.assertEqual((self.root / name).read_bytes(), content)
        self.assertEqual(self.rollout.read_bytes(), original_rollout)

    def test_database_failure_restores_bundle_rollout_and_rows(self):
        original_rollout = self.rollout.read_bytes()
        os.environ["COCKPIT_SSH_SYNC_TEST_FAIL_STAGE"] = "database"
        result = self.run_sync()
        self.assertFalse(result["success"])
        self.assertTrue(result["rollback_verified"])
        self.assertEqual(self.rollout.read_bytes(), original_rollout)
        connection = sqlite3.connect(self.db_path)
        row = connection.execute(
            "SELECT model_provider, has_user_event, rollout_path FROM threads WHERE id = 'thread-1'"
        ).fetchone()
        orphan_count = connection.execute(
            "SELECT COUNT(*) FROM threads WHERE id IN ('active-orphan', 'orphan')"
        ).fetchone()[0]
        connection.close()
        self.assertEqual(row, ("old", 0, "/missing/rollout.jsonl"))
        self.assertEqual(orphan_count, 0)

    def test_response_item_user_message_fallback_skips_synthetic_context(self):
        rollout = self.root / "sessions" / "2026" / "07" / "23" / "rollout-response-item.jsonl"
        rollout.parent.mkdir(parents=True)
        rollout.write_text(
            '{"type":"session_meta","payload":{"id":"response-item","model_provider":"old","cwd":"/fallback"}}\n'
            '{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>synthetic</environment_context>"}]}}\n'
            '{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"real fallback message"}]}}\n',
            encoding="utf-8",
        )
        result = self.run_sync()
        self.assertTrue(result["success"], result)
        connection = sqlite3.connect(self.db_path)
        row = connection.execute(
            "SELECT has_user_event, first_user_message FROM threads WHERE id = 'response-item'"
        ).fetchone()
        connection.close()
        self.assertEqual(row, (1, "real fallback message"))

    def test_unknown_required_schema_column_aborts_before_mutation(self):
        connection = sqlite3.connect(self.db_path)
        connection.executescript(
            """
            DROP TABLE threads;
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                has_user_event INTEGER NOT NULL,
                future_required TEXT NOT NULL
            );
            """
        )
        connection.close()
        originals = {name: (self.root / name).read_bytes() for name in self.bundle}
        result = self.run_sync()
        self.assertFalse(result["success"])
        self.assertIn("unknown required columns: future_required", result["error"])
        self.assertFalse(result["rollback_performed"])
        for name, content in originals.items():
            self.assertEqual((self.root / name).read_bytes(), content)

    def test_repeated_rollout_change_aborts_before_mutation(self):
        originals = {name: (self.root / name).read_bytes() for name in self.bundle}
        os.environ["COCKPIT_SSH_SYNC_TEST_MUTATE_ROLLOUT"] = "always"
        result = self.run_sync()
        self.assertFalse(result["success"])
        self.assertIn("changed during planning twice", result["error"])
        self.assertFalse(result["rollback_performed"])
        for name, content in originals.items():
            self.assertEqual((self.root / name).read_bytes(), content)


if __name__ == "__main__":
    unittest.main()
