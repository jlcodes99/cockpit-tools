use super::*;

struct Fixture {
    temp: tempfile::TempDir,
    root: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let f = Self {
            temp,
            root: root.to_string_lossy().into(),
        };
        let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT,metadata TEXT);
            CREATE TABLE project_roots(project_id TEXT,path TEXT,position INTEGER);
            CREATE TABLE threads(id TEXT PRIMARY KEY,cwd TEXT,source TEXT,project_id TEXT,has_user_event INTEGER);
            INSERT INTO projects VALUES('server','Project','{}');").unwrap();
        db.execute("INSERT INTO project_roots VALUES('server',?1,0)", [&f.root])
            .unwrap();
        db.execute(
            "INSERT INTO threads VALUES('chat',?1,'cli','server',1)",
            [&f.root],
        )
        .unwrap();
        db.execute(
            "INSERT INTO threads VALUES('child',?1,'subagent','server',1)",
            [&f.root],
        )
        .unwrap();
        f
    }
    fn home(&self) -> &Path {
        self.temp.path()
    }
    fn local(&self, alias: &str) -> Value {
        json!({
            "local-projects": {alias:{"id":alias,"name":"My project","rootPaths":[self.root]}},
            "project-order":[alias],
            MAP: {format!("local:{}",self.home().display()):{alias:"server"}},
            ASSIGN: {"chat":{"projectKind":"local","projectId":alias}},
            HOSTS: {"chat":"local"}
        })
    }
    fn write(&self, state: &Value) {
        fs::write(self.home().join(STATE), serde_json::to_vec(state).unwrap()).unwrap();
    }
    fn state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.home().join(STATE)).unwrap()).unwrap()
    }
}

// Exact relevant native constructor/identity behavior. A top-level-only repair
// fails this sequence because Desktop archives it, clears it, then loads a scope.
fn native_select(state: &mut Value, owner: Option<&str>) {
    let Some(mut scopes) = state.get(SCOPES).cloned() else {
        return;
    };
    if owner.is_some() && scopes["owner"].as_str() == owner {
        return;
    }
    let atoms = object(&state[ATOMS]);
    if let Some(current) = scopes["owner"].as_str().map(str::to_string) {
        scopes["inactive"][&current] = json!({
            "globals":scoped_globals(state),
            "atoms":atoms.iter().filter(|(key,_)| !SHARED_ATOMS.contains(&key.as_str()))
                .map(|(key,value)| (key.clone(),value.clone())).collect::<BTreeMap<_,_>>(),
        });
    }
    let target = owner
        .and_then(|id| scopes["inactive"].get(id))
        .cloned()
        .unwrap_or_else(|| json!({"globals":{},"atoms":{}}));
    for key in SCOPED_GLOBALS {
        state.as_object_mut().unwrap().remove(*key);
        if let Some(value) = target["globals"].get(*key) {
            state[*key] = value.clone();
        }
    }
    let mut next_atoms = object(&target["atoms"]);
    next_atoms.extend(
        atoms
            .into_iter()
            .filter(|(key, _)| SHARED_ATOMS.contains(&key.as_str())),
    );
    state[ATOMS] = json!(next_atoms);
    scopes["owner"] = json!(owner);
    if let Some(owner) = owner {
        scopes["inactive"].as_object_mut().unwrap().remove(owner);
    }
    state[SCOPES] = scopes;
}
fn native_start(mut state: Value, owner: Option<&str>) -> Value {
    native_select(&mut state, None);
    if owner.is_some() {
        native_select(&mut state, owner);
    }
    state
}

// Desktop's sendPersistedAtomState sends {} without an authenticated principal
// when the native account archive is active, even for shared onboarding flags.
fn native_synced_atoms(state: &Value, owner: Option<&str>) -> Value {
    if state.get(SCOPES).is_some() && owner.is_none() {
        json!({})
    } else {
        state[ATOMS].clone()
    }
}

#[test]
fn api_key_start_preserves_completed_onboarding_without_a_project_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(STATE);
    let original = json!({
        SCOPES:{"owner":"oauth-owner","inactive":{}},
        ATOMS:{"electron:onboarding-projectless-completed":true,
            "last_completed_onboarding":123,"private-oauth-preference":"owned"},
        "remote-projects":{"private-cloud-project":{}},
    });
    assert!(native_synced_atoms(&original, None).as_object().unwrap().is_empty());
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    assert!(prepare(temp.path(), None).unwrap());
    let prepared: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let booted = native_start(prepared, None);
    let atoms = native_synced_atoms(&booted, None);
    assert_eq!(atoms["electron:onboarding-projectless-completed"], true);
    assert_eq!(atoms["last_completed_onboarding"], 123);
    assert!(atoms.get("private-oauth-preference").is_none());
    assert!(booted.get("remote-projects").is_none());
    fs::write(&path, serde_json::to_vec(&booted).unwrap()).unwrap();
    assert!(!prepare(temp.path(), None).unwrap(), "API-key restart stays idempotent");
    assert!(prepare(temp.path(), Some("oauth-owner")).unwrap());
    let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let restored = native_start(restored, Some("oauth-owner"));
    assert_eq!(restored["remote-projects"], original["remote-projects"]);
    assert_eq!(restored[ATOMS], original[ATOMS]);
    assert!(restored.get(PARKED).is_none());
}

#[test]
fn projectless_api_key_start_does_not_invent_onboarding_completion() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("DELETE FROM threads; DELETE FROM project_roots; DELETE FROM projects;")
        .unwrap();
    drop(db);
    for completed in [None, Some(false), Some(true)] {
        let mut state = json!({SCOPES:{"owner":null,"inactive":{}},ATOMS:{}});
        if let Some(completed) = completed {
            state[ATOMS]["electron:onboarding-projectless-completed"] = json!(completed);
        }
        f.write(&state);
        assert!(prepare(f.home(), None).unwrap());
        let started = native_start(f.state(), None);
        assert_eq!(native_synced_atoms(&started, None), state[ATOMS]);
        assert!(started["local-projects"].as_object().is_none_or(|v| v.is_empty()));
    }
}

