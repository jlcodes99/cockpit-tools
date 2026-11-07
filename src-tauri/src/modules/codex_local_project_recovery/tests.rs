use super::*;

struct Fixture {
    temp: tempfile::TempDir,
    root: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Project");
        fs::create_dir(&root).unwrap();
        let f = Self {
            temp,
            root: root.to_string_lossy().into_owned(),
        };
        atomic_value(&f.home().join(STATE), &json!({"unrelated":{"keep":true}})).unwrap();
        let db = f.db();
        db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT,metadata TEXT,created_at_ms INTEGER,updated_at_ms INTEGER);
            CREATE TABLE project_roots(project_id TEXT,path TEXT,position INTEGER);
            CREATE TABLE threads(id TEXT PRIMARY KEY,cwd TEXT,source TEXT,thread_source TEXT,project_id TEXT,has_user_event INTEGER,first_user_message TEXT,archived INTEGER,model_provider TEXT,updated_at INTEGER);
            INSERT INTO projects VALUES('server','Project','{}',10,20);").unwrap();
        db.execute("INSERT INTO project_roots VALUES('server',?1,0)", [&f.root])
            .unwrap();
        for (id, source, kind, archive) in [
            ("chat", "cli", "user", 0),
            ("archived", "vscode", "user", 1),
            ("child", "{\"subagent\":{}}", "user", 0),
            ("guardian", "vscode", "guardian", 0),
        ] {
            db.execute(
                "INSERT INTO threads VALUES(?1,?2,?3,?4,NULL,1,'hello',?5,'original',123)",
                rusqlite::params![id, f.root, source, kind, archive],
            )
            .unwrap();
        }
        f
    }
    fn home(&self) -> &Path {
        self.temp.path()
    }
    fn db(&self) -> Connection {
        Connection::open(self.home().join("state_5.sqlite")).unwrap()
    }
    fn state(&self) -> Value {
        decode(read_bytes(&self.home().join(STATE)).unwrap().as_deref()).unwrap()
    }
    fn set(&self, state: &Value) {
        atomic_value(&self.home().join(STATE), state).unwrap();
    }
    fn scan(&self) -> Plan {
        scan(self.home(), None).unwrap()
    }
    fn apply(&self, plan: &Plan) -> PathBuf {
        let backup = self.home().join(format!("backup-{}", uuid::Uuid::new_v4()));
        apply(plan, &backup).unwrap();
        backup
    }
    fn project(&self, id: &str) -> Option<String> {
        self.db()
            .query_row("SELECT project_id FROM threads WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn catalog(&self) -> PathBuf {
        fs::create_dir_all(self.home().join("sqlite")).unwrap();
        let path = self.home().join("sqlite/codex-dev.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE local_thread_catalog(host_id TEXT,thread_id TEXT,project_id TEXT,cwd TEXT);
            CREATE TABLE local_thread_catalog_metadata(catalog_revision INTEGER);
            INSERT INTO local_thread_catalog_metadata VALUES(1);").unwrap();
        for host in ["local", "remote"] {
            db.execute(
                "INSERT INTO local_thread_catalog VALUES(?1,'chat',NULL,?2)",
                rusqlite::params![host, self.root],
            )
            .unwrap();
        }
        path
    }
}

#[test]
fn local_projects_restore_registry_memberships_and_catalog_without_changing_history() {
    let f = Fixture::new();
    let catalog = f.catalog();
    let before: Vec<(String, String, i64, i64)> = f
        .db()
        .prepare("SELECT id,model_provider,archived,updated_at FROM threads ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let plan = f.scan();
    assert_eq!(plan.counts.projects, 1);
    assert_eq!(plan.counts.assignments, 2);
    assert_eq!(plan.counts.thread_rows, 2);
    assert_eq!(plan.counts.catalog_rows, 1);
    assert!(
        f.state()["local-projects"].is_null(),
        "preview must not write"
    );
    f.apply(&plan);
    assert_eq!(f.project("chat").as_deref(), Some("server"));
    assert!(f.project("child").is_none());
    assert!(f.project("guardian").is_none());
    let after: Vec<(String, String, i64, i64)> = f
        .db()
        .prepare("SELECT id,model_provider,archived,updated_at FROM threads ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(before, after);
    assert_eq!(f.state()["unrelated"], json!({"keep":true}));
    let db = Connection::open(catalog).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT project_id FROM local_thread_catalog WHERE host_id='local'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "server"
    );
    assert_eq!(
        db.query_row(
            "SELECT project_id FROM local_thread_catalog WHERE host_id='remote'",
            [],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );
    assert!(!f.scan().changed(), "repeat repair must be idempotent");
}

#[test]
fn local_projects_respect_cloud_deletion_missing_roots_and_ambiguous_projects() {
    for case in ["cloud", "cloud-path", "deleted", "missing", "duplicate"] {
        let f = Fixture::new();
        match case {
            "cloud" => {
                f.db()
                    .execute(
                        "UPDATE projects SET metadata=?1",
                        [r#"{"codex.chatGptBacking":"other-account"}"#],
                    )
                    .unwrap();
            }
            "cloud-path" => {
                f.db()
                    .execute(
                        "UPDATE project_roots SET path='C:\\Users\\x\\.chatgpt-projects\\cloud'",
                        [],
                    )
                    .unwrap();
            }
            "deleted" => {
                let mut s = f.state();
                s["app-server-pending-project-deletions-by-host"] =
                    json!({host_key(&s,f.home()):{"legacy":{"projectId":"server"}}});
                f.set(&s);
            }
            "missing" => {
                fs::remove_dir(&f.root).unwrap();
            }
            _ => {
                f.db().execute_batch("INSERT INTO projects SELECT 'duplicate',name,metadata,created_at_ms,updated_at_ms FROM projects; INSERT INTO project_roots SELECT 'duplicate',path,position FROM project_roots;").unwrap();
            }
        }
        let plan = f.scan();
        assert_eq!(plan.counts.projects, 0, "{case}");
        assert_eq!(plan.counts.assignments, 0, "{case}");
        assert!(
            !plan.changed(),
            "excluded projects must not cause writes: {case}"
        );
    }
}

#[test]
fn local_projects_restore_legacy_ids_and_preserve_explicit_projectless_and_newer_assignments() {
    let f = Fixture::new();
    let host = host_key(&f.state(), f.home());
    let old = json!({"local-projects":{"legacy":{"id":"legacy","rootPaths":[f.root]}},
        MAP:{&host:{"legacy":"server"}},ASSIGN:{"chat":{"projectKind":"local","projectId":"legacy"}},
        "projectless-thread-ids":["archived"],"project-order":["legacy"]});
    atomic_value(&f.home().join(format!("{STATE}.old")), &old).unwrap();
    f.apply(&f.scan());
    assert_eq!(f.state()[ASSIGN]["chat"]["projectId"], "legacy");
    assert!(f.project("archived").is_none());
    assert!(!f.scan().changed());
    let f = Fixture::new();
    let mut s = f.state();
    s["thread-projectless-output-directories"] = json!({"chat":f.root});
    f.set(&s);
    f.db()
        .execute(
            "UPDATE threads SET project_id='newer' WHERE id='archived'",
            [],
        )
        .unwrap();
    f.apply(&f.scan());
    assert!(f.project("chat").is_none());
    assert_eq!(f.project("archived").as_deref(), Some("newer"));
}

#[test]
fn local_projects_selected_scope_keeps_unrelated_projects_and_threads_untouched() {
    let f = Fixture::new();
    let other = f.home().join("Other");
    fs::create_dir(&other).unwrap();
    f.db()
        .execute_batch("INSERT INTO projects VALUES('other','Other','{}',1,2)")
        .unwrap();
    f.db()
        .execute(
            "INSERT INTO project_roots VALUES('other',?1,0)",
            [other.to_string_lossy().as_ref()],
        )
        .unwrap();
    let plan = scan(f.home(), Some(&HashSet::from(["chat".into()]))).unwrap();
    assert_eq!(plan.counts.projects, 1);
    assert_eq!(plan.counts.assignments, 1);
    f.apply(&plan);
    assert!(f.state()["local-projects"]["other"].is_null());
    assert!(f.project("archived").is_none());
}

#[test]
fn local_projects_reject_concurrent_state_registry_and_thread_changes() {
    for case in ["json", "project", "thread", "source"] {
        let f = Fixture::new();
        let plan = f.scan();
        match case {
            "json" => {
                let mut s = f.state();
                s["new"] = json!(true);
                f.set(&s);
            }
            "project" => {
                f.db()
                    .execute("UPDATE projects SET name='Newer'", [])
                    .unwrap();
            }
            "thread" => {
                f.db()
                    .execute("UPDATE threads SET project_id='newer' WHERE id='chat'", [])
                    .unwrap();
            }
            _ => {
                f.db()
                    .execute(
                        "UPDATE threads SET thread_source='subagent' WHERE id='chat'",
                        [],
                    )
                    .unwrap();
            }
        }
        let result = apply(&plan, &f.home().join("backup"));
        assert!(
            result.unwrap_err().contains("CODEX_PROJECT_CHANGED"),
            "{case}"
        );
        assert!(f.state()["local-projects"].is_null(), "{case}");
        assert!(
            f.project("archived").is_none(),
            "no partial changes: {case}"
        );
        if case == "thread" {
            assert_eq!(f.project("chat").as_deref(), Some("newer"));
        }
    }
}

#[test]
fn local_projects_conditional_rollback_preserves_new_chats_and_newer_memberships() {
    let f = Fixture::new();
    let catalog = f.catalog();
    let plan = f.scan();
    f.apply(&plan);
    f.db().execute("INSERT INTO threads SELECT 'new-chat',cwd,source,thread_source,NULL,has_user_event,first_user_message,archived,model_provider,updated_at FROM threads WHERE id='chat'",[]).unwrap();
    f.db()
        .execute("UPDATE threads SET project_id='newer' WHERE id='chat'", [])
        .unwrap();
    let mut s = f.state();
    s[ASSIGN]["new-chat"] = json!({"projectKind":"local","projectId":"later"});
    s[ASSIGN]["chat"] = json!({"projectKind":"local","projectId":"newer"});
    f.set(&s);
    rollback(&plan).unwrap();
    assert_eq!(f.project("chat").as_deref(), Some("newer"));
    assert!(f.project("archived").is_none());
    assert!(f.state()[ASSIGN]["new-chat"].is_object());
    assert_eq!(f.state()[ASSIGN]["chat"]["projectId"], "newer");
    assert!(f.project("new-chat").is_none());
    let db = Connection::open(catalog).unwrap();
    assert!(db
        .query_row(
            "SELECT project_id FROM local_thread_catalog WHERE host_id='local'",
            [],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap()
        .is_none());
}

#[test]
fn local_projects_interrupted_multi_file_write_is_recoverable_and_retryable() {
    let f = Fixture::new();
    let plan = f.scan();
    let backup = f.home().join("interrupted");
    fs::create_dir(&backup).unwrap();
    atomic_value(&backup.join("project-recovery-plan.json"), &plan).unwrap();
    atomic_value(&f.home().join(JOURNAL), &backup).unwrap();
    // Simulate a crash after a database commit and JSON replacement, before .bak.
    update_rows(&f.db(), &plan.databases[0], false).unwrap();
    f.set(&plan.after);
    assert!(has_pending(f.home()));
    recover_pending(f.home()).unwrap();
    assert!(!has_pending(f.home()));
    assert!(f.project("chat").is_none());
    assert_eq!(f.state(), plan.before);
    f.apply(&f.scan());
    assert!(!f.scan().changed());
}

#[test]
fn local_projects_windows_aliases_unc_and_posix_case_and_backslashes() {
    assert_eq!(
        path_key(r"\\?\C:\Repo\Project\"),
        path_key("c:/repo/project")
    );
    assert_eq!(
        path_key(r"\\?\UNC\server\share\repo"),
        path_key(r"\\SERVER\SHARE\repo")
    );
    assert_ne!(path_key("/Users/me/Project"), path_key("/Users/me/project"));
    assert_ne!(path_key("/Users/me/a\\b"), path_key("/Users/me/a/b"));
    assert_ne!(
        path_key("/Users/me/project "),
        path_key("/Users/me/project")
    );
    let f = Fixture::new();
    f.db()
        .execute("UPDATE project_roots SET path=?1", [r"\\?\C:\Repo\Project"])
        .unwrap();
    f.db()
        .execute("UPDATE threads SET cwd='c:/repo/project'", [])
        .unwrap();
    let plan = scan_with_roots(f.home(), None, &|_| true).unwrap();
    assert_eq!(plan.counts.assignments, 2);
}

#[test]
fn local_projects_missing_or_unsupported_schema_never_creates_or_changes_data() {
    let f = Fixture::new();
    let state = f.state();
    f.db().execute_batch("DROP TABLE project_roots").unwrap();
    assert!(scan(f.home(), None)
        .unwrap_err()
        .contains("CODEX_PROJECT_SCHEMA_UNSUPPORTED"));
    assert_eq!(f.state(), state);
    let empty = tempfile::tempdir().unwrap();
    assert!(scan(empty.path(), None).is_err());
    assert!(!empty.path().join("state_5.sqlite").exists());
}

#[test]
fn local_projects_preserve_conflicting_catalog_and_remote_memberships() {
    let f = Fixture::new();
    let path = f.catalog();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE local_thread_catalog SET project_id='newer' WHERE host_id='local'",
            [],
        )
        .unwrap();
    let mut state = f.state();
    state[HOSTS] = json!({"archived":"remote-host"});
    f.set(&state);
    let plan = f.scan();
    assert_eq!(plan.counts.assignments, 0);
    assert_eq!(plan.counts.ambiguous_threads, 1);
    f.apply(&plan);
    assert!(f.project("chat").is_none());
    assert!(f.project("archived").is_none());
    assert_eq!(f.state()[HOSTS]["archived"], "remote-host");
}

#[test]
fn local_projects_use_database_membership_before_snapshot_or_directory_guessing() {
    let f = Fixture::new();
    f.catalog();
    // The existing catalog membership is explicit even when cwd no longer agrees.
    Connection::open(f.home().join("sqlite/codex-dev.db"))
        .unwrap()
        .execute(
            "UPDATE local_thread_catalog SET project_id='server' WHERE host_id='local'",
            [],
        )
        .unwrap();
    f.db()
        .execute(
            "UPDATE threads SET cwd='/different/worktree' WHERE id='chat'",
            [],
        )
        .unwrap();
    f.apply(&f.scan());
    assert_eq!(f.project("chat").as_deref(), Some("server"));
}

#[test]
fn local_projects_preserve_project_order_ids_and_default_process_context_on_all_platforms() {
    let state = json!({"project-order":["Project-A","project-a",r"\\?\C:\Repo\",r"C:\Repo"]});
    let normalized = super::super::normalized_global_state_entries(state.as_object().unwrap());
    assert_eq!(
        normalized["project-order"],
        json!(["Project-A", "project-a", r"C:\Repo"])
    );
    let mut instance = super::super::CodexSyncInstance {
        id: super::super::DEFAULT_INSTANCE_ID.into(),
        name: "Default".into(),
        data_dir: PathBuf::from("/synthetic/default"),
        last_pid: None,
    };
    assert_eq!(super::super::instance_process_home(&instance), None);
    instance.id = "managed".into();
    assert_eq!(
        super::super::instance_process_home(&instance),
        Some("/synthetic/default")
    );
    let options = super::super::CodexSessionVisibilityRepairOptions::for_auto_repair_mode(
        super::super::CodexSessionVisibilityAutoRepairMode::Current,
    );
    assert!(!options.normalize_global_state && !options.repair_local_thread_catalog);
    assert_eq!(
        options.mode,
        super::super::CodexSessionVisibilityRepairMode::Quick
    );
}

#[test]
fn local_projects_budget_expires_before_mutation_and_existing_data_stays_available() {
    let f = Fixture::new();
    let before = f.state();
    let result = super::super::with_launch_visibility_budget(std::time::Duration::ZERO, || {
        scan(f.home(), None)
    });
    assert!(result
        .unwrap_err()
        .contains("CODEX_SESSION_VISIBILITY_TIMEOUT"));
    assert_eq!(f.state(), before);
    assert!(f.project("chat").is_none());
    f.apply(&f.scan());
    assert!(!f.scan().changed());
}

#[test]
fn quick_projects_use_current_metadata_only_and_keep_precise_backups() {
    let f = Fixture::new();
    f.catalog();
    let host = host_key(&f.state(), f.home());
    let old =
        json!({"local-projects":{"legacy":{"rootPaths":[f.root]}},MAP:{host:{"legacy":"server"}}});
    atomic_value(&f.home().join(format!("{STATE}.old")), &old).unwrap();
    let plan = scan_for_repair(f.home(), None, true).unwrap().unwrap();
    assert_eq!(
        plan.after[ASSIGN]["chat"]["projectId"], "server",
        "quick must not read snapshot aliases"
    );
    let backup = f.apply(&plan);
    assert!(backup.join("project-recovery-plan.json").is_file());
    assert!(
        !backup.join("db").exists(),
        "quick must not copy whole databases"
    );
    rollback(&plan).unwrap();
    assert!(f.project("chat").is_none());
    let deep = scan_for_repair(f.home(), None, false).unwrap().unwrap();
    assert_eq!(
        deep.after[ASSIGN]["chat"]["projectId"], "legacy",
        "deep includes historical identity recovery"
    );
}

#[test]
fn quick_projects_limits_defer_before_mutation_and_deep_still_repairs_every_thread() {
    let f = Fixture::new();
    f.db().execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2100)
        INSERT INTO threads SELECT 'many-'||x,cwd,source,thread_source,NULL,has_user_event,first_user_message,archived,model_provider,updated_at FROM n JOIN threads ON id='chat'").unwrap();
    let before = f.state();
    assert!(scan_for_repair(f.home(), None, true)
        .unwrap_err()
        .contains("CODEX_PROJECT_QUICK_LIMIT"));
    assert_eq!(f.state(), before);
    assert!(f.project("chat").is_none());
    let full = scan_for_repair(f.home(), None, false).unwrap().unwrap();
    assert_eq!(full.counts.assignments, 2102);
    f.apply(&full);
    assert_eq!(f.project("many-2100").as_deref(), Some("server"));
}

fn integrated_options(
    mode: super::super::CodexSessionVisibilityRepairMode,
    preview: bool,
) -> super::super::CodexSessionVisibilityRepairOptions {
    let mut options = super::super::CodexSessionVisibilityRepairOptions::for_mode(mode);
    options.dry_run = preview;
    // Integration tests exercise the on-disk stages without starting an official app server.
    options.rebuild_metadata = false;
    options
}

fn integrated_repair(
    f: &Fixture,
    mode: super::super::CodexSessionVisibilityRepairMode,
    preview: bool,
) -> Result<super::super::CodexSessionVisibilityRepairSummary, String> {
    use super::super::*;
    let _guard = acquire_session_visibility_repair_lock()?;
    repair_session_visibility_for_instances_with_options(
        integrated_options(mode, preview),
        None,
        None,
        RepairTargetSelection::default(),
        vec![CodexSyncInstance {
            id: "project-integration".into(),
            name: "Project integration".into(),
            data_dir: f.home().to_path_buf(),
            last_pid: None,
        }],
    )
}

#[test]
fn quick_visibility_directly_repairs_projects_and_unified_launch_uses_the_same_path() {
    use super::super::CodexSessionVisibilityRepairMode as Mode;
    let f = Fixture::new();
    f.catalog();
    fs::write(
        f.home().join("thread_history_1.sqlite"),
        b"unusable derived history index",
    )
    .unwrap();
    let preview = integrated_repair(&f, Mode::Quick, true).unwrap();
    assert_eq!(preview.local_project_recovery.unwrap().assignments, 2);
    assert!(f.project("chat").is_none());
    let summary = super::super::repair_session_visibility_quick_for_instance(
        "project-integration",
        "Project integration",
        f.home(),
        false,
    )
    .unwrap();
    assert!(
        !summary.local_project_recovery_deferred,
        "{}",
        summary.message
    );
    assert_eq!(summary.local_project_recovery.unwrap().assignments, 2);
    assert_eq!(summary.mutated_instance_count, 1);
    assert_eq!(f.project("chat").as_deref(), Some("server"));
    assert!(summary.history_projection_count.is_none());
    let repeated = integrated_repair(&f, Mode::Quick, false).unwrap();
    assert_eq!(repeated.mutated_instance_count, 0);
    assert_eq!(repeated.local_project_recovery.unwrap().assignments, 0);
}

#[test]
fn quick_visibility_preserves_large_current_state_and_full_repairs_it() {
    use super::super::CodexSessionVisibilityRepairMode as Mode;
    let f = Fixture::new();
    let mut state = f.state();
    state["large-cache"] = json!("x".repeat(2_000_000));
    f.set(&state);
    let quick = integrated_repair(&f, Mode::Quick, false).unwrap();
    assert!(quick.local_project_recovery_deferred);
    assert_eq!(f.state(), state);
    assert!(f.project("chat").is_none());
    let full = integrated_repair(&f, Mode::Deep, false).unwrap();
    assert!(!full.local_project_recovery_deferred);
    assert_eq!(full.local_project_recovery.unwrap().assignments, 2);
    assert_eq!(f.state()["large-cache"], state["large-cache"]);
    assert_eq!(f.project("chat").as_deref(), Some("server"));
}

#[test]
fn full_visibility_includes_snapshot_and_chat_index_repair_in_one_existing_mode() {
    use super::super::CodexSessionVisibilityRepairMode as Mode;
    let f = Fixture::new();
    let id = "11111111-1111-4111-8111-111111111111";
    f.db()
        .execute("UPDATE threads SET id=?1 WHERE id='chat'", [id])
        .unwrap();
    f.db()
        .execute_batch("ALTER TABLE threads ADD COLUMN rollout_path TEXT")
        .unwrap();
    fs::create_dir(f.home().join("sessions")).unwrap();
    let rollout = f
        .home()
        .join(format!("sessions/rollout-2026-10-29T00-00-00-{id}.jsonl"));
    let contents = format!(
        "{}\n{}\n",
        json!({"ordinal":0,"type":"session_meta","payload":{"id":id,"model_provider":"original","cwd":f.root,"source":"cli"}}),
        json!({"ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"Keep history"}})
    );
    fs::write(&rollout, &contents).unwrap();
    f.db()
        .execute(
            "UPDATE threads SET rollout_path=?1 WHERE id=?2",
            rusqlite::params![rollout.to_string_lossy(), id],
        )
        .unwrap();
    let history = Connection::open(f.home().join("thread_history_1.sqlite")).unwrap();
    history.execute_batch("CREATE TABLE thread_items(thread_id TEXT NOT NULL,turn_id TEXT NOT NULL,item_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,item_json TEXT NOT NULL,PRIMARY KEY(thread_id,turn_id,item_id));
        CREATE TABLE thread_realtime_items(thread_id TEXT NOT NULL,item_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,item_type TEXT NOT NULL,item_json TEXT NOT NULL,PRIMARY KEY(thread_id,item_id));
        CREATE TABLE thread_turns(thread_id TEXT NOT NULL,turn_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,status TEXT NOT NULL,error_json TEXT,started_at INTEGER,completed_at INTEGER,duration_ms INTEGER,first_user_item_id TEXT,final_agent_item_id TEXT,PRIMARY KEY(thread_id,turn_id));
        CREATE TABLE thread_history_projection_state(thread_id TEXT PRIMARY KEY,next_rollout_byte_offset INTEGER NOT NULL,next_rollout_ordinal INTEGER NOT NULL);").unwrap();
    history
        .execute(
            "INSERT INTO thread_history_projection_state VALUES(?1,?2,2)",
            rusqlite::params![id, contents.len() as i64 + 100],
        )
        .unwrap();
    let host = host_key(&f.state(), f.home());
    atomic_value(
        &f.home().join(format!("{STATE}.old")),
        &json!({"local-projects":{"legacy":{"rootPaths":[f.root]}},MAP:{host:{"legacy":"server"}}}),
    )
    .unwrap();
    let preview = integrated_repair(&f, Mode::Deep, true).unwrap();
    assert_eq!(preview.history_projection_count, Some(1));
    assert!(f.project(id).is_none());
    let full = integrated_repair(&f, Mode::Deep, false).unwrap();
    assert_eq!(full.history_projection_count, Some(1));
    assert_eq!(f.state()[ASSIGN][id]["projectId"], "legacy");
    assert_eq!(f.project(id).as_deref(), Some("server"));
    assert_eq!(
        history
            .query_row(
                "SELECT COUNT(*) FROM thread_history_projection_state",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(fs::read_to_string(&rollout).unwrap(), contents);
    assert!(full.backup_dirs.len() >= 2);
}
