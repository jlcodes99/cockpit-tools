// Codex Local Access 测试：宿主内部请求（定时/手动唤醒、鹈鹕测试）统一走 API 服务 sidecar。
// 覆盖内部 API Key 清单、账号范围、并发闸门，以及与对外入口开关解耦的生命周期判断。
const INTERNAL_SERVICE_TEST_ACCOUNT_ID: &str = "internal-service-test-account";
static INTERNAL_GATE_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

fn internal_service_test_account() -> crate::models::codex::CodexAccount {
    crate::models::codex::CodexAccount::new(
        INTERNAL_SERVICE_TEST_ACCOUNT_ID.to_string(),
        "internal-service@example.com".to_string(),
        crate::models::codex::CodexTokens {
            id_token: "id-token".to_string(),
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
        },
    )
}

#[test]
fn api_service_manifest_carries_internal_key_and_scope() {
    super::register_internal_api_account(INTERNAL_SERVICE_TEST_ACCOUNT_ID)
        .expect("register internal API account");
    let collection = test_local_access_collection(Vec::new());

    let api_service_values = super::sidecar_api_key_manifest_values_with_internal(&collection, true);
    let internal_spec = api_service_values
        .iter()
        .find(|value| value["internal"].as_bool() == Some(true))
        .expect("API service manifest should carry the host-internal key");
    assert_eq!(
        internal_spec["key"].as_str(),
        Some(super::internal_api_service_key())
    );
    assert_eq!(internal_spec["enabled"].as_bool(), Some(true));
    assert!(
        internal_spec["accountIds"]
            .as_array()
            .expect("internal key account scope")
            .iter()
            .any(|value| value.as_str() == Some(INTERNAL_SERVICE_TEST_ACCOUNT_ID)),
        "internal key scope should contain the host-requested account"
    );

    let provider_gateway_values =
        super::sidecar_api_key_manifest_values_with_internal(&collection, false);
    assert!(
        provider_gateway_values
            .iter()
            .all(|value| value["internal"].as_bool() != Some(true)),
        "provider gateway configs must not expose the host-internal key"
    );
}

#[test]
fn internal_accounts_join_the_api_service_account_scope() {
    super::register_internal_api_account(INTERNAL_SERVICE_TEST_ACCOUNT_ID)
        .expect("register internal API account");
    let collection = test_local_access_collection(Vec::new());
    let overrides = std::collections::HashMap::from([(
        INTERNAL_SERVICE_TEST_ACCOUNT_ID.to_string(),
        internal_service_test_account(),
    )]);

    let api_service_accounts =
        super::effective_sidecar_account_ids_with_internal(&collection, true);
    assert!(api_service_accounts
        .iter()
        .any(|account_id| account_id == INTERNAL_SERVICE_TEST_ACCOUNT_ID));
    let provider_gateway_accounts =
        super::effective_sidecar_account_ids_with_internal(&collection, false);
    assert!(provider_gateway_accounts
        .iter()
        .all(|account_id| account_id != INTERNAL_SERVICE_TEST_ACCOUNT_ID));

    let api_service_keys =
        super::sidecar_client_api_keys_with_internal(&collection, &overrides, true);
    assert!(api_service_keys
        .iter()
        .any(|key| key == super::internal_api_service_key()));
    let provider_gateway_keys =
        super::sidecar_client_api_keys_with_internal(&collection, &overrides, false);
    assert!(provider_gateway_keys
        .iter()
        .all(|key| key != super::internal_api_service_key()));

    let api_service_scope =
        super::sidecar_api_key_account_scope_values_with_internal(&collection, &overrides, true);
    let internal_scope = api_service_scope
        .get(super::internal_api_service_key())
        .and_then(|value| value.as_array())
        .expect("internal key should map to the sidecar auth scope");
    assert!(!internal_scope.is_empty());
    let provider_gateway_scope =
        super::sidecar_api_key_account_scope_values_with_internal(&collection, &overrides, false);
    assert!(
        provider_gateway_scope
            .get(super::internal_api_service_key())
            .is_none(),
        "provider gateway scope must not contain the host-internal key"
    );
}

