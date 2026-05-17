use crate::utils::protobuf;
use base64::{engine::general_purpose, Engine as _};
use rusqlite::{Connection, Error as SqliteError, ErrorCode};
use std::path::{Path, PathBuf};

/// 获取 Antigravity 数据库路径
pub fn get_db_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path =
            home.join("Library/Application Support/Antigravity/User/globalStorage/state.vscdb");
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("数据库文件不存在: {:?}", path));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        let path = PathBuf::from(appdata).join("Antigravity\\User\\globalStorage\\state.vscdb");
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("数据库文件不存在: {:?}", path));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path = home.join(".config/Antigravity/User/globalStorage/state.vscdb");
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
    crate::modules::logger::log_info(&format!("注入 Token 到数据库: {:?}", db_path));

    // 新格式：antigravityUnifiedStateSync.oauthToken
    inject_unified_oauth_token_to_path(db_path, access_token, refresh_token, expiry)?;

    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

    // 读取旧格式数据（可能不存在）
    let current_data: Option<String> = match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?",
        ["jetskiStateSync.agentManagerInitState"],
        |row| row.get(0),
    ) {
        Ok(value) => Some(value),
        Err(SqliteError::QueryReturnedNoRows) => None,
        Err(e) => return Err(format!("读取数据失败: {}", e)),
    };

    if let Some(current_data) = current_data {
        // Base64 解码
        let blob = general_purpose::STANDARD
            .decode(&current_data)
            .map_err(|e| format!("Base64 解码失败: {}", e))?;

        // 移除旧 Field 6
        let clean_data = protobuf::remove_field(&blob, 6)?;

        // 创建新 Field 6
        let new_field = protobuf::create_oauth_field(access_token, refresh_token, expiry);

        // 合并数据
        let final_data = [clean_data, new_field].concat();
        let final_b64 = general_purpose::STANDARD.encode(&final_data);

        // 写入数据库
        conn.execute(
            "UPDATE ItemTable SET value = ? WHERE key = ?",
            [&final_b64, "jetskiStateSync.agentManagerInitState"],
        )
        .map_err(|e| format!("写入数据失败: {}", e))?;
    } else {
        crate::modules::logger::log_warn(
            "未找到 jetskiStateSync.agentManagerInitState，跳过旧格式注入",
        );
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

/// 注入 Token 到 antigravityUnifiedStateSync.oauthToken（新格式）
pub fn inject_unified_oauth_token_to_path(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

    // 创建 OAuthTokenInfo（二进制）
    let oauth_info = protobuf::create_oauth_info(access_token, refresh_token, expiry);
    let oauth_info_b64 = general_purpose::STANDARD.encode(&oauth_info);

    // InnerMessage2: field 1 = base64(oauth_info)
    let inner2 = protobuf::encode_string_field(1, &oauth_info_b64);

    // InnerMessage: field 1 = sentinel key, field 2 = inner2
    let inner1 = protobuf::encode_string_field(1, "oauthTokenInfoSentinelKey");
    let inner = [inner1, protobuf::encode_len_delim_field(2, &inner2)].concat();

    // OuterMessage: field 1 = inner
    let outer = protobuf::encode_len_delim_field(1, &inner);
    let outer_b64 = general_purpose::STANDARD.encode(&outer);

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.oauthToken", &outer_b64],
    )
    .map_err(|e| format!("写入新格式失败: {}", e))?;

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
