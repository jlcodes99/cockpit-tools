//! VS Code GitHub Copilot token injection module.
//!
//! Enables one-click Copilot account switching in VS Code by directly
//! writing auth sessions into VS Code's state.vscdb database.
//!
//! ## Platform crypto model
//!
//! - Windows: Local State `os_crypt.encrypted_key` + DPAPI, payload is `v10` + AES-256-GCM
//! - macOS: Keychain "Code Safe Storage" password, payload is `v10` + AES-128-CBC
//! - Linux: Secret Service password for `v11` + AES-128-CBC, fallback `v10` fixed key
//!
//! This module decrypts the existing GitHub auth sessions, replaces the token,
//! re-encrypts, and writes back.

#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

#[cfg(not(target_os = "windows"))]
use aes::Aes128;
#[cfg(target_os = "windows")]
use aes_gcm::aead::generic_array::GenericArray;
#[cfg(target_os = "windows")]
use aes_gcm::aead::{Aead, AeadCore, OsRng};
#[cfg(target_os = "windows")]
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
#[cfg(target_os = "windows")]
use base64::{engine::general_purpose, Engine as _};
#[cfg(not(target_os = "windows"))]
use cbc::cipher::block_padding::Pkcs7;
#[cfg(not(target_os = "windows"))]
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
#[cfg(not(target_os = "windows"))]
use pbkdf2::pbkdf2_hmac;
use rusqlite::Connection;
#[cfg(not(target_os = "windows"))]
use sha1::Sha1;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{LocalFree, HLOCAL};
#[cfg(target_os = "windows")]
use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

#[cfg(not(target_os = "windows"))]
type Aes128CbcEnc = cbc::Encryptor<Aes128>;
#[cfg(not(target_os = "windows"))]
type Aes128CbcDec = cbc::Decryptor<Aes128>;

const V10_PREFIX: &[u8] = b"v10";
const V11_PREFIX: &[u8] = b"v11";
#[cfg(not(target_os = "windows"))]
const CBC_IV: [u8; 16] = [b' '; 16];
#[cfg(not(target_os = "windows"))]
const SALT: &[u8] = b"saltysalt";

#[derive(Clone, Copy)]
enum SafeStorageReadMode {
    Default,
    AntigravityOnly,
    CodeBuddyOnly,
    CodeBuddyCnOnly,
    QoderOnly,
    WorkBuddyOnly,
}

// PBKDF2-HMAC-SHA1(1 iteration, key = "peanuts", salt = "saltysalt")
#[cfg(target_os = "linux")]
const LINUX_V10_KEY: [u8; 16] = [
    0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78,
];

// PBKDF2-HMAC-SHA1(1 iteration, key = "", salt = "saltysalt")
#[cfg(target_os = "linux")]
const LINUX_EMPTY_KEY: [u8; 16] = [
    0xd0, 0xd0, 0xec, 0x9c, 0x7d, 0x77, 0xd4, 0x3a, 0xc5, 0x41, 0x87, 0xfa, 0x48, 0x18, 0xd1, 0x7f,
];

fn resolve_vscode_data_root(user_data_dir: Option<&str>) -> Result<PathBuf, String> {
    crate::modules::vscode_paths::resolve_vscode_data_root(user_data_dir).map_err(|err| {
        if err == "GitHub Copilot 仅支持 macOS、Windows 和 Linux" {
            "Unsupported platform".to_string()
        } else {
            err
        }
    })
}

fn get_vscode_db_path_from_data_root(data_root: &Path) -> Result<PathBuf, String> {
    let path = crate::modules::vscode_paths::vscode_state_db_path(data_root);
    if path.exists() {
        Ok(path)
    } else {
        let attempted = crate::modules::vscode_paths::vscode_data_root_candidates()
            .ok()
            .filter(|candidates| candidates.iter().any(|candidate| candidate == data_root))
            .map(|candidates| {
                candidates
                    .iter()
                    .map(|candidate| {
                        crate::modules::vscode_paths::vscode_state_db_path(candidate)
                            .display()
                            .to_string()
                    })
                    .collect::<Vec<String>>()
                    .join(", ")
            });
        if let Some(paths) = attempted {
            Err(format!("VS Code database not found. Tried: {}", paths))
        } else {
            Err(format!("VS Code database not found: {}", path.display()))
        }
    }
}

fn build_secret_storage_item_key(extension_id: &str, key: &str) -> String {
    format!(
        r#"secret://{{"extensionId":"{}","key":"{}"}}"#,
        extension_id, key
    )
}

