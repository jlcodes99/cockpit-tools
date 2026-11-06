use super::*;

struct Fixture { temp: tempfile::TempDir, root: String }
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let f = Self { temp, root: root.to_string_lossy().into() };
        let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT,metadata TEXT);
            CREATE TABLE project_roots(project_id TEXT,path TEXT,position INTEGER);
            CREATE TABLE threads(id TEXT PRIMARY KEY,cwd TEXT,source TEXT,project_id TEXT,has_user_event INTEGER);
            INSERT INTO projects VALUES('server','Project','{}');").unwrap();
        db.execute("INSERT INTO project_roots VALUES('server',?1,0)", [&f.root]).unwrap();
        db.execute("INSERT INTO threads VALUES('chat',?1,'cli','server',1)", [&f.root]).unwrap();
        db.execute("INSERT INTO threads VALUES('child',?1,'subagent','server',1)", [&f.root]).unwrap();
        f
    }
    fn home(&self) -> &Path { self.temp.path() }
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
    let Some(mut scopes) = state.get(SCOPES).cloned() else { return; };
    if owner.is_some() && scopes["owner"].as_str() == owner { return; }
    let atoms = object(&state[ATOMS]);
    if let Some(current) = scopes["owner"].as_str().map(str::to_string) {
        scopes["inactive"][&current] = json!({
            "globals":scoped_globals(state),
            "atoms":atoms.iter().filter(|(key,_)| !SHARED_ATOMS.contains(&key.as_str()))
                .map(|(key,value)| (key.clone(),value.clone())).collect::<BTreeMap<_,_>>(),
        });
    }
    let target = owner.and_then(|id| scopes["inactive"].get(id)).cloned()
        .unwrap_or_else(|| json!({"globals":{},"atoms":{}}));
    for key in SCOPED_GLOBALS {
        state.as_object_mut().unwrap().remove(*key);
        if let Some(value) = target["globals"].get(*key) { state[*key] = value.clone(); }
    }
    let mut next_atoms = object(&target["atoms"]);
    next_atoms.extend(atoms.into_iter().filter(|(key,_)| SHARED_ATOMS.contains(&key.as_str())));
    state[ATOMS] = json!(next_atoms);
    scopes["owner"] = json!(owner);
    if let Some(owner) = owner { scopes["inactive"].as_object_mut().unwrap().remove(owner); }
    state[SCOPES] = scopes;
}
fn native_start(mut state: Value, owner: Option<&str>) -> Value {
    native_select(&mut state, None);
    if owner.is_some() { native_select(&mut state, owner); }
    state
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
    assert_eq!(actual[SCOPES]["inactive"]["a"]["globals"]["remote-projects"],
        old["remote-projects"]);
    f.write(&actual);
    assert!(!prepare(f.home(), Some("b")).unwrap(), "same-account startup is idempotent");
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert_eq!(back["remote-projects"], old["remote-projects"]);
    assert_eq!(back[ATOMS]["private-a"], true);
    assert!(back[ATOMS].get("private-b").is_none());
    assert_eq!(back[ASSIGN]["chat"]["projectId"], "legacy");
}

#[test]
fn account_project_switch_api_key_roundtrip_preserves_oauth_archive_without_leaking_it() {
    let f = Fixture::new();
    let mut state = f.local("legacy");
    state[SCOPES] = json!({"owner":"a","inactive":{}});
    state["remote-projects"] = json!({"cloud":"private-a"});
    state[ATOMS] = json!({"private-a":"private","thread-sort-key":"updated"});
    f.write(&state);
    prepare(f.home(), None).unwrap();
    let key = native_start(f.state(), None);
    assert!(key.get(SCOPES).is_none());
    assert!(key.get("remote-projects").is_none());
    assert!(key[ATOMS].get("private-a").is_none());
    assert_eq!(key[ASSIGN]["chat"]["projectId"], "legacy");
    assert_eq!(key[PARKED]["inactive"]["a"]["globals"]["remote-projects"], state["remote-projects"]);
    f.write(&key);
    assert!(!prepare(f.home(), None).unwrap());
    prepare(f.home(), Some("a")).unwrap();
    let back = native_start(f.state(), Some("a"));
    assert!(back.get(PARKED).is_none());
    assert_eq!(back["remote-projects"], state["remote-projects"]);
    assert_eq!(back[ATOMS]["private-a"], "private");
    assert_eq!(back["local-projects"], state["local-projects"]);
    let files: Vec<_> = fs::read_dir(f.home()).unwrap().map(|v| v.unwrap().file_name()).collect();
    assert_eq!(files.len(), 3, "only workspace, database and original global state");
}

#[test]
fn account_project_switch_keeps_target_alias_order_and_cloud_memberships() {
    let f = Fixture::new();
    let mut target = f.local("target-id");
    target["project-order"] = json!(["cloud-id","target-id"]);
    target["local-projects"]["cloud-id"] = json!({"id":"cloud-id","chatGptBacking":{}});
    target[ASSIGN]["cloud-thread"] = json!({"projectKind":"chatgpt","projectId":"cloud-id"});
    f.write(&json!({SCOPES:{"owner":"a","inactive":{
        "b":{"globals":target,"atoms":{"b-only":true}}
    }}, "local-projects":f.local("old-id")["local-projects"]}));
    prepare(f.home(), Some("b")).unwrap();
    let actual = native_start(f.state(), Some("b"));
    assert_eq!(actual["project-order"], target["project-order"]);
    assert!(actual["local-projects"].get("old-id").is_none());
    assert_eq!(actual[ASSIGN]["chat"]["projectId"], "target-id");
    assert_eq!(actual[ASSIGN]["cloud-thread"], target[ASSIGN]["cloud-thread"]);
}

#[test]
fn account_project_switch_excludes_cloud_missing_and_remote_only_records() {
    let f = Fixture::new();
    let db = Connection::open(f.home().join("state_5.sqlite")).unwrap();
    db.execute_batch("INSERT INTO projects VALUES('cloud','Cloud','{\"chatGptBacking\":{}}');
        INSERT INTO projects VALUES('missing','Missing','{}');
        INSERT INTO project_roots VALUES('missing','/definitely/not/present',0);
        INSERT INTO threads VALUES('remote-chat','remote','cli','server',1);").unwrap();
    db.execute("INSERT INTO project_roots VALUES('cloud',?1,0)", [&f.root]).unwrap();
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
fn account_project_switch_initializes_legacy_profile_without_extra_files() {
    let f = Fixture::new();
    f.write(&f.local("legacy"));
    prepare(f.home(), Some("a")).unwrap();
    let actual = native_start(f.state(), Some("a"));
    assert_eq!(actual["local-projects"]["legacy"]["name"], "My project");
    assert_eq!(actual[ASSIGN]["chat"]["projectId"], "legacy");
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
    assert!(actual["local-projects"].as_object().is_none_or(|v| v.is_empty()));
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
    assert_eq!(actual["local-projects"]["server"]["rootPaths"], json!([other.root]));
}
