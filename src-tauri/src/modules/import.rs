use crate::models;
use crate::modules;
use crate::utils;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::Emitter;

// ==================== 辅助结构体和函数 ====================

#[derive(Debug, Deserialize)]
pub struct OldToolAccount {
    pub email: String,
    pub name: Option<String>,
    pub token: models::TokenData,
}

#[derive(Debug, Deserialize)]
struct ExtensionCredentialsFile {
    accounts: HashMap<String, ExtensionCredential>,
}

const EXTENSION_SECRET_STORAGE_KEYS: [&str; 2] = [
    "antigravity.autoTrigger.credentials",
    "antigravity.autoTrigger.credential",
];
const EXTENSION_SECRET_STORAGE_EXTENSION_IDS: [&str; 2] = [
    "jlcodes.antigravity-cockpit",
    "jlcodes99.antigravity-cockpit",
];

#[derive(Debug, Deserialize)]
struct ExtensionCredential {
    pub email: Option<String>,
    #[serde(rename = "refreshToken", alias = "refresh_token")]
    pub refresh_token: Option<String>,
    #[serde(rename = "projectId", alias = "project_id")]
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ExtensionImportProgressPayload {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub email: Option<String>,
}

fn emit_extension_import_progress(
    app: Option<&tauri::AppHandle>,
    phase: &str,
    current: usize,
    total: usize,
    email: Option<&str>,
) {
    let Some(app_handle) = app else {
        return;
    };
    let payload = ExtensionImportProgressPayload {
        phase: phase.to_string(),
        current,
        total,
        email: email.map(|value| value.to_string()),
    };
    let _ = app_handle.emit("accounts:extension-import-progress", payload);
}

fn is_probably_email(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return false;
    }
    !local.is_empty() && domain.contains('.')
}

fn normalize_extension_credentials(
    accounts: HashMap<String, ExtensionCredential>,
) -> HashMap<String, ExtensionCredential> {
    let mut filtered = HashMap::new();
    for (key, mut item) in accounts {
        let email = item.email.clone().unwrap_or(key).trim().to_lowercase();
        let refresh_token = item
            .refresh_token
            .clone()
            .unwrap_or_default()
            .trim()
            .to_string();

        if !is_probably_email(&email) || refresh_token.is_empty() {
            continue;
        }

        item.email = Some(email.clone());
        item.refresh_token = Some(refresh_token);
        filtered.insert(email, item);
    }
    filtered
}

fn resolve_antigravity_user_data_dir() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        return crate::modules::antigravity_paths::default_user_data_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("Antigravity IDE")
                .to_string_lossy()
                .to_string(),
        );
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
            let trimmed = xdg_config_home.trim();
            if !trimmed.is_empty() {
                return Some(format!("{}/Antigravity IDE", trimmed));
            }
        }
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("Antigravity IDE")
                .to_string_lossy()
                .to_string(),
        );
    }
    #[allow(unreachable_code)]
    None
}

fn extension_secret_storage_sources() -> Vec<(Option<String>, &'static str)> {
    resolve_antigravity_user_data_dir()
        .map(|path| vec![(Some(path), "antigravity")])
        .unwrap_or_default()
}

fn parse_extension_credentials_payload(
    payload: &str,
) -> Result<HashMap<String, ExtensionCredential>, String> {
    if let Ok(parsed) = serde_json::from_str::<ExtensionCredentialsFile>(payload) {
        return Ok(parsed.accounts);
    }

    let single: ExtensionCredential = serde_json::from_str(payload)
        .map_err(|e| format!("解析插件 SecretStorage 凭据失败: {}", e))?;
    let mut accounts = HashMap::new();
    let key = single
        .email
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "__legacy__".to_string());
    accounts.insert(key, single);
    Ok(accounts)
}