#[cfg(target_os = "windows")]
fn get_local_state_path(data_root: &Path) -> Result<PathBuf, String> {
    let path = crate::modules::vscode_paths::vscode_local_state_path(data_root);
    if path.exists() {
        Ok(path)
    } else {
        let attempted = crate::modules::vscode_paths::vscode_data_root_candidates()
            .ok()
            .filter(|candidates| candidates.iter().any(|candidate| candidate == data_root))
            .map(|candidates| {
                candidates
                    .iter()
                    .map(|candidate| {
                        crate::modules::vscode_paths::vscode_local_state_path(candidate)
                            .display()
                            .to_string()
                    })
                    .collect::<Vec<String>>()
                    .join(", ")
            });
        if let Some(paths) = attempted {
            Err(format!("VS Code Local State not found. Tried: {}", paths))
        } else {
            Err(format!("VS Code Local State not found: {}", path.display()))
        }
    }
}

#[cfg(target_os = "windows")]
fn get_windows_encryption_key(data_root: Option<&Path>) -> Result<Vec<u8>, String> {
    let owned_root;
    let root = if let Some(path) = data_root {
        path
    } else {
        owned_root = crate::modules::vscode_paths::resolve_vscode_data_root_for_state_db()
            .map_err(|err| {
                if err == "GitHub Copilot 仅支持 macOS、Windows 和 Linux" {
                    "Unsupported platform".to_string()
                } else {
                    err
                }
            })?;
        owned_root.as_path()
    };
    let path = get_local_state_path(root)?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read Local State: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse Local State JSON: {}", e))?;

    let encrypted_key_b64 = json["os_crypt"]["encrypted_key"]
        .as_str()
        .ok_or("Cannot find os_crypt.encrypted_key in Local State")?;

    let encrypted_key_bytes = general_purpose::STANDARD
        .decode(encrypted_key_b64)
        .map_err(|e| format!("Base64 decode failed for encrypted_key: {}", e))?;

    if encrypted_key_bytes.len() < 6 {
        return Err("encrypted_key data too short".to_string());
    }

    let prefix = String::from_utf8_lossy(&encrypted_key_bytes[..5]);
    if prefix != "DPAPI" {
        return Err(format!(
            "encrypted_key prefix is not DPAPI, got: {}",
            prefix
        ));
    }

    let dpapi_blob = &encrypted_key_bytes[5..];
    let key = dpapi_decrypt(dpapi_blob)?;
    if key.len() != 32 {
        return Err(format!(
            "Decrypted AES key has unexpected length: {}",
            key.len()
        ));
    }
    Ok(key)
}

#[cfg(target_os = "windows")]
fn dpapi_decrypt(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        CryptUnprotectData(&mut input, None, None, None, None, 0, &mut output)
            .map_err(|_| "DPAPI CryptUnprotectData call failed".to_string())?;

        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(result)
    }
}

#[cfg(target_os = "windows")]
fn decrypt_windows_gcm_v10(key: &[u8], encrypted: &[u8]) -> Result<Vec<u8>, String> {
    if encrypted.len() < 31 {
        return Err("Encrypted data too short".to_string());
    }
    if &encrypted[..3] != V10_PREFIX {
        return Err(format!(
            "Not Windows v10 format, prefix: {:?}",
            &encrypted[..3]
        ));
    }

    let nonce_bytes = &encrypted[3..15];
    let ciphertext = &encrypted[15..];

    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-GCM decryption failed: {}", e))
}

#[cfg(target_os = "windows")]
fn encrypt_windows_gcm_v10(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("AES-GCM encryption failed: {}", e))?;

    let mut result = Vec::with_capacity(3 + 12 + ciphertext.len());
    result.extend_from_slice(V10_PREFIX);
    result.extend_from_slice(nonce.as_slice());
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

#[cfg(not(target_os = "windows"))]
fn decrypt_cbc_prefixed(
    encrypted: &[u8],
    expected_prefix: &[u8],
    key: &[u8; 16],
) -> Result<Vec<u8>, String> {
    if !encrypted.starts_with(expected_prefix) {
        return Err(format!(
            "Unexpected ciphertext prefix: {:?}",
            &encrypted[..encrypted.len().min(3)]
        ));
    }
    let raw = &encrypted[expected_prefix.len()..];
    let mut buf = raw.to_vec();
    let cipher = Aes128CbcDec::new_from_slices(key, &CBC_IV)
        .map_err(|e| format!("Failed to init AES-CBC decryptor: {}", e))?;
    let plaintext = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("AES-CBC decryption failed: {}", e))?
        .to_vec();
    Ok(plaintext)
}

