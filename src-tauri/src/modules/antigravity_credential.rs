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
