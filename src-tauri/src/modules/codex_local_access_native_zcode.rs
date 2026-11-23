//! Native ZCode API-key provider credentials.
//! The source API key is read from the encrypted ZCode account store only at request time.

use crate::models::{codex::CodexAccount, zcode::ZcodeAccount};
use axum::http::{HeaderMap, HeaderValue};

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

pub(super) fn base_url(source: &ZcodeAccount) -> &'static str {
    if source.provider.eq_ignore_ascii_case("bigmodel") {
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