fn load_extension_credentials_from_secret_storage(
) -> Result<HashMap<String, ExtensionCredential>, String> {
    for (user_data_dir, source_label) in extension_secret_storage_sources() {
        for extension_id in EXTENSION_SECRET_STORAGE_EXTENSION_IDS {
            // 优先读取多账号 key；只有该 key 不存在时才回退 legacy key
            let mut try_keys: Vec<&str> = Vec::new();
            let multi_key = EXTENSION_SECRET_STORAGE_KEYS[0];
            match modules::vscode_inject::read_antigravity_secret_storage_value(
                extension_id,
                multi_key,
                user_data_dir.as_deref(),
            ) {
                Ok(Some(content)) => {
                    let parsed = parse_extension_credentials_payload(&content).map_err(|e| {
                        format!(
                            "解析 VS Code SecretStorage 失败 (source={}, extensionId={}, key={}): {}",
                            source_label, extension_id, multi_key, e
                        )
                    })?;
                    let normalized = normalize_extension_credentials(parsed);
                    modules::logger::log_info(&format!(
                        "从插件 SecretStorage 读取多账号凭据 source={} extensionId={} count={}",
                        source_label,
                        extension_id,
                        normalized.len()
                    ));
                    return Ok(normalized);
                }
                Ok(None) => {
                    try_keys.push(EXTENSION_SECRET_STORAGE_KEYS[1]);
                }
                Err(err) => {
                    modules::logger::log_warn(&format!(
                        "读取插件 SecretStorage 失败，尝试下一个来源 source={} extensionId={} key={} error={}",
                        source_label, extension_id, multi_key, err
                    ));
                    continue;
                }
            }

            for secret_key in try_keys {
                match modules::vscode_inject::read_antigravity_secret_storage_value(
                    extension_id,
                    secret_key,
                    user_data_dir.as_deref(),
                ) {
                    Ok(Some(content)) => {
                        let parsed = parse_extension_credentials_payload(&content).map_err(|e| {
                            format!(
                                "解析 VS Code SecretStorage 失败 (source={}, extensionId={}, key={}): {}",
                                source_label, extension_id, secret_key, e
                            )
                        })?;
                        let normalized = normalize_extension_credentials(parsed);
                        modules::logger::log_info(&format!(
                            "从插件 SecretStorage 读取 legacy 凭据 source={} extensionId={} count={}",
                            source_label,
                            extension_id,
                            normalized.len()
                        ));
                        return Ok(normalized);
                    }
                    Ok(None) => continue,
                    Err(err) => {
                        modules::logger::log_warn(&format!(
                            "读取插件 SecretStorage 失败，尝试下一个来源 source={} extensionId={} key={} error={}",
                            source_label, extension_id, secret_key, err
                        ));
                    }
                }
            }
        }
    }
    Ok(HashMap::new())
}

// ==================== 导入命令逻辑 ====================

