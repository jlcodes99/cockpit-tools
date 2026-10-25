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
    #[serde(skip_serializing_if = "Option::is_none")]
    id_token: Option<String>,
}

#[cfg(target_os = "windows")]
#[derive(Debug, serde::Deserialize)]
struct StoredAntigravityCredentialToken {
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expiry: Option<String>,
}

#[cfg(target_os = "windows")]
#[derive(Debug, serde::Deserialize)]
struct StoredAntigravityCredentialPayload {
    token: StoredAntigravityCredentialToken,
    auth_method: Option<String>,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
pub struct AntigravitySystemCredential {
    pub access_token: Option<String>,
    pub refresh_token: String,
    pub token_type: Option<String>,
    pub expiry: Option<String>,
    pub auth_method: Option<String>,
}

#[cfg(target_os = "windows")]
fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(target_os = "windows")]
fn normalize_antigravity_credential_secret(secret: &str) -> Result<String, String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return Err("Antigravity 系统凭据为空".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(target_os = "windows")]
fn parse_antigravity_system_credential(
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
        id_token: account.token.id_token.clone(),
    })
    .map_err(|e| format!("序列化 Antigravity 系统凭据失败: {}", e))
}

pub fn write_antigravity_system_credential(account: &Account) -> Result<(), String> {
    let payload_json = build_antigravity_credential_payload(account)?;
    let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
    // Standalone 2.13.0 / 2.25.0's compositeTokenStorage falls back to this file when
    // Keyring is unavailable. This is distinct from Gemini CLI / agy storage.
    let fallback_path = home.join(".gemini").join("jetski-standalone-oauth-token");

    crate::modules::logger::log_info(&format!(
        "[Antigravity 2.0] 写入系统凭据: {}",
        account.email
    ));

    write_antigravity_credential_payload(
        &payload_json,
        &fallback_path,
        write_antigravity_system_payload,
    )?;
    crate::modules::logger::log_info("[Antigravity 2.0] 系统凭据与桌面回退凭据写入完成");
    Ok(())
}

fn write_antigravity_credential_payload(
    payload_json: &str,
    fallback_path: &std::path::Path,
    write_system: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    write_system(payload_json)?;
    // Use the same payload in both stores: an old fallback must not resurrect a
    // previous account after a Keyring timeout. Never report a partial sync as success.
    crate::modules::atomic_write::write_secret_string_atomic(fallback_path, payload_json).map_err(
        |error| format!("系统凭据已写入，但同步 Antigravity 桌面回退凭据失败，请重试切号: {error}"),
    )
}

fn write_antigravity_system_payload(payload_json: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    write_antigravity_keychain_payload(
        payload_json,
        std::ffi::OsStr::new("security"),
        std::time::Duration::from_secs(15),
    )?;

    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;

        #[repr(C)]
        struct FileTime {
            dw_low_date_time: u32,
            dw_high_date_time: u32,
        }

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

        #[link(name = "advapi32")]
        extern "system" {
            fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        }

        const CRED_TYPE_GENERIC: u32 = 1;
        const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

        let target_wide: Vec<u16> = OsStr::new("gemini:antigravity")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let user_wide: Vec<u16> = OsStr::new("antigravity")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let secret = payload_json.as_bytes();

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
            // CredWriteW replaces an existing target without deleting it first.
            if CredWriteW(&credential, 0) == 0 {
                return Err(format!(
                    "写入 Windows Credential Manager 失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mut command = Command::new("secret-tool");
        command.args([
            "store",
            "--label=gemini",
            "service",
            "gemini",
            "username",
            "antigravity",
        ]);
        let output = crate::modules::process_timeout::output_with_input_timeout(
            &mut command,
            payload_json.as_bytes(),
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("执行 Linux secret-tool 写入命令失败: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linux secret-tool 写入失败: {}", stderr.trim()));
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn write_antigravity_keychain_payload(
    payload_json: &str,
    program: &std::ffi::OsStr,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::process::Command;

    let keychain_value = format!("go-keyring-base64:{}", STANDARD.encode(payload_json));
    // Update the credential in place and preserve its existing access policy.
    // Passing -A (or -T) also calls SecKeychainItemSetAccess, which prompts for
    // authorization on every switch even when the password update is allowed.
    let mut command = Command::new(program);
    command.args([
        "add-generic-password",
        "-U",
        "-s",
        "gemini",
        "-a",
        "antigravity",
        "-w",
        &keychain_value,
    ]);
    let output = crate::modules::process_timeout::output_with_timeout(&mut command, timeout)
        .map_err(|e| format!("执行 macOS Keychain 写入命令失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("写入 macOS Keychain 失败: {}", stderr.trim()));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn read_antigravity_system_credential_secret() -> Result<Option<String>, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[repr(C)]
    struct FileTime {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

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

    #[link(name = "advapi32")]
    extern "system" {
        fn CredReadW(
            target_name: *const u16,
            type_: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        fn CredFree(buffer: *mut std::ffi::c_void);
    }

    const CRED_TYPE_GENERIC: u32 = 1;
    const ERROR_NOT_FOUND: i32 = 1168;

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
pub fn read_antigravity_system_credential() -> Result<Option<AntigravitySystemCredential>, String> {
    let Some(secret) = read_antigravity_system_credential_secret()? else {
        return Ok(None);
    };
    parse_antigravity_system_credential(&secret).map(Some)
}

#[cfg(test)]
#[path = "antigravity_credential_tests.rs"]
mod tests;