#[cfg(not(target_os = "windows"))]
fn encrypt_cbc_prefixed(
    prefix: &[u8],
    key: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = Aes128CbcEnc::new_from_slices(key, &CBC_IV)
        .map_err(|e| format!("Failed to init AES-CBC encryptor: {}", e))?;

    let mut buf = plaintext.to_vec();
    let msg_len = buf.len();
    let pad_len = 16 - (msg_len % 16);
    buf.resize(msg_len + pad_len, 0);
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
        .map_err(|e| format!("AES-CBC encryption failed: {}", e))?
        .to_vec();

    let mut result = Vec::with_capacity(prefix.len() + ciphertext.len());
    result.extend_from_slice(prefix);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

#[cfg(not(target_os = "windows"))]
fn pbkdf2_sha1_key(password: &str, iterations: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), SALT, iterations, &mut key);
    key
}

fn detect_prefix(encrypted: &[u8]) -> Option<&'static str> {
    if encrypted.starts_with(V10_PREFIX) {
        Some("v10")
    } else if encrypted.starts_with(V11_PREFIX) {
        Some("v11")
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn run_command_get_trimmed(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "macos")]
fn build_macos_safe_storage_candidates(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Vec<(String, Option<String>)> {
    if matches!(mode, SafeStorageReadMode::AntigravityOnly) {
        return vec![
            (
                "Antigravity Safe Storage".to_string(),
                Some("Antigravity".to_string()),
            ),
            ("Antigravity Safe Storage".to_string(), None),
            (
                "Antigravity Safe Storage".to_string(),
                Some("Antigravity Safe Storage".to_string()),
            ),
        ];
    }

    if matches!(mode, SafeStorageReadMode::CodeBuddyOnly) {
        return vec![
            (
                "CodeBuddy Safe Storage".to_string(),
                Some("CodeBuddy".to_string()),
            ),
            (
                "CodeBuddy Safe Storage".to_string(),
                Some("codebuddy".to_string()),
            ),
            (
                "CodeBuddy Safe Storage".to_string(),
                Some("CodeBuddy Key".to_string()),
            ),
            ("CodeBuddy Safe Storage".to_string(), None),
            (
                "CodeBuddy Safe Storage".to_string(),
                Some("CodeBuddy Safe Storage".to_string()),
            ),
        ];
    }

    if matches!(mode, SafeStorageReadMode::CodeBuddyCnOnly) {
        return vec![
            (
                "CodeBuddy CN Safe Storage".to_string(),
                Some("CodeBuddy CN".to_string()),
            ),
            (
                "CodeBuddy CN Safe Storage".to_string(),
                Some("codebuddy cn".to_string()),
            ),
            (
                "CodeBuddy CN Safe Storage".to_string(),
                Some("CodeBuddy CN Key".to_string()),
            ),
            ("CodeBuddy CN Safe Storage".to_string(), None),
            (
                "CodeBuddy CN Safe Storage".to_string(),
                Some("CodeBuddy CN Safe Storage".to_string()),
            ),
        ];
    }

    if matches!(mode, SafeStorageReadMode::QoderOnly) {
        return vec![
            ("Qoder Safe Storage".to_string(), Some("Qoder".to_string())),
            ("Qoder Safe Storage".to_string(), Some("qoder".to_string())),
            ("Qoder Safe Storage".to_string(), None),
            (
                "Qoder Safe Storage".to_string(),
                Some("Qoder Safe Storage".to_string()),
            ),
        ];
    }

    if matches!(mode, SafeStorageReadMode::WorkBuddyOnly) {
        return vec![
            (
                "WorkBuddy Safe Storage".to_string(),
                Some("WorkBuddy".to_string()),
            ),
            (
                "WorkBuddy Safe Storage".to_string(),
                Some("workbuddy".to_string()),
            ),
            (
                "WorkBuddy Safe Storage".to_string(),
                Some("WorkBuddy Key".to_string()),
            ),
            ("WorkBuddy Safe Storage".to_string(), None),
            (
                "WorkBuddy Safe Storage".to_string(),
                Some("WorkBuddy Safe Storage".to_string()),
            ),
        ];
    }

    let mut app_names: Vec<String> = Vec::new();
    if let Some(root) = data_root {
        if let Some(name) = root.file_name().and_then(|value| value.to_str()) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                app_names.push(trimmed.to_string());
            }
        }
    }

    // Default mode is used by VS Code / GitHub Copilot injection path.
    // Keep this list strictly VS Code-family to avoid cross-platform key probing.
    app_names.extend(
        [
            "Code",
            "Code - Insiders",
            "Visual Studio Code",
            "Visual Studio Code - Insiders",
            "Code - OSS",
            "VSCodium",
        ]
        .iter()
        .map(|value| value.to_string()),
    );

    let mut candidates: Vec<(String, Option<String>)> = Vec::new();
    let mut seen = HashSet::new();

    for app_name in app_names {
        let service = format!("{} Safe Storage", app_name);
        let account = Some(app_name.clone());
        if seen.insert((service.clone(), account.clone())) {
            candidates.push((service.clone(), account));
        }
        if seen.insert((service.clone(), None)) {
            candidates.push((service.clone(), None));
        }
        let alt_account = Some(service.clone());
        if seen.insert((service.clone(), alt_account.clone())) {
            candidates.push((service, alt_account));
        }
    }

    candidates
}

