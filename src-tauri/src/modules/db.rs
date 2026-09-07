use crate::{models::Account, utils::protobuf};
use base64::{engine::general_purpose, Engine as _};
use rusqlite::{Connection, Error as SqliteError, ErrorCode, OptionalExtension};
use std::path::{Path, PathBuf};

/// 获取 Antigravity IDE 数据库路径
pub fn get_db_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path =
            home.join("Library/Application Support/Antigravity IDE/User/globalStorage/state.vscdb");
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("数据库文件不存在: {:?}", path));
    }

    #[cfg(target_os = "windows")]
    {
        let path = crate::modules::antigravity_paths::state_db_path()?;
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("数据库文件不存在: {:?}", path));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path = home.join(".config/Antigravity IDE/User/globalStorage/state.vscdb");
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("数据库文件不存在: {:?}", path));
    }
}

pub fn is_unusable_sqlite_database_error(error: &SqliteError) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    ) || is_unusable_sqlite_database_message(&error.to_string())
}

pub fn is_unusable_sqlite_database_message(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("file is not a database")
        || lowered.contains("not a database")
        || lowered.contains("database disk image is malformed")
}

/// 注入 Token 到指定数据库路径
pub fn inject_token_to_path(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<String, String> {
    inject_token_to_path_with_metadata(
        db_path,
        access_token,
        refresh_token,
        expiry,
        None,
        None,
        None,
        None,
    )
}

/// 按账号数据注入 Token 到指定数据库路径。
pub fn inject_account_token_to_path(db_path: &Path, account: &Account) -> Result<String, String> {
    inject_token_to_path_with_metadata(
        db_path,
        &account.token.access_token,
        &account.token.refresh_token,
        account.token.expiry_timestamp,
        account.token.is_gcp_tos,
        account.token.id_token.as_deref(),
        Some(account.email.as_str()),
        account.token.project_id.as_deref(),
    )
}

/// 注入 Token 到 antigravityUnifiedStateSync.oauthToken
pub fn inject_unified_oauth_token_to_path(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;
    inject_unified_oauth_token(&conn, access_token, refresh_token, expiry, None, None, None)
}

fn inject_token_to_path_with_metadata(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
    is_gcp_tos: Option<bool>,
    id_token: Option<&str>,
    email: Option<&str>,
    project_id: Option<&str>,
) -> Result<String, String> {
    crate::modules::logger::log_info(&format!("注入 Token 到数据库: {:?}", db_path));

    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;
    inject_unified_oauth_token(
        &conn,
        access_token,
        refresh_token,
        expiry,
        is_gcp_tos,
        id_token,
        email,
    )?;

    if let Some(email) = email.map(str::trim).filter(|value| !value.is_empty()) {
        inject_user_status(&conn, email)?;
    }

    if let Some(project_id) = project_id.map(str::trim).filter(|value| !value.is_empty()) {
        inject_enterprise_project_preference(&conn, project_id)?;
    } else {
        clear_enterprise_project_preference(&conn)?;
    }

    // 注入 Onboarding 标记
    let onboarding_key = "antigravityOnboarding";
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        [onboarding_key, "true"],
    )
    .map_err(|e| format!("写入 Onboarding 标记失败: {}", e))?;

    crate::modules::logger::log_info("Token 注入成功");
    Ok(format!("Token 注入成功！\n数据库: {:?}", db_path))
}

fn inject_unified_oauth_token(
    conn: &Connection,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
    is_gcp_tos: Option<bool>,
    id_token: Option<&str>,
    email: Option<&str>,
) -> Result<(), String> {
    let current_topic = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?",
            ["antigravityUnifiedStateSync.oauthToken"],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("读取 Unified OAuth 数据失败: {}", e))?
        .map(|value| {
            general_purpose::STANDARD
                .decode(value)
                .map_err(|e| format!("Unified OAuth Base64 解码失败: {}", e))
        })
        .transpose()?
        .unwrap_or_default();
    let mut topic =
        protobuf::remove_unified_topic_entry(&current_topic, "oauthTokenInfoSentinelKey")?;
    topic = protobuf::remove_unified_topic_entry(&topic, "authStateWithContextSentinelKey")?;

    // 创建 OAuthTokenInfo（二进制）
    let oauth_info = protobuf::create_oauth_info_with_metadata(
        access_token,
        refresh_token,
        expiry,
        is_gcp_tos,
        id_token,
        email,
    );

    // Topic.data: repeated map entry, field 1 = entry
    topic.extend(protobuf::create_unified_topic_entry(
        "oauthTokenInfoSentinelKey",
        &oauth_info,
    ));
    let topic_b64 = general_purpose::STANDARD.encode(&topic);

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.oauthToken", &topic_b64],
    )
    .map_err(|e| format!("写入新格式失败: {}", e))?;

    Ok(())
}

fn inject_user_status(conn: &Connection, email: &str) -> Result<(), String> {
    let payload = protobuf::create_minimal_user_status_payload(email);
    let topic = protobuf::create_unified_topic_entry("userStatusSentinelKey", &payload);
    let topic_b64 = general_purpose::STANDARD.encode(topic);
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.userStatus", &topic_b64],
    )
    .map_err(|e| format!("写入 UserStatus 失败: {}", e))?;
    Ok(())
}

fn inject_enterprise_project_preference(conn: &Connection, project_id: &str) -> Result<(), String> {
    let payload = protobuf::create_string_value_payload(project_id);
    let topic = protobuf::create_unified_topic_entry("enterpriseGcpProjectId", &payload);
    let topic_b64 = general_purpose::STANDARD.encode(topic);
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        [
            "antigravityUnifiedStateSync.enterprisePreferences",
            &topic_b64,
        ],
    )
    .map_err(|e| format!("写入 Enterprise Preference 失败: {}", e))?;
    Ok(())
}

fn clear_enterprise_project_preference(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM ItemTable WHERE key = ?",
        ["antigravityUnifiedStateSync.enterprisePreferences"],
    )
    .map_err(|e| format!("清理 Enterprise Preference 失败: {}", e))?;
    Ok(())
}

/// 写入 serviceMachineId 到数据库
pub fn write_service_machine_id(service_machine_id: &str) -> Result<(), String> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["storage.serviceMachineId", service_machine_id],
    )
    .map_err(|e| format!("写入 serviceMachineId 失败: {}", e))?;

    crate::modules::logger::log_info(&format!("serviceMachineId 已写入: {}", service_machine_id));
    Ok(())
}
