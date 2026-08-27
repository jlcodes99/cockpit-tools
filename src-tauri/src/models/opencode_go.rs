use serde::{Deserialize, Serialize};

/// Cockpit-owned OpenCode Go account metadata.
///
/// API-key material deliberately lives only in the encrypted storage model and is never part of
/// this public account type. Consumers identify the credential by an irreversible display hint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoAccount {
    pub id: String,
    pub name: String,
    pub key_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<OpenCodeGoQuotaSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<OpenCodeGoQuotaError>,
}

/// Compatibility name for the command boundary while connection consumers migrate to accounts.
pub type OpenCodeGoConnectionSummary = OpenCodeGoAccount;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoQuotaWindowSnapshot {
    #[serde(default = "default_quota_window_status")]
    pub status: String,
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_provider() -> String {
    "go".to_string()
}

fn default_quota_window_status() -> String {
    "available".to_string()
}

/// Sanitized OpenCode Go quota data. Upstream response bodies are intentionally excluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoQuotaSnapshot {
    pub rolling: OpenCodeGoQuotaWindowSnapshot,
    pub weekly: OpenCodeGoQuotaWindowSnapshot,
    pub monthly: OpenCodeGoQuotaWindowSnapshot,
    pub status: String,
    pub queried_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoQuotaError {
    pub kind: String,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoAccountIndex {
    pub version: String,
    #[serde(default)]
    pub accounts: Vec<OpenCodeGoAccount>,
}

impl OpenCodeGoAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for OpenCodeGoAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoQuotaQueryResult {
    pub connection: OpenCodeGoAccount,
    pub quota: OpenCodeGoQuotaSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeGoQuotaBatchResult {
    pub connections: Vec<OpenCodeGoAccount>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> OpenCodeGoAccount {
        OpenCodeGoAccount {
            id: "ocg_account_1".to_string(),
            name: "Primary".to_string(),
            key_hint: "ocg_****demo".to_string(),
            created_at: 10,
            updated_at: 20,
            enabled: true,
            provider: "go".to_string(),
            quota: None,
            quota_error: None,
        }
    }

    #[test]
    fn public_account_serialization_never_has_key_material_field() {
        let value = serde_json::to_value(account()).expect("serialize account");
        let object = value.as_object().expect("account object");

        assert_eq!(
            object.get("keyHint").and_then(|value| value.as_str()),
            Some("ocg_****demo")
        );
        assert!(!object.contains_key("apiKey"));
        assert!(!object.contains_key("api_key"));
        assert!(!object.contains_key("accessToken"));
        assert!(!object.contains_key("access_token"));
    }

    #[test]
    fn account_index_defaults_legacy_missing_accounts_to_empty() {
        let index: OpenCodeGoAccountIndex =
            serde_json::from_str(r#"{"version":"1.0"}"#).expect("deserialize legacy index");

        assert!(index.accounts.is_empty());
        assert_eq!(OpenCodeGoAccountIndex::default().version, "1.0");
    }

    #[test]
    fn account_round_trip_preserves_sanitized_quota_and_error() {
        let mut expected = account();
        expected.quota = Some(OpenCodeGoQuotaSnapshot {
            rolling: OpenCodeGoQuotaWindowSnapshot {
                status: "available".to_string(),
                used_percent: Some(25.0),
                remaining_percent: Some(75.0),
                resets_at: Some(100),
                error: None,
            },
            weekly: OpenCodeGoQuotaWindowSnapshot {
                status: "available".to_string(),
                used_percent: Some(50.0),
                remaining_percent: Some(50.0),
                resets_at: Some(200),
                error: None,
            },
            monthly: OpenCodeGoQuotaWindowSnapshot {
                status: "available".to_string(),
                used_percent: Some(75.0),
                remaining_percent: Some(25.0),
                resets_at: Some(300),
                error: None,
            },
            status: "available".to_string(),
            queried_at: 50,
        });
        expected.quota_error = Some(OpenCodeGoQuotaError {
            kind: "network".to_string(),
            occurred_at: 40,
        });

        let encoded = serde_json::to_string(&expected).expect("serialize account");
        let restored: OpenCodeGoAccount =
            serde_json::from_str(&encoded).expect("deserialize account");

        assert_eq!(restored, expected);
    }
}
