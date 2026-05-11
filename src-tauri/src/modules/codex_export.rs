use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexExportFormat {
    CockpitTools,
    Sub2api,
    Cpa,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CodexExportContent {
    Single {
        file_name_base: String,
        json_content: String,
    },
    Multiple {
        file_name_base: String,
        documents: Vec<CodexExportDocument>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexExportDocument {
    pub id: String,
    pub label: String,
    pub file_name_base: String,
    pub json_content: String,
}

pub fn build_codex_export_content(
    raw_json: &str,
    format: CodexExportFormat,
    base_name: &str,
) -> Result<CodexExportContent, String> {
    let file_name_base = build_codex_export_file_name_base(base_name, &format);
    let accounts = parse_accounts(raw_json)?;

    if format != CodexExportFormat::Cpa || accounts.len() <= 1 {
        return Ok(CodexExportContent::Single {
            file_name_base,
            json_content: transform_codex_export_json(raw_json, format)?,
        });
    }

    let documents = accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            let account_id = resolve_account_id(account);
            let id = format!(
                "{}_{}",
                string_value(account.get("id"))
                    .or(account_id.clone())
                    .unwrap_or_else(|| "cpa_account".to_string()),
                index
            );
            Ok(CodexExportDocument {
                id,
                label: resolve_cpa_document_label(account, index),
                file_name_base: build_cpa_document_file_name_base(&file_name_base, account, index),
                json_content: serde_json::to_string_pretty(&to_portable_token_storage(account)?)
                    .map_err(|err| err.to_string())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(CodexExportContent::Multiple {
        file_name_base,
        documents,
    })
}

pub fn build_codex_export_file_name_base(base_name: &str, format: &CodexExportFormat) -> String {
    match format {
        CodexExportFormat::CockpitTools => base_name.to_string(),
        CodexExportFormat::Sub2api => format!("{base_name}_sub2api"),
        CodexExportFormat::Cpa => format!("{base_name}_cpa"),
    }
}

pub fn transform_codex_export_json(
    raw_json: &str,
    format: CodexExportFormat,
) -> Result<String, String> {
    let accounts = parse_accounts(raw_json)?;
    let payload = match format {
        CodexExportFormat::CockpitTools => Value::Array(
            accounts
                .iter()
                .map(to_cockpit_tools_portable_storage)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        CodexExportFormat::Sub2api => json!({
            "exported_at": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            "proxies": [],
            "accounts": accounts.iter().map(to_sub2api_account).collect::<Result<Vec<_>, _>>()?,
            "type": "sub2api-data",
            "version": 1
        }),
        CodexExportFormat::Cpa => {
            let items = accounts
                .iter()
                .map(to_portable_token_storage)
                .collect::<Result<Vec<_>, _>>()?;
            if items.len() == 1 {
                items.into_iter().next().unwrap_or(Value::Array(Vec::new()))
            } else {
                Value::Array(items)
            }
        }
    };
    serde_json::to_string_pretty(&payload).map_err(|err| err.to_string())
}

fn parse_accounts(raw_json: &str) -> Result<Vec<Value>, String> {
    let parsed = serde_json::from_str::<Value>(raw_json).map_err(|_| "invalid_json".to_string())?;
    Ok(match parsed {
        Value::Array(items) => items
            .into_iter()
            .filter(|item| item.as_object().is_some())
            .collect(),
        Value::Object(_) => vec![parsed],
        _ => Vec::new(),
    })
}

fn record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn number_value(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64().filter(|value| value.is_finite()),
        _ => None,
    }
}

fn decode_jwt_payload(token: Option<String>) -> Option<Value> {
    let token = token?;
    let payload_part = token.split('.').nth(1)?;
    let padded = format!(
        "{}{}",
        payload_part,
        "=".repeat((4 - (payload_part.len() % 4)) % 4)
    );
    let bytes = URL_SAFE.decode(padded).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

fn resolve_auth_payload(account: &Value) -> Option<Map<String, Value>> {
    let token = record(account)
        .and_then(|account| account.get("tokens"))
        .and_then(record)
        .and_then(|tokens| string_value(tokens.get("id_token")));
    decode_jwt_payload(token)
        .and_then(|payload| payload.get("https://api.openai.com/auth").cloned())
        .and_then(|payload| payload.as_object().cloned())
}

fn resolve_account_id(account: &Value) -> Option<String> {
    let auth_payload = resolve_auth_payload(account);
    string_value(record(account)?.get("account_id"))
        .or_else(|| {
            auth_payload
                .as_ref()
                .and_then(|auth| string_value(auth.get("chatgpt_account_id")))
        })
        .or_else(|| {
            auth_payload
                .as_ref()
                .and_then(|auth| string_value(auth.get("account_id")))
        })
}

fn resolve_user_id(account: &Value) -> Option<String> {
    let account_record = record(account)?;
    let id_token_payload = account_record
        .get("tokens")
        .and_then(record)
        .and_then(|tokens| string_value(tokens.get("id_token")))
        .and_then(|token| decode_jwt_payload(Some(token)));
    let auth_payload = resolve_auth_payload(account);
    string_value(account_record.get("user_id"))
        .or_else(|| {
            auth_payload
                .as_ref()
                .and_then(|auth| string_value(auth.get("chatgpt_user_id")))
        })
        .or_else(|| {
            auth_payload
                .as_ref()
                .and_then(|auth| string_value(auth.get("user_id")))
        })
        .or_else(|| {
            id_token_payload
                .as_ref()
                .and_then(|payload| string_value(payload.get("sub")))
        })
}

fn resolve_organization_id(account: &Value) -> Option<String> {
    let auth_payload = resolve_auth_payload(account);
    string_value(record(account)?.get("organization_id")).or_else(|| {
        auth_payload
            .as_ref()
            .and_then(|auth| string_value(auth.get("organization_id")))
    })
}

fn resolve_plan_type(account: &Value) -> Option<String> {
    let auth_payload = resolve_auth_payload(account);
    string_value(record(account)?.get("plan_type")).or_else(|| {
        auth_payload
            .as_ref()
            .and_then(|auth| string_value(auth.get("chatgpt_plan_type")))
    })
}

fn normalize_timestamp_to_iso(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(number) = trimmed.parse::<f64>() {
                return normalize_numeric_timestamp_to_iso(number);
            }
            DateTime::parse_from_rfc3339(trimmed)
                .map(|date| {
                    date.with_timezone(&Utc)
                        .to_rfc3339_opts(SecondsFormat::Millis, true)
                })
                .ok()
                .or_else(|| Some(trimmed.to_string()))
        }
        Some(Value::Number(_)) => number_value(value).and_then(normalize_numeric_timestamp_to_iso),
        _ => None,
    }
}

fn normalize_numeric_timestamp_to_iso(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let millis = if value > 1_000_000_000_000.0 {
        value as i64
    } else {
        (value * 1000.0) as i64
    };
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn resolve_subscription_expires_at(account: &Value) -> Option<String> {
    let auth_payload = resolve_auth_payload(account);
    normalize_timestamp_to_iso(record(account)?.get("subscription_active_until")).or_else(|| {
        auth_payload.as_ref().and_then(|auth| {
            normalize_timestamp_to_iso(auth.get("chatgpt_subscription_active_until"))
        })
    })
}

fn resolve_access_token_expiry(account: &Value) -> Option<String> {
    let account_record = record(account)?;
    let tokens = account_record.get("tokens").and_then(record)?;
    let access_token_payload =
        string_value(tokens.get("access_token")).and_then(|token| decode_jwt_payload(Some(token)));
    let id_token_payload =
        string_value(tokens.get("id_token")).and_then(|token| decode_jwt_payload(Some(token)));
    access_token_payload
        .as_ref()
        .and_then(|payload| number_value(payload.get("exp")))
        .and_then(normalize_numeric_timestamp_to_iso)
        .or_else(|| {
            id_token_payload
                .as_ref()
                .and_then(|payload| number_value(payload.get("exp")))
                .and_then(normalize_numeric_timestamp_to_iso)
        })
}

fn resolve_last_refresh(account: &Value) -> String {
    normalize_timestamp_to_iso(record(account).and_then(|account| account.get("token_updated_at")))
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn token_value(account: &Value, key: &str) -> String {
    record(account)
        .and_then(|account| account.get("tokens"))
        .and_then(record)
        .and_then(|tokens| string_value(tokens.get(key)))
        .unwrap_or_default()
}

fn build_sub2api_credentials(account: &Value) -> Result<Value, String> {
    let mut credentials = Map::new();
    credentials.insert(
        "access_token".to_string(),
        Value::String(token_value(account, "access_token")),
    );
    insert_if_some(
        &mut credentials,
        "expires_at",
        resolve_access_token_expiry(account),
    );
    insert_if_some(
        &mut credentials,
        "refresh_token",
        token_value_trimmed(account, "refresh_token"),
    );
    insert_if_some(
        &mut credentials,
        "id_token",
        token_value_trimmed(account, "id_token"),
    );
    insert_if_some(
        &mut credentials,
        "email",
        string_value(record(account).and_then(|a| a.get("email"))),
    );
    insert_if_some(
        &mut credentials,
        "chatgpt_account_id",
        resolve_account_id(account),
    );
    insert_if_some(
        &mut credentials,
        "chatgpt_user_id",
        resolve_user_id(account),
    );
    insert_if_some(
        &mut credentials,
        "organization_id",
        resolve_organization_id(account),
    );
    insert_if_some(&mut credentials, "plan_type", resolve_plan_type(account));
    insert_if_some(
        &mut credentials,
        "subscription_expires_at",
        resolve_subscription_expires_at(account),
    );
    Ok(Value::Object(credentials))
}

fn to_sub2api_account(account: &Value) -> Result<Value, String> {
    let account_record = record(account).ok_or_else(|| "invalid_account".to_string())?;
    Ok(json!({
        "name": string_value(account_record.get("account_name"))
            .or_else(|| string_value(account_record.get("email")))
            .or_else(|| string_value(account_record.get("id")))
            .unwrap_or_default(),
        "platform": "openai",
        "type": "oauth",
        "credentials": build_sub2api_credentials(account)?,
        "concurrency": 0,
        "priority": 0
    }))
}

fn token_value_trimmed(account: &Value, key: &str) -> Option<String> {
    let value = token_value(account, key);
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn to_portable_token_storage(account: &Value) -> Result<Value, String> {
    let account_record = record(account).ok_or_else(|| "invalid_account".to_string())?;
    Ok(json!({
        "id_token": token_value(account, "id_token"),
        "access_token": token_value(account, "access_token"),
        "refresh_token": token_value_trimmed(account, "refresh_token").unwrap_or_default(),
        "account_id": resolve_account_id(account).unwrap_or_default(),
        "last_refresh": resolve_last_refresh(account),
        "email": string_value(account_record.get("email")).unwrap_or_default(),
        "type": "codex",
        "expired": resolve_access_token_expiry(account).unwrap_or_default()
    }))
}

fn is_codex_api_key_account(account: &Value) -> bool {
    record(account)
        .and_then(|account| string_value(account.get("auth_mode")))
        .is_some_and(|mode| mode == "apikey")
        || record(account)
            .and_then(|account| string_value(account.get("openai_api_key")))
            .is_some()
}

fn to_portable_api_key_storage(account: &Value) -> Result<Value, String> {
    let account_record = record(account).ok_or_else(|| "invalid_account".to_string())?;
    let mut payload = Map::new();
    payload.insert("auth_mode".to_string(), Value::String("apikey".to_string()));
    payload.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(string_value(account_record.get("openai_api_key")).unwrap_or_default()),
    );
    payload.insert(
        "email".to_string(),
        Value::String(string_value(account_record.get("email")).unwrap_or_default()),
    );
    insert_if_some(
        &mut payload,
        "api_base_url",
        string_value(account_record.get("api_base_url")),
    );
    insert_if_some(
        &mut payload,
        "api_provider_id",
        string_value(account_record.get("api_provider_id")),
    );
    insert_if_some(
        &mut payload,
        "api_provider_name",
        string_value(account_record.get("api_provider_name")),
    );
    Ok(Value::Object(payload))
}

fn to_cockpit_tools_portable_storage(account: &Value) -> Result<Value, String> {
    if is_codex_api_key_account(account) {
        to_portable_api_key_storage(account)
    } else {
        to_portable_token_storage(account)
    }
}

fn insert_if_some(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::String(value));
    }
}

fn sanitize_file_name_segment(input: Option<String>, fallback: &str) -> String {
    let raw = input.unwrap_or_default();
    let mut normalized = String::new();
    let mut previous_underscore = false;
    for ch in raw.trim().chars() {
        let is_invalid = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || ch.is_control()
            || ch.is_whitespace();
        if is_invalid {
            if !previous_underscore {
                normalized.push('_');
                previous_underscore = true;
            }
        } else {
            normalized.push(ch);
            previous_underscore = false;
        }
    }
    let normalized = normalized.trim_matches('_').to_string();
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

fn resolve_cpa_document_label(account: &Value, index: usize) -> String {
    let account_record = record(account);
    account_record
        .and_then(|record| string_value(record.get("email")))
        .or_else(|| resolve_account_id(account))
        .or_else(|| account_record.and_then(|record| string_value(record.get("account_name"))))
        .or_else(|| account_record.and_then(|record| string_value(record.get("id"))))
        .unwrap_or_else(|| format!("account_{}", index + 1))
}

fn build_cpa_document_file_name_base(base_name: &str, account: &Value, index: usize) -> String {
    let account_record = record(account);
    let label = sanitize_file_name_segment(
        account_record
            .and_then(|record| string_value(record.get("email")))
            .or_else(|| resolve_account_id(account))
            .or_else(|| account_record.and_then(|record| string_value(record.get("id")))),
        &format!("account_{}", index + 1),
    );
    let account_id_suffix = sanitize_file_name_segment(resolve_account_id(account), "");
    let suffix = if !account_id_suffix.is_empty() && account_id_suffix != label {
        let suffix = account_id_suffix
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("_{suffix}")
    } else {
        String::new()
    };
    format!("{base_name}_{:02}_{label}{suffix}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_content_should_generate_cockpit_tools_api_key_payload() {
        let raw = json!([{
            "id": "acc-1",
            "email": "person@example.com",
            "auth_mode": "apikey",
            "openai_api_key": "sk-test",
            "api_base_url": " https://api.example.com ",
            "api_provider_id": " provider-1 ",
            "api_provider_name": " Example ",
            "tokens": { "id_token": "", "access_token": "" },
            "created_at": 1,
            "last_used": 1
        }])
        .to_string();

        let content =
            build_codex_export_content(&raw, CodexExportFormat::CockpitTools, "codex_accounts")
                .unwrap();

        let CodexExportContent::Single {
            file_name_base,
            json_content,
        } = content
        else {
            panic!("expected single export");
        };
        assert_eq!(file_name_base, "codex_accounts");
        let payload: serde_json::Value = serde_json::from_str(&json_content).unwrap();
        assert_eq!(payload[0]["auth_mode"], "apikey");
        assert_eq!(payload[0]["OPENAI_API_KEY"], "sk-test");
        assert_eq!(payload[0]["api_base_url"], "https://api.example.com");
    }

    #[test]
    fn build_content_should_split_multiple_cpa_accounts_into_documents() {
        let raw = json!([
            {
                "id": "acc-1",
                "email": "first@example.com",
                "account_id": "chatgpt-account-111111",
                "tokens": { "id_token": "id1", "access_token": "access1", "refresh_token": " refresh1 " },
                "created_at": 1,
                "last_used": 1
            },
            {
                "id": "acc-2",
                "email": "bad/name@example.com",
                "account_id": "chatgpt-account-222222",
                "tokens": { "id_token": "id2", "access_token": "access2" },
                "created_at": 2,
                "last_used": 2
            }
        ])
        .to_string();

        let content = build_codex_export_content(&raw, CodexExportFormat::Cpa, "codex").unwrap();

        let CodexExportContent::Multiple {
            file_name_base,
            documents,
        } = content
        else {
            panic!("expected multiple export");
        };
        assert_eq!(file_name_base, "codex_cpa");
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].label, "first@example.com");
        assert_eq!(
            documents[0].file_name_base,
            "codex_cpa_01_first@example.com_111111"
        );
        assert_eq!(
            documents[1].file_name_base,
            "codex_cpa_02_bad_name@example.com_222222"
        );
    }

    #[test]
    fn build_content_should_generate_sub2api_payload() {
        let raw = json!({
            "id": "acc-1",
            "email": "person@example.com",
            "account_name": " My Codex ",
            "account_id": "chatgpt-account",
            "user_id": "user-1",
            "organization_id": "org-1",
            "plan_type": "plus",
            "tokens": { "id_token": "id", "access_token": "access", "refresh_token": "refresh" },
            "created_at": 1,
            "last_used": 1
        })
        .to_string();

        let content =
            build_codex_export_content(&raw, CodexExportFormat::Sub2api, "codex").unwrap();

        let CodexExportContent::Single {
            file_name_base,
            json_content,
        } = content
        else {
            panic!("expected single export");
        };
        assert_eq!(file_name_base, "codex_sub2api");
        let payload: serde_json::Value = serde_json::from_str(&json_content).unwrap();
        assert_eq!(payload["type"], "sub2api-data");
        assert_eq!(payload["accounts"][0]["name"], "My Codex");
        assert_eq!(
            payload["accounts"][0]["credentials"]["chatgpt_account_id"],
            "chatgpt-account"
        );
    }
}
