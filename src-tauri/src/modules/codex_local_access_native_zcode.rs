//! Native ZCode API-key and OAuth provider credentials.
//! Sources are read from the encrypted ZCode account store only at request time.

use crate::models::{codex::CodexAccount, zcode::ZcodeAccount};
use axum::http::{HeaderMap, HeaderValue};

pub(super) const ZCODE_PLAN_API_BASE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/anthropic";
const ZCODE_APP_VERSION: &str = "3.17.2";

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
    let token = if source.team_plan.is_some() {
        source.access_token.trim()
    } else {
        source.zcode_jwt_token.trim()
    };
    if token.is_empty() {
        return Err("ZCODE_OAUTH_REAUTH_REQUIRED".to_string());
    }
    let mut headers = HeaderMap::new();
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "Invalid ZCode OAuth token")?;
    authorization.set_sensitive(true);
    headers.insert("authorization", authorization);
    headers.insert("x-title", HeaderValue::from_static("Z Code@electron"));
    headers.insert(
        "x-zcode-app-version",
        HeaderValue::from_static(ZCODE_APP_VERSION),
    );
    headers.insert(
        "x-zcode-query-source",
        HeaderValue::from_static("main_turn"),
    );
    headers.insert(
        "x-device-mid",
        HeaderValue::from_str(&source.id).map_err(|_| "Invalid ZCode device identity")?,
    );
    headers.insert("user-agent", HeaderValue::from_static("ZCode/3.17.2"));
    headers.insert("x-platform", HeaderValue::from_static(std::env::consts::OS));
    headers.insert(
        "x-session-id",
        HeaderValue::from_str(&source.id).map_err(|_| "Invalid ZCode session identity")?,
    );
    if let Some(team) = &source.team_plan {
        headers.insert(
            "bigmodel-organization",
            HeaderValue::from_str(&team.organization_id)
                .map_err(|_| "Invalid BigModel organization")?,
        );
        headers.insert(
            "bigmodel-project",
            HeaderValue::from_str(&team.project_id).map_err(|_| "Invalid BigModel project")?,
        );
        headers.insert("bigmodel-target-type", HeaderValue::from_static("TEAM"));
    }
    Ok(headers)
}

pub(super) fn base_url(source: &ZcodeAccount) -> &'static str {
    if source.auth_mode == crate::models::zcode::ZcodeAuthMode::Oauth {
        if source.team_plan.is_some() {
            crate::modules::zcode_account::BIGMODEL_API_BASE_URL
        } else {
            ZCODE_PLAN_API_BASE_URL
        }
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
    fn oauth_headers_use_session_token_and_plan_endpoint() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-source",
            "auth_mode":"oauth",
            "provider":"zai",
            "email":"fixture@example.invalid",
            "access_token":"provider-access",
            "zcode_jwt_token":"session-jwt",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        let headers = oauth_credential_headers(&source).unwrap();
        assert_eq!(headers["authorization"], "Bearer session-jwt");
        assert!(headers["authorization"].is_sensitive());
        assert_eq!(headers["x-title"], "Z Code@electron");
        assert_eq!(headers["x-zcode-query-source"], "main_turn");
        assert_eq!(base_url(&source), ZCODE_PLAN_API_BASE_URL);
    }

    #[test]
    fn oauth_without_session_credential_is_rejected() {
        let source: ZcodeAccount = serde_json::from_value(serde_json::json!({
            "id":"oauth-empty",
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
            oauth_credential_headers(&source).unwrap_err(),
            "ZCODE_OAUTH_REAUTH_REQUIRED"
        );
    }

    #[test]
    fn oauth_team_headers_use_bigmodel_access_token() {
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
        let headers = oauth_credential_headers(&source).unwrap();
        assert_eq!(headers["authorization"], "Bearer team-access");
        assert_eq!(headers["bigmodel-organization"], "org-1");
        assert_eq!(headers["bigmodel-project"], "project-1");
        assert_eq!(base_url(&source), crate::modules::zcode_account::BIGMODEL_API_BASE_URL);
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
