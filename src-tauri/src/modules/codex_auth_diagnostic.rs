use crate::models::codex::CodexTokens;
use crate::modules::logger;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// 将 token 转成可用于跨日志比对的摘要，不包含可直接认证的原文。
pub fn token_summary(token: &str, token_kind: &str) -> Value {
    let trimmed = token.trim();
    let mut result = Map::new();
    result.insert("kind".to_string(), json!(token_kind));
    result.insert("present".to_string(), json!(!trimmed.is_empty()));
    result.insert("length".to_string(), json!(trimmed.len()));
    if trimmed.is_empty() {
        return Value::Object(result);
    }

    result.insert("sha256".to_string(), json!(sha256_hex(trimmed)));
    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.len() != 3 {
        result.insert("format".to_string(), json!("opaque"));
        return Value::Object(result);
    }

    result.insert("format".to_string(), json!("jwt"));
    let claims = decode_jwt_payload(parts[1]);
    if let Some(claims) = claims.as_object() {
        result.insert("claims".to_string(), Value::Object(claims.clone()));
        for field in ["iat", "exp", "nbf", "auth_time"] {
            if let Some(timestamp) = claims.get(field).and_then(Value::as_i64) {
                result.insert(format!("{}_iso", field), json!(timestamp_to_iso(timestamp)));
            }
        }
    } else {
        result.insert("claims_parse_error".to_string(), json!(true));
    }
    Value::Object(result)
}

pub fn tokens_summary(tokens: &CodexTokens) -> Value {
    json!({
        "id_token": token_summary(&tokens.id_token, "id_token"),
        "access_token": token_summary(&tokens.access_token, "access_token"),
        "refresh_token": tokens.refresh_token.as_deref()
            .map(|token| token_summary(token, "refresh_token"))
            .unwrap_or_else(|| json!({"kind":"refresh_token", "present":false})),
    })
}

/// 记录 OAuth 响应的完整字段结构和所有非凭据标量字段；凭据字段只保留存在性、长度和指纹。
pub fn oauth_response_summary(response: &Value) -> Value {
    let Some(object) = response.as_object() else {
        return json!({"response_type": value_type(response)});
    };
    let mut safe_fields = Map::new();
    let mut sensitive_fields = Map::new();
    for (key, value) in object {
        if is_sensitive_field(key) {
            sensitive_fields.insert(key.clone(), sensitive_value_summary(value));
        } else {
            safe_fields.insert(key.clone(), summarize_safe_value(value));
        }
    }
    json!({
        "field_names": object.keys().collect::<Vec<_>>(),
        "safe_fields": safe_fields,
        "sensitive_fields": sensitive_fields,
    })
}

pub fn log_event(event: &str, fields: Value) {
    if !crate::modules::account::is_dev_profile() {
        return;
    }
    let payload = json!({
        "event": event,
        "fields": fields,
    });
    let message = serde_json::to_string(&payload).unwrap_or_else(|error| {
        format!(
            "{{\"event\":\"{}\",\"serialization_error\":\"{}\"}}",
            event, error
        )
    });
    logger::log_codex_auth_diagnostic(&message);
}

fn is_sensitive_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "authorization_code",
        "code_verifier",
        "assertion",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn sensitive_value_summary(value: &Value) -> Value {
    let text = value.as_str().unwrap_or_default();
    if text.is_empty() {
        return json!({"present": !value.is_null(), "length": 0});
    }
    json!({
        "present": true,
        "length": text.len(),
        "sha256": sha256_hex(text),
    })
}

fn summarize_safe_value(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
        Value::Array(items) => json!({"type":"array", "length":items.len()}),
        Value::Object(object) => json!({
            "type":"object",
            "field_names": object.keys().collect::<Vec<_>>(),
        }),
    }
}

fn decode_jwt_payload(payload: &str) -> Value {
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload) else {
        return Value::Null;
    };
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn timestamp_to_iso(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| "invalid_timestamp".to_string())
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::{oauth_response_summary, token_summary};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    #[test]
    fn token_summary_keeps_claims_but_never_token_value() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD
            .encode(r#"{"iat":1700000000,"exp":1700003600,"email":"test@example.com"}"#);
        let token = format!("{}.{}.signature-value", header, payload);
        let summary = token_summary(&token, "access_token");
        let serialized = summary.to_string();

        assert!(!serialized.contains(&token));
        assert_eq!(summary["format"], "jwt");
        assert_eq!(summary["claims"]["exp"], 1700003600);
        assert!(serialized.contains("2023-11-14T23:13:20+00:00"));
    }

    #[test]
    fn oauth_response_summary_only_fingerprints_sensitive_fields() {
        let response = serde_json::json!({
            "access_token": "access-secret",
            "refresh_token": "refresh-secret",
            "expires_in": 3600,
            "token_type": "Bearer",
        });
        let summary = oauth_response_summary(&response);
        let serialized = summary.to_string();

        assert!(!serialized.contains("access-secret"));
        assert!(!serialized.contains("refresh-secret"));
        assert_eq!(summary["safe_fields"]["expires_in"], 3600);
        assert_eq!(summary["sensitive_fields"]["access_token"]["present"], true);
    }
}
