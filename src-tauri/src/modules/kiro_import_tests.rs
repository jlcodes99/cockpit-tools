use super::*;
use serde_json::json;

fn parse(value: Value) -> KiroOAuthCompletePayload {
    parse_import_json(&value.to_string()).unwrap().remove(0).0
}

#[test]
fn electron_manager_nested_credentials_keep_registration_and_provider() {
    let payload = parse(json!({"version":"1","accounts":[{
        "id":"foreign-id", "email":"fixture@example.test", "idp":"IAM_SSO",
        "credentials":{"accessToken":"old-access", "refreshToken":"refresh-with-trailing--",
            "clientId":"client", "clientSecret":"client-secret--", "authMethod":"IdC",
            "region":"eu-central-1", "startUrl":"https://example.awsapps.com/start",
            "expiresAt":1800000000000_i64}
    }]}));
    assert_eq!(payload.login_provider.as_deref(), Some("Enterprise"));
    assert_eq!(
        payload.refresh_token.as_deref(),
        Some("refresh-with-trailing--")
    );
    assert_eq!(payload.idc_region.as_deref(), Some("eu-central-1"));
    assert_eq!(payload.expires_at, Some(1800000000));
    let raw = payload.kiro_auth_token_raw.unwrap();
    assert_eq!(raw["clientSecret"], "client-secret--");
    assert_eq!(raw["startUrl"], "https://example.awsapps.com/start");
}

#[test]
fn tauri_manager_refresh_only_exports_use_auth_region() {
    let payload = parse(
        json!([{"refreshToken":"refresh", "clientId":"client", "clientSecret":"secret",
        "authRegion":"eu-west-1", "region":"us-east-1", "provider":"BuilderId", "expiresAt":null}]),
    );
    assert!(payload.access_token.is_empty());
    assert_eq!(payload.refresh_token.as_deref(), Some("refresh"));
    assert_eq!(payload.idc_region.as_deref(), Some("eu-west-1"));
    assert_eq!(payload.kiro_auth_token_raw.unwrap()["authMethod"], "IdC");
}

#[test]
fn snake_case_foreign_fields_are_not_dropped_by_native_deserialization() {
    let payload = parse(
        json!({"id":"foreign", "email":"fixture@example.test", "created_at":1,"last_used":1,
        "access_token":"access", "refresh_token":"refresh", "client_id":"client", "client_secret":"secret",
        "auth_method":"IdC", "auth_region":"eu-central-1", "idp":"AWSIdC"}),
    );
    assert_eq!(
        payload.kiro_auth_token_raw.unwrap()["clientSecret"],
        "secret"
    );
    assert_eq!(payload.login_provider.as_deref(), Some("BuilderId"));
    assert_eq!(payload.idc_region.as_deref(), Some("eu-central-1"));
}

