//! Grok uses managed OAuth credentials and the gateway's protocol version.
//! An obsolete installed CLI must never downgrade API requests.
use crate::{
    models::{codex::CodexAccount, grok::GrokAccount},
    modules::codex_native_api::contract::EndpointOverride,
};
use axum::http::{header, HeaderMap, HeaderValue};
const API_BASE: &str = "https://api.x.ai/v1";

fn resolved_version(detected: Option<String>) -> String {
    crate::modules::grok_client_version::resolve(
        detected
            .and_then(|value| crate::commands::grok::parse_grok_client_version(&value))
            .as_deref(),
    )
}

pub(super) fn cached_client_version() -> String {
    crate::modules::grok_client_version::cached()
}

pub(super) fn credential_headers(token: &str, version: &str) -> Result<HeaderMap, String> {
    if token.trim().is_empty() {
        return Err("Grok access token is empty".into());
    }
    let mut headers = HeaderMap::new();
    let mut bearer =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "Invalid Grok token")?;
    bearer.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, bearer);
    let version = resolved_version(Some(version.to_owned()));
    for (key, value) in [
        ("x-xai-token-auth", "xai-grok-cli".to_string()),
        ("x-grok-cli-version", version.clone()),
        ("x-grok-client-version", version.clone()),
        ("x-grok-client-surface", "grok-cli".to_string()),
        ("x-grok-client-identifier", "cockpit-tools".to_string()),
        (
            "x-authenticateresponse",
            "authenticate-response".to_string(),
        ),
        ("user-agent", format!("grok-cli/{version}")),
    ] {
        headers.insert(
            key,
            HeaderValue::from_str(&value).map_err(|_| "Invalid Grok client header")?,
        );
    }
    Ok(headers)
}

pub(super) fn official_endpoint(headers: &HeaderMap) -> EndpointOverride {
    let mut auth = HeaderMap::new();
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        auth.insert(header::AUTHORIZATION, value.clone());
    }
    EndpointOverride {
        base_url: API_BASE.into(),
        headers: auth,
    }
}

pub(super) fn bind_owner(account: &mut CodexAccount, current: &GrokAccount) {
    // Preserve the current authenticated principal/workspace in continuation
    // ownership even if the linked Codex entry predates a Grok account update.
    account.user_id = current
        .principal_id
        .clone()
        .or_else(|| current.user_id.clone());
    account.account_id = current.team_id.clone();
}

/// Control messages must compare the same live principal/team as route creation.
/// Reading the source does not refresh credentials or change the active socket.
pub(super) async fn current_account(
    mut account: CodexAccount,
) -> Result<CodexAccount, super::native_contract::ServiceError> {
    let permit = super::NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            super::native_error(503, "account_read_failed", "Account reader is unavailable")
        })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let source = account
            .upstream_grok_account_id
            .as_deref()
            .and_then(crate::modules::grok_account::load_account)
            .filter(|source| !source.is_api_key_auth() && !source.access_token.is_empty())
            .ok_or_else(|| {
                super::native_error(
                    403,
                    "websocket_control_denied",
                    "The linked Grok authorization is no longer available",
                )
            })?;
        bind_owner(&mut account, &source);
        Ok(account)
    })
    .await
    .map_err(|_| {
        super::native_error(
            503,
            "account_read_failed",
            "Unable to read the linked Grok authorization",
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_grok_headers_use_host_version_rules_and_official_endpoint_removes_cli_headers() {
        for (detected, expected) in [
            ("grok 1.0.55 (test)", "1.0.55"),
            ("bad-version", "1.0.52"),
            ("0.2.93", "1.0.52"),
        ] {
            let headers = credential_headers("test-oauth-token", detected).unwrap();
            assert_eq!(headers["x-xai-token-auth"], "xai-grok-cli");
            assert_eq!(headers["x-grok-client-version"], expected);
            assert_eq!(headers["x-grok-cli-version"], expected);
            assert_eq!(headers["user-agent"], format!("grok-cli/{expected}"));
            assert_eq!(headers["x-authenticateresponse"], "authenticate-response");
            assert!(headers[header::AUTHORIZATION].is_sensitive());
            let official = official_endpoint(&headers);
            assert_eq!(official.base_url, "https://api.x.ai/v1");
            assert_eq!(
                official.headers[header::AUTHORIZATION],
                "Bearer test-oauth-token"
            );
            assert!(!official.headers.contains_key("x-xai-token-auth"));
            assert!(!official.headers.contains_key("x-grok-client-version"));
        }
        assert!(credential_headers("", "1.0.52").is_err());
        assert!(credential_headers("token\r\nX-Evil: value", "1.0.52").is_err());
    }
    #[test]
    fn native_grok_current_principal_and_team_determine_owner_without_persisting_tokens() {
        let mut account = CodexAccount::new(
            "linked".into(),
            "fixture@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: "not-a-grok-token".into(),
                id_token: String::new(),
                refresh_token: None,
            },
        );
        let mut source: GrokAccount = serde_json::from_value(serde_json::json!({
            "id":"grok-source","email":"fixture@example.invalid","access_token":"grok-token",
            "principal_id":"principal-one","team_id":"team-one","created_at":1,"last_used":1
        }))
        .unwrap();
        bind_owner(&mut account, &source);
        let owner =
            super::super::native_account_owner(&account, "https://cli-chat-proxy.grok.com/v1");
        assert!(owner.contains("principal-one:team-one"));
        assert_eq!(account.tokens.access_token, "not-a-grok-token");
        source.access_token = "rotated-grok-token".into();
        bind_owner(&mut account, &source);
        assert_eq!(
            owner,
            super::super::native_account_owner(&account, "https://cli-chat-proxy.grok.com/v1")
        );
        source.team_id = Some("team-two".into());
        bind_owner(&mut account, &source);
        assert_ne!(
            owner,
            super::super::native_account_owner(&account, "https://cli-chat-proxy.grok.com/v1")
        );
    }
}