#[test]
fn same_instance_switch_keeps_distinct_projects_sharing_roots_and_their_threads() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("INSERT INTO projects VALUES('second','Second','{}');
        INSERT INTO projects VALUES('third','Third','{}');
        INSERT INTO projects VALUES('cloud','Cloud','{\"chatGptBacking\":{}}');
        INSERT INTO threads VALUES('second-chat','unused','cli','second',1);
        INSERT INTO threads VALUES('third-chat','unused','cli','third',1);
        INSERT INTO threads VALUES('cloud-chat','unused','cli','cloud',1);").unwrap();
    for id in ["second", "third", "cloud"] {
        db.execute("INSERT INTO project_roots VALUES(?1,?2,0)", [id, &f.root]).unwrap();
    }
    drop(db);
    let host = format!("local:{}", f.home().display());
    let mut state = f.local("legacy");
    state["local-projects"]["second-legacy"] = json!({"id":"second-legacy","name":"Second","rootPaths":[f.root]});
    state[MAP][&host]["second-legacy"] = json!("second");
    // An already-native project ID is also authoritative without a legacy map.
    state["local-projects"]["third"] = json!({"id":"third","name":"Third","rootPaths":[f.root]});
    state["project-order"] = json!(["third", "legacy", "second-legacy"]);
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state[ATOMS] = json!({"private-a":true,"electron:onboarding-projectless-completed":true});
    f.write(&state);
    for owner in [Some("b"), None, Some("a"), Some("b"), None] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["local-projects"], state["local-projects"]);
        assert_eq!(booted["project-order"], state["project-order"]);
        for (thread, project) in [("chat", "legacy"), ("second-chat", "second-legacy"), ("third-chat", "third")] {
            assert_eq!(booted[ASSIGN][thread]["projectId"], project);
            assert_eq!(booted[HOSTS][thread], "local");
        }
        assert!(booted[ASSIGN].get("cloud-chat").is_none());
        assert!(booted[ASSIGN].get("child").is_none());
        assert_eq!(booted["local-projects"].as_object().unwrap().len(), 3);
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap(), "repeat launch: {owner:?}");
    }
}

#[test]
fn duplicate_roots_without_identity_do_not_guess_or_restore_deleted_projects() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("INSERT INTO projects VALUES('second','Second','{}');").unwrap();
    db.execute("INSERT INTO project_roots VALUES('second',?1,0)", [&f.root]).unwrap();
    drop(db);
    let host = format!("local:{}", f.home().display());
    let mut globals = f.local("legacy");
    globals.as_object_mut().unwrap().remove(MAP);
    f.write(&json!({SCOPES:{"owner":null,"inactive":{"a":{"globals":globals,"atoms":{}}}}}));
    prepare(f.home(), Some("b")).unwrap();
    assert!(f.state()["local-projects"].as_object().is_none_or(|p| p.is_empty()));
    globals[MAP] = json!({&host:{"legacy":"server"}});
    globals["app-server-pending-project-deletions-by-host"] = json!({&host:{"legacy":{"projectId":"server"}}});
    f.write(&json!({SCOPES:{"owner":null,"inactive":{"a":{"globals":globals,"atoms":{}}}}}));
    prepare(f.home(), Some("b")).unwrap();
    assert!(f.state()["local-projects"].as_object().is_none_or(|p| p.is_empty()));
    assert!(f.state()[ASSIGN].as_object().is_none_or(|p| p.is_empty()));
}

#[test]
fn account_project_switch_recovers_archived_groups_after_native_startup() {
    let f = Fixture::new();
    let mut old = f.local("legacy");
    old["remote-projects"] = json!({"remote-a":{"secret":"account-a-only"}});
    f.write(&json!({SCOPES:{"owner":"b","inactive":{
        "a":{"atoms":{"private-a":true},"globals":old}
    }}, ATOMS:{"private-b":true,"thread-sort-key":"updated"}, "untouched":42}));
    assert!(prepare(f.home(), Some("b")).unwrap());
    let actual = native_start(f.state(), Some("b"));
    assert_eq!(actual["local-projects"]["legacy"]["name"], "My project");
    assert_eq!(actual[ASSIGN]["chat"]["projectId"], "legacy");
    assert!(actual[ASSIGN].get("child").is_none());
    assert!(actual.get("remote-projects").is_none());
    assert_eq!(actual[ATOMS]["private-b"], true);
    assert!(actual[ATOMS].get("private-a").is_none());
    assert_eq!(actual[ATOMS]["thread-sort-key"], "updated");
    assert_eq!(actual["untouched"], 42);
    assert_eq!(
        actual[SCOPES]["inactive"]["a"]["globals"]["remote-projects"],
        old["remote-projects"]
    );
    f.write(&actual);
    assert!(
        !prepare(f.home(), Some("b")).unwrap(),
        "same-account startup is idempotent"
    );
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert_eq!(back["remote-projects"], old["remote-projects"]);
    assert_eq!(back[ATOMS]["private-a"], true);
    assert!(back[ATOMS].get("private-b").is_none());
    assert_eq!(back[ASSIGN]["chat"]["projectId"], "legacy");
}

#[test]
fn account_switch_keeps_configured_ssh_workspaces_and_latest_auto_connect_choices() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{
        "older":{"atoms":{},"globals":{
            "remote-connection-auto-connect-by-host-id":{"ssh-one":true,"ssh-two":true},
            "remote-projects":[
                {"id":"second","hostId":"ssh-two","label":"Second","remotePath":"/second"},
                {"id":"deleted","hostId":"removed-host","remotePath":"/removed"}
            ]
        }}
    }});
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh-one","hostname":"one.invalid","source":"discovered"},
        {"hostId":"ssh-two","hostname":"two.invalid","source":"discovered"}
    ]);
    state["remote-connection-auto-connect-by-host-id"] =
        json!({"ssh-one":false,"cloud-host":true});
    state["selected-remote-host-id"] = json!("ssh-one");
    state["active-remote-project-id"] = json!("first");
    state["remote-projects"] = json!([
        {"id":"first","hostId":"ssh-one","label":"First","remotePath":"/first",
            "unknown-account-data":"do-not-share"},
        {"id":"cloud","hostId":"cloud-host","remotePath":"/private"}
    ]);
    state["host-id-remote-control-allowed"] = json!({"cloud-host":true});
    state["added-remote-control-env-ids"] = json!(["cloud-host"]);
    f.write(&state);
    for owner in [Some("b"), None, Some("b")] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["codex-managed-remote-connections"], state["codex-managed-remote-connections"]);
        assert_eq!(booted["remote-connection-auto-connect-by-host-id"],
            json!({"ssh-one":false,"ssh-two":true}));
        assert_eq!(booted["selected-remote-host-id"], "ssh-one");
        assert_eq!(booted["active-remote-project-id"], "first");
        let projects = booted["remote-projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert!(projects.iter().all(|p| p.get("unknown-account-data").is_none()));
        assert!(booted.get("host-id-remote-control-allowed").is_none());
        assert!(booted.get("added-remote-control-env-ids").is_none());
        assert_eq!(booted[ASSIGN]["chat"]["projectId"], "legacy");
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap());
    }
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert_eq!(back["host-id-remote-control-allowed"], state["host-id-remote-control-allowed"]);
    assert_eq!(back["added-remote-control-env-ids"], state["added-remote-control-env-ids"]);
    assert_eq!(back["remote-projects"].as_array().unwrap().len(), 3);
}

