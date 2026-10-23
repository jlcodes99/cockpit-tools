use super::types::ChatgptPlanModel;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

pub(super) const RESOURCE: &str = "https://api.openai.com/v1";
pub(super) const ISSUER: &str = "https://auth.openai.com";
pub(super) const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";

pub(super) fn auth_url(path: &str) -> String {
    #[cfg(test)]
    if let Ok(base) = std::env::var("CHATGPT_PLAN_TEST_SERVER") {
        return format!("{base}{path}");
    }
    format!("{ISSUER}{path}")
}

pub(super) fn resource_url(path: &str) -> String {
    #[cfg(test)]
    if let Ok(base) = std::env::var("CHATGPT_PLAN_TEST_SERVER") {
        return format!("{base}/v1{path}");
    }
    format!("{RESOURCE}{path}")
}

pub(super) fn client(timeout_seconds: u64) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(test)]
    let builder = if std::env::var_os("CHATGPT_PLAN_TEST_SERVER").is_some() {
        builder.no_proxy()
    } else {
        builder
    };
    builder
        .build()
        .map_err(|_| "CHATGPT_PLAN_NETWORK".to_string())
}

pub(super) fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "CHATGPT_PLAN_TIMEOUT".into()
    } else {
        "CHATGPT_PLAN_NETWORK".into()
    }
}

pub(super) fn response_error(status: u16, value: &Value) -> String {
    let code = value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .or_else(|| value.get("code").and_then(Value::as_str))
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.pointer("/error/type").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .unwrap_or_default();
    match code {
        "invalid_grant"
        | "invalid_refresh_token"
        | "token_expired"
        | "refresh_token_expired"
        | "refresh_token_invalidated"
        | "refresh_token_reused"
        | "login_required" => "CHATGPT_PLAN_REAUTH_REQUIRED".into(),
        "invalid_client" => "CHATGPT_PLAN_CONFIGURATION".into(),
        "subscription_sharing_user_not_eligible" => "CHATGPT_PLAN_NOT_ELIGIBLE".into(),
        "subscription_sharing_usage_limit_exceeded" => "CHATGPT_PLAN_USAGE_LIMIT".into(),
        "subscription_sharing_usage_unavailable" => "CHATGPT_PLAN_USAGE_UNAVAILABLE".into(),
        "subscription_sharing_unsupported_capability" => {
            "CHATGPT_PLAN_UNSUPPORTED_CAPABILITY".into()
        }
        "subscription_sharing_route_not_supported"
        | "subscription_sharing_invalid_user"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => "CHATGPT_PLAN_CONFIGURATION".into(),
        "subscription_sharing_user_unavailable" => "CHATGPT_PLAN_NETWORK".into(),
        "insufficient_scope" | "subscription_sharing_not_enabled" => {
            "CHATGPT_PLAN_PERMISSION_REQUIRED".into()
        }
        _ => match status {
            401 => "CHATGPT_PLAN_REAUTH_REQUIRED".into(),
            403 => "CHATGPT_PLAN_CONFIGURATION".into(),
            429 => "CHATGPT_PLAN_USAGE_LIMIT".into(),
            500..=599 => "CHATGPT_PLAN_NETWORK".into(),
            _ => "CHATGPT_PLAN_VERIFY_FAILED".into(),
        },
    }
}

pub(super) async fn json(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("CHATGPT_PLAN_VERIFY_FAILED".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| {
        if status == 401 {
            "CHATGPT_PLAN_REAUTH_REQUIRED".to_string()
        } else if status >= 500 {
            "CHATGPT_PLAN_NETWORK".to_string()
        } else {
            "CHATGPT_PLAN_VERIFY_FAILED".to_string()
        }
    })?;
    if !(200..300).contains(&status) {
        return Err(response_error(status, &value));
    }
    Ok(value)
}

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scope: Option<String>,
    pub token_type: String,
    pub expires_in: i64,
    pub earliest_refresh_at: Option<i64>,
}

