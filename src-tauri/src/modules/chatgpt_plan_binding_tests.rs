use super::super::types::{ChatgptPlanAccountSummary, Credentials};
use super::*;
use crate::models::codex::CodexTokens;

struct TestData {
    directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    previous_server: Option<std::ffi::OsString>,
}
impl TestData {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        std::env::set_var("CHATGPT_PLAN_TEST_SERVER", "http://127.0.0.1:9");
        Self {
            directory,
            previous,
            previous_server,
        }
    }
    fn source_path(&self, id: &str) -> std::path::PathBuf {
        self.directory
            .path()
            .join("codex_accounts")
            .join(format!("{id}.json"))
    }
}
impl Drop for TestData {
    fn drop(&mut self) {
        for (key, previous) in [
            ("COCKPIT_TOOLS_TEST_DATA_DIR", &self.previous),
            ("CHATGPT_PLAN_TEST_SERVER", &self.previous_server),
        ] {
            if let Some(previous) = previous {
                std::env::set_var(key, previous);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn source(id: &str) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.into(),
        "User@example.invalid".into(),
        CodexTokens {
            id_token: "synthetic-codex-id".into(),
            access_token: "synthetic-codex-access".into(),
            refresh_token: Some("synthetic-codex-refresh".into()),
        },
    );
    account.user_id = Some("codex-private-user-id".into());
    account.account_id = Some("codex-workspace".into());
    account.organization_id = Some("codex-organization".into());
    account
}

fn registration() -> Registration {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let mut record = Registration {
        summary: ChatgptPlanAccountSummary {
            id,
            label: "Fixture".into(),
            email: Some("user@example.invalid".into()),
            subject: "siwc-client-specific-subject".into(),
            client_id: "oaiapp_synthetic".into(),
            source_account_id: None,
            source_binding_valid: false,
            source_binding_error: None,
            connected: false,
            needs_reauth: false,
            last_checked_at: None,
            check_error: None,
            last_service_mode: None,
            verified_at: Some(now),
            verified_model: Some("model".into()),
            expires_at: None,
            created_at: now,
            updated_at: now,
        },
        revision: uuid::Uuid::new_v4().to_string(),
        authorization_revision: uuid::Uuid::new_v4().to_string(),
        encrypted_credentials: None,
        revocation_confirmed: false,
        source_binding: None,
            source_binding_required: false,
            model_catalog: Vec::new(),
    };
    record
        .set_credentials(&Credentials {
            client_id: record.summary.client_id.clone(),
            subject: record.summary.subject.clone(),
            access_token: "synthetic-siwc-access".into(),
            refresh_token: "synthetic-siwc-refresh".into(),
            id_token: "synthetic-siwc-id".into(),
            scopes: vec![
                "offline_access".into(),
                "resource.invoke".into(),
                "chatgpt.tokens.use.direct".into(),
            ],
            expires_at: now + 3600,
            earliest_refresh_at: None,
        })
        .unwrap();
    storage::save(&record).unwrap();
    record
}

#[test]
fn old_registration_deserialization_is_unbound_and_never_has_inferred_identity() {
    let summary: ChatgptPlanAccountSummary = serde_json::from_value(serde_json::json!({
        "id":"fixture","label":"Fixture","email":"user@example.invalid","subject":"subject","client_id":"client",
        "connected":true,"needs_reauth":false,"verified_at":null,"verified_model":null,"expires_at":null,"created_at":1,"updated_at":1
    })).unwrap();
    assert!(summary.source_account_id.is_none());
    assert!(!summary.source_binding_valid);
    assert!(summary.source_binding_error.is_none());
    let record: Registration = serde_json::from_value(serde_json::json!({
        "summary":summary,"revision":"fixture","encrypted_credentials":null
    }))
    .unwrap();
    assert!(record.source_binding.is_none());
    let public = serde_json::to_value(&record.summary).unwrap();
    assert!(public.get("source_account_id").is_some());
    assert!(public.get("identity_fingerprint").is_none());
    assert!(public.get("workspace_id").is_none());
}

#[test]
fn source_fingerprint_tracks_identity_and_workspace_but_not_token_refresh() {
    let account = source("source");
    let original = source_identity(&account).unwrap().1;
    let mut refreshed = account.clone();
    refreshed.tokens.access_token = "rotated-access".into();
    refreshed.tokens.id_token = "rotated-id".into();
    refreshed.tokens.refresh_token = Some("rotated-refresh".into());
    refreshed.token_generation += 1;
    refreshed.requires_reauth = true;
    assert_eq!(source_identity(&refreshed).unwrap().1, original);
    refreshed.account_id = Some("other-workspace".into());
    assert_ne!(source_identity(&refreshed).unwrap().1, original);
    let mut unknown = account.clone();
    unknown.user_id = None;
    assert_eq!(
        source_identity(&unknown).unwrap_err(),
        "CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN"
    );
    for kind in ["apikey", "web", "grok"] {
        let mut unsupported = account.clone();
        match kind {
            "apikey" => unsupported.auth_mode = CodexAuthMode::Apikey,
            "web" => unsupported.token_source_mode = "chatgpt_web_session".into(),
            _ => unsupported.upstream_grok_account_id = Some("grok".into()),
        }
        assert_eq!(
            source_identity(&unsupported).unwrap_err(),
            "CHATGPT_PLAN_SOURCE_UNSUPPORTED"
        );
    }
}

#[tokio::test]
async fn explicit_binding_is_idempotent_one_to_one_and_does_not_modify_either_credential() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    codex_account::save_account(&source("first")).unwrap();
    let mut second = source("second");
    second.account_id = Some("other-workspace".into());
    codex_account::save_account(&second).unwrap();
    let record = registration();
    let other = registration();
    let source_bytes = std::fs::read(data.source_path("first")).unwrap();
    let bound = crate::commands::chatgpt_plan::chatgpt_plan_bind_account(
        record.summary.id.clone(),
        "first".into(),
    )
    .await
    .unwrap();
    assert_eq!(bound.source_account_id.as_deref(), Some("first"));
    assert!(bound.source_binding_valid);
    let stored = storage::read(&record.summary.id).await.unwrap();
    assert!(
        !stored.summary.source_binding_valid,
        "durable flags cannot substitute for current identity validation"
    );
    assert_eq!(stored.encrypted_credentials, record.encrypted_credentials);
    assert_eq!(
        std::fs::read(data.source_path("first")).unwrap(),
        source_bytes
    );
    assert_eq!(
        stored.summary.subject, "siwc-client-specific-subject",
        "never copy Codex private user/workspace identity into SIWC claims"
    );
    let revision = stored.authorization_revision;
    let duplicate = bind_account(&record.summary.id, "first").await.unwrap();
    assert!(duplicate.source_binding_valid);
    assert_eq!(
        storage::read(&record.summary.id)
            .await
            .unwrap()
            .authorization_revision,
        revision
    );
    assert_eq!(
        bind_account(&record.summary.id, "second")
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_BINDING_CONFLICT"
    );
    assert_eq!(
        bind_account(&other.summary.id, "first").await.unwrap_err(),
        "CHATGPT_PLAN_BINDING_CONFLICT"
    );
    let unbound = unbind_account(&record.summary.id, "first").await.unwrap();
    assert!(unbound.source_account_id.is_none());
    assert!(
        bind_account(&other.summary.id, "first")
            .await
            .unwrap()
            .source_binding_valid
    );
}

#[tokio::test]
async fn binding_rejects_wrong_unknown_or_disconnected_identity_without_mutating_registration() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _data = TestData::new();
    let mut wrong = source("wrong");
    wrong.email = "other@example.invalid".into();
    codex_account::save_account(&wrong).unwrap();
    let mut unknown = source("unknown");
    unknown.account_id = None;
    codex_account::save_account(&unknown).unwrap();
    let record = registration();
    let original = record.encrypted_credentials.clone();
    assert_eq!(
        bind_account(&record.summary.id, "wrong").await.unwrap_err(),
        "CHATGPT_PLAN_SOURCE_EMAIL_MISMATCH"
    );
    assert_eq!(
        bind_account(&record.summary.id, "unknown")
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN"
    );
    assert_eq!(
        bind_account(&record.summary.id, "../source")
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_SOURCE_NOT_FOUND"
    );
    assert!(storage::read(&record.summary.id)
        .await
        .unwrap()
        .source_binding
        .is_none());
    assert_eq!(
        storage::read(&record.summary.id)
            .await
            .unwrap()
            .encrypted_credentials,
        original
    );
    let mut disconnected = record;
    disconnected.summary.connected = false;
    storage::save(&disconnected).unwrap();
    assert_eq!(
        bind_account(&disconnected.summary.id, "wrong")
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_DISCONNECTED"
    );
}