#[cfg(target_os = "macos")]
fn get_macos_safe_storage_password(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<String, String> {
    let candidates = build_macos_safe_storage_candidates(data_root, mode);
    for (service, account) in candidates {
        if let Some(account_value) = account.as_deref() {
            if let Some(password) = run_command_get_trimmed(
                "security",
                &[
                    "find-generic-password",
                    "-w",
                    "-s",
                    &service,
                    "-a",
                    account_value,
                ],
            ) {
                return Ok(password);
            }
        }
        if let Some(password) =
            run_command_get_trimmed("security", &["find-generic-password", "-w", "-s", &service])
        {
            return Ok(password);
        }
    }
    Err("Failed to read Safe Storage password from Keychain".to_string())
}

#[cfg(target_os = "linux")]
fn run_command_get_trimmed(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "linux")]
fn get_linux_v11_key(mode: SafeStorageReadMode) -> Option<[u8; 16]> {
    let app_names: &[&str] = match mode {
        SafeStorageReadMode::CodeBuddyOnly => &["CodeBuddy", "codebuddy"],
        SafeStorageReadMode::CodeBuddyCnOnly => &[
            "CodeBuddy CN",
            "codebuddy cn",
            "codebuddy-cn",
            "codebuddycn",
        ],
        SafeStorageReadMode::QoderOnly => &["Qoder", "qoder"],
        SafeStorageReadMode::WorkBuddyOnly => {
            &["WorkBuddy", "workbuddy", "workbuddy-cn", "workbuddycn"]
        }
        _ => &[
            "code",
            "Code",
            "code-insiders",
            "Code - Insiders",
            "code-oss",
            "Code - OSS",
            "VSCodium",
        ],
    };

    for app in app_names {
        if let Some(password) =
            run_command_get_trimmed("secret-tool", &["lookup", "application", app])
        {
            return Some(pbkdf2_sha1_key(&password, 1));
        }
    }

    None
}

