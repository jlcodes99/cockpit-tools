use super::*;
use crate::models::codex::{CodexAgentIdentity, CodexTokens};
use serde_json::json;

fn account(id: &str) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.into(),
        "fixture@example.invalid".into(),
        CodexTokens {
            access_token: "fixture-access".into(),
            id_token: "fixture-id".into(),
            refresh_token: Some("fixture-refresh".into()),
        },
    );
    account.user_id = Some("fixture-principal".into());
    account.account_id = Some("fixture-workspace".into());
    account
}

struct Store(std::path::PathBuf);
impl Store {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("cockpit-account-installation-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.join(STORE_FILE))
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn installation_survives_restart_refresh_and_engine_projection() {
    let store = Store::new();
    let mut account = account("a");
    let initial = resolve_at(&store.0, &account).unwrap();
    assert_eq!(
        Uuid::parse_str(&initial.installation_id)
            .unwrap()
            .get_version_num(),
        4
    );
    account.tokens.access_token = "refreshed-access".into();
    account.tokens.id_token = "refreshed-id".into();
    account.token_generation += 1;
    account.egress_proxy_url = Some("http://fixture:1234".into());
    let resumed = resolve_at(&store.0, &account).unwrap();
    assert_eq!(initial, resumed);
    let manifest = serde_json::to_value(&resumed).unwrap();
    assert_eq!(manifest["installationId"], initial.installation_id);
    assert_eq!(manifest["scope"], initial.scope);
    assert!(manifest.get("installation_id").is_none());
    assert_eq!(
        serde_json::from_value::<InstallationProfile>(manifest).unwrap(),
        initial
    );
    let raw = std::fs::read(&store.0).unwrap();
    assert!(!raw
        .windows("fixture-access".len())
        .any(|bytes| bytes == b"fixture-access"));
    assert!(!raw
        .windows("fixture-principal".len())
        .any(|bytes| bytes == b"fixture-principal"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&store.0).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn principal_workspace_key_and_realm_changes_isolate_installations() {
    let store = Store::new();
    let a = account("a");
    let initial = resolve_at(&store.0, &a).unwrap();
    let mut variants = vec![account("b")];
    let mut changed = a.clone();
    changed.user_id = Some("other-principal".into());
    variants.push(changed);
    let mut changed = a.clone();
    changed.account_id = Some("other-workspace".into());
    variants.push(changed);
    let mut api = CodexAccount::new_api_key(
        "api".into(),
        "fixture@example.invalid".into(),
        "first-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://fixture.invalid/v1".into()),
        None,
        None,
        vec![],
    );
    let first_api = resolve_at(&store.0, &api).unwrap();
    api.openai_api_key = Some("second-key".into());
    let second_api = resolve_at(&store.0, &api).unwrap();
    assert_ne!(first_api.scope, second_api.scope);
    assert_ne!(first_api.installation_id, second_api.installation_id);
    api.api_base_url = Some("https://other.invalid/v1".into());
    assert_ne!(second_api, resolve_at(&store.0, &api).unwrap());
    for changed in variants {
        let profile = resolve_at(&store.0, &changed).unwrap();
        assert_ne!(initial.scope, profile.scope);
        assert_ne!(initial.installation_id, profile.installation_id);
    }
    assert_eq!(initial, resolve_at(&store.0, &a).unwrap());
}

#[test]
fn equivalent_provider_urls_keep_one_installation() {
    let store = Store::new();
    let mut a = account("api");
    a.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    a.openai_api_key = Some("fixture-api-key".into());
    let first = resolve_at(&store.0, &a).unwrap();
    for url in [
        None,
        Some(""),
        Some("https://api.openai.com/v1"),
        Some("https://API.OPENAI.COM:443/v1/"),
    ] {
        a.api_base_url = url.map(str::to_owned);
        assert_eq!(first, resolve_at(&store.0, &a).unwrap());
    }
}

#[test]
fn agent_tasks_and_grok_owner_enrichment_do_not_rotate_installations() {
    let store = Store::new();
    let mut a = account("agent");
    a.agent_identity = Some(CodexAgentIdentity {
        agent_runtime_id: "runtime".into(),
        agent_private_key: "fixture-key".into(),
        task_id: Some("task-a".into()),
        account_id: "workspace".into(),
        chatgpt_user_id: "principal".into(),
        chatgpt_account_is_fedramp: false,
        email: None,
        plan_type: None,
    });
    let first = resolve_at(&store.0, &a).unwrap();
    a.agent_identity.as_mut().unwrap().task_id = Some("task-b".into());
    assert_eq!(first, resolve_at(&store.0, &a).unwrap());
    a.agent_identity.as_mut().unwrap().chatgpt_user_id = "other-user".into();
    assert_ne!(first, resolve_at(&store.0, &a).unwrap());
    let mut grok = account("grok");
    grok.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    grok.upstream_grok_account_id = Some("grok-source".into());
    let first = resolve_at(&store.0, &grok).unwrap();
    grok.user_id = Some("resolved-grok-principal".into());
    grok.account_id = Some("resolved-grok-team".into());
    assert_eq!(first, resolve_at(&store.0, &grok).unwrap());
}

#[test]
fn concurrent_creation_commits_one_identity_and_corruption_never_rotates_it() {
    let store = Store::new();
    let a = account("a");
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let path = store.0.clone();
            let a = a.clone();
            std::thread::spawn(move || resolve_at(&path, &a).unwrap())
        })
        .collect();
    let profiles: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(profiles.iter().all(|p| p == &profiles[0]));
    let connection = Connection::open(&store.0).unwrap();
    connection
        .execute(
            "UPDATE account_installation_v1 SET installation_id='broken'",
            [],
        )
        .unwrap();
    assert!(resolve_at(&store.0, &a).is_err());
    let raw: String = connection
        .query_row(
            "SELECT installation_id FROM account_installation_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, "broken");
    connection.execute_batch("PRAGMA user_version=99;").unwrap();
    assert!(resolve_at(&store.0, &account("new")).is_err());
    let count: u32 = connection
        .query_row("SELECT count(*) FROM account_installation_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn existing_installation_reads_do_not_wait_for_other_account_writes() {
    let store = Store::new();
    let a = account("a");
    let initial = resolve_at(&store.0, &a).unwrap();
    let mut writer = Connection::open(&store.0).unwrap();
    let transaction = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction.execute("INSERT INTO account_installation_v1(scope,installation_id,revision) VALUES('another-scope',?1,1)", [Uuid::new_v4().to_string()]).unwrap();
    assert_eq!(initial, resolve_at(&store.0, &a).unwrap());
    drop(transaction);
}

#[test]
fn only_known_installation_fields_change_and_sessions_tools_and_cipher_remain() {
    let store = Store::new();
    let profile = resolve_at(&store.0, &account("a")).unwrap();
    let canonical = json!({"installation_id":"client-installation","thread_id":"child-thread","parent_thread_id":"parent-thread","note":"中文🙂"});
    let mut headers = HeaderMap::new();
    headers.append(INSTALLATION_HEADER, HeaderValue::from_static("client-a"));
    headers.append(INSTALLATION_HEADER, HeaderValue::from_static("client-b"));
    headers.insert("session-id", HeaderValue::from_static("session"));
    headers.insert("thread-id", HeaderValue::from_static("thread"));
    headers.insert(
        METADATA_HEADER,
        HeaderValue::from_bytes(serde_json::to_string(&canonical).unwrap().as_bytes()).unwrap(),
    );
    let mut body = json!({"installation_id":"client-a","prompt_cache_key":"cache","previous_response_id":"response",
        "client_metadata":{"x-codex-installation-id":"client-b", "x-codex-turn-metadata":canonical.to_string()},
        "input":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","arguments":"{\"installation_id\":\"tool-installation\",\"message\":\"cipher\"}"}],
        "tools":[{"type":"function","name":"installation_id","parameters":{"installation_id":"business-value"}}]});
    let original = body.clone();
    apply_headers(&profile, &mut headers).unwrap();
    apply_body(&profile, &mut body).unwrap();
    assert_eq!(headers.get_all(INSTALLATION_HEADER).iter().count(), 1);
    assert_eq!(headers[INSTALLATION_HEADER], profile.installation_id);
    assert_eq!(headers["session-id"], "session");
    assert_eq!(headers["thread-id"], "thread");
    let header_metadata: Value =
        serde_json::from_str(headers[METADATA_HEADER].to_str().unwrap()).unwrap();
    let body_metadata: Value =
        serde_json::from_str(body["client_metadata"][METADATA_HEADER].as_str().unwrap()).unwrap();
    assert_eq!(header_metadata, body_metadata);
    assert_eq!(header_metadata["installation_id"], profile.installation_id);
    assert_eq!(header_metadata["note"], canonical["note"]);
    assert_eq!(header_metadata["parent_thread_id"], "parent-thread");
    for key in ["input", "tools", "previous_response_id", "prompt_cache_key"] {
        assert_eq!(body[key], original[key]);
    }
    assert_eq!(body["installation_id"], profile.installation_id);
}
