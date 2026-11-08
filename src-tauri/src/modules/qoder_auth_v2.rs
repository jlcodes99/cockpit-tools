//! 新版 Qoder 客户端（数据目录 `com.qoder.app.stable`）auth 存储读写。
//!
//! 旧版 Qoder 是 VS Code fork，账号凭据在 `User/globalStorage/state.vscdb` 的
//! secret storage 里，`Local State` 与其同目录；新版客户端改为自有布局，
//! 凭据保存在数据目录下的 `auth.v1.dat`。该文件是 Chromium safeStorage v10
//! 密文，明文 JSON 结构：
//!
//! ```json
//! {
//!   "schemaVersion": 1,
//!   "authenticationContext": "[\"https://qoder.com\",\"https://openapi.qoder.sh\",\"<会话id>\",\"<短码>\"]",
//!   "token": "...",
//!   "refreshToken": "...",
//!   "expiresAt": "2026-11-27T14:39:57Z",
//!   "refreshTokenExpiresAt": "2027-10-23T14:39:57Z",
//!   "user": { "id": "...", "name": "...", "email": "...", "phone": null, "avatarUrl": "..." }
//! }
//! ```
//!
//! `authenticationContext` 是客户端环境元数据（登录域、OpenAPI 域、会话标识），
//! 与账号无关，注入时原样保留。密钥来自同目录 `Local State` 的
//! `os_crypt.encrypted_key`（DPAPI 包装），与 [`crate::modules::vscode_inject`]
//! 使用的 safeStorage 机制同源。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::modules::logger;

const AUTH_FILE_NAME: &str = "auth.v1.dat";
const LOCAL_STATE_FILE_NAME: &str = "Local State";
const AUTH_SCHEMA_VERSION: i64 = 1;

/// 新版 Qoder 数据目录（Windows: `%APPDATA%/com.qoder.app.stable`）。
fn new_layout_data_root() -> Result<PathBuf, String> {
    let appdata = std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
    Ok(PathBuf::from(appdata).join("com.qoder.app.stable"))
}

/// 新版布局可用 = 数据目录下同时存在 `Local State`（解密密钥）与
/// `auth.v1.dat`（客户端至少成功登录过一次，`authenticationContext` 才完整）。
pub fn is_new_layout_available() -> bool {
    match new_layout_data_root() {
        Ok(root) => is_new_layout_available_at(&root),
        Err(_) => false,
    }
}

fn is_new_layout_available_at(data_root: &Path) -> bool {
    data_root.join(LOCAL_STATE_FILE_NAME).exists() && data_root.join(AUTH_FILE_NAME).exists()
}