fn decrypt_secret_payload_with_mode(
    encrypted: &[u8],
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<Vec<u8>, String> {
    #[cfg(not(target_os = "windows"))]
    let _ = (data_root, mode);

    #[cfg(target_os = "windows")]
    {
        let _ = mode;
        let key = get_windows_encryption_key(data_root)?;
        return decrypt_windows_gcm_v10(&key, encrypted);
    }

    #[cfg(target_os = "macos")]
    {
        let password = get_macos_safe_storage_password(data_root, mode)?;
        let key = pbkdf2_sha1_key(&password, 1003);
        return decrypt_cbc_prefixed(encrypted, V10_PREFIX, &key);
    }

    #[cfg(target_os = "linux")]
    {
        match detect_prefix(encrypted) {
            Some("v11") => {
                let key = get_linux_v11_key(mode).ok_or(
                    "Cannot load Linux secret storage key for VS Code (v11 payload)".to_string(),
                )?;
                match decrypt_cbc_prefixed(encrypted, V11_PREFIX, &key) {
                    Ok(value) => Ok(value),
                    Err(_) => decrypt_cbc_prefixed(encrypted, V11_PREFIX, &LINUX_EMPTY_KEY),
                }
            }
            Some("v10") => match decrypt_cbc_prefixed(encrypted, V10_PREFIX, &LINUX_V10_KEY) {
                Ok(value) => Ok(value),
                Err(_) => decrypt_cbc_prefixed(encrypted, V10_PREFIX, &LINUX_EMPTY_KEY),
            },
            _ => Err(format!(
                "Unsupported Linux ciphertext prefix: {:?}",
                &encrypted[..encrypted.len().min(3)]
            )),
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = encrypted;
        let _ = (data_root, mode);
        Err("Unsupported platform".to_string())
    }
}

fn decrypt_secret_payload(encrypted: &[u8], data_root: Option<&Path>) -> Result<Vec<u8>, String> {
    decrypt_secret_payload_with_mode(encrypted, data_root, SafeStorageReadMode::Default)
}

fn encrypt_secret_payload(
    plaintext: &[u8],
    preferred_prefix: Option<&str>,
    data_root: Option<&Path>,
) -> Result<Vec<u8>, String> {
    encrypt_secret_payload_with_mode(
        plaintext,
        preferred_prefix,
        data_root,
        SafeStorageReadMode::Default,
    )
}

fn encrypt_secret_payload_with_mode(
    plaintext: &[u8],
    preferred_prefix: Option<&str>,
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<Vec<u8>, String> {
    #[cfg(not(target_os = "linux"))]
    let _ = preferred_prefix;

    #[cfg(target_os = "windows")]
    {
        let _ = mode;
        let key = get_windows_encryption_key(data_root)?;
        return encrypt_windows_gcm_v10(&key, plaintext);
    }

    #[cfg(target_os = "macos")]
    {
        let password = get_macos_safe_storage_password(data_root, mode)?;
        let key = pbkdf2_sha1_key(&password, 1003);
        return encrypt_cbc_prefixed(V10_PREFIX, &key, plaintext);
    }

    #[cfg(target_os = "linux")]
    {
        let _ = data_root;
        let target_prefix = if let Some(prefix) = preferred_prefix {
            prefix
        } else if get_linux_v11_key(mode).is_some() {
            "v11"
        } else {
            "v10"
        };

        if target_prefix == "v11" {
            let key = get_linux_v11_key(mode).ok_or(
                "Cannot load Linux secret storage key for VS Code (v11 payload)".to_string(),
            )?;
            return encrypt_cbc_prefixed(V11_PREFIX, &key, plaintext);
        }

        return encrypt_cbc_prefixed(V10_PREFIX, &LINUX_V10_KEY, plaintext);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (plaintext, preferred_prefix, data_root, mode);
        Err("Unsupported platform".to_string())
    }
}

const GITHUB_AUTH_SECRET_KEY: &str =
    r#"secret://{"extensionId":"vscode.github-authentication","key":"github.auth"}"#;
const GITHUB_COPILOT_LOGIN_KEY: &str = "github.copilot-github";

fn encode_secret_buffer(encrypted: Vec<u8>) -> Result<String, String> {
    let buffer_json = serde_json::json!({
        "type": "Buffer",
        "data": encrypted
    });
    serde_json::to_string(&buffer_json).map_err(|e| format!("Failed to serialize Buffer: {}", e))
}

fn read_item_table_value(db_path: &Path, key: &str) -> Result<Option<String>, String> {
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(db_path).map_err(|e| {
        format!(
            "Failed to open VS Code database {}: {}",
            db_path.display(),
            e
        )
    })?;
    match conn.query_row("SELECT value FROM ItemTable WHERE key = ?", [key], |row| row.get(0)) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!(
            "Failed to query key '{}' from VS Code database {}: {}",
            key,
            db_path.display(),
            e
        )),
    }
}

fn write_copilot_items_to_db(
    db_path: &Path,
    secret_value: &str,
    username: &str,
) -> Result<(), String> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create VS Code database parent dir: {}", e))?;
    }

    let conn = Connection::open(db_path).map_err(|e| {
        format!(
            "Failed to open VS Code database {}: {}",
            db_path.display(),
            e
        )
    })?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("Failed to init ItemTable in {}: {}", db_path.display(), e))?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        [GITHUB_AUTH_SECRET_KEY, secret_value],
    )
    .map_err(|e| format!("Failed to write github.auth to {}: {}", db_path.display(), e))?;

    tx.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        [GITHUB_COPILOT_LOGIN_KEY, username],
    )
    .map_err(|e| {
        format!(
            "Failed to write github.copilot-github to {}: {}",
            db_path.display(),
            e
        )
    })?;

    tx.commit()
        .map_err(|e| format!("Failed to commit transaction: {}", e))
}

