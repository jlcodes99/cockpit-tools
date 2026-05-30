use crate::utils::protobuf;
use base64::{engine::general_purpose, Engine as _};
use rusqlite::{Connection, OptionalExtension};
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
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        let path = PathBuf::from(appdata).join("Antigravity IDE\\User\\globalStorage\\state.vscdb");
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

/// 注入 Token 到指定数据库路径
pub fn inject_token_to_path(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<String, String> {
    crate::modules::logger::log_info(&format!("注入 Token 到数据库: {:?}", db_path));

    inject_unified_oauth_token_to_path(db_path, access_token, refresh_token, expiry)?;
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

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

/// 注入 Token 到 antigravityUnifiedStateSync.oauthToken
pub fn inject_unified_oauth_token_to_path(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;
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

    // 创建 OAuthTokenInfo（二进制）
    let oauth_info = protobuf::create_oauth_info(access_token, refresh_token, expiry);
    let oauth_info_b64 = general_purpose::STANDARD.encode(&oauth_info);

    // Topic.data[oauthTokenInfoSentinelKey].Row.value = base64(OAuthTokenInfo)
    let row = protobuf::encode_string_field(1, &oauth_info_b64);
    let inner1 = protobuf::encode_string_field(1, "oauthTokenInfoSentinelKey");
    let inner = [inner1, protobuf::encode_len_delim_field(2, &row)].concat();

    // Topic.data: repeated map entry, field 1 = entry
    topic.extend(protobuf::encode_len_delim_field(1, &inner));
    let topic_b64 = general_purpose::STANDARD.encode(&topic);

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.oauthToken", &topic_b64],
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
