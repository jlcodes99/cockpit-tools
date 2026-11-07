use super::super::{
    auth_registry_entry, load_account, managed_profile_dir, refresh_account_inner_with_io,
    save_account_locked, write_account_to_profile, GrokAccount, GrokQuotaQuery,
};
use super::{sample_account, EnvironmentGuard, TestDir};
use crate::models::grok::GrokQuota;
use crate::modules::grok_oauth::GrokTokenResponse;
use futures::future::BoxFuture;

fn rotate_token<'a>(
    refresh_token: &'a str,
    _endpoint: Option<&'a str>,
    _client_id: Option<&'a str>,
) -> BoxFuture<'a, Result<GrokTokenResponse, String>> {
    Box::pin(async move {
        let next = match refresh_token {
            "secret-refresh" => "2",
            "refresh-2" => "3",
            other => panic!("unexpected refresh credential: {other}"),
        };
        Ok(GrokTokenResponse {
            access_token: format!("access-{next}"),
            refresh_token: Some(format!("refresh-{next}")),
            id_token: None,
            token_type: Some("Bearer".to_string()),
            expires_in: Some(3600),
        })
    })
}

fn pending_quota(_account: &mut GrokAccount) -> BoxFuture<'_, Result<(), String>> {
    Box::pin(std::future::pending())
}

fn unauthorized_then_pending(account: &mut GrokAccount) -> BoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        if account.access_token == "access-2" {
            return Err("查询 Grok 配额返回 401".to_string());
        }
        assert_eq!(account.access_token, "access-3");
        std::future::pending().await
    })
}

async fn cancel_at_quota_and_check(
    account: &GrokAccount,
    query: GrokQuotaQuery,
    expected_access: &str,
    expected_refresh: &str,
) {
    // Exercise the real token-adoption/rotation, persistence and 401 retry state machine; only
    // the external HTTP exchange and quota network wait are replaced by deterministic futures.
    let mut refresh = Box::pin(refresh_account_inner_with_io(
        &account.id,
        false,
        rotate_token,
        query,
    ));
    assert!(futures::poll!(refresh.as_mut()).is_pending());
    drop(refresh); // Same cancellation semantics as an outer tokio::time::timeout.

    let persisted = load_account(&account.id).expect("credential checkpoint survives cancellation");
    assert_eq!(persisted.access_token, expected_access);
    assert_eq!(persisted.refresh_token.as_deref(), Some(expected_refresh));
    assert_eq!(persisted.quota.unwrap().weekly_limit_percent, Some(37.0));
    assert_eq!(persisted.usage_updated_at, Some(1234));
    let profile = managed_profile_dir(&account.id).unwrap().join("auth.json");
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(profile).unwrap()).unwrap();
    assert_eq!(
        auth_registry_entry(&auth).unwrap()["refresh_token"],
        expected_refresh
    );
}

fn quota_fixture() -> GrokAccount {
    let mut account = sample_account();
    account.expires_at = Some(1);
    account.quota = Some(GrokQuota {
        weekly_limit_percent: Some(37.0),
        ..Default::default()
    });
    account.usage_updated_at = Some(1234);
    account
}

#[tokio::test]
async fn cancellation_during_first_quota_preserves_rotated_tokens_and_quota_cache() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let temp = TestDir::new();
    let _environment = EnvironmentGuard::new(&temp.0);
    let account = quota_fixture();
    save_account_locked(&account).unwrap();
    cancel_at_quota_and_check(&account, pending_quota, "access-2", "refresh-2").await;
}

#[tokio::test]
async fn cancellation_during_quota_401_retry_preserves_second_rotated_refresh_token() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let temp = TestDir::new();
    let _environment = EnvironmentGuard::new(&temp.0);
    let account = quota_fixture();
    save_account_locked(&account).unwrap();
    cancel_at_quota_and_check(&account, unauthorized_then_pending, "access-3", "refresh-3").await;
}

#[tokio::test]
async fn cancellation_during_quota_preserves_adopted_live_credentials() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let temp = TestDir::new();
    let _environment = EnvironmentGuard::new(&temp.0);
    let account = quota_fixture();
    save_account_locked(&account).unwrap();
    let mut live = account.clone();
    live.access_token = "adopted-access".to_string();
    live.refresh_token = Some("adopted-refresh".to_string());
    live.expires_at = Some(chrono::Utc::now().timestamp() + 3600);
    write_account_to_profile(&live, &managed_profile_dir(&account.id).unwrap()).unwrap();
    cancel_at_quota_and_check(&account, pending_quota, "adopted-access", "adopted-refresh").await;
}

fn quota_yields_once(_account: &mut GrokAccount) -> BoxFuture<'_, Result<(), String>> {
    Box::pin(async {
        tokio::task::yield_now().await;
        Ok(())
    })
}

#[tokio::test]
async fn quota_completion_cannot_resurrect_an_account_deleted_while_waiting() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let temp = TestDir::new();
    let _environment = EnvironmentGuard::new(&temp.0);
    let account = quota_fixture();
    save_account_locked(&account).unwrap();
    let mut refresh = Box::pin(refresh_account_inner_with_io(
        &account.id,
        false,
        rotate_token,
        quota_yields_once,
    ));
    // The real refresh has persisted its new credential and is now waiting for quota I/O.
    assert!(futures::poll!(refresh.as_mut()).is_pending());
    assert_eq!(load_account(&account.id).unwrap().access_token, "access-2");
    super::super::remove_account(&account.id).expect("delete without waiting for the token lock");

    let error = refresh
        .await
        .expect_err("late quota save must not recreate the deleted account");
    assert!(error.contains("账号已删除"));
    assert!(load_account(&account.id).is_none());
    assert!(!super::super::load_index()
        .unwrap()
        .accounts
        .iter()
        .any(|entry| entry.id == account.id));
    assert!(!managed_profile_dir(&account.id)
        .unwrap()
        .join("auth.json")
        .exists());
}