#[test]
fn empty_snapshot_fields_do_not_mask_supplied_credentials() {
    let payload = parse(
        json!({"kiro_auth_token_raw":{"accessToken":"", "refreshToken":null},
        "access_token":"new-access", "refresh_token":"new-refresh", "clientId":"client", "clientSecret":"secret"}),
    );
    assert_eq!(payload.access_token, "new-access");
    assert_eq!(payload.refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(payload.idc_region.as_deref(), Some("us-east-1"));
}

#[test]
fn native_export_keeps_identity_tags_and_normalizes_old_auth_aliases() {
    let source = json!({"id":"native", "email":"fixture@example.test", "access_token":"access", "refresh_token":"refresh",
        "created_at":1, "last_used":2,"tags":["keep"],"plan_name":"KIRO PRO",
        "kiro_auth_token_raw":{"client_secret":"secret", "client_id":"client", "auth_region":"eu-west-1"}});
    let (payload, native) = parse_import_json(&source.to_string()).unwrap().remove(0);
    let native = native.unwrap();
    assert_eq!(native.id, "native");
    assert_eq!(native.tags, Some(vec!["keep".to_string()]));
    assert_eq!(native.plan_name.as_deref(), Some("KIRO PRO"));
    assert_eq!(
        payload.kiro_auth_token_raw.unwrap()["clientSecret"],
        "secret"
    );
}

#[test]
fn refresh_only_social_does_not_invent_access_token_or_infer_provider_from_email() {
    let payload = parse(json!({"email":"fixture@gmail.com", "refreshToken":"refresh"}));
    assert!(payload.access_token.is_empty());
    assert!(payload.login_provider.is_none());
    assert!(payload.status.is_none());
}

#[test]
fn microsoft_credentials_keep_their_own_endpoint_and_scopes() {
    let payload = parse(
        json!({"auth_method":"external_idp", "refresh_token":"refresh", "client_id":"client",
        "token_endpoint":"https://login.microsoftonline.com/tenant/oauth2/v2.0/token", "scopes":["openid","offline_access"]}),
    );
    let raw = payload.kiro_auth_token_raw.unwrap();
    assert_eq!(raw["authMethod"], "external_idp");
    assert_eq!(raw["scopes"], "openid offline_access");
    assert!(kiro_oauth::import_credentials::microsoft_token_endpoint(
        raw["tokenEndpoint"].as_str().unwrap()
    ));
    assert!(!kiro_oauth::import_credentials::microsoft_token_endpoint(
        "https://login.microsoftonline.com.example.test/tenant/oauth2/v2.0/token"
    ));
}

#[test]
fn complete_batch_is_validated_before_persistence_and_errors_omit_credentials() {
    let error = parse_import_json(
        &json!([{"refreshToken":"secret-marker"}, {"email":"missing@example.test"}]).to_string(),
    )
    .unwrap_err();
    assert!(error.contains("KIRO_IMPORT_ITEM:2:"));
    assert!(!error.contains("secret-marker"));
    assert!(parse_import_json("[]").is_err());
    assert!(parse_import_json("not-json").is_err());
    assert!(parse_import_json(&format!("\u{feff}{}", json!({"refreshToken":"refresh"}))).is_ok());
}

#[test]
fn refresh_guard_deduplicates_and_releases_on_cancellation() {
    let guard = begin_refresh("fixture-import-refresh-flight").unwrap();
    assert!(begin_refresh("fixture-import-refresh-flight").is_err());
    drop(guard);
    assert!(begin_refresh("fixture-import-refresh-flight").is_ok());
}

#[test]
fn refreshed_tokens_replace_old_aliases_without_losing_registration() {
    let context = json!({"accessToken":"old", "refreshToken":"old-refresh", "expiresAt":1,
        "clientSecret":"registration", "region":"eu-west-1", "authMethod":"IdC"});
    let merged = kiro_oauth::import_credentials::merge_refreshed(
        &json!({"access_token":"new", "refresh_token":"rotated"}),
        &context,
    )
    .unwrap();
    assert_eq!(merged["accessToken"], "new");
    assert_eq!(merged["refreshToken"], "rotated");
    assert_eq!(merged["clientSecret"], "registration");
    assert!(merged.get("expiresAt").is_none());
    assert!(kiro_oauth::import_credentials::merge_refreshed(
        &json!({"refresh_token":"rotated"}),
        &context
    )
    .is_err());
}

#[test]
fn registration_claims_recover_enterprise_context_without_trusting_foreign_hosts() {
    use base64::Engine;
    let secret = |url: &str| {
        format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(json!({"initiateLoginUri":url}).to_string())
        )
    };
    let payload = parse(
        json!({"refreshToken":"refresh", "clientId":"client", "clientSecret":secret("https://example.awsapps.com/start"), "provider":"BuilderId"}),
    );
    assert_eq!(payload.login_provider.as_deref(), Some("Enterprise"));
    assert_eq!(
        payload.kiro_auth_token_raw.unwrap()["startUrl"],
        "https://example.awsapps.com/start"
    );
    let invalid = parse(
        json!({"refreshToken":"refresh", "clientId":"client", "clientSecret":secret("https://awsapps.com.example.test/start")}),
    );
    assert!(invalid
        .kiro_auth_token_raw
        .unwrap()
        .get("startUrl")
        .is_none());
}

#[test]
fn imported_storage_retains_rotated_tokens_and_rejects_stale_or_deleted_refreshes() {
    let _env_guard = crate::modules::test_support::env_lock().lock().unwrap();
    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            if let Some(value) = &self.0 {
                std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value);
            } else {
                std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
            }
        }
    }
    let _restore = RestoreEnv(std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"));
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", dir.path());
    let raw = json!({"email":"fixture@example.test", "credentials":{"refreshToken":"original-refresh", "provider":"Google"}}).to_string();
    let imported = import_from_json(&raw).unwrap().remove(0);
    assert!(imported.access_token.is_empty());
    assert_eq!(import_from_json(&raw).unwrap()[0].id, imported.id);
    let payload = parse(
        json!({"email":"fixture@example.test", "accessToken":"fresh-access", "refreshToken":"rotated-refresh", "provider":"Google"}),
    );
    let saved = commit_refresh(imported, Ok(payload.clone()), false).unwrap();
    assert_eq!(
        load_account(&saved.id).unwrap().refresh_token.as_deref(),
        Some("rotated-refresh")
    );
    let mut failed_quota = payload.clone();
    failed_quota.status = Some("error".into());
    failed_quota.status_reason = Some("KIRO_USAGE_HTTP:429".into());
    let saved = commit_refresh(saved, Ok(failed_quota), true).unwrap();
    assert_eq!(saved.refresh_token.as_deref(), Some("rotated-refresh"));
    assert_eq!(
        saved.quota_query_last_error.as_deref(),
        Some("KIRO_USAGE_HTTP:429")
    );

    let mut newer = saved.clone();
    newer.access_token = "newer-import".into();
    upsert_account_record(newer).unwrap();
    let retained = commit_refresh(saved.clone(), Ok(payload.clone()), true).unwrap();
    assert_eq!(retained.access_token, "newer-import");
    remove_account(&saved.id).unwrap();
    assert!(commit_refresh(saved.clone(), Ok(payload), true).is_err());
    assert!(load_account(&saved.id).is_none());
}