#[tokio::test]
async fn disabled_api_service_still_runs_for_internal_requests() {
    let _test_guard = INTERNAL_GATE_TEST_LOCK.lock().await;
    let _permit = super::acquire_internal_request_permit(INTERNAL_SERVICE_TEST_ACCOUNT_ID)
        .await
        .expect("internal request permit");
    let mut collection = test_local_access_collection(Vec::new());
    collection.enabled = false;

    assert!(
        super::local_access_gateway_should_run(&collection),
        "host-internal requests must keep the API service sidecar alive after the public entry is disabled"
    );
}

#[test]
fn failed_sidecar_stop_still_attempts_profile_restore() {
    let mut restore_attempted = false;
    let result = super::finish_local_access_disable(Err("port still bound".to_string()), || {
        restore_attempted = true;
        Ok(())
    });

    assert!(restore_attempted);
    assert_eq!(result, Err("port still bound".to_string()));
}

#[test]
fn disable_preserves_stop_and_profile_restore_errors() {
    let result = super::finish_local_access_disable(Err("sidecar stop failed".to_string()), || {
        Err("profile restore failed".to_string())
    });

    assert_eq!(
        result,
        Err("sidecar stop failed; 恢复 Codex 配置时也失败: profile restore failed".to_string())
    );
}

#[tokio::test]
async fn internal_requests_serialize_per_account() {
    let _test_guard = INTERNAL_GATE_TEST_LOCK.lock().await;
    let account_id = "internal-scheduler-account-a";
    let other_account_id = "internal-scheduler-account-b";

    let first = super::acquire_internal_request_permit(account_id)
        .await
        .expect("first internal permit");
    let blocked = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        super::acquire_internal_request_permit(account_id),
    )
    .await;
    assert!(
        blocked.is_err(),
        "同一账号的内部请求必须串行，避免并发刷新和超额消耗"
    );

    let other = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::acquire_internal_request_permit(other_account_id),
    )
    .await;
    assert!(other.is_ok(), "不同账号的内部请求不应互相阻塞");

    drop(first);
    let released = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::acquire_internal_request_permit(account_id),
    )
    .await;
    assert!(released.is_ok(), "请求结束后必须释放账号闸门");
}

#[tokio::test]
async fn internal_request_permit_tracks_activity_and_blocks_shutdown() {
    let _test_guard = INTERNAL_GATE_TEST_LOCK.lock().await;
    let account_id = "internal-scheduler-active-account";
    let permit = super::acquire_internal_request_permit(account_id)
        .await
        .expect("internal request permit");

    assert_eq!(
        super::INTERNAL_ACTIVE_REQUESTS.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(super::INTERNAL_REQUEST_GATE
        .clone()
        .try_acquire_many_owned(super::INTERNAL_REQUEST_CONCURRENCY as u32)
        .is_err());

    drop(permit);
    assert_eq!(
        super::INTERNAL_ACTIVE_REQUESTS.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let shutdown_guard = super::INTERNAL_REQUEST_GATE
        .clone()
        .try_acquire_many_owned(super::INTERNAL_REQUEST_CONCURRENCY as u32)
        .expect("idle scheduler should allow shutdown guard");
    drop(shutdown_guard);
}

/// 内部请求必须落在 sidecar 真正注册的 `/v1/*` 路由上。
///
/// 回归背景：内部请求一度把客户端路径先解析成上游路径（`/v1/responses` → `/responses`），
/// 唤醒返回 404 `endpoint not supported`，鹈鹕测试则在本地解析阶段直接报
/// 「仅支持 /v1 或 /backend-api/codex 路径」。
#[test]
fn internal_requests_use_api_service_public_paths() {
    assert_eq!(
        super::resolve_internal_api_service_target(super::RESPONSES_PATH)
            .expect("responses path resolves"),
        "/v1/responses"
    );
    assert_eq!(
        super::resolve_internal_api_service_target(super::RESPONSES_COMPACT_PATH)
            .expect("compact path resolves"),
        "/v1/responses/compact"
    );
    assert_eq!(
        super::resolve_internal_api_service_target("/backend-api/codex/responses")
            .expect("backend codex path resolves"),
        "/v1/responses"
    );
    assert_eq!(
        super::resolve_internal_api_service_target("/v1/responses?debug=1")
            .expect("query string is preserved"),
        "/v1/responses?debug=1"
    );
    assert!(
        super::resolve_internal_api_service_target("/responses").is_err(),
        "上游路径不能再进入内部请求，避免退化成 sidecar 404"
    );
}