#[test]
fn ssh_auto_connect_survives_api_key_start_without_a_project_database() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({
        SCOPES:{"owner":"a","inactive":{}},
        "codex-managed-remote-connections":[{"hostId":"ssh","hostname":"fixture.invalid"}],
        "remote-connection-auto-connect-by-host-id":{"ssh":true},
        "selected-remote-host-id":"ssh"
    });
    fs::write(temp.path().join(STATE), serde_json::to_vec(&state).unwrap()).unwrap();
    prepare(temp.path(), None).unwrap();
    let prepared: Value = serde_json::from_slice(&fs::read(temp.path().join(STATE)).unwrap()).unwrap();
    let booted = native_start(prepared, None);
    assert_eq!(booted["remote-connection-auto-connect-by-host-id"], state["remote-connection-auto-connect-by-host-id"]);
    assert_eq!(booted["selected-remote-host-id"], "ssh");
}

// Desktop discovers SSH before account globals return. Its native startup
// config then refreshes the catalog and actively connects configured aliases.
fn native_startup_ssh_aliases(home: &Path, prepared: Value) -> Vec<String> {
    let mut booting = prepared;
    native_select(&mut booting, None);
    let config = fs::read(home.join("codex-app/config.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| json!({}));
    config["remoteConnections"].as_array().into_iter().flatten()
        .filter_map(|entry| entry["sshAlias"].as_str())
        .filter(|alias| booting["codex-managed-remote-connections"].as_array().into_iter().flatten()
            .any(|host| host["alias"] == *alias
                && booting["remote-connection-auto-connect-by-host-id"][host["hostId"].as_str().unwrap()] != false))
        .map(str::to_string).collect()
}

#[test]
fn switching_accounts_queues_enabled_ssh_aliases_during_native_startup() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh-on","alias":"enabled","hostname":null},
        {"hostId":"ssh-off","alias":"disabled","hostname":null},
        {"hostId":"ssh-unused","alias":"unused","hostname":null}
    ]);
    state["remote-connection-auto-connect-by-host-id"] = json!({"ssh-on":true,"ssh-off":false});
    f.write(&state);
    for owner in [Some("b"), Some("a"), None] {
        prepare(f.home(), owner).unwrap();
        assert_eq!(native_startup_ssh_aliases(f.home(), f.state()), vec!["enabled"],
            "restoring the flag after discovery must also schedule a real connection");
        f.write(&native_start(f.state(), owner));
    }
}

#[test]
fn ssh_startup_removes_disabled_owned_aliases_and_preserves_user_configuration() {
    let f = Fixture::new();
    let config_path = f.home().join("codex-app/config.json");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let user_config = json!({"version":1,"remoteConnectionMaxRetryAttempts":4,
        "sshConnectTimeoutSeconds":9,"remoteConnections":[
            {"sshAlias":"admin","projects":[{"remotePath":"/admin","label":"Team"}]}
        ]});
    fs::write(&config_path, serde_json::to_vec(&user_config).unwrap()).unwrap();
    let existing_backup = config_path.with_extension("json.bak");
    fs::write(&existing_backup, b"existing native backup").unwrap();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","alias":"resume","hostname":null},
        {"hostId":"admin-host","alias":"admin","hostname":null}
    ]);
    state["remote-connection-auto-connect-by-host-id"] = json!({"ssh":true,"admin-host":true});
    f.write(&state);
    prepare(f.home(), Some("b")).unwrap();
    let configured: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(configured["remoteConnections"].as_array().unwrap().len(), 2);
    assert_eq!(configured["remoteConnections"][0], user_config["remoteConnections"][0]);
    assert_eq!(fs::read(&existing_backup).unwrap(), b"existing native backup");
    let booted = native_start(f.state(), Some("b"));
    f.write(&booted);
    assert!(!prepare(f.home(), Some("b")).unwrap(), "startup writes are idempotent");

    let mut disconnected = f.state();
    disconnected["remote-connection-auto-connect-by-host-id"]["ssh"] = json!(false);
    f.write(&disconnected);
    prepare(f.home(), Some("a")).unwrap();
    let after_disable: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(after_disable, user_config, "only Cockpit's added startup entry is removed");

    let mut removed = f.state();
    removed["codex-managed-remote-connections"] = json!([]);
    f.write(&removed);
    prepare(f.home(), Some("b")).unwrap();
    let after_remove: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(after_remove, user_config, "existing administrator entries remain user-owned");
}

#[test]
fn same_account_and_unscoped_starts_reconcile_ssh_startup_entries() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","alias":"resume","hostname":null}
    ]);
    state["remote-connection-auto-connect-by-host-id"] = json!({"ssh":true});
    f.write(&state);
    assert!(prepare(f.home(), None).unwrap());
    assert_eq!(native_startup_ssh_aliases(f.home(), f.state()), vec!["resume"]);
    assert!(!prepare(f.home(), None).unwrap());
    state["codex-managed-remote-connections"] = json!([]);
    f.write(&state);
    assert!(prepare(f.home(), None).unwrap());
    assert!(native_startup_ssh_aliases(f.home(), f.state()).is_empty());
}