/// 从旧版 ~/.antigravity_tools/ 导入账号
pub async fn import_from_old_tools_logic() -> Result<Vec<models::Account>, String> {
    use std::fs;

    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let old_dir = home.join(".antigravity_tools");

    if !old_dir.exists() {
        return Err("未找到旧版数据目录 ~/.antigravity_tools/".to_string());
    }

    let old_accounts_dir = old_dir.join("accounts");
    if !old_accounts_dir.exists() {
        return Err("未找到旧版账号目录 ~/.antigravity_tools/accounts/".to_string());
    }

    modules::logger::log_info("开始从旧版目录导入账号...");

    let mut imported = Vec::new();

    // 读取旧版索引
    let old_index_path = old_dir.join("accounts.json");
    if old_index_path.exists() {
        let content =
            fs::read_to_string(&old_index_path).map_err(|e| format!("读取旧版索引失败: {}", e))?;

        let old_index: models::AccountIndex =
            serde_json::from_str(&content).map_err(|e| format!("解析旧版索引失败: {}", e))?;

        for summary in old_index.accounts {
            let old_account_path = old_accounts_dir.join(format!("{}.json", summary.id));
            if old_account_path.exists() {
                match fs::read_to_string(&old_account_path) {
                    Ok(account_content) => {
                        match serde_json::from_str::<OldToolAccount>(&account_content) {
                            Ok(old_account) => {
                                // 使用 upsert 导入（避免重复）
                                match modules::upsert_account(
                                    old_account.email.clone(),
                                    old_account.name.clone(),
                                    old_account.token.clone(),
                                ) {
                                    Ok(new_account) => {
                                        modules::logger::log_info(&format!(
                                            "导入账号: {}",
                                            new_account.email
                                        ));
                                        imported.push(new_account);
                                    }
                                    Err(e) => {
                                        modules::logger::log_error(&format!(
                                            "导入账号失败 {}: {}",
                                            old_account.email, e
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                modules::logger::log_error(&format!(
                                    "解析账号文件失败 {:?}: {}",
                                    old_account_path, e
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        modules::logger::log_error(&format!(
                            "读取账号文件失败 {:?}: {}",
                            old_account_path, e
                        ));
                    }
                }
            }
        }
    }

    modules::logger::log_info(&format!("导入完成，共导入 {} 个账号", imported.len()));

    // 广播数据变更通知
    if !imported.is_empty() {
        modules::websocket::broadcast_data_changed("import_from_old_tools");
    }

    Ok(imported)
}

/// 从本地 Antigravity IDE 客户端导入当前账号
#[cfg(target_os = "windows")]
pub async fn import_from_local_logic() -> Result<models::Account, String> {
    modules::logger::log_info("开始从 Windows Credential Manager 导入 Antigravity 账号...");
    let system_credential = modules::antigravity_credential::read_antigravity_system_credential()?
        .ok_or_else(|| {
            "未找到 Antigravity 系统凭据，请确保 Antigravity IDE 客户端已登录".to_string()
        })?;
    import_from_refresh_token(system_credential.refresh_token, "Antigravity 系统凭据").await
}

/// 从本地 Antigravity IDE 客户端导入当前账号
#[cfg(not(target_os = "windows"))]
pub async fn import_from_local_logic() -> Result<models::Account, String> {
    import_from_local_state_db_logic().await
}

async fn import_from_refresh_token(
    refresh_token: String,
    source_label: &str,
) -> Result<models::Account, String> {
    if refresh_token.trim().is_empty() {
        return Err(format!("{} refresh_token 为空", source_label));
    }

    modules::logger::log_info(&format!(
        "从{}获取到 refresh_token (len={})",
        source_label,
        refresh_token.len()
    ));

    // 使用 refresh_token 获取新的 access_token
    let token_response = modules::oauth::refresh_access_token(&refresh_token).await?;

    // 获取用户信息
    let user_info = modules::oauth::get_user_info(&token_response.access_token).await?;
    let email = user_info.email.clone();

    // 构建 TokenData
    let token = models::TokenData::new(
        token_response.access_token,
        token_response.refresh_token.unwrap_or(refresh_token),
        token_response.expires_in,
        Some(email.clone()),
        None,
        None,
    )
    .with_oauth_metadata(token_response.oauth_client_key, token_response.id_token);

    // 添加或更新账号
    let account = modules::upsert_account(email.clone(), user_info.get_display_name(), token)?;

    modules::logger::log_info(&format!("本地账号导入成功: {}", email));

    // 广播数据变更通知
    modules::websocket::broadcast_data_changed("import_from_local");

    Ok(account)
}

async fn import_from_local_state_db_logic() -> Result<models::Account, String> {
    use base64::{engine::general_purpose, Engine as _};

    modules::logger::log_info("开始从本地 Antigravity IDE 客户端导入...");

    // 读取 state.vscdb
    let db_path = modules::db::get_db_path()?;
    let conn =
        rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

    // 读取新版 Unified State Sync OAuth 数据
    let state_data: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?",
            ["antigravityUnifiedStateSync.oauthToken"],
            |row| row.get(0),
        )
        .map_err(|_| "未找到登录状态，请确保 Antigravity IDE 客户端已登录")?;

    // Base64 解码
    let blob = general_purpose::STANDARD
        .decode(&state_data)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;

    // 解析 protobuf 获取 refresh_token
    let refresh_token = utils::protobuf::extract_refresh_token_from_unified_oauth_token(&blob)
        .ok_or("无法从本地数据解析 refresh_token")?;

    if refresh_token.is_empty() {
        return Err("本地 refresh_token 为空".to_string());
    }

    modules::logger::log_info(&format!(
        "获取到本地 refresh_token (len={})",
        refresh_token.len()
    ));

    import_from_refresh_token(refresh_token, "Antigravity state.vscdb").await
}

/// 从 JSON 导入账号
pub async fn import_from_json_logic(json_content: String) -> Result<Vec<models::Account>, String> {
    modules::logger::log_info("开始从 JSON 导入账号...");

    let trimmed = json_content.trim();
    if trimmed.contains("----") && !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let mut imported = Vec::new();
        for (index, line) in trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .enumerate()
        {
            let parts: Vec<&str> = line.splitn(4, "----").map(str::trim).collect();
            if parts.len() < 3 || !parts[0].contains('@') {
                return Err(format!("第 {} 行待授权账号格式无效", index + 1));
            }
            imported.push(modules::account::create_pending_oauth_account(
                parts[0].to_string(),
                modules::account::AccountNoteUpdate {
                    note: None,
                    account_password: (!parts[1].is_empty()).then(|| parts[1].to_string()),
                    two_factor_secret: (!parts[2].is_empty()).then(|| parts[2].to_string()),
                    phone_number: None,
                    mail_url: parts
                        .get(3)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.to_string()),
                },
            )?);
        }
        return Ok(imported);
    }

    // 简化格式: [{"email": "xxx", "refresh_token": "..."}]
    #[derive(Debug, serde::Deserialize)]
    struct SimpleAccount {
        #[serde(default)]
        email: Option<String>,
        #[serde(default, alias = "refreshToken")]
        refresh_token: Option<String>,
        #[serde(default, alias = "pendingOAuth")]
        pending_oauth: bool,
        #[serde(default, alias = "authorizationStatus")]
        authorization_status: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(
            default,
            alias = "account_note",
            alias = "accountNote",
            alias = "note",
            alias = "remark"
        )]
        notes: Option<String>,
        #[serde(
            default,
            alias = "twoFactorSecret",
            alias = "account_two_factor_secret",
            alias = "accountTwoFactorSecret"
        )]
        two_factor_secret: Option<String>,
        #[serde(default, alias = "accountPassword", alias = "password")]
        account_password: Option<String>,
        #[serde(
            default,
            alias = "phoneNumber",
            alias = "account_phone_number",
            alias = "accountPhoneNumber"
        )]
        phone_number: Option<String>,
        #[serde(
            default,
            alias = "mailUrl",
            alias = "mail_address",
            alias = "mailAddress",
            alias = "mail_query_url",
            alias = "mailQueryUrl"
        )]
        mail_url: Option<String>,
    }

    // 尝试解析为简化格式数组
    let simple_accounts: Result<Vec<SimpleAccount>, _> = serde_json::from_str(&json_content)
        .or_else(|_| {
            // 单个简化账号
            serde_json::from_str::<SimpleAccount>(&json_content).map(|a| vec![a])
        });

    if let Ok(accounts) = simple_accounts {
        let mut imported = Vec::new();

        for simple in accounts {
            modules::logger::log_info(&format!(
                "正在导入账号: {}",
                simple.email.as_deref().unwrap_or("<auto>")
            ));

            let has_pending_authorization_status = simple
                .authorization_status
                .as_deref()
                .map(str::trim)
                .map(|value| {
                    value.eq_ignore_ascii_case("pending")
                        || value.eq_ignore_ascii_case("pending_oauth")
                })
                .unwrap_or(false);
            if simple.pending_oauth
                || has_pending_authorization_status
                || simple
                    .refresh_token
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                let Some(email) = simple.email.clone().filter(|value| value.contains('@')) else {
                    continue;
                };
                match modules::account::create_pending_oauth_account(
                    email,
                    modules::account::AccountNoteUpdate {
                        note: simple.notes,
                        two_factor_secret: simple.two_factor_secret,
                        account_password: simple.account_password,
                        phone_number: simple.phone_number,
                        mail_url: simple.mail_url,
                    },
                ) {
                    Ok(account) => imported.push(account),
                    Err(error) => {
                        modules::logger::log_error(&format!("保存待授权账号失败: {}", error))
                    }
                }
                continue;
            }
            let refresh_token = simple.refresh_token.clone().unwrap_or_default();
            // 使用 refresh_token 获取 access_token
            match modules::oauth::refresh_access_token(&refresh_token).await {
                Ok(token_response) => {
                    let email = match simple
                        .email
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                    {
                        Some(email) => email,
                        None => match modules::oauth::get_user_info(&token_response.access_token)
                            .await
                        {
                            Ok(info) => info.email,
                            Err(error) => {
                                modules::logger::log_error(&format!(
                                    "自动识别导入账号邮箱失败: {}",
                                    error
                                ));
                                continue;
                            }
                        },
                    };
                    // 构建 TokenData
                    let token = models::TokenData::new(
                        token_response.access_token,
                        token_response.refresh_token.unwrap_or(refresh_token),
                        token_response.expires_in,
                        Some(email.clone()),
                        None,
                        None,
                    )
                    .with_oauth_metadata(token_response.oauth_client_key, token_response.id_token);

                    match modules::upsert_account(email.clone(), None, token) {
                        Ok(mut new_account) => {
                            if !simple.tags.is_empty() {
                                if let Ok(acc) = modules::account::update_account_tags(
                                    &new_account.id,
                                    simple.tags,
                                ) {
                                    new_account = acc;
                                }
                            }
                            let note_update = modules::account::AccountNoteUpdate {
                                note: simple.notes,
                                two_factor_secret: simple.two_factor_secret,
                                account_password: simple.account_password,
                                phone_number: simple.phone_number,
                                mail_url: simple.mail_url,
                            };
                            match modules::account::update_account_note(
                                &new_account.id,
                                note_update,
                            ) {
                                Ok(acc) => new_account = acc,
                                Err(error) => {
                                    modules::logger::log_error(&format!(
                                        "保存账号备注失败 {}: {}",
                                        new_account.email, error
                                    ));
                                    continue;
                                }
                            }
                            modules::logger::log_info(&format!(
                                "导入账号成功: {}",
                                new_account.email
                            ));
                            imported.push(new_account);
                        }
                        Err(e) => {
                            modules::logger::log_error(&format!("保存账号失败 {}: {}", email, e));
                        }
                    }
                }
                Err(e) => {
                    modules::logger::log_error(&format!(
                        "刷新 Token 失败 {}: {}",
                        simple.email.as_deref().unwrap_or("<auto>"),
                        e
                    ));
                }
            }
        }

        modules::logger::log_info(&format!("JSON 导入完成，共导入 {} 个账号", imported.len()));
        return Ok(imported);
    }

    // 尝试解析为完整账号格式（向后兼容）
    let accounts: Vec<models::Account> = serde_json::from_str(&json_content)
        .or_else(|_| serde_json::from_str::<models::Account>(&json_content).map(|a| vec![a]))
        .map_err(|e| format!("JSON 格式错误: {}", e))?;

    let mut imported = Vec::new();

    for old_account in accounts {
        match modules::upsert_account(
            old_account.email.clone(),
            old_account.name.clone(),
            old_account.token.clone(),
        ) {
            Ok(mut new_account) => {
                if !old_account.tags.is_empty() {
                    if let Ok(acc) =
                        modules::account::update_account_tags(&new_account.id, old_account.tags)
                    {
                        new_account = acc;
                    }
                }
                match modules::account::update_account_note(
                    &new_account.id,
                    modules::account::AccountNoteUpdate {
                        note: old_account.notes,
                        two_factor_secret: old_account.two_factor_secret,
                        account_password: old_account.account_password,
                        phone_number: old_account.phone_number,
                        mail_url: old_account.mail_url,
                    },
                ) {
                    Ok(acc) => new_account = acc,
                    Err(error) => {
                        modules::logger::log_error(&format!(
                            "保存账号备注失败 {}: {}",
                            new_account.email, error
                        ));
                        continue;
                    }
                }
                modules::logger::log_info(&format!("导入账号: {}", new_account.email));
                imported.push(new_account);
            }
            Err(e) => {
                modules::logger::log_error(&format!("导入账号失败 {}: {}", old_account.email, e));
            }
        }
    }

    modules::logger::log_info(&format!("JSON 导入完成，共导入 {} 个账号", imported.len()));

    // 广播数据变更通知
    if !imported.is_empty() {
        modules::websocket::broadcast_data_changed("import_from_json");
    }

    Ok(imported)
}