fn read_auth_plain(data_root: &Path) -> Result<Value, String> {
    let path = data_root.join(AUTH_FILE_NAME);
    let encrypted =
        fs::read(&path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let plaintext =
        crate::modules::vscode_inject::decrypt_safe_storage_bytes(&encrypted, data_root)?;
    serde_json::from_slice(&plaintext)
        .map_err(|e| format!("解析 {} 明文失败: {}", path.display(), e))
}

fn write_auth_plain(data_root: &Path, value: &Value) -> Result<(), String> {
    let path = data_root.join(AUTH_FILE_NAME);
    let plaintext =
        serde_json::to_vec(value).map_err(|e| format!("序列化 {} 失败: {}", path.display(), e))?;
    let encrypted =
        crate::modules::vscode_inject::encrypt_safe_storage_bytes(&plaintext, data_root)?;
    crate::modules::atomic_write::write_bytes_atomic(&path, &encrypted)
}

fn get_string_by_keys(root: &Value, keys: &[&str]) -> Option<String> {
    let object = root.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// 毫秒时间戳 → 新版客户端使用的 ISO8601 UTC（`2026-11-27T14:39:57Z`）。
fn ms_to_iso8601(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// ISO8601 / 毫秒字符串 → 毫秒时间戳（导入 auth.v1.dat 时换算 expireTime）。
fn iso8601_to_ms(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(ms) = trimmed.parse::<i64>() {
        return Some(ms);
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn copy_string_field(source: &Value, target: &mut serde_json::Map<String, Value>, keys: &[&str], target_key: &str) {
    if let Some(value) = get_string_by_keys(source, keys) {
        target.insert(target_key.to_string(), Value::String(value));
    }
}

/// 新版布局可用时把账号注入 `auth.v1.dat`。
/// 返回 `Ok(false)` 表示新版布局不可用（调用方应回落旧 vscdb 注入）。
pub fn inject_account_if_new_layout(account_id: &str) -> Result<bool, String> {
    let data_root = match new_layout_data_root() {
        Ok(root) => root,
        Err(err) => {
            logger::log_warn(&format!("[Qoder Auth v2] 无法定位新版数据目录，回落旧注入链路: {}", err));
            return Ok(false);
        }
    };
    if !is_new_layout_available_at(&data_root) {
        return Ok(false);
    }

    inject_account_at(&data_root, account_id)?;
    Ok(true)
}

fn inject_account_at(data_root: &Path, account_id: &str) -> Result<(), String> {
    let account = crate::modules::qoder_account::load_account(account_id)
        .ok_or_else(|| format!("Qoder 账号不存在: {}", account_id))?;
    let user_info = account
        .auth_user_info_raw
        .as_ref()
        .ok_or_else(|| "Qoder 账号缺少登录信息，请先在工具内重新登录或从本地导入".to_string())?;

    let token = get_string_by_keys(user_info, &["token", "securityOauthToken", "accessToken"])
        .ok_or_else(|| "Qoder 账号缺少 access token，请先在工具内重新登录或从本地导入".to_string())?;
    let refresh_token = get_string_by_keys(user_info, &["refreshToken", "refresh_token"]);
    let expire_ms = get_string_by_keys(user_info, &["expireTime", "expireTimeMs", "expiresAt"])
        .and_then(|value| iso8601_to_ms(&value));
    let refresh_expire_ms = get_string_by_keys(user_info, &["refreshTokenExpireTime", "refreshTokenExpiresAt"])
        .and_then(|value| iso8601_to_ms(&value));

    let mut auth = read_auth_plain(data_root)?;

    let existing_user = auth
        .get("user")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(object) = auth.as_object_mut() {
        object.insert("schemaVersion".to_string(), Value::from(AUTH_SCHEMA_VERSION));
        object.insert("token".to_string(), Value::String(token));
        if let Some(value) = refresh_token {
            object.insert("refreshToken".to_string(), Value::String(value));
        }
        if let Some(ms) = expire_ms {
            if let Some(iso) = ms_to_iso8601(ms) {
                object.insert("expiresAt".to_string(), Value::String(iso));
            }
        }
        if let Some(ms) = refresh_expire_ms {
            if let Some(iso) = ms_to_iso8601(ms) {
                object.insert("refreshTokenExpiresAt".to_string(), Value::String(iso));
            }
        }

        let mut user = existing_user;
        copy_string_field(user_info, &mut user, &["id", "uid"], "id");
        copy_string_field(user_info, &mut user, &["name", "displayName"], "name");
        copy_string_field(user_info, &mut user, &["email", "mail"], "email");
        copy_string_field(user_info, &mut user, &["avatarUrl", "avatar_url"], "avatarUrl");
        object.insert("user".to_string(), Value::Object(user));
    }

    write_auth_plain(data_root, &auth)?;

    // 回读校验，确保写出的密文能被本进程解密且账号一致。
    let verify = read_auth_plain(data_root)?;
    let verify_email = verify
        .get("user")
        .and_then(|user| user.get("email"))
        .and_then(Value::as_str);
    let email_matches = verify_email
        .map(|value| value.eq_ignore_ascii_case(&account.email))
        .unwrap_or(false);
    if !email_matches {
        return Err(format!(
            "Qoder Auth v2 写入校验失败: 期望 email={:?}, 实际={:?}",
            account.email,
            verify_email.unwrap_or("<none>")
        ));
    }

    logger::log_info(&format!(
        "[Qoder Auth v2] 注入成功: account_id={}, email={}, auth_file={}",
        account.id,
        account.email,
        data_root.join(AUTH_FILE_NAME).display()
    ));
    Ok(())
}

/// 读取新版布局 `auth.v1.dat` 的当前登录快照，转换为工具标准的
/// `user_info_raw`（与 [`crate::modules::qoder_oauth::build_initial_user_info_raw`]
/// 同一形状），供 `import_from_local` 走统一的账号入库链路。
/// 新版布局不可用或未登录时返回 `Ok(None)`。
pub fn read_local_user_info_snapshot() -> Result<Option<Value>, String> {
    let data_root = match new_layout_data_root() {
        Ok(root) => root,
        Err(_) => return Ok(None),
    };
    if !is_new_layout_available_at(&data_root) {
        return Ok(None);
    }

    let auth = match read_auth_plain(&data_root) {
        Ok(value) => value,
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder Auth v2] 读取新版 auth 失败，将尝试旧版 vscdb 链路: {}",
                err
            ));
            return Ok(None);
        }
    };

    let token = get_string_by_keys(&auth, &["token"]);
    let user_email = auth
        .get("user")
        .and_then(|user| get_string_by_keys(user, &["email", "mail"]));
    if token.is_none() || user_email.is_none() {
        logger::log_warn("[Qoder Auth v2] 新版 auth 缺少 token 或 user.email，视为未登录");
        return Ok(None);
    }

    let mut user_info = serde_json::Map::new();
    if let Some(token) = token {
        user_info.insert("token".to_string(), Value::String(token));
    }
    if let Some(value) = get_string_by_keys(&auth, &["refreshToken"]) {
        user_info.insert("refreshToken".to_string(), Value::String(value));
    }
    if let Some(value) = auth.get("expiresAt").and_then(Value::as_str) {
        if let Some(ms) = iso8601_to_ms(value) {
            user_info.insert("expireTime".to_string(), Value::String(ms.to_string()));
        }
    }
    if let Some(value) = auth.get("refreshTokenExpiresAt").and_then(Value::as_str) {
        if let Some(ms) = iso8601_to_ms(value) {
            user_info.insert(
                "refreshTokenExpireTime".to_string(),
                Value::String(ms.to_string()),
            );
        }
    }
    if let Some(user) = auth.get("user").and_then(Value::as_object) {
        copy_string_field(&auth, &mut user_info, &["authenticationContext"], "authenticationContext");
        for key in ["id", "name", "email", "avatarUrl"] {
            if let Some(value) = user.get(key) {
                if value.is_string() || value.is_null() {
                    user_info.insert(key.to_string(), value.clone());
                }
            }
        }
    }

    logger::log_info(&format!(
        "[Qoder Auth v2] 已读取本地登录快照: email={:?}, auth_file={}",
        user_email.unwrap_or_default(),
        data_root.join(AUTH_FILE_NAME).display()
    ));
    Ok(Some(Value::Object(user_info)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ms_to_iso8601_utc() {
        // 2026-11-27T14:39:57Z
        let ms: i64 = 1_794_176_397_000;
        assert_eq!(ms_to_iso8601(ms).as_deref(), Some("2026-11-27T14:39:57Z"));
        assert_eq!(ms_to_iso8601(0).as_deref(), Some("1970-01-01T00:00:00Z"));
    }

    #[test]
    fn parses_iso_and_ms_back_to_ms() {
        assert_eq!(
            iso8601_to_ms("2026-11-27T14:39:57Z"),
            Some(1_794_176_397_000)
        );
        assert_eq!(iso8601_to_ms("1794176397000"), Some(1_794_176_397_000));
        assert_eq!(iso8601_to_ms(""), None);
        assert_eq!(iso8601_to_ms("not-a-time"), None);
    }
}
