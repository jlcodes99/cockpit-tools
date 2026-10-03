use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single provider credential as stored in pi's `auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiProviderCredential {
    /// Provider id used as the `auth.json` key (e.g. `anthropic`, `openai-codex`).
    pub provider: String,
    /// Raw `auth.json` entry (`{"type":"api_key",...}` or `{"type":"oauth",...}`).
    pub entry: Value,
}

impl PiProviderCredential {
    pub fn kind(&self) -> String {
        self.entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    }

    pub fn expires_at(&self) -> Option<i64> {
        self.entry.get("expires").and_then(Value::as_i64)
    }
}

/// A custom (third-party gateway) provider merged into pi's `models.json`.
/// `config` is the raw `providers.<id>` object without the API key, which lives
/// in the matching `auth.json` credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiCustomProvider {
    pub provider: String,
    pub config: Value,
}

/// A pi "account" is a profile: a set of provider credentials plus default model settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAccount {
    pub id: String,
    /// Display name shown as the account label.
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub credentials: Vec<PiProviderCredential>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_providers: Vec<PiCustomProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiProviderSummary {
    pub provider: String,
    /// `api_key` | `oauth` | other raw type.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Gateway base URL when the provider is a custom `models.json` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auth_header: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Masked key tail (e.g. `****9liY`) for api_key credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hint: Option<String>,
}

/// Sanitized account DTO for the UI. Credentials never cross IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAccountView {
    pub id: String,
    pub email: String,
    // Kept for the shared frontend account shape; real credentials never cross IPC.
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub providers: Vec<PiProviderSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl From<&PiAccount> for PiAccountView {
    fn from(account: &PiAccount) -> Self {
        Self {
            id: account.id.clone(),
            email: account.email.clone(),
            access_token: String::new(),
            tags: account.tags.clone(),
            providers: account
                .credentials
                .iter()
                .map(|cred| {
                    let custom = account
                        .custom_providers
                        .iter()
                        .find(|item| item.provider == cred.provider);
                    let field = |key: &str| {
                        custom
                            .and_then(|item| item.config.get(key))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    };
                    PiProviderSummary {
                        provider: cred.provider.clone(),
                        kind: cred.kind(),
                        expires_at: cred.expires_at(),
                        base_url: field("baseUrl"),
                        api: field("api"),
                        name: field("name"),
                        auth_header: custom
                            .and_then(|item| item.config.get("authHeader"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        models: custom
                            .and_then(|item| item.config.get("models"))
                            .and_then(Value::as_array)
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|m| m.get("id").and_then(Value::as_str))
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        key_hint: cred
                            .entry
                            .get("key")
                            .and_then(Value::as_str)
                            .map(mask_key),
                    }
                })
                .collect(),
            default_provider: account.default_provider.clone(),
            default_model: account.default_model.clone(),
            default_thinking_level: account.default_thinking_level.clone(),
            working_dir: account.working_dir.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        }
    }
}

fn mask_key(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("****{}", tail)
}