pub(super) async fn token(form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let response = client(25)?
        .post(auth_url("/api/accounts/oauth/token"))
        .form(form)
        .send()
        .await
        .map_err(network_error)?;
    let value = json(response).await?;
    let tokens: TokenResponse =
        serde_json::from_value(value).map_err(|_| "CHATGPT_PLAN_INVALID_ID_TOKEN".to_string())?;
    if tokens.access_token.is_empty()
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
        || !(1..=86_400).contains(&tokens.expires_in)
        || tokens.access_token.contains(['\r', '\n'])
    {
        return Err("CHATGPT_PLAN_INVALID_ID_TOKEN".into());
    }
    Ok(tokens)
}

pub(super) fn scopes(scope: &str) -> Result<Vec<String>, String> {
    let values: Vec<String> = scope.split_whitespace().map(str::to_string).collect();
    if [DIRECT_SCOPE, "resource.invoke", "offline_access"]
        .iter()
        .any(|required| !values.iter().any(|value| value == required))
    {
        return Err("CHATGPT_PLAN_PERMISSION_REQUIRED".into());
    }
    Ok(values)
}

#[derive(Deserialize)]
pub(super) struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub revocation_endpoint: Option<String>,
}

pub(super) fn trusted_https(url: &str) -> bool {
    #[cfg(test)]
    if let Ok(base) = std::env::var("CHATGPT_PLAN_TEST_SERVER") {
        if url.starts_with(&format!("{base}/")) {
            return true;
        }
    }
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("auth.openai.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && url.fragment().is_none()
    })
}

pub(super) async fn discovery() -> Result<Discovery, String> {
    let response = client(10)?
        .get(auth_url("/.well-known/openid-configuration"))
        .send()
        .await
        .map_err(network_error)?;
    let discovery: Discovery = serde_json::from_value(json(response).await?)
        .map_err(|_| "CHATGPT_PLAN_INVALID_ID_TOKEN".to_string())?;
    if discovery.issuer != ISSUER || !trusted_https(&discovery.jwks_uri) {
        return Err("CHATGPT_PLAN_INVALID_ID_TOKEN".into());
    }
    Ok(discovery)
}

#[derive(Deserialize)]
pub(super) struct Identity {
    pub sub: String,
    pub email: Option<String>,
    pub iss: String,
    pub aud: Value,
    pub exp: i64,
    pub nonce: Option<String>,
    pub nbf: Option<i64>,
    pub iat: Option<i64>,
    pub azp: Option<String>,
}

fn validate_identity_claims(
    identity: &Identity,
    client_id: &str,
    nonce: Option<&str>,
    now: i64,
) -> Result<(), String> {
    let audience_matches = identity.aud.as_str() == Some(client_id)
        || identity
            .aud
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(client_id)));
    if identity.iss != ISSUER
        || !audience_matches
        || identity.exp <= now
        || identity.nbf.is_some_and(|nbf| nbf > now + 30)
        || identity.iat.is_some_and(|iat| iat > now + 60)
        || identity.azp.as_deref().is_some_and(|azp| azp != client_id)
        || identity.sub.trim().is_empty()
        || identity.sub.len() > 512
        || nonce.is_some_and(|nonce| identity.nonce.as_deref() != Some(nonce))
    {
        return Err("CHATGPT_PLAN_INVALID_ID_TOKEN".into());
    }
    Ok(())
}

fn validate_id_token_with_jwks(
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
    jwks: &Value,
    now: i64,
) -> Result<Identity, String> {
    let invalid = || "CHATGPT_PLAN_INVALID_ID_TOKEN".to_string();
    if token.len() > 65_536 {
        return Err(invalid());
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let header: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") || header.get("crit").is_some() {
        return Err(invalid());
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let key = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter().find(|key| {
                key.get("kid").and_then(Value::as_str) == Some(kid)
                    && key.get("kty").and_then(Value::as_str) == Some("RSA")
                    && key.get("use").and_then(Value::as_str).unwrap_or("sig") == "sig"
                    && key.get("alg").and_then(Value::as_str).unwrap_or("RS256") == "RS256"
            })
        })
        .ok_or_else(invalid)?;
    let n = URL_SAFE_NO_PAD
        .decode(key.get("n").and_then(Value::as_str).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let e = URL_SAFE_NO_PAD
        .decode(key.get("e").and_then(Value::as_str).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| invalid())?;
    let message = format!("{}.{}", parts[0], parts[1]);
    ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            message.as_bytes(),
            &signature,
        )
        .map_err(|_| invalid())?;
    let identity: Identity =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    validate_identity_claims(&identity, client_id, nonce, now)?;
    Ok(identity)
}