fn copilot_state_db_write_paths(data_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        paths.push(crate::modules::vscode_paths::vscode_state_db_path(data_root));
        if let Some(shared_path) = crate::modules::vscode_paths::vscode_shared_storage_db_path(data_root)
        {
            if !paths.iter().any(|path| path == &shared_path) {
                paths.push(shared_path);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        paths.push(get_vscode_db_path_from_data_root(data_root)?);
    }

    Ok(paths)
}

fn find_existing_github_auth_value(write_paths: &[PathBuf]) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    let ordered_paths: Vec<&PathBuf> = write_paths.iter().rev().collect();
    #[cfg(not(target_os = "windows"))]
    let ordered_paths: Vec<&PathBuf> = write_paths.iter().collect();

    for path in ordered_paths {
        if let Some(value) = read_item_table_value(path, GITHUB_AUTH_SECRET_KEY)? {
            return Ok(Some(value));
        }
    }

    Ok(None)
}

fn inject_copilot_token_with_data_root(
    data_root: &Path,
    username: &str,
    token: &str,
    github_user_id: Option<&str>,
) -> Result<String, String> {
    let write_paths = copilot_state_db_write_paths(data_root)?;
    let existing = find_existing_github_auth_value(&write_paths)?;

    let (new_sessions, existing_prefix) = build_github_auth_sessions(
        existing.as_deref(),
        Some(data_root),
        username,
        token,
        github_user_id,
    )?;

    let sessions_json = serde_json::to_string(&new_sessions)
        .map_err(|e| format!("Failed to serialize sessions: {}", e))?;
    let encrypted = encrypt_secret_payload(
        sessions_json.as_bytes(),
        existing_prefix.as_deref(),
        Some(data_root),
    )?;
    let buffer_str = encode_secret_buffer(encrypted)?;

    for db_path in &write_paths {
        write_copilot_items_to_db(db_path, &buffer_str, username)?;
    }

    Ok(format!(
        "Successfully injected {} into VS Code ({} state DB{})",
        username,
        write_paths.len(),
        if write_paths.len() == 1 { "" } else { "s" }
    ))
}

fn decode_buffer_data(buffer: &serde_json::Value) -> Result<Vec<u8>, String> {
    let data_arr = buffer["data"]
        .as_array()
        .ok_or("Secret data is not in Buffer format")?;

    let mut encrypted_bytes: Vec<u8> = Vec::with_capacity(data_arr.len());
    for (idx, v) in data_arr.iter().enumerate() {
        let n = v
            .as_u64()
            .ok_or_else(|| format!("Secret data element at index {} is not an integer", idx))?;
        if n > 255 {
            return Err(format!(
                "Secret data element at index {} is out of range ({} > 255)",
                idx, n
            ));
        }
        encrypted_bytes.push(n as u8);
    }

    Ok(encrypted_bytes)
}

fn decode_secret_storage_value_with_mode(
    raw_value: &str,
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<String, String> {
    let parsed: serde_json::Value = match serde_json::from_str(raw_value) {
        Ok(value) => value,
        Err(_) => return Ok(raw_value.to_string()),
    };

    if parsed.get("data").is_some() {
        let encrypted_bytes = decode_buffer_data(&parsed)?;
        let decrypted = decrypt_secret_payload_with_mode(&encrypted_bytes, data_root, mode)?;
        return String::from_utf8(decrypted)
            .map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e));
    }

    if let Some(value) = parsed.as_str() {
        return Ok(value.to_string());
    }

    Ok(raw_value.to_string())
}

fn read_secret_storage_value_with_data_root_and_mode(
    data_root: &Path,
    extension_id: &str,
    key: &str,
    mode: SafeStorageReadMode,
) -> Result<Option<String>, String> {
    let db_path = data_root
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(&db_path).map_err(|e| {
        format!(
            "Failed to open VS Code database {}: {}",
            db_path.display(),
            e
        )
    })?;
    let secret_key = build_secret_storage_item_key(extension_id, key);
    let raw_value: Option<String> = match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [secret_key.as_str()],
        |row| row.get(0),
    ) {
        Ok(value) => Some(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(err) => {
            return Err(format!(
                "Failed to query VS Code secret '{}' for extension '{}': {}",
                key, extension_id, err
            ))
        }
    };

    match raw_value {
        Some(value) => {
            decode_secret_storage_value_with_mode(&value, Some(data_root), mode).map(Some)
        }
        None => Ok(None),
    }
}

pub fn read_antigravity_secret_storage_value(
    extension_id: &str,
    key: &str,
    user_data_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let data_root = resolve_vscode_data_root(user_data_dir)?;
    read_secret_storage_value_with_data_root_and_mode(
        &data_root,
        extension_id,
        key,
        SafeStorageReadMode::AntigravityOnly,
    )
}

