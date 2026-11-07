use crate::modules::codex_reset_automation::{self as reset_automation, AccountRule, Rule, View};

#[tauri::command]
pub async fn get_codex_reset_automation() -> Result<View, String> {
    tauri::async_runtime::spawn_blocking(reset_automation::view)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn save_codex_reset_public_rule(
    rule: Rule,
    expected: Rule,
    apply_to_all: bool,
    expected_accounts: Option<std::collections::HashMap<String, AccountRule>>,
) -> Result<View, String> {
    tauri::async_runtime::spawn_blocking(move || {
        reset_automation::save_public(rule, expected, apply_to_all, expected_accounts)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn save_codex_reset_account_rule(
    account_id: String,
    rule: AccountRule,
    expected: AccountRule,
) -> Result<View, String> {
    tauri::async_runtime::spawn_blocking(move || {
        reset_automation::save_account(&account_id, rule, expected)
    })
    .await
    .map_err(|e| e.to_string())?
}
