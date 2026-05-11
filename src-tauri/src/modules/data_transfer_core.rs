use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const DATA_TRANSFER_SCHEMA: &str = "cockpit-tools.data-transfer";
pub const DATA_TRANSFER_VERSION: i64 = 1;
pub const ACCOUNT_TRANSFER_SCHEMA: &str = "cockpit-tools.account-transfer";
pub const ACCOUNT_TRANSFER_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectedTransferFormat {
    DataBundle,
    AccountBundle,
    LegacyAccountJson,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataTransferAnalysis {
    pub detected_format: DetectedTransferFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_account_platform: Option<String>,
}

pub fn analyze_data_transfer_json(json_content: &str) -> Result<DataTransferAnalysis, String> {
    let parsed = parse_json(json_content)?;
    if is_schema(&parsed, DATA_TRANSFER_SCHEMA) {
        ensure_version(&parsed, DATA_TRANSFER_VERSION)?;
        return Ok(DataTransferAnalysis {
            detected_format: DetectedTransferFormat::DataBundle,
            legacy_account_platform: None,
        });
    }
    if is_schema(&parsed, ACCOUNT_TRANSFER_SCHEMA) {
        ensure_version(&parsed, ACCOUNT_TRANSFER_VERSION)?;
        return Ok(DataTransferAnalysis {
            detected_format: DetectedTransferFormat::AccountBundle,
            legacy_account_platform: None,
        });
    }
    let platform = detect_legacy_platform(&parsed)
        .ok_or_else(|| "unsupported_legacy_account_json".to_string())?;
    Ok(DataTransferAnalysis {
        detected_format: DetectedTransferFormat::LegacyAccountJson,
        legacy_account_platform: Some(platform.to_string()),
    })
}

pub fn parse_account_transfer_platforms(json_content: &str) -> Result<Value, String> {
    let parsed = parse_json(json_content)?;
    if !parsed.is_object() {
        return Err("invalid_bundle_root".to_string());
    }
    if !is_schema(&parsed, ACCOUNT_TRANSFER_SCHEMA) {
        return Err("invalid_bundle_schema".to_string());
    }
    ensure_version(&parsed, ACCOUNT_TRANSFER_VERSION)?;
    let raw_platforms = parsed
        .get("platforms")
        .and_then(Value::as_object)
        .ok_or_else(|| "invalid_bundle_platforms".to_string())?;
    let mut platforms = Map::new();
    for platform in ALL_PLATFORM_IDS.iter().copied() {
        let payload = raw_platforms
            .get(platform)
            .map(resolve_platform_payload)
            .unwrap_or_else(empty_platform_payload);
        platforms.insert(platform.to_string(), payload);
    }
    Ok(Value::Object(platforms))
}

const ALL_PLATFORM_IDS: &[&str] = &[
    "antigravity",
    "codex",
    "zed",
    "github-copilot",
    "windsurf",
    "kiro",
    "cursor",
    "gemini",
    "devin-cli",
    "codebuddy",
    "codebuddy_cn",
    "qoder",
    "trae",
    "workbuddy",
];

fn parse_json(json_content: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(json_content).map_err(|_| "invalid_json".to_string())
}

fn is_schema(value: &Value, schema: &str) -> bool {
    value
        .as_object()
        .and_then(|record| record.get("schema"))
        .and_then(Value::as_str)
        == Some(schema)
}

fn ensure_version(value: &Value, version: i64) -> Result<(), String> {
    let matches_version = value
        .as_object()
        .and_then(|record| record.get("version"))
        .and_then(Value::as_i64)
        == Some(version);
    if matches_version {
        Ok(())
    } else {
        Err("invalid_bundle_version".to_string())
    }
}

fn empty_platform_payload() -> Value {
    json!({
        "account_count": 0,
        "exported_data": []
    })
}

fn resolve_platform_payload(raw: &Value) -> Value {
    if raw.is_null() {
        return empty_platform_payload();
    }
    if let Some(record) = raw.as_object() {
        if record.contains_key("account_count") || record.contains_key("exported_data") {
            let exported_data = record
                .get("exported_data")
                .or_else(|| record.get("data"))
                .or_else(|| record.get("accounts"))
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let account_count = record
                .get("account_count")
                .and_then(Value::as_i64)
                .filter(|count| *count >= 0)
                .unwrap_or_else(|| {
                    exported_data
                        .as_array()
                        .map_or(0, |items| items.len() as i64)
                });
            return json!({
                "account_count": account_count,
                "exported_data": exported_data
            });
        }
    }
    json!({
        "account_count": raw.as_array().map_or(0, |items| items.len() as i64),
        "exported_data": raw.clone()
    })
}

fn first_legacy_sample(value: &Value) -> Option<&Map<String, Value>> {
    if let Some(items) = value.as_array() {
        return items.iter().find_map(Value::as_object);
    }
    value.as_object()
}

fn normalize_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
        }
        _ => None,
    })
}

fn string_contains(value: Option<&Value>, keyword: &str) -> bool {
    normalize_string(value).is_some_and(|value| value.contains(&keyword.to_ascii_lowercase()))
}