#[tokio::test]
async fn concurrent_bindings_cannot_assign_one_source_or_one_grant_twice() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _data = TestData::new();
    for id in ["first", "second"] {
        codex_account::save_account(&source(id)).unwrap();
    }
    let first = registration();
    let second = registration();
    let (a, b) = tokio::join!(
        bind_account(&first.summary.id, "first"),
        bind_account(&second.summary.id, "first")
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        a.as_ref().err().or(b.as_ref().err()).unwrap(),
        "CHATGPT_PLAN_BINDING_CONFLICT"
    );
    let third = registration();
    let (a, b) = tokio::join!(
        bind_account(&third.summary.id, "second"),
        bind_account(&third.summary.id, "first")
    );
    assert!(a.is_ok());
    assert_eq!(b.unwrap_err(), "CHATGPT_PLAN_BINDING_CONFLICT");
}

#[tokio::test]
async fn bound_getters_detect_source_deletion_or_reuse_while_refresh_preserves_association() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    let mut account = source("source");
    codex_account::save_account(&account).unwrap();
    let record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    assert_eq!(
        super::super::inference_token(&record.summary.id, false)
            .await
            .unwrap(),
        "synthetic-siwc-access"
    );
    account.tokens.access_token = "new-codex-access".into();
    account.tokens.id_token = "new-codex-id".into();
    account.tokens.refresh_token = Some("new-codex-refresh".into());
    account.token_generation += 1;
    codex_account::save_account(&account).unwrap();
    assert!(
        validate_source_binding(&record.summary.id, "source")
            .await
            .unwrap()
            .source_binding_valid
    );
    assert_eq!(
        super::super::inference_token(&record.summary.id, false)
            .await
            .unwrap(),
        "synthetic-siwc-access"
    );
    std::fs::remove_file(data.source_path("source")).unwrap();
    let summary = super::super::get_account(&record.summary.id).await.unwrap();
    assert!(!summary.source_binding_valid);
    assert_eq!(
        summary.source_binding_error.as_deref(),
        Some("CHATGPT_PLAN_SOURCE_NOT_FOUND")
    );
    assert_eq!(
        super::super::inference_token(&record.summary.id, false)
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_SOURCE_NOT_FOUND"
    );
    account.account_id = Some("reused-id-other-workspace".into());
    codex_account::save_account(&account).unwrap();
    assert_eq!(
        super::super::inference_token(&record.summary.id, false)
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_SOURCE_CHANGED"
    );
    let summary = super::super::get_account(&record.summary.id).await.unwrap();
    assert!(!summary.source_binding_valid);
    assert_eq!(
        summary.source_binding_error.as_deref(),
        Some("CHATGPT_PLAN_SOURCE_CHANGED")
    );
}

