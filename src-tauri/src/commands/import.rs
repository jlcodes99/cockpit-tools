use crate::models;
use crate::modules;
use tauri::AppHandle;

#[tauri::command]
pub async fn import_from_old_tools() -> Result<Vec<models::Account>, String> {
    modules::import::import_from_old_tools_logic().await
}

#[tauri::command]
pub async fn import_from_local(app: AppHandle) -> Result<models::Account, String> {
    let account = modules::import::import_from_local_logic().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn import_from_json(json_content: String) -> Result<Vec<models::Account>, String> {
    modules::import::import_from_json_logic(json_content).await
}

#[tauri::command]
pub async fn import_from_files(
    file_paths: Vec<String>,
) -> Result<modules::import::FileImportResult, String> {
    modules::import::import_from_files_logic(file_paths).await
}

#[tauri::command]
pub async fn export_accounts(account_ids: Vec<String>) -> Result<String, String> {
    let mut accounts_to_export = Vec::new();

    if account_ids.is_empty() {
        // 导出全部
        accounts_to_export = modules::list_accounts()?;
    } else {
        for id in &account_ids {
            if let Ok(account) = modules::load_account(id) {
                accounts_to_export.push(account);
            }
        }
    }

    #[derive(serde::Serialize)]
    struct SimpleAccount {
        email: String,
        refresh_token: String,
    }

    let simplified: Vec<SimpleAccount> = accounts_to_export
        .into_iter()
        .map(|account| SimpleAccount {
            email: account.email,
            refresh_token: account.token.refresh_token,
        })
        .collect();

    let json =
        serde_json::to_string_pretty(&simplified).map_err(|e| format!("序列化失败: {}", e))?;

    Ok(json)
}
