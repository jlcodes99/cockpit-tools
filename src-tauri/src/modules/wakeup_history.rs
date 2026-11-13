use super::wakeup_history_store::{self, HistoryRow, HistoryStore};
use crate::modules;
use serde::{Deserialize, Serialize};

pub(crate) const HISTORY_FILE: &str = "wakeup_history.json";
const MAX_HISTORY_ITEMS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupHistoryItem {
    pub id: String,
    pub timestamp: i64,
    pub trigger_type: String,
    pub trigger_source: String,
    pub task_name: Option<String>,
    pub account_email: String,
    pub model_id: String,
    pub prompt: Option<String>,
    pub success: bool,
    pub status: Option<String>,
    pub message: Option<String>,
    pub duration: Option<u64>,
}

impl HistoryRow for WakeupHistoryItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

fn store() -> Result<HistoryStore, String> {
    Ok(history_store_at(
        modules::account::get_data_dir()?.join(HISTORY_FILE),
    ))
}

pub(crate) fn history_store_at(path: std::path::PathBuf) -> HistoryStore {
    wakeup_history_store::wakeup(path, "antigravity", MAX_HISTORY_ITEMS)
}

pub fn schedule_migration() -> Result<(), String> {
    store()?.schedule::<WakeupHistoryItem>();
    Ok(())
}

pub fn load_history(
    retry: bool,
) -> Result<wakeup_history_store::HistoryRead<WakeupHistoryItem>, String> {
    wakeup_history_store::read_available(store()?, retry)
}

pub fn add_history_items(new_items: Vec<WakeupHistoryItem>) -> Result<(), String> {
    store()?.add(&new_items)
}

pub fn clear_history() -> Result<(), String> {
    store()?.clear()
}

/// 记录单条状态历史（用于确认/跳过等场景）
pub fn record_status(
    task: &crate::modules::wakeup_scheduler::WakeupTask,
    trigger_source: &str,
    status: &str,
) -> Result<(), String> {
    let item = WakeupHistoryItem {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        trigger_type: "scheduled".to_string(),
        trigger_source: trigger_source.to_string(),
        task_name: Some(task.name.clone()),
        account_email: task
            .schedule
            .selected_accounts
            .first()
            .cloned()
            .unwrap_or_default(),
        model_id: task
            .schedule
            .selected_models
            .first()
            .cloned()
            .unwrap_or_default(),
        prompt: task.schedule.custom_prompt.clone(),
        success: status == "success",
        status: Some(status.to_string()),
        message: Some(format!("Status: {}", status)),
        duration: Some(0),
    };

    add_history_items(vec![item])
}
