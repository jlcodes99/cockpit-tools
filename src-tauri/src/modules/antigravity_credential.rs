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
    })
    .map_err(|e| format!("序列化 Antigravity 系统凭据失败: {}", e))
}


/// 同步 Antigravity CLI 及 Language Server 凭据到本地文件
pub fn write_antigravity_cli_credential(account: &Account) -> Result<(), String> {
    let payload_json = build_antigravity_credential_payload(account)?;
    let gemini_dir = crate::modules::antigravity_paths::gemini_dir()?;
    if !gemini_dir.exists() {
        std::fs::create_dir_all(&gemini_dir)
            .map_err(|e| format!("创建 .gemini 目录失败: {}", e))?;
    }

    let jetski_token_path = crate::modules::antigravity_paths::jetski_oauth_token_path()?;
    write_secure_credential_file(&jetski_token_path, &payload_json)?;

    let cli_dir = crate::modules::antigravity_paths::antigravity_cli_dir()?;
    if !cli_dir.exists() {
        let _ = std::fs::create_dir_all(&cli_dir);
    }
    let cli_token_path = crate::modules::antigravity_paths::antigravity_cli_oauth_token_path()?;

    let is_symlink_to_jetski = match std::fs::read_link(&cli_token_path) {
        Ok(target) => {
            target == jetski_token_path
                || target.file_name() == jetski_token_path.file_name()
                || target.to_string_lossy().contains("jetski-standalone-oauth-token")
        }
        Err(_) => false,
    };
    if !is_symlink_to_jetski {
        write_secure_credential_file(&cli_token_path, &payload_json)?;
    }

    let google_accounts_path = crate::modules::antigravity_paths::google_accounts_path()?;
    update_google_accounts_active(&google_accounts_path, &account.email)?;

    crate::modules::logger::log_info(&format!(
        "[Antigravity CLI] 自动同步 CLI 凭据完成: {}",
        account.email
    ));
    Ok(())
}

fn write_secure_credential_file(path: &std::path::Path, content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)
        .map_err(|e| format!("创建凭据文件失败 {:?}: {}", path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(content.as_bytes())
        .map_err(|e| format!("写入凭据文件失败 {:?}: {}", path, e))?;
    file.flush()
        .map_err(|e| format!("刷新凭据文件失败 {:?}: {}", path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn update_google_accounts_active(path: &std::path::Path, email: &str) -> Result<(), String> {
    let mut root: serde_json::Value = if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        }
    } else {
        serde_json::json!({})
    };

    if !root.is_object() {
        root = serde_json::json!({});
    }

    if let Some(obj) = root.as_object_mut() {
        obj.insert(
            "active".to_string(),
            serde_json::Value::String(email.to_string()),
        );
        if !obj.contains_key("old") {
            obj.insert("old".to_string(), serde_json::json!([]));
        }
    }

    let serialized = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("序列化 google_accounts.json 失败: {}", e))?;
    std::fs::write(path, serialized)
        .map_err(|e| format!("写入 google_accounts.json 失败: {}", e))?;
    Ok(())
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
        use std::process::Command;

        let encoded_payload = STANDARD.encode(&payload_json);
        let keychain_value = format!("go-keyring-base64:{}", encoded_payload);

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
                &keychain_value,
                "-A",
            ])
            .output()
            .map_err(|e| format!("执行 macOS Keychain 写入命令失败: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("写入 macOS Keychain 失败: {}", stderr.trim()));
        }
    }

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
            fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
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
            let _ = CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0);
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
                .write_all(payload_json.as_bytes())
                .map_err(|e| format!("写入 Linux secret-tool 输入失败: {}", e))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("等待 Linux secret-tool 失败: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linux secret-tool 写入失败: {}", stderr.trim()));
        }
    }

    crate::modules::logger::log_info("[Antigravity 2.0] 系统凭据写入完成");
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
mod tests {
    use super::*;
    use crate::models::TokenData;

    #[test]
    fn test_build_antigravity_credential_payload() {
        let token = TokenData::new(
            "test-access-token".to_string(),
            "test-refresh-token".to_string(),
            3600,
            Some("test@example.com".to_string()),
            None,
            None,
        );
        let account = Account::new(
            "account-id".to_string(),
            "test@example.com".to_string(),
            token,
        );
        let payload = build_antigravity_credential_payload(&account).expect("payload");
        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid json");
        assert_eq!(parsed["auth_method"], "consumer");
        assert_eq!(parsed["token"]["access_token"], "test-access-token");
        assert_eq!(parsed["token"]["refresh_token"], "test-refresh-token");
        assert_eq!(parsed["token"]["token_type"], "Bearer");
        assert!(parsed["token"]["expiry"].as_str().unwrap().contains("T"));
    }

    #[test]
    fn test_write_secure_credential_file() {
        let temp_dir = std::env::temp_dir().join(format!(
            "ag-cred-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test-token");
        write_secure_credential_file(&file_path, "{\"test\": true}").expect("write secure");
        let read_back = std::fs::read_to_string(&file_path).expect("read");
        assert_eq!(read_back, "{\"test\": true}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&file_path).expect("meta");
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_update_google_accounts_active() {
        let temp_dir = std::env::temp_dir().join(format!(
            "ag-google-acc-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("google_accounts.json");

        update_google_accounts_active(&file_path, "user1@example.com").expect("create accounts");
        let parsed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file_path).unwrap()).unwrap();
        assert_eq!(parsed["active"], "user1@example.com");
        assert!(parsed["old"].is_array());

        update_google_accounts_active(&file_path, "user2@example.com").expect("update active");
        let parsed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file_path).unwrap()).unwrap();
        assert_eq!(parsed["active"], "user2@example.com");

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
