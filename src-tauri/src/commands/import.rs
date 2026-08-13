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
        #[serde(skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pending_oauth: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        two_factor_secret: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phone_number: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mail_url: Option<String>,
    }

    let simplified: Vec<SimpleAccount> = accounts_to_export
        .into_iter()
        .map(|account| SimpleAccount {
            email: account.email,
            refresh_token: (!account.pending_oauth
                && !account.token.refresh_token.trim().is_empty())
            .then_some(account.token.refresh_token),
            pending_oauth: account.pending_oauth,
            tags: account.tags,
            notes: account.notes,
            two_factor_secret: account.two_factor_secret,
            account_password: account.account_password,
            phone_number: account.phone_number,
            mail_url: account.mail_url,
        })
        .collect();

    let json =
        serde_json::to_string_pretty(&simplified).map_err(|e| format!("序列化失败: {}", e))?;

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn pending_oauth_export_omits_refresh_token_and_preserves_notes() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let data_dir = std::env::temp_dir().join(format!(
            "antigravity-pending-export-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&data_dir);
        fs::create_dir_all(&data_dir).expect("create test data dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = modules::account::create_pending_oauth_account(
            "pending-export@example.com".to_string(),
            modules::account::AccountNoteUpdate {
                note: Some("delivery note".to_string()),
                two_factor_secret: Some("JBSWY3DPEHPK3PXP".to_string()),
                account_password: Some("password-1".to_string()),
                phone_number: Some("13800000000".to_string()),
                mail_url: Some("https://mail.example.test/inbox".to_string()),
            },
        )
        .expect("create pending account");
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let raw = runtime
            .block_on(export_accounts(vec![account.id]))
            .expect("export pending account");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse export JSON");
        let exported = &value.as_array().expect("export array")[0];

        assert_eq!(exported["pending_oauth"], true);
        assert!(exported.get("refresh_token").is_none());
        assert_eq!(exported["notes"], "delivery note");
        assert_eq!(exported["account_password"], "password-1");
        assert_eq!(exported["two_factor_secret"], "JBSWY3DPEHPK3PXP");
        assert_eq!(exported["phone_number"], "13800000000");
        assert_eq!(exported["mail_url"], "https://mail.example.test/inbox");

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(&data_dir);
    }
}
