use super::*;
use crate::modules::{
    chatgpt_plan_service as service, codex_account, codex_local_access as access,
};

#[path = "chatgpt_plan_service_member_lifecycle_tests.rs"]
mod service_members;
#[path = "chatgpt_plan_canonical_members_tests.rs"]
mod canonical_members;

pub(super) fn lifecycle_source(id: &str) -> crate::models::codex::CodexAccount {
    let mut source = crate::models::codex::CodexAccount::new(
        id.into(),
        "test@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "synthetic-codex-access".into(),
            id_token: "synthetic-codex-id".into(),
            refresh_token: Some("synthetic-codex-refresh".into()),
        },
    );
    source.user_id = Some("source-user".into());
    source.account_id = Some("source-workspace".into());
    source.plan_type = Some("plus".into());
    codex_account::save_account(&source).unwrap();
    source
}

fn authorization_case<F, Fut>(operation: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(access::with_chatgpt_plan_test_runtime(operation));
}

#[test]
fn authorization_join_remove_rejoin_reuses_consent_without_models_or_responses() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("consent-source");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        let before = storage::read(&grant.id).await.unwrap();
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        // Even a failed catalog cannot become a prerequisite for activation.
        fixture.state.lock().unwrap().model_status = Some(503);
        service::set_service_mode(&source.id, Some(&grant.id), "official", None)
            .await
            .unwrap();
        service::remove_from_service(&grant.id).await.unwrap();
        let removed = get_account(&grant.id).await.unwrap();
        assert!(removed.connected && !removed.needs_reauth);
        assert_eq!(removed.last_service_mode.as_deref(), Some("official"));
        assert!(removed.verified_at.is_none() && removed.verified_model.is_none());
        service::join(&grant.id, "").await.unwrap();
        let after = storage::read(&grant.id).await.unwrap();
        assert_eq!(after.authorization_revision, before.authorization_revision);
        assert_eq!(
            after.credentials().unwrap().access_token,
            before.credentials().unwrap().access_token
        );
        assert_eq!(
            after.credentials().unwrap().refresh_token,
            before.credentials().unwrap().refresh_token
        );
        assert_eq!(fixture.state.lock().unwrap().refresh_count, 0);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        assert!(inference_token(&grant.id, false).await.is_ok());
        assert!(!access::get_local_access_state().await.unwrap().running);
        service::disconnect(&grant.id).await.unwrap();
        assert_eq!(
            codex_account::load_account(&source.id)
                .unwrap()
                .tokens
                .access_token,
            source.tokens.access_token
        );
    });
}

#[test]
fn authorization_check_is_non_generating_and_distinguishes_transient_errors_from_reauth() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("checked-binding-source");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        let checked = service::check_authorization(&grant.id).await.unwrap();
        assert!(checked.connected && !checked.needs_reauth && checked.last_checked_at.is_some());
        assert!(checked.source_binding_valid);
        assert!(checked.source_binding_error.is_none());
        assert!(checked.check_error.is_none());
        assert!(checked.verified_at.is_none());
        for (status, error) in [
            (503, "CHATGPT_PLAN_NETWORK"),
            (429, "CHATGPT_PLAN_USAGE_LIMIT"),
            (403, "CHATGPT_PLAN_CONFIGURATION"),
        ] {
            fixture.state.lock().unwrap().model_status = Some(status);
            let checked = service::check_authorization(&grant.id).await.unwrap();
            assert_eq!(checked.check_error.as_deref(), Some(error));
            assert!(checked.connected && !checked.needs_reauth);
            assert!(inference_token(&grant.id, false).await.is_ok());
        }
        fixture.state.lock().unwrap().model_status = Some(200);
        expire(&grant.id).await;
        let checked = service::check_authorization(&grant.id).await.unwrap();
        assert!(checked.check_error.is_none());
        assert_eq!(fixture.state.lock().unwrap().refresh_count, 1);
        let mut changed_source = source.clone();
        changed_source.account_id = Some("changed-workspace".into());
        codex_account::save_account(&changed_source).unwrap();
        let checked = service::check_authorization(&grant.id).await.unwrap();
        assert!(!checked.source_binding_valid);
        assert_eq!(
            checked.source_binding_error.as_deref(),
            Some("CHATGPT_PLAN_SOURCE_CHANGED")
        );
        assert!(checked.connected && !checked.needs_reauth);
        codex_account::save_account(&source).unwrap();
        fixture.state.lock().unwrap().model_status = Some(401);
        let checked = service::check_authorization(&grant.id).await.unwrap();
        assert!(checked.needs_reauth);
        assert_eq!(
            checked.check_error.as_deref(),
            Some("CHATGPT_PLAN_REAUTH_REQUIRED")
        );
        assert_eq!(
            inference_token(&grant.id, false).await.unwrap_err(),
            "CHATGPT_PLAN_REAUTH_REQUIRED"
        );
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        assert!(fixture.state.lock().unwrap().model_count >= 6);
        service::disconnect(&grant.id).await.unwrap();
    });
}