#[test]
fn ssh_startup_keeps_adopted_entries_and_leaves_unsupported_files_untouched() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","alias":"resume","hostname":null}
    ]);
    state["remote-connection-auto-connect-by-host-id"] = json!({"ssh":true});
    f.write(&state);
    prepare(f.home(), None).unwrap();
    let path = f.home().join("codex-app/config.json");
    let mut adopted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    adopted["remoteConnections"][0]["projects"] = json!([{"remotePath":"/team"}]);
    fs::write(&path, serde_json::to_vec(&adopted).unwrap()).unwrap();
    state["remote-connection-auto-connect-by-host-id"]["ssh"] = json!(false);
    f.write(&state);
    prepare(f.home(), None).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(), adopted);

    state["remote-connection-auto-connect-by-host-id"]["ssh"] = json!(true);
    f.write(&state);
    let journal = f.home().join("codex-app/.cockpit-ssh-startup.json");
    let original_journal = fs::read(&journal).unwrap();
    for raw in [r#"{"version":2,"remoteConnections":[]}"#, "broken-json",
        r#"{"version":null,"remoteConnections":[]}"#,
        r#"{"version":1,"remoteConnections":[{"sshAlias":"admin","projects":[{"remotePath":"/team","label":null}]}]}"#] {
        fs::write(&path, raw).unwrap();
        prepare(f.home(), None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), raw.as_bytes());
        assert_eq!(fs::read(&journal).unwrap(), original_journal);
    }
}

#[test]
fn desktop_launch_restores_archived_groups_before_optional_catalog_scan_defers() {
    let f = Fixture::new();
    fs::write(f.home().join("config.toml"), "cli_auth_credentials_store = 'file'\n").unwrap();
    let auth = br#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-key"}"#;
    fs::write(f.home().join("auth.json"), auth).unwrap();
    // Reproduce a multi-account state above the former 2 MB launch threshold.
    let state = json!({SCOPES:{"owner":"b","inactive":{
        "a":{"globals":f.local("legacy"),"atoms":{}}
    }},"large-cache":"x".repeat(3_500_000)});
    f.write(&state);
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE local_thread_catalog(host_id TEXT,thread_id TEXT,project_id TEXT,cwd TEXT);
        WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<4096)
        INSERT INTO local_thread_catalog SELECT 'local','catalog-'||x,'server','unused' FROM n;").unwrap();
    drop(db);
    let summary = super::super::super::repair_session_visibility_quick_for_instance(
        "archived-projects", "Archived projects", f.home(), true,
    ).unwrap();
    assert!(summary.local_project_recovery_deferred);
    let detail = summary.local_project_recovery_deferred_message.as_deref().unwrap();
    let actual_size = fs::metadata(f.home().join(STATE)).unwrap().len().to_string();
    assert!(detail.contains(&actual_size), "{detail}");
    assert!(summary.message.contains(detail));
    let booted = native_start(f.state(), None);
    assert_eq!(booted["local-projects"]["legacy"]["name"], "My project");
    assert_eq!(booted[ASSIGN]["chat"]["projectId"], "legacy");
    assert_eq!(booted["large-cache"], state["large-cache"]);
    assert_eq!(fs::read(f.home().join("auth.json")).unwrap(), auth);
}