pub(super) async fn validate_id_token(
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<Identity, String> {
    let discovery = discovery().await?;
    let response = client(10)?
        .get(discovery.jwks_uri)
        .send()
        .await
        .map_err(network_error)?;
    validate_id_token_with_jwks(
        token,
        client_id,
        nonce,
        &json(response).await?,
        chrono::Utc::now().timestamp(),
    )
}

pub(super) fn parse_models(value: &Value) -> Result<Vec<ChatgptPlanModel>, String> {
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or("CHATGPT_PLAN_NO_MODELS")?;
    let mut seen = HashSet::new();
    let models: Vec<_> = models
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|model| {
            let id = model.get("slug")?.as_str()?.trim();
            if id.is_empty() || id.len() > 256 || !seen.insert(id.to_string()) {
                return None;
            }
            Some(ChatgptPlanModel {
                id: id.to_string(),
                display_name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(id)
                    .to_string(),
            })
        })
        .collect();
    if models.is_empty() {
        return Err("CHATGPT_PLAN_NO_MODELS".into());
    }
    Ok(models)
}

// SSE parsing is incremental: neither an early text delta nor [DONE] proves a
// successful request. Preserve frame boundaries even when UTF-8 is split.
pub(super) struct VerificationStream {
    buffer: Vec<u8>,
    pub completed: bool,
}