#[tokio::test]
async fn disconnected_or_scope_disabled_grants_keep_identity_association_but_cannot_infer() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _data = TestData::new();
    codex_account::save_account(&source("source")).unwrap();
    let mut record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    record = storage::read(&record.summary.id).await.unwrap();
    let mut credentials = record.credentials().unwrap();
    credentials
        .scopes
        .retain(|scope| scope != "chatgpt.tokens.use.direct");
    record.set_credentials(&credentials).unwrap();
    record.summary.needs_reauth = true;
    storage::save(&record).unwrap();
    assert!(
        super::super::get_account(&record.summary.id)
            .await
            .unwrap()
            .source_binding_valid
    );
    assert_eq!(
        super::super::inference_token(&record.summary.id, false)
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_PERMISSION_REQUIRED"
    );
    record.encrypted_credentials = None;
    record.summary.connected = false;
    storage::save(&record).unwrap();
    let summary = super::super::get_account(&record.summary.id).await.unwrap();
    assert!(summary.source_binding_valid);
    assert_eq!(summary.source_account_id.as_deref(), Some("source"));
    assert_eq!(
        validate_source_binding(&record.summary.id, "source")
            .await
            .unwrap_err(),
        "CHATGPT_PLAN_DISCONNECTED"
    );
}

#[tokio::test]
async fn list_cache_invalidates_changed_metadata_and_caches_failed_validation() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    let mut account = source("source");
    codex_account::save_account(&account).unwrap();
    let record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    let bound = storage::read(&record.summary.id).await.unwrap();
    assert!(
        cache::cached_summary(&bound)
            .unwrap()
            .0
            .source_binding_valid
    );
    account.account_id = Some("another-workspace".into());
    codex_account::save_account(&account).unwrap();
    let (pending, job) = cache::cached_summary(&bound).unwrap();
    assert!(
        !pending.source_binding_valid,
        "metadata changes must not keep an old positive identity cache"
    );
    let job = job.unwrap();
    let outcome = association_outcome(&bound);
    let (failed, changed) = cache::finish(job, outcome).unwrap();
    assert!(changed);
    assert_eq!(
        failed.source_binding_error.as_deref(),
        Some("CHATGPT_PLAN_SOURCE_CHANGED")
    );
    let (cached, retry) = cache::cached_summary(&bound).unwrap();
    assert!(!cached.source_binding_valid);
    assert!(
        retry.is_none(),
        "failed identity validation must not create a list/event retry loop"
    );
    assert!(std::fs::read_to_string(data.source_path("source"))
        .unwrap()
        .contains("ciphertext"));
}