#[test]
fn desktop_launch_rejects_unrestorable_renderer_state_before_history_writes() {
    let f = Fixture::new();
    fs::write(f.home().join("config.toml"), "cli_auth_credentials_store = 'file'\n").unwrap();
    fs::write(f.home().join("auth.json"), br#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-key"}"#).unwrap();
    let invalid = json!({SCOPES:{"owner":"a","inactive":[]}});
    f.write(&invalid);
    let error = super::super::super::repair_session_visibility_quick_for_instance(
        "unsupported-state", "Unsupported state", f.home(), true,
    ).unwrap_err();
    assert!(error.contains("CODEX_DESKTOP_STATE_PREPARATION_FAILED"), "{error}");
    assert!(!super::super::super::can_defer_launch_snapshot_error(&error));
    assert_eq!(f.state(), invalid);
}

#[test]
fn switch_without_launch_preserves_projects_with_large_catalog_and_history() {
    use base64::Engine as _;

    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<300)
         INSERT INTO projects SELECT 'extra-'||x,'Extra','{}' FROM n;
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2100)
         INSERT INTO threads SELECT 'extra-'||x,cwd,source,project_id,has_user_event
         FROM n JOIN threads ON id='chat';
         ALTER TABLE threads ADD COLUMN model_provider TEXT DEFAULT 'openai';",
    )
    .unwrap();
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        br#"{"https://api.openai.com/auth":{"chatgpt_user_id":"user-b","chatgpt_account_id":"account-b"}}"#,
    );
    let auth = json!({"tokens":{"access_token":format!("e30.{token}.fixture")}});
    fs::write(f.home().join("config.toml"), "cli_auth_credentials_store = 'file'\n").unwrap();
    fs::write(f.home().join("auth.json"), serde_json::to_vec(&auth).unwrap()).unwrap();
    let owner = format!("{:x}", Sha256::digest(br#"["user-b","account-b"]"#));
    let original = json!({
        SCOPES:{"owner":"previous","inactive":{}},
        "local-projects":f.local("legacy")["local-projects"],
        "project-order":["legacy"],
        PINNED_PROJECTS:["legacy"],
        PINNED_THREADS:["chat"],
        THREAD_ORDERS:{"legacy":{"threadIds":["chat"]}},
        MAP:f.local("legacy")[MAP],
        ASSIGN:f.local("legacy")[ASSIGN],
        "remote-projects":{"remote-a":{"name":"Previous account only"}}
    });
    f.write(&original);

    assert!(
        crate::modules::codex_session_visibility::prepare_desktop_project_state_after_switch(
            "test-pure-switch",
            f.home(),
        ).unwrap()
    );
    let after_native_start = native_start(f.state(), Some(&owner));
    assert_eq!(after_native_start["local-projects"]["legacy"]["name"], "My project");
    assert_eq!(after_native_start["project-order"], json!(["legacy"]));
    assert_eq!(after_native_start[PINNED_PROJECTS], json!(["legacy"]));
    assert_eq!(after_native_start[PINNED_THREADS], json!(["chat"]));
    assert_eq!(after_native_start[THREAD_ORDERS]["legacy"]["threadIds"], json!(["chat"]));
    assert!(after_native_start.get("remote-projects").is_none());
    assert_eq!(
        after_native_start[SCOPES]["inactive"]["previous"]["globals"][ASSIGN]["chat"]["projectId"],
        "legacy"
    );
    assert_eq!(
        db.query_row("SELECT project_id FROM threads WHERE id='chat'", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "server"
    );

    f.write(&original);
    let launch = crate::modules::codex_session_visibility::repair_session_visibility_quick_for_instance(
        "test-switch-and-start",
        "Fixture",
        f.home(),
        true,
    )
    .unwrap();
    assert!(launch.local_project_recovery_deferred);
    assert_eq!(launch.mutated_instance_count, 1);
    let after_launch = native_start(f.state(), Some(&owner));
    assert_eq!(after_launch["project-order"], json!(["legacy"]));
    assert_eq!(after_launch[PINNED_PROJECTS], json!(["legacy"]));
}

#[test]
fn account_project_switch_api_key_roundtrip_preserves_oauth_archive_without_leaking_it() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state["remote-projects"] = json!({"cloud":"private-a"});
    state[PINNED_PROJECTS] = json!(["legacy", "cloud"]);
    state[ATOMS] = json!({"private-a":"private","thread-sort-key":"updated"});
    f.write(&state);
    prepare(f.home(), None).unwrap();
    let key = native_start(f.state(), None);
    assert!(key.get(SCOPES).is_none());
    assert!(key.get("remote-projects").is_none());
    assert_eq!(key[PINNED_PROJECTS], json!(["legacy"]));
    assert!(key[ATOMS].get("private-a").is_none());
    assert_eq!(key[ASSIGN]["chat"]["projectId"], "legacy");
    assert_eq!(
        key[PARKED]["inactive"]["a"]["globals"]["remote-projects"],
        state["remote-projects"]
    );
    f.write(&key);
    assert!(!prepare(f.home(), None).unwrap());

    // A relay scope prepared by an older build may have no pin list yet.
    let mut old_key = key;
    old_key.as_object_mut().unwrap().remove(PINNED_PROJECTS);
    f.write(&old_key);
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert!(back.get(PARKED).is_none());
    assert_eq!(back["remote-projects"], state["remote-projects"]);
    assert_eq!(back[PINNED_PROJECTS], state[PINNED_PROJECTS]);
    assert_eq!(back[ATOMS]["private-a"], "private");
    assert_eq!(back["local-projects"], state["local-projects"]);
    let files: Vec<_> = fs::read_dir(f.home())
        .unwrap()
        .map(|v| v.unwrap().file_name())
        .collect();
    assert_eq!(
        files.len(),
        3,
        "only workspace, database and original global state"
    );
}

#[test]
fn account_project_switch_keeps_target_alias_order_and_cloud_memberships() {
    let f = Fixture::new();
    let mut target = f.local("target-id");
    target["project-order"] = json!(["cloud-id", "target-id"]);
    target[PINNED_PROJECTS] = json!(["cloud-id"]);
    target["local-projects"]["cloud-id"] = json!({"id":"cloud-id","chatGptBacking":{}});
    target[ASSIGN]["cloud-thread"] = json!({"projectKind":"chatgpt","projectId":"cloud-id"});
    f.write(&json!({SCOPES:{"owner":"a","inactive":{
        "b":{"globals":target,"atoms":{"b-only":true}}
    }}, "local-projects":f.local("old-id")["local-projects"],
        PINNED_PROJECTS:["old-id", "source-cloud-id"]}));
    prepare(f.home(), Some("b")).unwrap();
    let actual = native_start(f.state(), Some("b"));
    assert_eq!(actual["project-order"], target["project-order"]);
    assert_eq!(actual[PINNED_PROJECTS], json!(["cloud-id", "target-id"]));
    assert!(actual["local-projects"].get("old-id").is_none());
    assert_eq!(actual[ASSIGN]["chat"]["projectId"], "target-id");
    assert_eq!(
        actual[ASSIGN]["cloud-thread"],
        target[ASSIGN]["cloud-thread"]
    );
}

#[test]
fn account_project_switch_keeps_an_explicit_unpin_across_scopes() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state[PINNED_PROJECTS] = json!(["server"]);
    f.write(&state);
    prepare(f.home(), None).unwrap();
    let mut key = native_start(f.state(), None);
    assert_eq!(key[PINNED_PROJECTS], json!(["legacy"]));

    // The official client persists an empty list when the last pin is removed.
    key[PINNED_PROJECTS] = json!([]);
    f.write(&key);
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert_eq!(back[PINNED_PROJECTS], json!([]));
    assert_eq!(back["local-projects"], state["local-projects"]);
}

#[test]
fn account_project_switch_reconciles_native_and_parked_archives_by_owner() {
    let f = Fixture::new();
    let mut state = f.local("current");
    state[ATOMS] = json!({"active-private":"native-current"});
    state[SCOPES] = json!({"owner":"b","inactive":{
        "c":{"atoms":{"inactive-private":"native-c"},"globals":{}}
    }});
    state[PARKED] = json!({"owner":null,"inactive":{
        "a":{"atoms":{"parked-private":"a"},"globals":f.local("original")},
        "b":{"atoms":{"active-private":"old-b","obsolete":true},"globals":{}},
        "c":{"atoms":{"inactive-private":"old-c","obsolete":true},"globals":{}}
    }});
    f.write(&state);

    assert!(prepare(f.home(), Some("a")).unwrap());
    let actual = native_start(f.state(), Some("a"));
    assert!(actual.get(PARKED).is_none());
    assert_eq!(actual[ATOMS], json!({"parked-private":"a"}));
    assert_eq!(actual[SCOPES]["inactive"]["b"]["atoms"], json!({"active-private":"native-current"}));
    assert_eq!(actual[SCOPES]["inactive"]["c"]["atoms"], json!({"inactive-private":"native-c"}));
    f.write(&actual);
    assert!(!prepare(f.home(), Some("a")).unwrap());
}

