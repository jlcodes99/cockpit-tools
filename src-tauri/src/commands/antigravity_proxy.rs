use crate::modules::antigravity_proxy::{self, View};
use std::collections::BTreeMap;

#[tauri::command]
pub async fn antigravity_proxy_list() -> Result<Vec<View>, String> {
    antigravity_proxy::list().await
}
#[tauri::command]
pub async fn antigravity_proxy_get(account_id: String) -> Result<View, String> {
    antigravity_proxy::get(account_id).await
}
#[tauri::command]
pub async fn antigravity_proxy_bind(
    account_id: String,
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<View, String> {
    antigravity_proxy::bind(account_id, source_id, item_id, selections, group_id).await
}
#[tauri::command]
pub async fn antigravity_proxy_clear(account_id: String) -> Result<View, String> {
    antigravity_proxy::clear(account_id).await
}