fn detect_legacy_platform(value: &Value) -> Option<&'static str> {
    let sample = first_legacy_sample(value)?;
    let id = normalize_string(sample.get("id"));
    if id
        .as_deref()
        .is_some_and(|id| id.starts_with("codebuddy_cn_"))
    {
        return Some("codebuddy_cn");
    }
    if id.as_deref().is_some_and(|id| id.starts_with("workbuddy_")) {
        return Some("workbuddy");
    }
    if id.as_deref().is_some_and(|id| id.starts_with("codebuddy_")) {
        return Some("codebuddy");
    }
    if sample.contains_key("tokens")
        || sample.contains_key("OPENAI_API_KEY")
        || sample.contains_key("auth_mode")
        || sample.contains_key("authMode")
    {
        return Some("codex");
    }
    if sample.contains_key("windsurf_api_key")
        || sample.contains_key("windsurf_auth_token")
        || sample.contains_key("windsurf_plan_status")
    {
        return Some("windsurf");
    }
    if sample.contains_key("copilot_token") {
        return Some("github-copilot");
    }
    if sample.contains_key("zed")
        || sample.contains_key("user_raw")
        || sample.contains_key("subscription_raw")
        || sample.contains_key("plan_raw")
    {
        return Some("zed");
    }
    if sample.contains_key("kiro_auth_token_raw")
        || sample.contains_key("kiro_usage_raw")
        || sample.contains_key("login_provider")
    {
        return Some("kiro");
    }
    if sample.contains_key("gemini_auth_raw")
        || sample.contains_key("gemini_usage_raw")
        || sample.contains_key("selected_auth_type")
    {
        return Some("gemini");
    }
    if sample.contains_key("cursor_auth_raw")
        || sample.contains_key("cursor_usage_raw")
        || sample.contains_key("membership_type")
    {
        return Some("cursor");
    }
    if sample.contains_key("trae_auth_raw")
        || sample.contains_key("trae_profile_raw")
        || sample.contains_key("trae_server_raw")
    {
        return Some("trae");
    }
    if sample.contains_key("auth_user_info_raw")
        || sample.contains_key("auth_credit_usage_raw")
        || sample.contains_key("credits_usage_percent")
    {
        return Some("qoder");
    }
    if sample.contains_key("uid")
        || sample.contains_key("enterprise_id")
        || sample.contains_key("dosage_notify_code")
    {
        if string_contains(sample.get("domain"), "workbuddy") {
            return Some("workbuddy");
        }
        if string_contains(sample.get("domain"), "codebuddy.cn") {
            return Some("codebuddy_cn");
        }
        if string_contains(sample.get("domain"), "codebuddy") {
            return Some("codebuddy");
        }
        return if id.as_deref().is_some_and(|id| id.starts_with("workbuddy_")) {
            Some("workbuddy")
        } else if id
            .as_deref()
            .is_some_and(|id| id.starts_with("codebuddy_cn_"))
        {
            Some("codebuddy_cn")
        } else {
            Some("codebuddy")
        };
    }
    if sample.contains_key("github_login")
        && sample.contains_key("user_id")
        && (sample.contains_key("plan_raw") || sample.contains_key("usage_raw"))
    {
        return Some("zed");
    }
    if sample.contains_key("github_login") || sample.contains_key("github_id") {
        return Some("github-copilot");
    }
    if sample.contains_key("token")
        || (sample.contains_key("refresh_token") && sample.contains_key("email"))
    {
        return Some("antigravity");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn analyze_should_detect_data_transfer_bundle() {
        let raw = json!({
            "schema": DATA_TRANSFER_SCHEMA,
            "version": DATA_TRANSFER_VERSION,
            "sections": { "accounts": true, "config": false }
        })
        .to_string();

        let analysis = analyze_data_transfer_json(&raw).unwrap();

        assert_eq!(analysis.detected_format, DetectedTransferFormat::DataBundle);
        assert_eq!(analysis.legacy_account_platform, None);
    }

    #[test]
    fn analyze_should_reject_wrong_data_transfer_version() {
        let raw = json!({
            "schema": DATA_TRANSFER_SCHEMA,
            "version": 999,
            "sections": { "accounts": true, "config": false }
        })
        .to_string();

        let error = analyze_data_transfer_json(&raw).unwrap_err();

        assert_eq!(error, "invalid_bundle_version");
    }

    #[test]
    fn analyze_should_detect_legacy_codex_json() {
        let raw = json!({
            "id": "codex-1",
            "email": "person@example.com",
            "tokens": { "access_token": "access", "id_token": "id" }
        })
        .to_string();

        let analysis = analyze_data_transfer_json(&raw).unwrap();

        assert_eq!(
            analysis.detected_format,
            DetectedTransferFormat::LegacyAccountJson
        );
        assert_eq!(analysis.legacy_account_platform.as_deref(), Some("codex"));
    }

    #[test]
    fn parse_account_transfer_should_normalize_missing_platforms() {
        let raw = json!({
            "schema": ACCOUNT_TRANSFER_SCHEMA,
            "version": ACCOUNT_TRANSFER_VERSION,
            "platforms": {
                "codex": [{ "id": "codex-1" }],
                "windsurf": { "account_count": 2, "exported_data": [{ "id": "w1" }, { "id": "w2" }] }
            }
        })
        .to_string();

        let platforms = parse_account_transfer_platforms(&raw).unwrap();

        assert_eq!(platforms["codex"]["account_count"], 1);
        assert!(platforms["codex"]["exported_data"].is_array());
        assert_eq!(platforms["windsurf"]["account_count"], 2);
        assert_eq!(platforms["antigravity"]["account_count"], 0);
    }
}