#[test]
fn account_project_switch_carries_only_verified_local_sidebar_presentation() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "ALTER TABLE threads ADD COLUMN thread_section_id TEXT;
         INSERT INTO threads SELECT 'chat-2',cwd,source,project_id,has_user_event,NULL FROM threads WHERE id='chat';
         INSERT INTO threads SELECT 'remote',cwd,source,project_id,has_user_event,NULL FROM threads WHERE id='chat';
         INSERT INTO threads SELECT 'migrated',cwd,source,project_id,has_user_event,NULL FROM threads WHERE id='chat';
         INSERT INTO threads SELECT 'native',cwd,source,project_id,has_user_event,'section' FROM threads WHERE id='chat';",
    ).unwrap();
    let mut target = f.local("target-id");
    let target_appearance = json!({"color":"blue","marker":{"kind":"icon","icon":"folder"}});
    target[PROJECT_APPEARANCES] = json!({"target-cloud":target_appearance});
    target[PINNED_THREADS] = json!(["target-remote"]);
    target[THREAD_ORDERS] = json!({"linked:target-cloud":{"threadIds":["target-remote"]}});
    let mut current = f.local("old-id");
    let current_appearance = json!({"color":"red","marker":{"kind":"emoji","emoji":"📁"}});
    current[PROJECT_APPEARANCES] = json!({"old-id":current_appearance,"source-cloud":target_appearance});
    current[HOSTS]["remote"] = json!("ssh-host");
    current[PINNED_THREADS] = json!(["chat-2","remote","child","migrated","native"]);
    current[MIGRATED_PINS] = json!({format!("local:{}",f.home().display()):["migrated"]});
    current[THREAD_ORDERS] = json!({
        "old-id":{"threadIds":["chat-2","remote","child","chat"]},
        "linked:source-cloud":{"threadIds":["remote"]}
    });
    current[SCOPES] = json!({"owner":"a","inactive":{"b":{"atoms":{},"globals":target}}});
    f.write(&current);

    assert!(prepare(f.home(), Some("b")).unwrap());
    let actual = native_start(f.state(), Some("b"));
    assert_eq!(actual[PROJECT_APPEARANCES], json!({
        "target-id":current_appearance,"target-cloud":target_appearance
    }));
    assert_eq!(actual[PINNED_THREADS], json!(["target-remote","chat-2"]));
    assert_eq!(actual[THREAD_ORDERS], json!({
        "target-id":{"threadIds":["chat-2","chat"]},
        "linked:target-cloud":{"threadIds":["target-remote"]}
    }));
    assert!(actual.get(MIGRATED_PINS).is_none());
}

#[test]
fn account_project_switch_excludes_cloud_missing_and_remote_only_records() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "INSERT INTO projects VALUES('cloud','Cloud','{\"chatGptBacking\":{}}');
        INSERT INTO projects VALUES('missing','Missing','{}');
        INSERT INTO project_roots VALUES('missing','/definitely/not/present',0);
        INSERT INTO threads VALUES('remote-chat','remote','cli','server',1);",
    )
    .unwrap();
    db.execute("INSERT INTO project_roots VALUES('cloud',?1,0)", [&f.root])
        .unwrap();
    let mut source = f.local("legacy");
    source[HOSTS]["remote-chat"] = json!("remote-host");
    source["remote-projects"] = json!({"remote-id":{}});
    f.write(&json!({SCOPES:{"owner":null,"inactive":{
        "a":{"globals":source,"atoms":{}}
    }}}));
    prepare(f.home(), Some("b")).unwrap();
    let actual = native_start(f.state(), Some("b"));
    assert_eq!(actual["local-projects"].as_object().unwrap().len(), 1);
    assert!(actual[ASSIGN].get("remote-chat").is_none());
    assert!(actual.get("remote-projects").is_none());
    assert!(actual[ASSIGN].get("child").is_none());
}

#[test]
fn account_project_switch_rejects_unknown_schema_without_writing() {
    let f = Fixture::new();
    let state = json!({SCOPES:{"owner":"a","inactive":{"bad":{"globals":[],"atoms":{}}}}});
    f.write(&state);
    let before = fs::read(f.home().join(STATE)).unwrap();
    assert!(prepare(f.home(), Some("b")).is_err());
    assert_eq!(fs::read(f.home().join(STATE)).unwrap(), before);
}

#[test]
fn account_project_switch_leaves_unscoped_legacy_state_untouched() {
    let f = Fixture::new();
    let mut legacy = f.local("legacy");
    legacy[ATOMS] = json!({"existing-preference": true});
    f.write(&legacy);
    assert!(!prepare(f.home(), Some("a")).unwrap());
    assert_eq!(f.state(), legacy);
}

#[test]
fn account_project_switch_respects_pending_deletion() {
    let f = Fixture::new();
    let mut source = f.local("legacy");
    source["app-server-pending-project-deletions-by-host"] = json!({
        format!("local:{}",f.home().display()): {"legacy":{"projectId":"server"}}
    });
    f.write(&json!({SCOPES:{"owner":null,"inactive":{
        "a":{"globals":source,"atoms":{}}
    }}}));
    prepare(f.home(), Some("b")).unwrap();
    let actual = native_start(f.state(), Some("b"));
    assert!(actual["local-projects"]
        .as_object()
        .is_none_or(|v| v.is_empty()));
    assert!(actual[ASSIGN].as_object().is_none_or(|v| v.is_empty()));
}

#[test]
fn account_project_switch_does_not_copy_groups_to_another_profile() {
    let source = Fixture::new();
    let other = Fixture::new();
    let source_globals = source.local("legacy");
    other.write(&json!({SCOPES:{"owner":null,"inactive":{
        "a":{"globals":source_globals,"atoms":{}}
    }}}));
    prepare(other.home(), Some("b")).unwrap();
    let actual = native_start(other.state(), Some("b"));
    assert!(actual["local-projects"].get("legacy").is_none());
    assert!(actual["local-projects"]
        .as_object()
        .is_none_or(|groups| groups.is_empty()));
}

#[test]
fn account_project_switch_preserves_large_supported_state_and_original_backup() {
    let f = Fixture::new();
    let state = json!({SCOPES:{"owner":"b","inactive":{
        "a":{"globals":f.local("legacy"),"atoms":{}}
    }}, "untouched": "x".repeat(2_500_000)});
    f.write(&state);
    let backup = f.home().join(format!("{STATE}.bak"));
    fs::write(&backup, b"previous backup").unwrap();
    assert!(prepare(f.home(), Some("b")).unwrap());
    assert_eq!(f.state()["untouched"], state["untouched"]);
    assert_eq!(f.state()["local-projects"]["legacy"]["name"], "My project");
    assert_eq!(fs::read(&backup).unwrap(), b"previous backup");
}

