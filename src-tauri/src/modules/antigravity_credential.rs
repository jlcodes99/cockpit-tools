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
    id_token: Option<String>,
}

#[derive(Clone)]
pub struct AntigravitySystemCredential {
    pub access_token: Option<String>,
    pub refresh_token: String,
    pub token_type: Option<String>,
    pub expiry: Option<String>,
    pub auth_method: Option<String>,
    pub id_token: Option<String>,
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
    if let Some(encoded) = trimmed.strip_prefix("go-keyring-base64:") {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| "Antigravity 系统凭据 Base64 格式无效".to_string())?;
        return String::from_utf8(bytes).map_err(|_| "Antigravity 系统凭据编码无效".to_string());
    }
    Ok(trimmed.to_string())
}

fn parse_antigravity_system_credential(
    secret: &str,
) -> Result<AntigravitySystemCredential, String> {
    let payload_json = normalize_antigravity_credential_secret(secret)?;
    let payload: StoredAntigravityCredentialPayload = serde_json::from_str(&payload_json)
        .map_err(|_| "解析 Antigravity 系统凭据失败：凭据 JSON 格式无效".to_string())?;
    let refresh_token = normalize_non_empty(payload.token.refresh_token.as_deref())
        .ok_or_else(|| "Antigravity 系统凭据缺少 refresh_token".to_string())?;

    Ok(AntigravitySystemCredential {
        access_token: normalize_non_empty(payload.token.access_token.as_deref()),
        refresh_token,
        token_type: normalize_non_empty(payload.token.token_type.as_deref()),
        expiry: normalize_non_empty(payload.token.expiry.as_deref()),
        auth_method: normalize_non_empty(payload.auth_method.as_deref()),
        id_token: normalize_non_empty(payload.id_token.as_deref()),
    })
}