impl VerificationStream {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            completed: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > 1024 * 1024 {
            return Err("CHATGPT_PLAN_VERIFY_FAILED".into());
        }
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "CHATGPT_PLAN_VERIFY_FAILED".to_string())?
                .trim();
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                continue;
            }
            let event: Value =
                serde_json::from_str(data).map_err(|_| "CHATGPT_PLAN_VERIFY_FAILED".to_string())?;
            match event.get("type").and_then(Value::as_str) {
                Some("response.completed") => {
                    if event
                        .pointer("/response/status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| status != "completed")
                    {
                        return Err("CHATGPT_PLAN_VERIFY_FAILED".into());
                    }
                    self.completed = true;
                }
                Some("response.failed") | Some("error") => {
                    let value = event.get("response").unwrap_or(&event);
                    return Err(response_error(400, value));
                }
                Some("response.incomplete") => return Err("CHATGPT_PLAN_STREAM_INCOMPLETE".into()),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn finish(&self) -> Result<(), String> {
        if self.completed {
            Ok(())
        } else {
            Err("CHATGPT_PLAN_STREAM_INCOMPLETE".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_claims_bind_registration_and_nonce() {
        let mut identity: Identity = serde_json::from_value(serde_json::json!({
            "sub":"subject", "iss":ISSUER, "aud":["client"], "exp":2000,"nonce":"nonce"
        }))
        .unwrap();
        assert!(validate_identity_claims(&identity, "client", Some("nonce"), 1000).is_ok());
        assert!(validate_identity_claims(&identity, "other", Some("nonce"), 1000).is_err());
        assert!(validate_identity_claims(&identity, "client", Some("other"), 1000).is_err());
        identity.exp = 999;
        assert!(validate_identity_claims(&identity, "client", Some("nonce"), 1000).is_err());
    }

    #[test]
    fn unsigned_jwt_is_always_rejected() {
        let token = format!(
            "{}.e30.",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"key"}"#)
        );
        assert!(validate_id_token_with_jwks(
            &token,
            "client",
            None,
            &serde_json::json!({"keys":[]}),
            0
        )
        .is_err());
    }

    #[test]
    fn scopes_require_explicit_plan_and_resource_grants() {
        assert!(scopes("openid profile email offline_access").is_err());
        assert!(scopes("offline_access resource.invoke chatgpt.tokens.use.direct").is_ok());
    }

    #[test]
    fn plan_eligibility_errors_do_not_request_another_oauth_login() {
        for value in [
            serde_json::json!({"error":{"code":"subscription_sharing_user_not_eligible"}}),
            serde_json::json!({"type":"error","code":"subscription_sharing_user_not_eligible"}),
            serde_json::json!({"error":{"type":"subscription_sharing_user_not_eligible"}}),
        ] {
            assert_eq!(response_error(403, &value), "CHATGPT_PLAN_NOT_ELIGIBLE");
        }
        assert_eq!(
            response_error(
                429,
                &serde_json::json!({"error":{"code":"subscription_sharing_usage_limit_exceeded"}})
            ),
            "CHATGPT_PLAN_USAGE_LIMIT"
        );
        assert_eq!(
            response_error(
                503,
                &serde_json::json!({"error":{"code":"subscription_sharing_usage_unavailable"}})
            ),
            "CHATGPT_PLAN_USAGE_UNAVAILABLE"
        );
    }

    #[test]
    fn plan_error_codes_preserve_transient_and_configuration_failures() {
        for code in [
            "subscription_sharing_route_not_supported",
            "subscription_sharing_invalid_user",
            "chatpass_v2_scope_not_authorized",
            "chatpass_v2_invalid_authorization_context",
        ] {
            assert_eq!(
                response_error(403, &serde_json::json!({"error":{"code":code}})),
                "CHATGPT_PLAN_CONFIGURATION"
            );
        }
        assert_eq!(
            response_error(
                503,
                &serde_json::json!({"error":{"code":"subscription_sharing_user_unavailable"}})
            ),
            "CHATGPT_PLAN_NETWORK"
        );
        assert_eq!(
            response_error(
                400,
                &serde_json::json!({"error":{"code":"subscription_sharing_unsupported_capability", "param":"tools[0]"}})
            ),
            "CHATGPT_PLAN_UNSUPPORTED_CAPABILITY"
        );
        // A precise code wins over a generic or contradictory type.
        assert_eq!(
            response_error(
                403,
                &serde_json::json!({"error":{"code":"subscription_sharing_user_not_eligible", "type":"insufficient_scope"}})
            ),
            "CHATGPT_PLAN_NOT_ELIGIBLE"
        );
    }

    #[test]
    fn model_picker_filters_visibility_and_preserves_server_order() {
        let models = parse_models(&serde_json::json!({"models":[
            {"slug":"z", "display_name":"Z", "visibility":"list"},
            {"slug":"hidden", "visibility":"hidden"},
            {"slug":"a", "display_name":"A", "visibility":"list"}
        ]}))
        .unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["z", "a"]
        );
    }

    #[test]
    fn verification_requires_completed_and_catches_late_usage_error() {
        let mut stream = VerificationStream::new();
        stream
            .push(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n")
            .unwrap();
        assert!(stream.finish().is_err());
        assert_eq!(stream.push(b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n"), Err("CHATGPT_PLAN_USAGE_LIMIT".into()));
        let mut stream = VerificationStream::new();
        stream.push(b"data: {\"type\":\"response.com").unwrap();
        stream.push(b"pleted\"}\r\n\r\n").unwrap();
        assert!(stream.finish().is_ok());
    }

    #[test]
    fn discovery_cannot_send_tokens_to_foreign_or_plaintext_endpoints() {
        assert!(trusted_https("https://auth.openai.com/oauth/revoke"));
        for url in [
            "http://auth.openai.com/revoke",
            "https://evil.example/revoke",
            "https://auth.openai.com:444/revoke",
            "https://user@auth.openai.com/revoke",
        ] {
            assert!(!trusted_https(url));
        }
    }
}
