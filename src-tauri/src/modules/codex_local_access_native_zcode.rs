//! Native ZCode API-key and OAuth provider credentials.
//! Sources are read from the encrypted ZCode account store only at request time.

use crate::models::{codex::CodexAccount, zcode::ZcodeAccount};
use axum::http::{HeaderMap, HeaderValue};

use crate::modules::zcode_account::ZCODE_PLAN_API_BASE_URL;

pub(super) fn credential_headers(source: &ZcodeAccount) -> Result<HeaderMap, String> {
    let api_key = source
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ZCode API Key is empty".to_string())?;
    let mut headers = HeaderMap::new();
    let mut value = HeaderValue::from_str(api_key).map_err(|_| "Invalid ZCode API key")?;
    value.set_sensitive(true);
    headers.insert("x-api-key", value);
    Ok(headers)
}

pub(super) fn oauth_credential_headers(source: &ZcodeAccount) -> Result<HeaderMap, String> {
    oauth_credential_headers_with_client_headers(
        source,
        crate::modules::zcode_account::zcode_client_headers,
    )
}

fn oauth_credential_headers_with_client_headers(
    source: &ZcodeAccount,
    client_headers: impl FnOnce() -> HeaderMap,
) -> Result<HeaderMap, String> {
    let session_token = crate::modules::zcode_account::oauth_inference_session(source)?;
    let mut headers = client_headers();
    let mut authorization = HeaderValue::from_str(&format!("Bearer {session_token}"))
        .map_err(|_| "Invalid ZCode OAuth token")?;
    authorization.set_sensitive(true);
    headers.insert("authorization", authorization);
    Ok(headers)
}

pub(super) fn base_url(source: &ZcodeAccount) -> &'static str {
    if source.auth_mode == crate::models::zcode::ZcodeAuthMode::Oauth {
        // bigmodel OAuth 已在凭据构造处 fail closed；这里只承接 Z.ai 个人 Start Plan。
        ZCODE_PLAN_API_BASE_URL
    } else if source.provider.eq_ignore_ascii_case("bigmodel") {
        crate::modules::zcode_account::BIGMODEL_API_BASE_URL
    } else {
        crate::modules::zcode_account::ZAI_API_BASE_URL
    }
}