#[tokio::test]
async fn background_list_validation_recovers_refresh_and_caches_missing_source() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    let mut account = source("source");
    codex_account::save_account(&account).unwrap();
    let record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    account.tokens.access_token = "refreshed-and-longer-access".into();
    codex_account::save_account(&account).unwrap();
    let pending = super::super::list_accounts().await.unwrap();
    assert!(!pending[0].source_binding_valid);
    cache::wait_for_idle().await;
    let current = super::super::list_accounts().await.unwrap();
    assert!(current[0].source_binding_valid);
    assert!(current[0].source_binding_error.is_none());
    std::fs::remove_file(data.source_path("source")).unwrap();
    assert!(!super::super::list_accounts().await.unwrap()[0].source_binding_valid);
    cache::wait_for_idle().await;
    let failed = super::super::list_accounts().await.unwrap();
    assert_eq!(
        failed[0].source_binding_error.as_deref(),
        Some("CHATGPT_PLAN_SOURCE_NOT_FOUND")
    );
    let reloaded = storage::read(&record.summary.id).await.unwrap();
    assert!(cache::cached_summary(&reloaded).unwrap().1.is_none());
    cache::wait_for_idle().await;
}

#[tokio::test]
async fn service_commit_snapshot_rejects_source_replacement_and_reauthorization_but_allows_refresh()
{
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _data = TestData::new();
    let mut account = source("source");
    codex_account::save_account(&account).unwrap();
    let record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    let snapshot = validate_source_binding_snapshot(&record.summary.id, "source")
        .await
        .unwrap();
    account.tokens.access_token = "refreshed-access".into();
    account.tokens.refresh_token = Some("refreshed-refresh".into());
    codex_account::save_account(&account).unwrap();
    assert!(validate_source_binding_snapshot_for_commit(&snapshot).is_ok());
    let mut refreshed_record = storage::read(&record.summary.id).await.unwrap();
    let mut credentials = refreshed_record.credentials().unwrap();
    credentials.access_token = "refreshed-siwc-access".into();
    refreshed_record.set_credentials(&credentials).unwrap();
    storage::save(&refreshed_record).unwrap();
    assert!(validate_source_binding_snapshot_for_commit(&snapshot).is_ok());
    account.user_id = Some("replacement-user".into());
    codex_account::save_account(&account).unwrap();
    assert_eq!(
        validate_source_binding_snapshot_for_commit(&snapshot).unwrap_err(),
        "CHATGPT_PLAN_SOURCE_CHANGED"
    );
    account.user_id = Some("codex-private-user-id".into());
    codex_account::save_account(&account).unwrap();
    refreshed_record.authorization_revision = uuid::Uuid::new_v4().to_string();
    storage::save(&refreshed_record).unwrap();
    assert_eq!(
        validate_source_binding_snapshot_for_commit(&snapshot).unwrap_err(),
        "CHATGPT_PLAN_REGISTRATION_CHANGED"
    );
    refreshed_record.summary.verified_at = None;
    refreshed_record.summary.verified_model = None;
    storage::save(&refreshed_record).unwrap();
    assert_eq!(validate_source_binding_snapshot_for_commit(&snapshot).unwrap_err(), "CHATGPT_PLAN_REGISTRATION_CHANGED");
    let untested = validate_source_binding_snapshot(&record.summary.id, "source").await.unwrap();
    assert!(validate_source_binding_snapshot_for_commit(&untested).is_ok(), "valid consent does not require a model test");

}

