use super::wakeup_history_store::{self, HistoryRow, HistoryStore};
use crate::modules;
use serde::{Deserialize, Serialize};

pub(crate) const HISTORY_FILE: &str = "antigravity_switch_history.json";
const MAX_HISTORY_ITEMS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityAutoSwitchHitGroup {
    pub group_id: String,
    pub group_name: String,
    pub percentage: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityAutoSwitchReason {
    pub rule: String,
    pub threshold: i32,
    pub scope_mode: String,
    #[serde(default)]
    pub credits_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits_threshold: Option<i32>,
    #[serde(default)]
    pub credits_triggered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_credits_remaining: Option<f64>,
    #[serde(default)]
    pub selected_group_ids: Vec<String>,
    #[serde(default)]
    pub selected_group_names: Vec<String>,
    #[serde(default)]
    pub hit_groups: Vec<AntigravityAutoSwitchHitGroup>,
    pub candidate_count: usize,
    pub selected_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravitySwitchHistoryItem {
    pub id: String,
    pub timestamp: i64,
    pub account_id: String,
    pub target_email: String,
    #[serde(default = "default_trigger_type")]
    pub trigger_type: String,
    #[serde(default = "default_trigger_source")]
    pub trigger_source: String,
    pub local_ok: bool,
    pub seamless_ok: bool,
    pub success: bool,
    pub local_duration_ms: u64,
    pub seamless_duration_ms: Option<u64>,
    pub total_duration_ms: u64,
    pub error_stage: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub seamless_effective_mode: Option<String>,
    pub seamless_from_email: Option<String>,
    pub seamless_to_email: Option<String>,
    pub seamless_execution_id: Option<String>,
    pub seamless_finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_switch_reason: Option<AntigravityAutoSwitchReason>,
}

fn default_trigger_type() -> String {
    "manual".to_string()
}

fn default_trigger_source() -> String {
    "tools.account.switch".to_string()
}

impl HistoryRow for AntigravitySwitchHistoryItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

pub fn schedule_migration() -> Result<(), String> {
    store()?.schedule::<AntigravitySwitchHistoryItem>();
    Ok(())
}

pub fn load_history(
    retry: bool,
) -> Result<wakeup_history_store::HistoryRead<AntigravitySwitchHistoryItem>, String> {
    wakeup_history_store::read_available(store()?, retry)
}

pub fn add_history_item(item: AntigravitySwitchHistoryItem) -> Result<(), String> {
    store()?.replace_item(item)
}

pub fn clear_history() -> Result<(), String> {
    store()?.clear()
}

fn store() -> Result<HistoryStore, String> {
    Ok(history_store_at(modules::account::get_data_dir()?.join(HISTORY_FILE)))
}

pub(crate) fn history_store_at(path: std::path::PathBuf) -> HistoryStore {
    wakeup_history_store::switches(path, "antigravity", MAX_HISTORY_ITEMS)
}
