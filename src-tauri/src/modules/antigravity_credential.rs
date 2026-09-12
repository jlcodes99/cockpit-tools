use crate::models::Account;

#[derive(serde::Serialize)]
struct AntigravityCredentialToken {
    access_token: String,
    token_type: String,
    refresh_token: String,
    expiry: String,
}

#[derive(serde::Serialize)]
struct AntigravityCredentialPayload {
    token: AntigravityCredentialToken,
    auth_method: String,
}

#[derive(Debug, serde::Deserialize)]
struct StoredAntigravityCredentialToken {
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expiry: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct StoredAntigravityCredentialPayload {
    token: StoredAntigravityCredentialToken,
    auth_method: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AntigravitySystemCredential {
    pub access_token: Option<String>,
    pub refresh_token: String,
    pub token_type: Option<String>,
    pub expiry: Option<String>,
    pub auth_method: Option<String>,
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_antigravity_credential_secret(secret: &str) -> Result<String, String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err("Antigravity 系统凭据为空".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(b64) = trimmed.strip_prefix("go-keyring-base64:") {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            let decoded = STANDARD
                .decode(b64)
                .map_err(|e| format!("Base64 解码 Keychain 凭据失败: {}", e))?;
            return String::from_utf8(decoded)
                .map_err(|e| format!("解析 Keychain 凭据 UTF-8 失败: {}", e));
        }
    }

    Ok(trimmed.to_string())
}

pub fn parse_antigravity_system_credential(
    secret: &str,
) -> Result<AntigravitySystemCredential, String> {
    let payload_json = normalize_antigravity_credential_secret(secret)?;
    let payload: StoredAntigravityCredentialPayload = serde_json::from_str(&payload_json)
        .map_err(|e| format!("解析 Antigravity 系统凭据失败: {}", e))?;
    let refresh_token = normalize_non_empty(payload.token.refresh_token.as_deref())
        .ok_or_else(|| "Antigravity 系统凭据缺少 refresh_token".to_string())?;

    Ok(AntigravitySystemCredential {
        access_token: normalize_non_empty(payload.token.access_token.as_deref()),
        refresh_token,
        token_type: normalize_non_empty(payload.token.token_type.as_deref()),
        expiry: normalize_non_empty(payload.token.expiry.as_deref()),
        auth_method: normalize_non_empty(payload.auth_method.as_deref()),
    })
}

fn build_antigravity_credential_payload(account: &Account) -> Result<String, String> {
    let expiry = chrono::DateTime::from_timestamp(account.token.expiry_timestamp, 0)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);

    serde_json::to_string(&AntigravityCredentialPayload {
        token: AntigravityCredentialToken {
            access_token: account.token.access_token.clone(),
            token_type: "Bearer".to_string(),
            refresh_token: account.token.refresh_token.clone(),
            expiry,
        },
        auth_method: "consumer".to_string(),
    })
    .map_err(|e| format!("序列化 Antigravity 系统凭据失败: {}", e))
}

pub fn write_antigravity_system_credential(account: &Account) -> Result<(), String> {
    let payload_json = build_antigravity_credential_payload(account)?;

    crate::modules::logger::log_info(&format!(
        "[Antigravity 2.0] 写入系统凭据: {}",
        account.email
    ));

    #[cfg(target_os = "macos")]
    {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let encoded_payload = STANDARD.encode(&payload_json);
        let keychain_value = format!("go-keyring-base64:{}", encoded_payload);
        write_raw_antigravity_system_credential(&keychain_value)?;
    }

    #[cfg(target_os = "windows")]
    {
        write_raw_antigravity_system_credential(&payload_json)?;
    }

    #[cfg(target_os = "linux")]
    {
        write_raw_antigravity_system_credential(&payload_json)?;
    }

    crate::modules::logger::log_info("[Antigravity 2.0] 系统凭据写入完成");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn read_antigravity_system_credential_secret() -> Result<Option<String>, String> {
    use std::process::Command;
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
        ])
        .output()
        .map_err(|e| format!("执行 macOS Keychain 查找凭据失败: {}", e))?;

    if !output.status.success() {
        return Ok(None);
    }

    let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if secret.is_empty() {
        Ok(None)
    } else {
        Ok(Some(secret))
    }
}