pub fn read_codebuddy_secret_storage_value(
    extension_id: &str,
    key: &str,
    user_data_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let data_root = resolve_vscode_data_root(user_data_dir)?;
    read_secret_storage_value_with_data_root_and_mode(
        &data_root,
        extension_id,
        key,
        SafeStorageReadMode::CodeBuddyOnly,
    )
}

pub fn read_codebuddy_cn_secret_storage_value(
    extension_id: &str,
    key: &str,
    user_data_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let data_root = resolve_vscode_data_root(user_data_dir)?;
    read_secret_storage_value_with_data_root_and_mode(
        &data_root,
        extension_id,
        key,
        SafeStorageReadMode::CodeBuddyCnOnly,
    )
}

pub fn read_workbuddy_secret_storage_value(
    extension_id: &str,
    key: &str,
    user_data_dir: Option<&str>,
) -> Result<Option<String>, String> {
    let data_root = resolve_vscode_data_root(user_data_dir)?;
    read_secret_storage_value_with_data_root_and_mode(
        &data_root,
        extension_id,
        key,
        SafeStorageReadMode::WorkBuddyOnly,
    )
}

fn read_github_auth_secret_for_data_root(data_root: &Path) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    if let Some(shared_path) = crate::modules::vscode_paths::vscode_shared_storage_db_path(data_root)
    {
        if let Some(raw_value) = read_item_table_value(&shared_path, GITHUB_AUTH_SECRET_KEY)? {
            return decode_secret_storage_value_with_mode(
                &raw_value,
                Some(data_root),
                SafeStorageReadMode::Default,
            )
            .map(Some);
        }
    }

    read_secret_storage_value_with_data_root_and_mode(
        data_root,
        "vscode.github-authentication",
        "github.auth",
        SafeStorageReadMode::Default,
    )
}

pub fn read_github_auth_sessions(
    user_data_dir: Option<&str>,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    let data_root = resolve_vscode_data_root(user_data_dir)?;
    let Some(raw_value) = read_github_auth_secret_for_data_root(&data_root)? else {
        return Ok(None);
    };

    let sessions: Vec<serde_json::Value> = serde_json::from_str(&raw_value)
        .map_err(|e| format!("解析 VS Code GitHub 登录会话失败: {}", e))?;

    if sessions.is_empty() {
        return Ok(None);
    }

    Ok(Some(sessions))
}

fn resolve_data_root_from_state_db_path(db_path: &Path) -> Result<&Path, String> {
    db_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            format!(
                "Cannot determine data root from db path: {}",
                db_path.display()
            )
        })
}

fn read_secret_storage_value_by_db_path_and_mode(
    db_path: &Path,
    db_key: &str,
    mode: SafeStorageReadMode,
) -> Result<Option<String>, String> {
    if !db_path.exists() {
        return Ok(None);
    }

    let data_root = resolve_data_root_from_state_db_path(db_path)?;
    let conn = Connection::open(db_path).map_err(|e| {
        format!(
            "Failed to open VS Code database {}: {}",
            db_path.display(),
            e
        )
    })?;

    let raw_value: Option<String> = match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [db_key],
        |row| row.get(0),
    ) {
        Ok(value) => Some(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(err) => {
            return Err(format!(
                "Failed to query VS Code secret key '{}': {}",
                db_key, err
            ))
        }
    };

    match raw_value {
        Some(value) => {
            decode_secret_storage_value_with_mode(&value, Some(data_root), mode).map(Some)
        }
        None => Ok(None),
    }
}

pub fn read_qoder_secret_storage_value_by_db_path(
    db_path: &Path,
    db_key: &str,
) -> Result<Option<String>, String> {
    read_secret_storage_value_by_db_path_and_mode(db_path, db_key, SafeStorageReadMode::QoderOnly)
}

fn load_existing_sessions(
    existing_encrypted_value: Option<&str>,
    data_root: Option<&Path>,
) -> Result<(Vec<serde_json::Value>, Option<String>), String> {
    let Some(value) = existing_encrypted_value else {
        return Ok((Vec::new(), None));
    };

    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|e| format!("Failed to parse existing secret JSON: {}", e))?;

    if parsed.is_array() {
        let sessions: Vec<serde_json::Value> = serde_json::from_value(parsed)
            .map_err(|e| format!("Existing sessions JSON is invalid: {}", e))?;
        return Ok((sessions, None));
    }

    let encrypted_bytes = decode_buffer_data(&parsed)?;
    let prefix = detect_prefix(&encrypted_bytes).map(|s| s.to_string());
    let decrypted = decrypt_secret_payload(&encrypted_bytes, data_root)?;
    let json_str = String::from_utf8(decrypted)
        .map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))?;
    let sessions: Vec<serde_json::Value> = serde_json::from_str(&json_str)
        .map_err(|e| format!("Decrypted github.auth is not a valid sessions array: {}", e))?;

    Ok((sessions, prefix))
}