pub(super) fn bind_owner(account: &mut CodexAccount, source: &ZcodeAccount) {
    account.user_id = source.user_id.clone();
    account.account_id = Some(source.id.clone());
    account.openai_api_key = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_headers_use_session_jwt_and_plan_endpoint() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-source",
            "subscription_raw":[{"status":"active","name":"ZCode Start Plan"}],
            "auth_mode":"oauth",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"provider-access",
            "zcode_jwt_token":"session-jwt",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        let headers =
            oauth_credential_headers_with_client_headers(&source, HeaderMap::new).unwrap();
        assert_eq!(headers["authorization"], "Bearer session-jwt");
        assert!(headers["authorization"].is_sensitive());
        assert!(!headers.contains_key("x-bigmodel-authorization"));
        assert!(!headers.contains_key("x-session-id"));
        assert!(!headers.contains_key("x-zcode-query-source"));
        assert_eq!(base_url(&source), ZCODE_PLAN_API_BASE_URL);
    }

    #[test]
    fn oauth_headers_merge_confirmed_client_identity_without_fabrication() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-source",
            "subscription_raw":[{"status":"active","name":"ZCode Start Plan"}],
            "auth_mode":"oauth",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"provider-access",
            "zcode_jwt_token":"session-jwt",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        let mut client = HeaderMap::new();
        client.insert(
            "x-device-mid",
            HeaderValue::from_static("fixture-device-mid"),
        );
        client.insert("x-title", HeaderValue::from_static("Z Code@electron"));
        let headers = oauth_credential_headers_with_client_headers(&source, || client).unwrap();
        assert_eq!(headers["x-device-mid"], "fixture-device-mid");
        assert_eq!(headers["x-title"], "Z Code@electron");
        assert_ne!(headers["x-device-mid"], source.id.as_str());
        assert!(!headers.contains_key("x-session-id"));
    }

    #[test]
    fn oauth_without_session_credential_is_rejected() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-empty",
            "subscription_raw":[{"status":"active","name":"ZCode Start Plan"}],
            "auth_mode":"oauth",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"",
            "zcode_jwt_token":"",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        assert_eq!(
            oauth_credential_headers_with_client_headers(&source, HeaderMap::new).unwrap_err(),
            "ZCODE_OAUTH_REAUTH_REQUIRED"
        );
    }

    #[test]
    fn oauth_bigmodel_team_plan_requires_project_api_key_and_fails_closed() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-team",
            "auth_mode":"oauth",
            "provider":"bigmodel",
            "email":"fixture@example.invalid",
            "access_token":"team-access",
            "zcode_jwt_token":"session-jwt",
            "team_plan":{"organization_id":"org-1","project_id":"project-1"},
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        assert_eq!(
            oauth_credential_headers_with_client_headers(&source, HeaderMap::new).unwrap_err(),
            "ZCODE_BIGMODEL_PROJECT_API_KEY_REQUIRED"
        );
    }

    #[test]
    fn oauth_bigmodel_inference_never_receives_the_quota_session_jwt() {
        // 配额（zcode.z.ai/api/v1/mcp/usage）使用的会话 JWT 不能发送到
        // open.bigmodel.cn/api/anthropic 推理端点；BigModel OAuth 一律 fail closed，
        // 不把 session JWT 或 access_token 猜测发送。
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-bigmodel-personal",
            "auth_mode":"oauth",
            "provider":"bigmodel",
            "email":"fixture@example.invalid",
            "access_token":"provider-access",
            "zcode_jwt_token":"quota-session-jwt",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        assert_eq!(
            oauth_credential_headers_with_client_headers(&source, HeaderMap::new).unwrap_err(),
            "ZCODE_BIGMODEL_PROJECT_API_KEY_REQUIRED"
        );
    }

    #[test]
    fn oauth_bigmodel_without_tokens_also_fails_closed() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-bigmodel-empty",
            "auth_mode":"oauth",
            "provider":"bigmodel",
            "email":"fixture@example.invalid",
            "access_token":"",
            "zcode_jwt_token":"",
            "team_plan":{"organization_id":"org-1","project_id":"project-1"},
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        assert_eq!(
            oauth_credential_headers_with_client_headers(&source, HeaderMap::new).unwrap_err(),
            "ZCODE_BIGMODEL_PROJECT_API_KEY_REQUIRED"
        );
    }

    #[test]
    fn headers_are_sensitive_and_provider_base_url_is_selected() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"zcode-source",
            "auth_mode":"api_key",
            "provider":"bigmodel",
            "email":"fixture@example.invalid",
            "access_token":"",
            "zcode_jwt_token":"",
            "api_key":"secret-key",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        let headers = credential_headers(&source).unwrap();
        assert_eq!(headers["x-api-key"], "secret-key");
        assert!(headers["x-api-key"].is_sensitive());
        assert_eq!(
            base_url(&source),
            crate::modules::zcode_account::BIGMODEL_API_BASE_URL
        );
    }

    #[test]
    fn empty_api_key_is_rejected() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"zcode-source",
            "auth_mode":"api_key",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"",
            "zcode_jwt_token":"",
            "api_key":"",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        assert!(credential_headers(&source).is_err());
    }

    #[test]
    fn owner_uses_source_identity_without_copying_the_secret() {
        let mut account = CodexAccount::new_api_key(
            "linked".into(),
            "fixture@example.invalid".into(),
            String::new(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some(crate::modules::zcode_account::ZAI_API_BASE_URL.into()),
            Some("builtin:zai".into()),
            Some("Z.ai".into()),
            vec!["GLM-5.2".into()],
        );
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"zcode-source",
            "auth_mode":"api_key",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"",
            "zcode_jwt_token":"",
            "api_key":"secret-key",
            "user_id":"user-one",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        bind_owner(&mut account, &source);
        assert_eq!(account.user_id.as_deref(), Some("user-one"));
        assert_eq!(account.account_id.as_deref(), Some("zcode-source"));
        assert!(account.openai_api_key.is_none());
    }
}