#[cfg(target_os = "macos")]
pub fn write_raw_antigravity_system_credential(raw_secret: &str) -> Result<(), String> {
    use std::process::Command;

    let _ = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
        ])
        .output();

    let output = Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
            raw_secret,
            "-A",
        ])
        .output()
        .map_err(|e| format!("执行 macOS Keychain 写入命令失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("写入 macOS Keychain 失败: {}", stderr.trim()));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn delete_antigravity_system_credential() -> Result<(), String> {
    use std::process::Command;
    let _ = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
        ])
        .output();
    Ok(())
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct FileTime {
    dw_low_date_time: u32,
    dw_high_date_time: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct CredentialW {
    flags: u32,
    cred_type: u32,
    target_name: *const u16,
    comment: *const u16,
    last_written: FileTime,
    credential_blob_size: u32,
    credential_blob: *const u8,
    persist: u32,
    attribute_count: u32,
    attributes: *const std::ffi::c_void,
    target_alias: *const u16,
    user_name: *const u16,
}

#[cfg(target_os = "windows")]
#[link(name = "advapi32")]
extern "system" {
    fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
    fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(
        target_name: *const u16,
        type_: u32,
        flags: u32,
        credential: *mut *mut CredentialW,
    ) -> i32;
    fn CredFree(buffer: *mut std::ffi::c_void);
}

#[cfg(target_os = "windows")]
const CRED_TYPE_GENERIC: u32 = 1;
#[cfg(target_os = "windows")]
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
#[cfg(target_os = "windows")]
const ERROR_NOT_FOUND: i32 = 1168;

#[cfg(target_os = "windows")]
pub fn read_antigravity_system_credential_secret() -> Result<Option<String>, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    let target_wide: Vec<u16> = OsStr::new("gemini:antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut credential_ptr: *mut CredentialW = ptr::null_mut();

    unsafe {
        if CredReadW(
            target_wide.as_ptr(),
            CRED_TYPE_GENERIC,
            0,
            &mut credential_ptr,
        ) == 0
        {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND) {
                return Ok(None);
            }
            return Err(format!(
                "读取 Windows Credential Manager Antigravity 凭据失败: {}",
                error
            ));
        }

        if credential_ptr.is_null() {
            return Ok(None);
        }

        let credential = &*credential_ptr;
        let secret = if credential.credential_blob.is_null() || credential.credential_blob_size == 0
        {
            String::new()
        } else {
            let bytes = std::slice::from_raw_parts(
                credential.credential_blob,
                credential.credential_blob_size as usize,
            );
            String::from_utf8_lossy(bytes).trim().to_string()
        };
        CredFree(credential_ptr.cast());

        if secret.is_empty() {
            Ok(None)
        } else {
            Ok(Some(secret))
        }
    }
}

#[cfg(target_os = "windows")]
pub fn write_raw_antigravity_system_credential(raw_secret: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    let target_wide: Vec<u16> = OsStr::new("gemini:antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let user_wide: Vec<u16> = OsStr::new("antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let secret = raw_secret.as_bytes();

    let credential = CredentialW {
        flags: 0,
        cred_type: CRED_TYPE_GENERIC,
        target_name: target_wide.as_ptr(),
        comment: ptr::null(),
        last_written: FileTime {
            dw_low_date_time: 0,
            dw_high_date_time: 0,
        },
        credential_blob_size: secret.len() as u32,
        credential_blob: secret.as_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attribute_count: 0,
        attributes: ptr::null(),
        target_alias: ptr::null(),
        user_name: user_wide.as_ptr(),
    };

    unsafe {
        let _ = CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0);
        if CredWriteW(&credential, 0) == 0 {
            return Err(format!(
                "写入 Windows Credential Manager 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn delete_antigravity_system_credential() -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let target_wide: Vec<u16> = OsStr::new("gemini:antigravity")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn read_antigravity_system_credential_secret() -> Result<Option<String>, String> {
    use std::process::Command;
    let output = Command::new("secret-tool")
        .args([
            "lookup",
            "service",
            "gemini",
            "username",
            "antigravity",
        ])
        .output()
        .map_err(|e| format!("执行 Linux secret-tool 查找凭据失败: {}", e))?;

    if !output.status.success() {
        return Ok(None);
    }

    let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if secret.is_empty() {
        Ok(None)
    } else {
        Ok(Some(secret))
    }
}

#[cfg(target_os = "linux")]
pub fn write_raw_antigravity_system_credential(raw_secret: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label=gemini",
            "service",
            "gemini",
            "username",
            "antigravity",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 Linux secret-tool 失败: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(raw_secret.as_bytes())
            .map_err(|e| format!("写入 Linux secret-tool 输入失败: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 Linux secret-tool 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Linux secret-tool 写入失败: {}", stderr.trim()));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn delete_antigravity_system_credential() -> Result<(), String> {
    use std::process::Command;
    let _ = Command::new("secret-tool")
        .args([
            "clear",
            "service",
            "gemini",
            "username",
            "antigravity",
        ])
        .output();
    Ok(())
}

pub fn read_antigravity_system_credential() -> Result<Option<AntigravitySystemCredential>, String> {
    let Some(secret) = read_antigravity_system_credential_secret()? else {
        return Ok(None);
    };
    parse_antigravity_system_credential(&secret).map(Some)
}

pub fn snapshot_antigravity_system_credential() -> Result<Option<String>, String> {
    read_antigravity_system_credential_secret()
}

pub fn restore_antigravity_system_credential(snapshot: Option<&str>) -> Result<(), String> {
    match snapshot {
        Some(raw) => write_raw_antigravity_system_credential(raw),
        None => delete_antigravity_system_credential(),
    }
}

pub fn verify_antigravity_system_credential(expected: &Account) -> Result<(), String> {
    let secret = read_antigravity_system_credential_secret()?
        .ok_or_else(|| "CREDENTIAL_VERIFY_FAILED: 写入凭据后未能从系统凭据库读取".to_string())?;

    let parsed = parse_antigravity_system_credential(&secret)?;
    if parsed.refresh_token != expected.token.refresh_token {
        return Err(format!(
            "CREDENTIAL_VERIFY_FAILED: 凭据 refresh_token 不匹配 (预期账号: {})",
            expected.email
        ));
    }
    Ok(())
}

pub fn detect_auth_backend() -> String {
    #[cfg(target_os = "macos")]
    {
        "keyring".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "keyring".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if Command::new("secret-tool").arg("--version").output().is_ok() {
            "keyring".to_string()
        } else {
            "unknown".to_string()
        }
    }
}