#[tokio::test]
async fn codex_return_snapshot_ignores_official_token_state_but_rejects_source_reuse_and_unbind() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    let mut account = source("source");
    codex_account::save_account(&account).unwrap();
    let record = registration();
    bind_account(&record.summary.id, "source").await.unwrap();
    let before_disconnect = validate_source_association_snapshot(&record.summary.id, "source")
        .await
        .unwrap();
    let mut disconnected = storage::read(&record.summary.id).await.unwrap();
    disconnected.encrypted_credentials = None;
    disconnected.summary.connected = false;
    disconnected.summary.needs_reauth = true;
    disconnected.summary.verified_at = None;
    disconnected.summary.verified_model = None;
    disconnected.authorization_revision = uuid::Uuid::new_v4().to_string();
    storage::save(&disconnected).unwrap();
    assert!(validate_source_association_snapshot_for_commit(&before_disconnect).is_ok());
    let snapshot = validate_source_association_snapshot(&record.summary.id, "source")
        .await
        .unwrap();
    assert!(validate_source_association_snapshot_for_commit(&snapshot).is_ok());
    account.tokens.refresh_token = None;
    account.tokens.access_token = "access-only-original-codex".into();
    codex_account::save_account(&account).unwrap();
    assert!(validate_source_association_snapshot_for_commit(&snapshot).is_ok());
    account.account_id = Some("reused-local-id-workspace".into());
    codex_account::save_account(&account).unwrap();
    assert_eq!(
        validate_source_association_snapshot_for_commit(&snapshot).unwrap_err(),
        "CHATGPT_PLAN_SOURCE_CHANGED"
    );
    std::fs::remove_file(data.source_path("source")).unwrap();
    assert_eq!(
        validate_source_association_snapshot_for_commit(&snapshot).unwrap_err(),
        "CHATGPT_PLAN_SOURCE_NOT_FOUND"
    );
    unbind_account(&record.summary.id, "source").await.unwrap();
    assert_eq!(
        validate_source_association_snapshot_for_commit(&snapshot).unwrap_err(),
        "CHATGPT_PLAN_REGISTRATION_CHANGED"
    );
}
