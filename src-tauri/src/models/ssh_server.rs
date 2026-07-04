use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshServerStore {
    pub version: String,
    pub selected_server_id: Option<String>,
    pub servers: Vec<SshServer>,
}

impl Default for SshServerStore {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            selected_server_id: None,
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub codex_home: String,
    pub auth: SshAuthConfig,
    pub sync_on_codex_switch: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_sync: Option<SshCodexSyncStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SshAuthConfig {
    Agent,
    PrivateKeyFile { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshCodexSyncStatus {
    pub account_id: String,
    pub account_email: String,
    pub token_generation: u64,
    pub bundle_hash: String,
    pub synced_at: i64,
    pub verified: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshCodexSyncResult {
    pub server_id: String,
    pub server_name: String,
    pub account_id: String,
    pub account_email: String,
    pub token_generation: u64,
    pub bundle_hash: String,
    pub verified: bool,
    pub error: Option<String>,
    pub synced_at: i64,
}
