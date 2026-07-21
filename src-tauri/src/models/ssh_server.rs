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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshCodexStateRepairStatus {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub error_stage: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub database_found: bool,
    pub backup_path: Option<String>,
    pub provider_schema_supported: bool,
    pub visibility_schema_supported: bool,
    #[serde(default)]
    pub rollout_schema_supported: bool,
    pub provider_rows_to_repair: u64,
    pub visibility_rows_to_repair: u64,
    #[serde(default)]
    pub rollout_files_to_repair: u64,
    pub rows_repaired: u64,
    #[serde(default)]
    pub rollout_files_repaired: u64,
    pub provider_rows_remaining: u64,
    pub visibility_rows_remaining: u64,
    #[serde(default)]
    pub rollout_files_remaining: u64,
    pub quick_check: Option<String>,
    #[serde(default)]
    pub rollback_performed: bool,
    #[serde(default)]
    pub rollback_verified: bool,
    #[serde(default)]
    pub orphan_rollouts_found: u64,
    #[serde(default)]
    pub orphan_threads_recovered: u64,
    #[serde(default)]
    pub rollout_paths_repaired: u64,
    #[serde(default)]
    pub user_events_recovered: u64,
    #[serde(default)]
    pub cwd_rows_repaired: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshCodexSyncStatus {
    pub account_id: String,
    pub account_email: String,
    pub token_generation: u64,
    pub bundle_hash: String,
    #[serde(default)]
    pub bundle_verified: bool,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model_provider_verified: bool,
    #[serde(default)]
    pub state_repair: Option<SshCodexStateRepairStatus>,
    #[serde(default)]
    pub app_server_reload_status: Option<String>,
    #[serde(default)]
    pub app_server_quiesce_status: Option<String>,
    #[serde(default)]
    pub app_server_restore_status: Option<String>,
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
    pub bundle_verified: bool,
    pub model_provider: Option<String>,
    pub model_provider_verified: bool,
    pub state_repair: Option<SshCodexStateRepairStatus>,
    pub app_server_reload_status: Option<String>,
    pub app_server_quiesce_status: Option<String>,
    pub app_server_restore_status: Option<String>,
    pub verified: bool,
    pub error: Option<String>,
    pub synced_at: i64,
}