#[test]
fn account_project_switch_respects_launch_deadline_without_writing() {
    let f = Fixture::new();
    f.write(&json!({SCOPES:{"owner":"b","inactive":{
        "a":{"globals":f.local("legacy"),"atoms":{}}
    }}}));
    let before = fs::read(f.home().join(STATE)).unwrap();
    let result =
        super::super::super::with_launch_visibility_budget(std::time::Duration::ZERO, || {
            prepare(f.home(), Some("b"))
        });
    assert!(result
        .unwrap_err()
        .contains("CODEX_SESSION_VISIBILITY_TIMEOUT"));
    assert_eq!(fs::read(f.home().join(STATE)).unwrap(), before);
}

fn state_with_ssh(f: &Fixture) -> Value {
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","hostname":"fixture.invalid"}
    ]);
    state["remote-connection-auto-connect-by-host-id"] = json!({"ssh":true});
    state["selected-remote-host-id"] = json!("ssh");
    state["active-remote-project-id"] = json!("bookmark");
    state["remote-projects"] = json!([
        {"id":"bookmark","hostId":"ssh","label":"Before","remotePath":"/before"}
    ]);
    state
}

#[test]
fn large_history_recovers_saved_memberships_without_scanning_unreferenced_threads() {
    for project in [None, Some("server")] {
        let f = Fixture::new();
        f.write(&state_with_ssh(&f));
        let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
        db.execute("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<2046)
            INSERT INTO threads SELECT 'extra-'||x,'unused','cli',?1,1 FROM n;", [project]).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0)).unwrap(), 2049);
        drop(db);
        for owner in [Some("a"), Some("b"), None] {
            with_launch_visibility_budget(std::time::Duration::from_secs(5), || {
                prepare(f.home(), owner)
            })
            .unwrap();
            let booted = native_start(f.state(), owner);
            assert_eq!(booted[ASSIGN]["chat"]["projectId"], "legacy");
            assert!(booted[ASSIGN].get("extra-2046").is_none());
            assert!(booted["remote-projects"].as_array().is_some_and(|projects| !projects.is_empty()));
            f.write(&booted);
        }
    }
}

#[test]
fn large_project_registry_does_not_block_account_state_preparation() {
    let f = Fixture::new();
    f.write(&state_with_ssh(&f));
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<255)
        INSERT INTO projects SELECT 'extra-'||x,'Project','{}' FROM n;").unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM projects", [], |r| r.get::<_, i64>(0)).unwrap(), 257);
    drop(db);
    prepare(f.home(), Some("b")).unwrap();
    let booted = native_start(f.state(), Some("b"));
    assert_eq!(booted["local-projects"].as_object().unwrap().len(), 1);
    assert_eq!(booted[ASSIGN]["chat"]["projectId"], "legacy");
}

#[test]
fn slow_history_does_not_block_local_and_ssh_project_restoration() {
    let f = Fixture::new();
    let original = state_with_ssh(&f);
    f.write(&original);
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    // Synthesize a history whose ordered scan cannot finish within the launch
    // budget, without generating or writing millions of fixture rows.
    db.execute_batch(
        "ALTER TABLE threads RENAME TO fixture_threads;
         CREATE VIEW threads AS
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000)
         SELECT id,cwd,source,project_id,has_user_event FROM fixture_threads
         UNION ALL SELECT 'extra-'||x,'unused','cli','server',1 FROM n;",
    )
    .unwrap();
    drop(db);

    with_launch_visibility_budget(std::time::Duration::from_secs(2), || {
        prepare(f.home(), Some("b"))
    })
    .unwrap();
    let booted = native_start(f.state(), Some("b"));
    assert_eq!(booted["local-projects"], original["local-projects"]);
    assert_eq!(booted["remote-projects"], original["remote-projects"]);
    assert!(booted.get(ASSIGN).is_none());
    assert_eq!(
        booted[SCOPES]["inactive"]["a"]["globals"][ASSIGN],
        original[ASSIGN]
    );
}

#[test]
fn deleted_ssh_bookmark_stays_deleted_after_restart_and_account_switch() {
    let f = Fixture::new();
    f.write(&state_with_ssh(&f));
    prepare(f.home(), Some("b")).unwrap();
    let mut current = native_start(f.state(), Some("b"));
    current["remote-projects"] = json!([]);
    current["active-remote-project-id"] = Value::Null;
    f.write(&current);
    for owner in [Some("b"), Some("a"), None, Some("b")] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["remote-projects"], json!([]));
        assert!(booted["active-remote-project-id"].is_null());
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap());
    }
}

#[test]
fn renamed_ssh_bookmark_follows_latest_machine_state() {
    let f = Fixture::new();
    f.write(&state_with_ssh(&f));
    prepare(f.home(), Some("b")).unwrap();
    let mut current = native_start(f.state(), Some("b"));
    current["remote-projects"][0]["label"] = json!("After");
    current["remote-projects"][0]["remotePath"] = json!("/after");
    f.write(&current);
    for owner in [Some("a"), None, Some("b")] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["remote-projects"][0]["label"], "After");
        assert_eq!(booted["remote-projects"][0]["remotePath"], "/after");
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap());
    }
}

#[test]
fn removed_ssh_hosts_do_not_reappear_from_other_account_archives() {
    let f = Fixture::new();
    f.write(&state_with_ssh(&f));
    prepare(f.home(), Some("b")).unwrap();
    let mut current = native_start(f.state(), Some("b"));
    current["codex-managed-remote-connections"] = json!([]);
    current["remote-projects"] = json!([]);
    current["remote-connection-auto-connect-by-host-id"] = json!({});
    current["selected-remote-host-id"] = json!("local");
    current["active-remote-project-id"] = Value::Null;
    f.write(&current);
    for owner in [Some("b"), Some("a"), None, Some("b")] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["remote-projects"], json!([]));
        assert!(booted["remote-connection-auto-connect-by-host-id"].get("ssh").is_none());
        assert_eq!(booted["selected-remote-host-id"], "local");
        assert!(booted["active-remote-project-id"].is_null());
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap());
    }
}