#[test]
fn authorization_inference_errors_are_fenced_by_token_and_authorization_revision() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let grant = fixture.login().await;
        let original = storage::read(&grant.id).await.unwrap();
        let fingerprint = token_fingerprint(&original.credentials().unwrap().access_token);
        let mut replacement = original.clone();
        replacement.authorization_revision = uuid::Uuid::new_v4().to_string();
        storage::save(&replacement).unwrap();
        observe_inference_result(
            &grant.id,
            &original.authorization_revision,
            &fingerprint,
            401,
            None,
        )
        .await;
        assert!(
            !get_account(&grant.id).await.unwrap().needs_reauth,
            "late same-token error cannot disable a new authorization epoch"
        );
        let mut credentials = replacement.credentials().unwrap();
        credentials.access_token = "synthetic-replacement".into();
        replacement.set_credentials(&credentials).unwrap();
        storage::save(&replacement).unwrap();
        observe_inference_result(
            &grant.id,
            &replacement.authorization_revision,
            &fingerprint,
            401,
            None,
        )
        .await;
        assert!(
            !get_account(&grant.id).await.unwrap().needs_reauth,
            "late rotated-token error cannot disable current consent"
        );
        let current = token_fingerprint(&credentials.access_token);
        observe_inference_result(
            &grant.id,
            &replacement.authorization_revision,
            &current,
            503,
            None,
        )
        .await;
        assert!(!get_account(&grant.id).await.unwrap().needs_reauth);
        observe_inference_result(
            &grant.id,
            &replacement.authorization_revision,
            &current,
            401,
            Some("CHATGPT_PLAN_CONFIGURATION"),
        )
        .await;
        assert!(!get_account(&grant.id).await.unwrap().needs_reauth);
        observe_inference_result(
            &grant.id,
            &replacement.authorization_revision,
            &current,
            401,
            None,
        )
        .await;
        assert!(get_account(&grant.id).await.unwrap().needs_reauth);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        service::disconnect(&grant.id).await.unwrap();
    });
}

#[test]
fn authorization_legacy_local_mode_is_durable_before_removal_and_rejoin() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("legacy-mode-source");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        access::append_local_access_accounts(vec![source.id.clone()])
            .await
            .unwrap();
        assert!(storage::read(&grant.id)
            .await
            .unwrap()
            .summary
            .last_service_mode
            .is_none());
        access::remove_local_access_account(&source.id)
            .await
            .unwrap();
        let saved = storage::read(&grant.id).await.unwrap();
        assert_eq!(saved.summary.last_service_mode.as_deref(), Some("codex"));
        assert!(saved.summary.connected);
        service::set_service_mode(
            &source.id,
            Some(&grant.id),
            saved.summary.last_service_mode.as_deref().unwrap(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            access::chatgpt_plan_pool_snapshot().await.unwrap().1,
            vec![source.id.clone()]
        );
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        assert_eq!(fixture.state.lock().unwrap().model_count, 0);
        service::unbind(&grant.id).await.unwrap();
        let detached = get_account(&grant.id).await.unwrap();
        assert!(detached.source_account_id.is_none() && detached.connected);
        service::disconnect(&grant.id).await.unwrap();
    });
}

#[test]
fn authorization_disconnect_clears_unreadable_local_envelope_without_claiming_remote_revoke() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let grant = fixture.login().await;
        let mut broken = storage::read(&grant.id).await.unwrap();
        broken.encrypted_credentials = Some("invalid-local-envelope".into());
        storage::save(&broken).unwrap();
        let result = service::disconnect(&grant.id).await.unwrap();
        assert!(!result.remote_revoked);
        let saved = storage::read(&grant.id).await.unwrap();
        assert!(!saved.summary.connected && saved.encrypted_credentials.is_none());
        assert_eq!(fixture.state.lock().unwrap().revoke_count, 0);
    });
}

#[test]
fn authorization_cold_catalog_sends_real_client_model_with_existing_scope_fences() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("cold-catalog-source");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        fixture.state.lock().unwrap().model_status = Some(503);
        service::set_service_mode(&source.id, Some(&grant.id), "official", None)
            .await
            .unwrap();
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        fixture.state.lock().unwrap().expected_model = Some("gpt-6.1-sol".into());
        access::assert_chatgpt_plan_cold_catalog_real_http(
            &source.id,
            &http::resource_url(""),
        )
        .await;
        assert_eq!(fixture.state.lock().unwrap().response_started, 1);
        service::disconnect(&grant.id).await.unwrap();
    });
}

#[test]
fn authorization_late_check_does_not_overwrite_rotated_token_health() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let grant = fixture.login().await;
        {
            let mut state = fixture.state.lock().unwrap();
            state.model_status = Some(503);
            state.model_delay = true;
        }
        let id = grant.id.clone();
        let check = tokio::spawn(async move { service::check_authorization(&id).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while fixture.state.lock().unwrap().model_count == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        {
            let _guard = storage::lock(&grant.id).await.unwrap();
            let mut record = storage::read(&grant.id).await.unwrap();
            let mut credentials = record.credentials().unwrap();
            credentials.access_token = "new-token-while-check-in-flight".into();
            record.set_credentials(&credentials).unwrap();
            record.summary.last_checked_at = Some(4242);
            record.summary.check_error = None;
            storage::save(&record).unwrap();
        }
        assert_eq!(check.await.unwrap().unwrap_err(), "CHATGPT_PLAN_NETWORK");
        let current = storage::read(&grant.id).await.unwrap();
        assert_eq!(current.summary.last_checked_at, Some(4242));
        assert!(current.summary.check_error.is_none() && !current.summary.needs_reauth);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        service::disconnect(&grant.id).await.unwrap();
    });
}