fn build_antigravity_credential_payload(account: &Account) -> Result<String, String> {
    if account.pending_oauth
        || account.token.refresh_token.trim().is_empty()
        || account.token.access_token.trim().is_empty()
    {
        return Err("账号尚未完成 OAuth 授权，无法写入系统凭据".to_string());
    }
    let expiry = chrono::DateTime::from_timestamp(account.token.expiry_timestamp, 0)
        .ok_or_else(|| "账号 Token 到期时间无效".to_string())?
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

        let output = Command::new("security")
            .args([
                "add-generic-password",
                "-U",
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
            return Err("写入 macOS Keychain 失败，请解锁钥匙串并允许访问".to_string());
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
                "--label=Password for 'antigravity' on 'gemini'",
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
            return Err("Linux secret-tool 写入失败，请确认钥匙环服务运行中且已解锁".to_string());
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

pub fn read_antigravity_system_credential() -> Result<Option<AntigravitySystemCredential>, String> {
    let Some(secret) = read_antigravity_system_credential_secret()? else {
        return Ok(None);
    };
    parse_antigravity_system_credential(&secret).map(Some)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_antigravity_system_credential_secret() -> Result<Option<String>, String> {
    use std::process::Command;
    #[cfg(target_os = "linux")]
    let output = Command::new("secret-tool")
        .args(["lookup", "service", "gemini", "username", "antigravity"])
        .output()
        .map_err(|e| {
            format!("无法读取系统钥匙环，请安装 libsecret 的 secret-tool 并解锁登录钥匙环: {e}")
        })?;
    #[cfg(target_os = "macos")]
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
        .map_err(|e| format!("无法读取 macOS Keychain: {e}"))?;

    if !output.status.success() {
        #[cfg(target_os = "linux")]
        let missing = output.status.code() == Some(1) && output.stderr.is_empty();
        #[cfg(target_os = "macos")]
        let missing = output.status.code() == Some(44);
        if missing {
            return Ok(None);
        }
        // Never echo command output: keyring helpers may include secret content.
        return Err("无法读取 Antigravity 系统凭据，请确认钥匙环服务运行中且已解锁".to_string());
    }
    let secret =
        String::from_utf8(output.stdout).map_err(|_| "Antigravity 系统凭据编码无效".to_string())?;
    if secret.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(secret))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TokenData;

    #[cfg(target_os = "linux")]
    #[test]
    fn secret_tool_roundtrip_and_failures_preserve_credentials() {
        use std::os::unix::fs::PermissionsExt;
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("agy-keyring-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("secret-tool");
        std::fs::write(
            &helper,
            r#"#!/bin/sh
case "$1" in
  store)
    test "$3 $4 $5 $6" = 'service gemini username antigravity' || exit 9
    if test "$AGY_TEST_KEYRING_MODE" = fail; then
      echo DO_NOT_LOG >&2
      exit 1
    fi
    /bin/cat > "$AGY_TEST_KEYRING_FILE"
    ;;
  lookup)
    test "$2 $3 $4 $5" = 'service gemini username antigravity' || exit 9
    if test "$AGY_TEST_KEYRING_MODE" = missing; then exit 1; fi
    if test "$AGY_TEST_KEYRING_MODE" = fail; then
      echo DO_NOT_LOG >&2
      exit 1
    fi
    /bin/cat "$AGY_TEST_KEYRING_FILE"
    ;;
  *) exit 9 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Restore process environment even if an assertion fails. The helper
        // isolates this test from the user's real Secret Service and credentials.
        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (key, value) in &self.0 {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let _env = EnvGuard(
            ["PATH", "AGY_TEST_KEYRING_FILE", "AGY_TEST_KEYRING_MODE"]
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect(),
        );
        std::env::set_var("PATH", &dir);
        let stored = dir.join("credential.json");
        std::env::set_var("AGY_TEST_KEYRING_FILE", &stored);
        std::env::set_var("AGY_TEST_KEYRING_MODE", "ok");
        let token = TokenData::new("access".into(), "refresh".into(), 3600, None, None, None);
        let mut account = Account::new("test".into(), "test@example.com".into(), token);
        write_antigravity_system_credential(&account).unwrap();
        assert_eq!(
            read_antigravity_system_credential()
                .unwrap()
                .unwrap()
                .refresh_token,
            "refresh"
        );
        let original = std::fs::read(&stored).unwrap();
        std::env::set_var("AGY_TEST_KEYRING_MODE", "fail");
        account.token.refresh_token = "new-refresh".into();
        let error = write_antigravity_system_credential(&account).unwrap_err();
        assert!(!error.contains("DO_NOT_LOG"));
        assert_eq!(std::fs::read(&stored).unwrap(), original);
        assert!(!read_antigravity_system_credential()
            .err()
            .unwrap()
            .contains("DO_NOT_LOG"));
        std::env::set_var("AGY_TEST_KEYRING_MODE", "missing");
        assert!(read_antigravity_system_credential().unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_linux_and_windows_json_and_macos_go_keyring_payload() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let raw = r#"{"token":{"access_token":"access","refresh_token":"refresh","token_type":"Bearer","expiry":"2030-01-01T00:00:00Z"},"auth_method":"consumer","id_token":"identity"}"#;
        for secret in [
            raw.to_string(),
            format!("go-keyring-base64:{}", STANDARD.encode(raw)),
        ] {
            let parsed = parse_antigravity_system_credential(&secret).unwrap();
            assert_eq!(parsed.refresh_token, "refresh");
            assert_eq!(parsed.id_token.as_deref(), Some("identity"));
            assert_eq!(parsed.auth_method.as_deref(), Some("consumer"));
        }
    }

    #[test]
    fn invalid_payload_errors_do_not_expose_secret_values() {
        for raw in [
            "",
            "go-keyring-base64:invalid!",
            r#"{"token":{"refresh_token":" "}}"#,
            r#"{"token":{"refresh_token":42},"secret":"DO_NOT_LOG"}"#,
        ] {
            let error = parse_antigravity_system_credential(raw).err().unwrap();
            assert!(!error.contains("DO_NOT_LOG"));
        }
    }

    #[test]
    fn written_payload_roundtrips_identity_and_expiry() {
        let token = TokenData::new("access".into(), "refresh".into(), 3600, None, None, None)
            .with_oauth_metadata(None, Some("identity".into()));
        let mut account = Account::new("test".into(), "test@example.com".into(), token);
        let secret = build_antigravity_credential_payload(&account).unwrap();
        let parsed = parse_antigravity_system_credential(&secret).unwrap();
        assert_eq!(parsed.id_token, account.token.id_token);
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(parsed.expiry.as_deref().unwrap())
                .unwrap()
                .timestamp(),
            account.token.expiry_timestamp
        );
        account.pending_oauth = true;
        assert!(build_antigravity_credential_payload(&account).is_err());
    }
}
