from contextlib import closing
import importlib.util
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("repair", Path(__file__).parents[1] / "scripts/repair_codex_local_projects.py")
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


class ProjectRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.home = Path(self.temp.name)
        self.root = str(self.home / "project")
        Path(self.root).mkdir()
        self.host = "local:" + str(self.home)
        self.state = {"local-projects": {}, "unrelated": {"preserve": True}}
        repair.atomic_json(self.home / repair.STATE, self.state)
        with closing(sqlite3.connect(self.home / "state_5.sqlite")) as db, db:
            db.executescript("""
                CREATE TABLE projects (id TEXT PRIMARY KEY,name TEXT,metadata TEXT,position INTEGER,created_at_ms INTEGER,updated_at_ms INTEGER);
                CREATE TABLE project_roots (project_id TEXT,position INTEGER,path TEXT);
                CREATE TABLE threads (id TEXT PRIMARY KEY,cwd TEXT,source TEXT,thread_source TEXT,project_id TEXT,has_user_event INTEGER,first_user_message TEXT,archived INTEGER);
            """)
            db.execute("INSERT INTO projects VALUES ('server-1','project','{}',0,10,20)")
            db.execute("INSERT INTO project_roots VALUES ('server-1',0,?)", (self.root,))
            db.execute("INSERT INTO threads VALUES ('user',?,'vscode','user',NULL,1,'hello',0)", (self.root,))
            db.execute("INSERT INTO threads VALUES ('child',?,'vscode','subagent',NULL,1,'hi',0)", (self.root,))
            db.execute("INSERT INTO threads VALUES ('guardian',?,'{\"subagent\":{\"other\":\"guardian\"}}','user',NULL,1,'hi',0)", (self.root,))
            db.execute("INSERT INTO threads VALUES ('archived',?,'vscode','user',NULL,1,'hi',1)", (self.root,))

    def tearDown(self):
        self.temp.cleanup()

    def plan(self):
        return repair.build_plan(self.home)

    def test_restore_projects_and_only_primary_threads(self):
        plan = self.plan()
        self.assertEqual(plan['summary']['nativeProjects'], 1)
        self.assertEqual({r['id'] for r in plan['threadPatches']}, {'user','archived'})
        self.assertEqual(plan['stateAfter']['unrelated'], self.state['unrelated'])

    def test_cloud_project_not_recreated(self):
        with closing(sqlite3.connect(self.home / "state_5.sqlite")) as db, db:
            db.execute("UPDATE projects SET metadata=?", (json.dumps({'codex.chatGptBacking':'other-account'}),))
        self.assertEqual(self.plan()['summary']['visibleProjectsAfter'], 0)

    def test_cloud_mirror_not_recreated_even_without_metadata(self):
        with closing(sqlite3.connect(self.home / "state_5.sqlite")) as db, db:
            db.execute("UPDATE project_roots SET path='C:\\home\\.codex\\.chatgpt-projects\\g-p-other'")
        self.assertEqual(repair.build_plan(self.home, exists=lambda p: True)['summary']['nativeProjects'], 0)

    def test_restore_legacy_identity_from_snapshot(self):
        old = {'local-projects': {'legacy': {'id':'legacy','name':'project','rootPaths':[self.root]}}, repair.MAP: {self.host: {'legacy':'server-1'}}}
        repair.atomic_json(self.home / (repair.STATE + '.bak.bak'), old)
        plan = self.plan()
        self.assertIn('legacy', plan['stateAfter']['local-projects'])
        self.assertEqual(plan['stateAfter'][repair.ASSIGN]['user']['projectId'], 'legacy')

    def test_respect_explicit_projectless(self):
        self.state['projectless-thread-ids'] = ['user']
        repair.atomic_json(self.home / repair.STATE, self.state)
        self.assertNotIn('user', self.plan()['stateAfter'][repair.ASSIGN])

    def test_honor_pending_project_deletion(self):
        self.state['app-server-pending-project-deletions-by-host'] = {self.host: {'legacy': {'projectId':'server-1'}}}
        repair.atomic_json(self.home / repair.STATE, self.state)
        self.assertEqual(self.plan()['summary']['nativeProjects'], 0)

    def test_dont_overwrite_newer_membership(self):
        with closing(sqlite3.connect(self.home / 'state_5.sqlite')) as db, db:
            db.execute("UPDATE threads SET project_id='different' WHERE id='user'")
        self.assertNotIn('user', self.plan()['stateAfter'][repair.ASSIGN])

    def test_apply_idempotent_archive_preserved_and_rollback(self):
        result = repair.apply_plan(self.plan(), self.home / 'backups', check_running=False)
        with closing(sqlite3.connect(self.home / 'state_5.sqlite')) as db, db:
            self.assertEqual(db.execute("SELECT project_id,archived FROM threads WHERE id='archived'").fetchone(), ('server-1',1))
            self.assertIsNone(db.execute("SELECT project_id FROM threads WHERE id='child'").fetchone()[0])
        again = self.plan()
        self.assertEqual(again['changedKeys'], [])
        self.assertEqual(again['threadPatches'], [])
        self.assertEqual(repair.apply_plan(again, self.home/'backups', check_running=False)['status'], 'no_changes')
        repair.rollback(Path(result['backup']), check_running=False)
        self.assertEqual(repair.load(self.home / repair.STATE), self.state)

    def test_concurrent_state_change_is_rejected(self):
        plan = self.plan()
        repair.atomic_json(self.home / repair.STATE, {'new':'data'})
        with self.assertRaisesRegex(RuntimeError, 'changed after audit'):
            repair.apply_plan(plan, self.home/'backups', check_running=False)

    def test_missing_root_not_recreated(self):
        self.assertEqual(repair.build_plan(self.home, exists=lambda p: False)['summary']['nativeProjects'], 0)

    def test_windows_path_normalization(self):
        self.assertEqual(repair.norm(r'\\?\E:\Folder\Project'), repair.norm('e:/folder/project/'))
        self.assertEqual(repair.norm(r'\\?\UNC\host\share\a'), repair.norm(r'\\host\share\a'))

    def test_old_projectless_survives_repeat_and_account_switch(self):
        old = {'local-projects': {'legacy': {'id':'legacy','name':'project','rootPaths':[self.root]}},
               repair.MAP: {self.host: {'legacy':'server-1'}}, 'projectless-thread-ids':['user']}
        repair.atomic_json(self.home / (repair.STATE + '.bak.bak'), old)
        repair.apply_plan(self.plan(), self.home/'backups', check_running=False)
        again = self.plan()
        self.assertNotIn('user', again['stateAfter'][repair.ASSIGN])
        self.assertEqual(again['changedKeys'], [])
        self.assertEqual(again['threadPatches'], [])
        self.assertNotIn('chatGptBacking', again['stateAfter']['local-projects']['legacy'])

    def test_catalog_updated_and_conditional_rollback_keeps_newer_chats(self):
        (self.home/'sqlite').mkdir()
        catalog = self.home/'sqlite/codex-dev.db'
        with closing(sqlite3.connect(catalog)) as db, db:
            db.executescript("CREATE TABLE local_thread_catalog (host_id TEXT,thread_id TEXT,project_id TEXT,cwd TEXT); CREATE TABLE local_thread_catalog_metadata (catalog_revision INTEGER); INSERT INTO local_thread_catalog_metadata VALUES (1);")
            db.execute("INSERT INTO local_thread_catalog VALUES ('local','user',NULL,?)", (self.root,))
        result = repair.apply_plan(self.plan(), self.home/'backups', check_running=False)
        with closing(sqlite3.connect(catalog)) as db, db:
            self.assertEqual(db.execute('SELECT project_id FROM local_thread_catalog').fetchone()[0], 'server-1')
        with closing(sqlite3.connect(self.home/'state_5.sqlite')) as db, db:
            db.execute("INSERT INTO threads VALUES ('new-chat',?,'vscode','user',NULL,1,'new',0)", (self.root,))
        repair.rollback(Path(result['backup']), check_running=False)
        with closing(sqlite3.connect(catalog)) as db, db:
            self.assertIsNone(db.execute('SELECT project_id FROM local_thread_catalog').fetchone()[0])
        with closing(sqlite3.connect(self.home/'state_5.sqlite')) as db, db:
            self.assertEqual(db.execute("SELECT count(*) FROM threads WHERE id='new-chat'").fetchone()[0], 1)


if __name__ == '__main__':
    unittest.main()