fn build_github_auth_sessions(
    existing_encrypted_value: Option<&str>,
    data_root: Option<&Path>,
    username: &str,
    token: &str,
    github_user_id: Option<&str>,
) -> Result<(serde_json::Value, Option<String>), String> {
    let (mut sessions, existing_prefix) =
        load_existing_sessions(existing_encrypted_value, data_root)?;

    let user_id = github_user_id.unwrap_or("0");
    let new_session = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "scopes": ["read:user", "user:email"],
        "accessToken": token,
        "account": {
            "label": username,
            "id": user_id
        }
    });

    let mut replaced = false;
    for session in &mut sessions {
        if let Some(scopes) = session["scopes"].as_array() {
            let is_github_user_session = scopes.iter().any(|scope| {
                matches!(scope.as_str(), Some("read:user") | Some("user:email"))
            });
            if is_github_user_session {
                *session = new_session.clone();
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        sessions.push(new_session);
    }

    Ok((serde_json::Value::Array(sessions), existing_prefix))
}

pub fn inject_copilot_token_for_user_data_dir(
    user_data_dir: &str,
    username: &str,
    token: &str,
    github_user_id: Option<&str>,
) -> Result<String, String> {
    let data_root = resolve_vscode_data_root(Some(user_data_dir))?;
    inject_copilot_token_with_data_root(&data_root, username, token, github_user_id)
}

pub fn inject_secret_to_state_db_for_codebuddy(
    db_path: &std::path::Path,
    db_key: &str,
    plaintext: &str,
) -> Result<(), String> {
    inject_secret_to_state_db_with_mode(
        db_path,
        db_key,
        plaintext,
        SafeStorageReadMode::CodeBuddyOnly,
    )
}

pub fn inject_secret_to_state_db_for_codebuddy_cn(
    db_path: &std::path::Path,
    db_key: &str,
    plaintext: &str,
) -> Result<(), String> {
    inject_secret_to_state_db_with_mode(
        db_path,
        db_key,
        plaintext,
        SafeStorageReadMode::CodeBuddyCnOnly,
    )
}

pub fn inject_secret_to_state_db_for_qoder(
    db_path: &std::path::Path,
    db_key: &str,
    plaintext: &str,
) -> Result<(), String> {
    inject_secret_to_state_db_with_mode(db_path, db_key, plaintext, SafeStorageReadMode::QoderOnly)
}

pub fn inject_secret_to_state_db_for_workbuddy(
    db_path: &std::path::Path,
    db_key: &str,
    plaintext: &str,
) -> Result<(), String> {
    inject_secret_to_state_db_with_mode(
        db_path,
        db_key,
        plaintext,
        SafeStorageReadMode::WorkBuddyOnly,
    )
}

fn inject_secret_to_state_db_with_mode(
    db_path: &std::path::Path,
    db_key: &str,
    plaintext: &str,
    mode: SafeStorageReadMode,
) -> Result<(), String> {
    let data_root = db_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            format!(
                "Cannot determine data root from db path: {}",
                db_path.display()
            )
        })?;

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create state.vscdb parent dir: {}", e))?;
    }

    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("Failed to open state.vscdb: {}", e))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("Failed to init ItemTable: {}", e))?;

    let existing_prefix: Option<String> = match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?",
        [db_key],
        |row| row.get::<_, String>(0),
    ) {
        Ok(val) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&val) {
                if let Ok(bytes) = decode_buffer_data(&parsed) {
                    detect_prefix(&bytes).map(|s| s.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
        Err(_) => None,
    };

    let encrypted = encrypt_secret_payload_with_mode(
        plaintext.as_bytes(),
        existing_prefix.as_deref(),
        Some(data_root),
        mode,
    )?;

    let buffer_json = serde_json::json!({
        "type": "Buffer",
        "data": encrypted
    });
    let buffer_str = serde_json::to_string(&buffer_json)
        .map_err(|e| format!("Failed to serialize Buffer: {}", e))?;

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        rusqlite::params![db_key, buffer_str],
    )
    .map_err(|e| format!("Failed to write to state.vscdb: {}", e))?;

    Ok(())
}