fn add_remote_catalog(f: &Fixture) {
    fs::create_dir_all(f.home().join("sqlite")).unwrap();
    let db = Connection::open(f.home().join("sqlite/codex-dev.db")).unwrap();
    db.execute_batch("CREATE TABLE local_thread_catalog(host_id TEXT,thread_id TEXT,project_id TEXT,cwd TEXT,source_kind TEXT,source_detail TEXT);
        INSERT INTO local_thread_catalog VALUES('ssh','remote-chat',NULL,'/before','vscode',NULL);
        INSERT INTO local_thread_catalog VALUES('other-host','wrong-host',NULL,'/before','vscode',NULL);
        INSERT INTO local_thread_catalog VALUES('ssh','child',NULL,'/before','subagent',NULL);
        INSERT INTO local_thread_catalog VALUES('ssh','wrong-directory',NULL,'/elsewhere','vscode',NULL);").unwrap();
}

#[test]
fn discovered_ssh_alias_restores_bookmarks_order_and_verified_thread_memberships() {
    let f = Fixture::new();
    add_remote_catalog(&f);
    let mut archived = state_with_ssh(&f);
    archived["project-order"] = json!(["bookmark", "legacy"]);
    for id in ["remote-chat", "wrong-host", "child", "wrong-directory"] {
        archived[ASSIGN][id] = json!({"projectKind":"remote","projectId":"bookmark"});
        archived[HOSTS][id] = json!("ssh");
    }
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"current","inactive":{
        "old":{"globals":scoped_globals(&archived),"atoms":{}}
    }});
    // Real Desktop discovery records resolve SSH aliases later; hostname and
    // identity are null. The previous build consequently saved an empty marker.
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","alias":"work","hostname":null,"identity":null,"source":"discovered","sshPort":null}
    ]);
    state["cockpit-shared-ssh-hosts-v1"] = json!([]);
    f.write(&state);
    for owner in [Some("next"), None, Some("old")] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert_eq!(booted["remote-projects"][0]["id"], "bookmark");
        assert!(booted["project-order"].as_array().unwrap().contains(&json!("bookmark")));
        assert_eq!(booted[ASSIGN]["remote-chat"], json!({"projectKind":"remote","projectId":"bookmark"}));
        assert_eq!(booted[HOSTS]["remote-chat"], "ssh");
        for id in ["wrong-host", "child", "wrong-directory"] {
            assert_ne!(booted[ASSIGN][id]["projectKind"], "remote");
        }
        assert_eq!(booted[ASSIGN]["chat"]["projectKind"], "local");
        f.write(&booted);
        assert!(!prepare(f.home(), owner).unwrap());
    }
}

#[test]
fn adding_an_alias_host_does_not_restore_deleted_projects_on_initialized_hosts() {
    let f = Fixture::new();
    let mut archived = state_with_ssh(&f);
    archived["remote-projects"].as_array_mut().unwrap().push(json!({
        "id":"new-project","hostId":"new-ssh","label":"New","remotePath":"/new"
    }));
    archived["project-order"] = json!(["bookmark","new-project"]);
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"current","inactive":{
        "old":{"globals":scoped_globals(&archived),"atoms":{}}
    }});
    state["codex-managed-remote-connections"] = json!([
        {"hostId":"ssh","alias":"existing","hostname":null,"source":"discovered"},
        {"hostId":"new-ssh","alias":"new","hostname":null,"source":"discovered"}
    ]);
    state["cockpit-shared-ssh-hosts-v1"] = json!(["ssh"]);
    state["remote-projects"] = json!([]);
    f.write(&state);
    prepare(f.home(), Some("next")).unwrap();
    let booted = native_start(f.state(), Some("next"));
    assert_eq!(booted["remote-projects"].as_array().unwrap().len(), 1);
    assert_eq!(booted["remote-projects"][0]["id"], "new-project");
    assert!(!booted["project-order"].as_array().unwrap().contains(&json!("bookmark")));
}

#[test]
fn legacy_ssh_project_ids_for_the_same_bookmark_share_one_group() {
    let f = Fixture::new();
    add_remote_catalog(&f);
    let mut current = state_with_ssh(&f);
    current["project-order"] = json!(["legacy", "bookmark"]);
    let mut old = scoped_globals(&current);
    old["remote-projects"][0]["id"] = json!("older-id");
    old["project-order"] = json!(["older-id"]);
    old[ASSIGN]["remote-chat"] = json!({"projectKind":"remote","projectId":"older-id"});
    old[HOSTS]["remote-chat"] = json!("ssh");
    current[SCOPES]["inactive"]["old"] = json!({"globals":old,"atoms":{}});
    f.write(&current);
    prepare(f.home(), Some("next")).unwrap();
    let booted = native_start(f.state(), Some("next"));
    assert_eq!(booted["remote-projects"].as_array().unwrap().len(), 1);
    assert_eq!(booted["remote-projects"][0]["id"], "bookmark");
    assert_eq!(booted[ASSIGN]["remote-chat"]["projectId"], "bookmark");
    assert!(!booted["project-order"].as_array().unwrap().contains(&json!("older-id")));
}

#[test]
fn deleting_a_restored_ssh_project_removes_its_order_and_memberships() {
    let f = Fixture::new();
    add_remote_catalog(&f);
    let mut state = state_with_ssh(&f);
    state["project-order"] = json!(["bookmark", "legacy"]);
    state[ASSIGN]["remote-chat"] = json!({"projectKind":"remote","projectId":"bookmark"});
    state[HOSTS]["remote-chat"] = json!("ssh");
    f.write(&state);
    prepare(f.home(), Some("b")).unwrap();
    let mut current = native_start(f.state(), Some("b"));
    assert_eq!(current[ASSIGN]["remote-chat"]["projectKind"], "remote");
    current["remote-projects"] = json!([]);
    f.write(&current);
    for owner in [Some("b"), Some("a"), None] {
        prepare(f.home(), owner).unwrap();
        let booted = native_start(f.state(), owner);
        assert!(booted["remote-projects"].as_array().unwrap().is_empty());
        assert!(!booted["project-order"].as_array().unwrap().contains(&json!("bookmark")));
        assert!(booted[ASSIGN].get("remote-chat").is_none());
        assert!(booted[HOSTS].get("remote-chat").is_none());
        assert_eq!(booted[ASSIGN]["chat"]["projectKind"], "local");
        f.write(&booted);
    }
}