#[derive(serde::Serialize, Clone)]
pub struct FileImportResult {
    pub imported: Vec<models::Account>,
    pub failed: Vec<FileImportFailure>,
}

#[derive(serde::Serialize, Clone)]
pub struct FileImportFailure {
    pub email: String,
    pub error: String,
}

/// 从本地文件导入账号（支持多种 JSON 格式）
pub async fn import_from_files_logic(file_paths: Vec<String>) -> Result<FileImportResult, String> {
    use std::fs;
    use std::path::Path;

    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }

    modules::logger::log_info(&format!("开始从 {} 个文件导入账号...", file_paths.len()));

    // 收集所有候选条目
    let mut candidates: Vec<ImportEntry> = Vec::new();

    for file_path in &file_paths {
        let path = Path::new(file_path);
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                modules::logger::log_error(&format!("读取文件失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 从文件名推断 email
        let filename_email = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.replace("_at_", "@").replace("_AT_", "@"))
            .filter(|s| s.contains('@'));

        // 尝试解析为 JSON
        let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                modules::logger::log_error(&format!("JSON 解析失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        match parsed {
            serde_json::Value::Array(arr) => {
                // Format B: JSON 数组
                for item in arr {
                    if let Some(entry) = extract_import_entry(&item, &filename_email) {
                        candidates.push(entry);
                    }
                }
            }
            serde_json::Value::Object(_) => {
                // Format A / D: 单个对象
                if let Some(entry) = extract_import_entry(&parsed, &filename_email) {
                    candidates.push(entry);
                }
            }
            _ => {
                modules::logger::log_error(&format!("不支持的 JSON 格式 {:?}", file_path));
            }
        }
    }

    if candidates.is_empty() {
        return Err("未找到有效的 refresh_token".to_string());
    }

    modules::logger::log_info(&format!(
        "发现 {} 个候选账号，开始导入...",
        candidates.len()
    ));

    let mut imported = Vec::new();
    let mut failed: Vec<FileImportFailure> = Vec::new();
    let total = candidates.len();

    for (index, entry) in candidates.into_iter().enumerate() {
        // 发送进度事件
        if let Some(app_handle) = crate::get_app_handle() {
            let _ = app_handle.emit(
                "accounts:file-import-progress",
                serde_json::json!({
                    "current": index + 1,
                    "total": total,
                    "email": entry.email.as_deref().unwrap_or(""),
                }),
            );
        }

        // 使用 refresh_token 获取 access_token
        match modules::oauth::refresh_access_token(&entry.refresh_token).await {
            Ok(token_response) => {
                // 尝试获取用户信息以确定 email
                let email = if let Some(ref e) = entry.email {
                    e.clone()
                } else {
                    match modules::oauth::get_user_info(&token_response.access_token).await {
                        Ok(info) => info.email,
                        Err(e) => {
                            modules::logger::log_error(&format!(
                                "获取用户信息失败，跳过此条目: {}",
                                e
                            ));
                            continue;
                        }
                    }
                };

                let token = models::TokenData::new(
                    token_response.access_token,
                    token_response.refresh_token.unwrap_or(entry.refresh_token),
                    token_response.expires_in,
                    Some(email.clone()),
                    None,
                    None,
                )
                .with_oauth_metadata(token_response.oauth_client_key, token_response.id_token);

                match modules::upsert_account(email.clone(), None, token) {
                    Ok(mut new_account) => {
                        if !entry.tags.is_empty() {
                            if let Ok(acc) =
                                modules::account::update_account_tags(&new_account.id, entry.tags)
                            {
                                new_account = acc;
                            }
                        }
                        match modules::account::update_account_note(
                            &new_account.id,
                            modules::account::AccountNoteUpdate {
                                note: entry.notes,
                                two_factor_secret: entry.two_factor_secret,
                                account_password: entry.account_password,
                                phone_number: entry.phone_number,
                                mail_url: entry.mail_url,
                            },
                        ) {
                            Ok(acc) => new_account = acc,
                            Err(error) => {
                                let msg = format!("保存账号备注失败: {}", error);
                                modules::logger::log_error(&format!("{}: {}", email, msg));
                                failed.push(FileImportFailure { email, error: msg });
                                continue;
                            }
                        }
                        modules::logger::log_info(&format!("导入账号成功: {}", new_account.email));
                        imported.push(new_account);
                    }
                    Err(e) => {
                        let msg = format!("保存失败: {}", e);
                        modules::logger::log_error(&format!("保存账号失败 {}: {}", email, msg));
                        failed.push(FileImportFailure { email, error: msg });
                    }
                }
            }
            Err(e) => {
                let label = entry.email.as_deref().unwrap_or("unknown").to_string();
                let msg = format!("Token 刷新失败: {}", e);
                modules::logger::log_error(&format!("{}: {}", label, msg));
                failed.push(FileImportFailure {
                    email: label,
                    error: msg,
                });
            }
        }
    }

    modules::logger::log_info(&format!(
        "文件导入完成，成功 {} 个，失败 {} 个",
        imported.len(),
        failed.len()
    ));

    if !imported.is_empty() {
        modules::websocket::broadcast_data_changed("import_from_files");
    }

    Ok(FileImportResult { imported, failed })
}

struct ImportEntry {
    email: Option<String>,
    refresh_token: String,
    tags: Vec<String>,
    notes: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
}

/// 从 JSON 值中提取 ImportEntry
fn extract_import_entry(
    value: &serde_json::Value,
    fallback_email: &Option<String>,
) -> Option<ImportEntry> {
    let obj = value.as_object()?;

    // 提取 refresh_token：顶层 或 token.refresh_token
    let refresh_token = obj
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .or_else(|| {
            obj.get("token")
                .and_then(|t| t.get("refresh_token"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;

    // 提取 email：顶层
    let email = obj
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| fallback_email.clone());

    let tags = obj
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|val| val.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let read_string = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| obj.get(*key).and_then(|value| value.as_str()))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let notes = read_string(&["notes", "note", "account_note", "accountNote", "remark"]);
    let two_factor_secret = read_string(&[
        "two_factor_secret",
        "twoFactorSecret",
        "account_two_factor_secret",
        "accountTwoFactorSecret",
    ]);
    let account_password = read_string(&["account_password", "accountPassword", "password"]);
    let phone_number = read_string(&[
        "phone_number",
        "phoneNumber",
        "account_phone_number",
        "accountPhoneNumber",
    ]);
    let mail_url = read_string(&[
        "mail_url",
        "mailUrl",
        "mail_address",
        "mailAddress",
        "mail_query_url",
        "mailQueryUrl",
    ]);

    Some(ImportEntry {
        email,
        refresh_token,
        tags,
        notes,
        two_factor_secret,
        account_password,
        phone_number,
        mail_url,
    })
}

/// 从 VS Code SecretStorage 导入插件账号
pub async fn import_from_extension_credentials(
    app: Option<&tauri::AppHandle>,
) -> Result<usize, String> {
    let parsed_accounts = load_extension_credentials_from_secret_storage()?;

    if parsed_accounts.is_empty() {
        return Ok(0);
    }

    // 现有账号邮箱，用于“仅新增”导入（已存在账号一律跳过，不覆盖）
    let existing_accounts = modules::list_accounts()?;
    let mut existing_emails = HashSet::new();
    for acc in existing_accounts {
        existing_emails.insert(acc.email.trim().to_lowercase());
    }

    let mut imported_count = 0;
    let mut imported_account_ids: Vec<String> = Vec::new();
    let candidates: Vec<(String, ExtensionCredential)> = parsed_accounts.into_iter().collect();
    let total_candidates = candidates.len();

    for (index, (key, item)) in candidates.into_iter().enumerate() {
        let email = item.email.unwrap_or_else(|| key.clone());
        emit_extension_import_progress(app, "import", index + 1, total_candidates, Some(&email));
        let refresh_token = match item.refresh_token {
            Some(token) if !token.trim().is_empty() => token,
            _ => continue,
        };

        if existing_emails.contains(&email.trim().to_lowercase()) {
            modules::logger::log_info(&format!("插件导入跳过已存在账号: {}", email));
            continue;
        }

        match modules::oauth::refresh_access_token(&refresh_token).await {
            Ok(token_response) => {
                let user_info = modules::oauth::get_user_info(&token_response.access_token).await?;
                let token = models::TokenData::new(
                    token_response.access_token,
                    token_response.refresh_token.unwrap_or(refresh_token),
                    token_response.expires_in,
                    Some(user_info.email.clone()),
                    item.project_id.clone(),
                    None,
                )
                .with_oauth_metadata(token_response.oauth_client_key, token_response.id_token);

                match modules::add_account(
                    user_info.email.clone(),
                    user_info.get_display_name(),
                    token,
                ) {
                    Ok(account) => {
                        imported_count += 1;
                        imported_account_ids.push(account.id.clone());
                        existing_emails.insert(account.email.trim().to_lowercase());
                    }
                    Err(e) => {
                        modules::logger::log_error(&format!("导入账号失败 {}: {}", email, e));
                    }
                }
            }
            Err(e) => {
                modules::logger::log_error(&format!("刷新 Token 失败 {}: {}", email, e));
            }
        }
    }

    let total_refresh = imported_account_ids.len();
    for (index, account_id) in imported_account_ids.into_iter().enumerate() {
        let mut account = match modules::load_account(&account_id) {
            Ok(value) => value,
            Err(e) => {
                modules::logger::log_warn(&format!("导入后加载账号失败 {}: {}", account_id, e));
                continue;
            }
        };
        emit_extension_import_progress(
            app,
            "quota",
            index + 1,
            total_refresh,
            Some(&account.email),
        );
        match modules::fetch_quota_with_fresh_token(&mut account, true).await {
            Ok(quota) => {
                if let Err(e) = modules::update_account_quota(&account_id, quota) {
                    modules::logger::log_warn(&format!(
                        "导入后刷新订阅失败 {}: {}",
                        account.email, e
                    ));
                }
            }
            Err(e) => {
                modules::logger::log_warn(&format!("导入后刷新配额失败 {}: {}", account.email, e));
            }
        }
    }

    if imported_count > 0 {
        modules::websocket::broadcast_data_changed("extension_sync");
    }

    Ok(imported_count)
}

#[cfg(test)]
mod account_note_import_tests {
    use super::*;
    use std::fs;

    fn with_test_data_dir<T>(name: &str, run: impl FnOnce() -> T) -> T {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let data_dir = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&data_dir);
        fs::create_dir_all(&data_dir).expect("create test data dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);
        let result = run();
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(&data_dir);
        result
    }

    #[test]
    fn import_pending_oauth_json_preserves_sensitive_notes() {
        with_test_data_dir("antigravity-pending-json-import-test", || {
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");
            let accounts = runtime
                .block_on(import_from_json_logic(
                    r#"{
                      "email": "pending-json@example.com",
                      "pending_oauth": true,
                      "notes": "delivery note",
                      "account_password": "password-1",
                      "two_factor_secret": "JBSWY3DPEHPK3PXP",
                      "phone_number": "13800000000",
                      "mail_url": "https://mail.example.test/inbox"
                    }"#
                    .to_string(),
                ))
                .expect("import pending JSON");
            assert_eq!(accounts.len(), 1);
            let account = &accounts[0];
            assert!(account.pending_oauth);
            assert_eq!(account.account_password.as_deref(), Some("password-1"));
            assert_eq!(
                account.two_factor_secret.as_deref(),
                Some("JBSWY3DPEHPK3PXP")
            );
            assert_eq!(account.phone_number.as_deref(), Some("13800000000"));
            assert_eq!(
                account.mail_url.as_deref(),
                Some("https://mail.example.test/inbox")
            );
        });
    }

    #[test]
    fn import_pending_oauth_delimited_line_preserves_fields() {
        with_test_data_dir("antigravity-pending-line-import-test", || {
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");
            let accounts = runtime
                .block_on(import_from_json_logic(
                    "line@example.com----password-2----JBSWY3DPEHPK3PXP----https://mail.example.test/open"
                        .to_string(),
                ))
                .expect("import pending line");
            assert_eq!(accounts.len(), 1);
            let account = &accounts[0];
            assert!(account.pending_oauth);
            assert_eq!(account.account_password.as_deref(), Some("password-2"));
            assert_eq!(
                account.two_factor_secret.as_deref(),
                Some("JBSWY3DPEHPK3PXP")
            );
            assert_eq!(
                account.mail_url.as_deref(),
                Some("https://mail.example.test/open")
            );
        });
    }
}
