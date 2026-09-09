use std::fs;
use std::path::Path;
use chrono::{TimeZone, Utc};
use serde_json::Value;

use crate::models::qoder::QoderAccount;
#[cfg(target_os = "windows")]
use crate::modules::logger;

const AUTH_V1_FILENAME: &str = "auth.v1.dat";
const MACHINE_ID_FILENAME: &str = "auth.machine-id";

/// 毫秒时间戳转 ISO 8601 字符串（如 "2026-09-27T06:10:46Z"）
fn ms_epoch_to_iso8601(val: Option<&Value>) -> Option<String> {
    let Some(val) = val else { return None };
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains('T') || trimmed.contains('-') {
            return Some(trimmed.to_string());
        }
        if let Ok(millis) = trimmed.parse::<i64>() {
            return Utc.timestamp_millis_opt(millis).single().map(|dt| dt.to_rfc3339());
        }
        return Some(trimmed.to_string());
    }
    if let Some(millis) = val.as_i64() {
        return Utc.timestamp_millis_opt(millis).single().map(|dt| dt.to_rfc3339());
    }
    None
}

/// 从 Qoder App 数据目录（如 %APPDATA%\com.qoder.app.stable）读取并解密 auth.v1.dat
pub fn read_qoder_app_auth(data_dir: &Path) -> Result<Option<Value>, String> {
    let auth_path = data_dir.join(AUTH_V1_FILENAME);
    if !auth_path.exists() {
        return Ok(None);
    }

    #[cfg(target_os = "windows")]
    {
        let master_key = crate::modules::vscode_inject::get_windows_encryption_key(Some(data_dir))
            .map_err(|e| format!("读取 Local State 密钥失败({}): {}", data_dir.display(), e))?;

        let encrypted = fs::read(&auth_path)
            .map_err(|e| format!("读取 {} 失败: {}", auth_path.display(), e))?;

        let decrypted = crate::modules::vscode_inject::decrypt_windows_gcm_v10(&master_key, &encrypted)
            .map_err(|e| format!("解密 {} 失败: {}", auth_path.display(), e))?;

        let json_text = String::from_utf8(decrypted)
            .map_err(|e| format!("解析 {} 明文为 UTF-8 失败: {}", auth_path.display(), e))?;

        let value: Value = serde_json::from_str(&json_text)
            .map_err(|e| format!("解析 {} 为 JSON 失败: {}", auth_path.display(), e))?;

        Ok(Some(value))
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err("Qoder App (Launcher 架构) 当前仅支持 Windows 系统".to_string())
    }
}

/// 将 QoderAccount 转化为 Qoder App (auth.v1.dat) 标准 schemaVersion 1 结构
pub fn build_app_auth_value(account: &QoderAccount) -> Value {
    let raw_info = account.auth_user_info_raw.as_ref();

    let token = raw_info
        .and_then(|v| v.get("token").or_else(|| v.get("accessToken")))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let refresh_token = raw_info
        .and_then(|v| v.get("refreshToken"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    // Qoder App asar 要求 expiresAt 必须为 ISO 8601 字符串
    let expires_at = ms_epoch_to_iso8601(
        raw_info.and_then(|v| v.get("expiresAt").or_else(|| v.get("expireTime"))),
    )
    .unwrap_or_else(|| Utc::now().to_rfc3339());

    let refresh_token_expires_at = ms_epoch_to_iso8601(
        raw_info.and_then(|v| {
            v.get("refreshTokenExpiresAt")
                .or_else(|| v.get("refreshTokenExpireTime"))
        }),
    )
    .unwrap_or_else(|| {
        let future = Utc::now() + chrono::Duration::days(365);
        future.to_rfc3339()
    });

    let user_id = account.user_id.clone().unwrap_or_default();
    let user_name = account.display_name.clone().unwrap_or_default();
    let email = account.email.clone();
    let avatar_url = raw_info
        .and_then(|v| v.get("avatarUrl").or_else(|| v.get("imageUrl")))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    serde_json::json!({
        "schemaVersion": 1,
        "token": token,
        "refreshToken": refresh_token,
        "expiresAt": expires_at,
        "refreshTokenExpiresAt": refresh_token_expires_at,
        "user": {
            "id": user_id,
            "name": user_name,
            "email": email,
            "avatarUrl": avatar_url,
            "tier": account.plan_type.as_deref().unwrap_or("Personal")
        }
    })
}

/// 原子加密写入 auth.v1.dat 到指定 App 数据目录
pub fn write_qoder_app_auth(data_dir: &Path, auth_json: &Value) -> Result<(), String> {
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)
            .map_err(|e| format!("创建数据目录失败 {}: {}", data_dir.display(), e))?;
    }

    #[cfg(target_os = "windows")]
    {
        let local_state = data_dir.join("Local State");
        if !local_state.exists() {
            return Err(format!(
                "数据目录缺失 Local State: {}。请先运行一次该程序使其生成加密密钥后重试。",
                data_dir.display()
            ));
        }

        let master_key = crate::modules::vscode_inject::get_windows_encryption_key(Some(data_dir))
            .map_err(|e| format!("解密 Local State 失败: {}", e))?;

        let plain_text = serde_json::to_string(auth_json)
            .map_err(|e| format!("序列化 auth JSON 失败: {}", e))?;

        let encrypted = crate::modules::vscode_inject::encrypt_windows_gcm_v10(
            &master_key,
            plain_text.as_bytes(),
        )?;

        let auth_path = data_dir.join(AUTH_V1_FILENAME);
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();

        // 备份原凭据
        if auth_path.exists() {
            let bak_path = data_dir.join(format!("{}.bak_{}", AUTH_V1_FILENAME, ts));
            if let Err(e) = fs::copy(&auth_path, &bak_path) {
                logger::log_warn(&format!("[Qoder App] 备份原凭据失败: {}", e));
            }
        }

        // tmp + atomic replace 写入
        let tmp_path = data_dir.join(format!("{}.tmp_{}", AUTH_V1_FILENAME, ts));
        fs::write(&tmp_path, &encrypted)
            .map_err(|e| format!("写入临时凭据文件失败 {}: {}", tmp_path.display(), e))?;

        fs::rename(&tmp_path, &auth_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            format!("原子替换凭据文件失败: {}", e)
        })?;

        logger::log_info(&format!(
            "[Qoder App Auth] 成功加密写入凭据: path={}, len={}",
            auth_path.display(),
            encrypted.len()
        ));

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (data_dir, auth_json);
        Err("Qoder App 凭据写入仅支持 Windows 系统".to_string())
    }
}
