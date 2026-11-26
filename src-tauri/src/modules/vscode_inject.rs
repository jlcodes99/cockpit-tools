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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::{LazyLock, Mutex};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(not(target_os = "windows"))]
use aes::Aes128;
#[cfg(target_os = "windows")]
use aes_gcm::aead::generic_array::GenericArray;
#[cfg(target_os = "windows")]
use aes_gcm::aead::{Aead, AeadCore, OsRng};
#[cfg(target_os = "windows")]
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
const MACOS_KEYCHAIN_PROMPT_TIMEOUT: Duration = Duration::from_secs(45);
#[cfg(target_os = "macos")]
const MACOS_KEYCHAIN_INTERACTIVE_COOLDOWN: Duration = Duration::from_secs(15);
#[cfg(target_os = "macos")]
const MACOS_KEYCHAIN_PASSIVE_COOLDOWN: Duration = Duration::from_secs(600);

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SafeStorageCacheKey {
    mode: SafeStorageReadMode,
    data_root_app_name: Option<String>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
enum SafeStorageCacheEntry {
    Password {
        password: String,
        service: String,
    },
    Cooldown {
        failed_at: Instant,
        reason: String,
    },
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MacosKeychainReadOutcome {
    Found(String),
    NotFound,
    EmptyPassword,
    AccessBlocked(String),
}

#[cfg(target_os = "macos")]
static MACOS_SAFE_STORAGE_CACHE: LazyLock<Mutex<HashMap<SafeStorageCacheKey, SafeStorageCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(target_os = "macos")]
static MACOS_KEYCHAIN_PROBE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(target_os = "macos")]
static MACOS_FALLBACK_SERVICE_ATTEMPTS: LazyLock<Mutex<HashSet<(SafeStorageCacheKey, String)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[cfg(target_os = "macos")]
fn build_macos_safe_storage_cache_key(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> SafeStorageCacheKey {
    let data_root_app_name = if matches!(mode, SafeStorageReadMode::Default) {
        data_root
            .and_then(|root| root.file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    } else {
        None
    };
    SafeStorageCacheKey {
        mode,
        data_root_app_name,
    }
}

#[cfg(target_os = "macos")]
fn classify_macos_security_result(
    result: std::io::Result<std::process::Output>,
) -> MacosKeychainReadOutcome {
    match result {
        Ok(output) => {
            if output.status.success() {
                let password = String::from_utf8_lossy(&output.stdout)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                if password.is_empty() {
                    MacosKeychainReadOutcome::EmptyPassword
                } else {
                    MacosKeychainReadOutcome::Found(password)
                }
            } else {
                let code = output.status.code();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let lower = stderr.to_ascii_lowercase();
                if code == Some(44)
                    || lower.contains("could not be found")
                    || lower.contains("errsecitemnotfound")
                {
                    MacosKeychainReadOutcome::NotFound
                } else if code == Some(128)
                    || lower.contains("user canceled")
                    || lower.contains("user cancelled")
                    || lower.contains("-128")
                {
                    MacosKeychainReadOutcome::AccessBlocked(
                        "Keychain access was canceled by user".to_string(),
                    )
                } else if code == Some(36)
                    || lower.contains("interaction is not allowed")
                    || lower.contains("-25308")
                {
                    MacosKeychainReadOutcome::AccessBlocked(
                        "Keychain user interaction is not allowed or keychain is locked"
                            .to_string(),
                    )
                } else if code == Some(51)
                    || lower.contains("passphrase")
                    || lower.contains("auth")
                    || lower.contains("denied")
                    || lower.contains("-25293")
                {
                    MacosKeychainReadOutcome::AccessBlocked(
                        "Keychain authorization failed or was denied".to_string(),
                    )
                } else if stderr.is_empty() {
                    MacosKeychainReadOutcome::AccessBlocked(format!(
                        "Keychain lookup failed with status {:?}",
                        code
                    ))
                } else {
                    MacosKeychainReadOutcome::AccessBlocked(format!(
                        "Keychain lookup failed: {}",
                        stderr
                    ))
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            MacosKeychainReadOutcome::AccessBlocked("Keychain authorization timed out".to_string())
        }
        Err(error) => MacosKeychainReadOutcome::AccessBlocked(format!(
            "Failed to execute security CLI: {}",
            error
        )),
    }
}

#[cfg(target_os = "macos")]
fn probe_macos_security_password(
    service: &str,
    account: Option<&str>,
) -> MacosKeychainReadOutcome {
    let mut cmd = Command::new("security");
    cmd.args(["find-generic-password", "-w", "-s", service]);
    if let Some(account_value) = account {
        cmd.args(["-a", account_value]);
    }
    let result =
        crate::modules::process_timeout::output_with_timeout(&mut cmd, MACOS_KEYCHAIN_PROMPT_TIMEOUT);
    classify_macos_security_result(result)
}

#[cfg(target_os = "macos")]
fn deduplicate_candidates(
    raw_candidates: Vec<(String, Option<String>)>,
) -> Vec<(String, Option<String>)> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(raw_candidates.len());
    for candidate in raw_candidates {
        if seen.insert(candidate.clone()) {
            deduped.push(candidate);
        }
    }
    deduped
}

#[cfg(target_os = "macos")]
fn build_macos_safe_storage_candidates(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Vec<(String, Option<String>)> {
    if matches!(mode, SafeStorageReadMode::AntigravityOnly) {
        return deduplicate_candidates(vec![
            (
                "Antigravity Safe Storage".to_string(),
                Some("Antigravity Key".to_string()),
            ),
            (
                "Antigravity Safe Storage".to_string(),
                Some("Antigravity Safe Storage".to_string()),
            ),
            ("Antigravity Safe Storage".to_string(), None),
        ]);
    }

    if matches!(mode, SafeStorageReadMode::CodeBuddyOnly) {
        return deduplicate_candidates(vec![
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
            (
                "CodeBuddy Safe Storage".to_string(),
                Some("CodeBuddy Safe Storage".to_string()),
            ),
            ("CodeBuddy Safe Storage".to_string(), None),
        ]);
    }

    if matches!(mode, SafeStorageReadMode::CodeBuddyCnOnly) {
        return deduplicate_candidates(vec![
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
            (
                "CodeBuddy CN Safe Storage".to_string(),
                Some("CodeBuddy CN Safe Storage".to_string()),
            ),
            ("CodeBuddy CN Safe Storage".to_string(), None),
        ]);
    }

    if matches!(mode, SafeStorageReadMode::QoderOnly) {
        return deduplicate_candidates(vec![
            ("Qoder Safe Storage".to_string(), Some("Qoder Key".to_string())),
            ("Qoder Safe Storage".to_string(), Some("Qoder".to_string())),
            (
                "Qoder App Safe Storage".to_string(),
                Some("Qoder App Key".to_string()),
            ),
            ("Qoder Safe Storage".to_string(), Some("qoder".to_string())),
            (
                "Qoder Safe Storage".to_string(),
                Some("Qoder Safe Storage".to_string()),
            ),
            ("Qoder Safe Storage".to_string(), None),
            ("Qoder App Safe Storage".to_string(), None),
        ]);
    }

    if matches!(mode, SafeStorageReadMode::WorkBuddyOnly) {
        return deduplicate_candidates(vec![
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
            (
                "WorkBuddy Safe Storage".to_string(),
                Some("WorkBuddy Safe Storage".to_string()),
            ),
            ("WorkBuddy Safe Storage".to_string(), None),
        ]);
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
    for app_name in app_names {
        let service = format!("{} Safe Storage", app_name);
        candidates.push((service.clone(), Some(app_name.clone())));
        candidates.push((service.clone(), Some(service.clone())));
        candidates.push((service, None));
    }

    deduplicate_candidates(candidates)
}

#[cfg(target_os = "macos")]
fn probe_macos_keychain_candidates_with<F>(
    candidates: &[(String, Option<String>)],
    mut probe_fn: F,
) -> Result<(String, String), (bool, String)>
where
    F: FnMut(&str, Option<&str>) -> MacosKeychainReadOutcome,
{
    let mut missing_services = HashSet::new();
    for (service, account) in candidates {
        if missing_services.contains(service) {
            continue;
        }
        match probe_fn(service, account.as_deref()) {
            MacosKeychainReadOutcome::Found(password) => {
                return Ok((password, service.clone()));
            }
            MacosKeychainReadOutcome::NotFound => {
                if account.is_none() {
                    missing_services.insert(service.clone());
                }
            }
            MacosKeychainReadOutcome::EmptyPassword => {}
            MacosKeychainReadOutcome::AccessBlocked(reason) => {
                return Err((true, reason));
            }
        }
    }
    Err((
        false,
        "Failed to read Safe Storage password from Keychain".to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn resolve_macos_safe_storage_password_with<F, N>(
    cache: &Mutex<HashMap<SafeStorageCacheKey, SafeStorageCacheEntry>>,
    probe_lock: &Mutex<()>,
    key: SafeStorageCacheKey,
    candidates: &[(String, Option<String>)],
    cooldown: Duration,
    now: N,
    probe_fn: F,
) -> Result<String, String>
where
    F: FnMut(&str, Option<&str>) -> MacosKeychainReadOutcome,
    N: Fn() -> Instant,
{
    {
        let guard = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.get(&key) {
            Some(SafeStorageCacheEntry::Password { password, .. }) => {
                return Ok(password.clone());
            }
            Some(SafeStorageCacheEntry::Cooldown { failed_at, reason })
                if now().saturating_duration_since(*failed_at) < cooldown =>
            {
                return Err(reason.clone());
            }
            _ => {}
        }
    }

    let _probe_guard = probe_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    {
        let guard = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.get(&key) {
            Some(SafeStorageCacheEntry::Password { password, .. }) => {
                return Ok(password.clone());
            }
            Some(SafeStorageCacheEntry::Cooldown { failed_at, reason })
                if now().saturating_duration_since(*failed_at) < cooldown =>
            {
                return Err(reason.clone());
            }
            _ => {}
        }
    }

    match probe_macos_keychain_candidates_with(candidates, probe_fn) {
        Ok((password, service)) => {
            let mut guard = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.insert(
                key,
                SafeStorageCacheEntry::Password {
                    password: password.clone(),
                    service,
                },
            );
            Ok(password)
        }
        Err((enter_cooldown, reason)) => {
            if enter_cooldown {
                let mut guard = cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.insert(
                    key,
                    SafeStorageCacheEntry::Cooldown {
                        failed_at: now(),
                        reason: reason.clone(),
                    },
                );
            }
            Err(reason)
        }
    }
}

#[cfg(target_os = "macos")]
fn get_macos_safe_storage_password_with_cooldown(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
    cooldown: Duration,
) -> Result<String, String> {
    let key = build_macos_safe_storage_cache_key(data_root, mode);
    let candidates = build_macos_safe_storage_candidates(data_root, mode);
    resolve_macos_safe_storage_password_with(
        &MACOS_SAFE_STORAGE_CACHE,
        &MACOS_KEYCHAIN_PROBE_LOCK,
        key,
        &candidates,
        cooldown,
        Instant::now,
        probe_macos_security_password,
    )
}

#[cfg(target_os = "macos")]
fn get_macos_safe_storage_password(
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<String, String> {
    get_macos_safe_storage_password_with_cooldown(
        data_root,
        mode,
        MACOS_KEYCHAIN_INTERACTIVE_COOLDOWN,
    )
}

#[cfg(target_os = "macos")]
fn try_decrypt_with_fallback_macos_services(
    encrypted: &[u8],
    prefix: &[u8],
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
    cooldown: Duration,
) -> Option<Vec<u8>> {
    let key = build_macos_safe_storage_cache_key(data_root, mode);
    let primary_service = {
        let guard = MACOS_SAFE_STORAGE_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.get(&key) {
            Some(SafeStorageCacheEntry::Password { service, .. }) => service.clone(),
            Some(SafeStorageCacheEntry::Cooldown { failed_at, .. })
                if Instant::now().saturating_duration_since(*failed_at) < cooldown =>
            {
                return None;
            }
            _ => return None,
        }
    };

    let candidates: Vec<(String, Option<String>)> =
        build_macos_safe_storage_candidates(data_root, mode)
            .into_iter()
            .filter(|(service, _)| service != &primary_service)
            .collect();
    if candidates.is_empty() {
        return None;
    }

    let _probe_guard = MACOS_KEYCHAIN_PROBE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut missing_services = HashSet::new();
    for (service, account) in candidates {
        if missing_services.contains(&service) {
            continue;
        }
        {
            let mut attempted = MACOS_FALLBACK_SERVICE_ATTEMPTS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !attempted.insert((key.clone(), format!("{}:{:?}", service, account))) {
                continue;
            }
        }
        match probe_macos_security_password(&service, account.as_deref()) {
            MacosKeychainReadOutcome::Found(password) => {
                let derived_1003 = pbkdf2_sha1_key(&password, 1003);
                if let Ok(plaintext) = decrypt_cbc_prefixed(encrypted, prefix, &derived_1003) {
                    let mut guard = MACOS_SAFE_STORAGE_CACHE
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.insert(
                        key,
                        SafeStorageCacheEntry::Password {
                            password,
                            service,
                        },
                    );
                    return Some(plaintext);
                }
                if prefix == V11_PREFIX {
                    let derived_1 = pbkdf2_sha1_key(&password, 1);
                    if let Ok(plaintext) = decrypt_cbc_prefixed(encrypted, prefix, &derived_1) {
                        let mut guard = MACOS_SAFE_STORAGE_CACHE
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        guard.insert(
                            key,
                            SafeStorageCacheEntry::Password {
                                password,
                                service,
                            },
                        );
                        return Some(plaintext);
                    }
                }
            }
            MacosKeychainReadOutcome::NotFound => {
                if account.is_none() {
                    missing_services.insert(service);
                }
            }
            MacosKeychainReadOutcome::EmptyPassword => {}
            MacosKeychainReadOutcome::AccessBlocked(reason) => {
                let mut guard = MACOS_SAFE_STORAGE_CACHE
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.insert(
                    key,
                    SafeStorageCacheEntry::Cooldown {
                        failed_at: Instant::now(),
                        reason,
                    },
                );
                return None;
            }
        }
    }

    None
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
    decrypt_secret_payload_with_mode_and_cooldown(encrypted, data_root, mode, false)
}

fn decrypt_secret_payload_with_mode_and_cooldown(
    encrypted: &[u8],
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
    passive_cooldown: bool,
) -> Result<Vec<u8>, String> {
    #[cfg(not(target_os = "windows"))]
    let _ = (data_root, mode);
    #[cfg(not(target_os = "macos"))]
    let _ = passive_cooldown;

    #[cfg(target_os = "windows")]
    {
        let _ = mode;
        let key = get_windows_encryption_key(data_root)?;
        return decrypt_windows_gcm_v10(&key, encrypted);
    }

    #[cfg(target_os = "macos")]
    {
        let prefix = match detect_prefix(encrypted) {
            Some("v10") => V10_PREFIX,
            Some("v11") => V11_PREFIX,
            _ => {
                return Err(format!(
                    "Unexpected ciphertext prefix: {:?}",
                    &encrypted[..encrypted.len().min(3)]
                ));
            }
        };
        let cooldown = if passive_cooldown {
            MACOS_KEYCHAIN_PASSIVE_COOLDOWN
        } else {
            MACOS_KEYCHAIN_INTERACTIVE_COOLDOWN
        };
        let password =
            get_macos_safe_storage_password_with_cooldown(data_root, mode, cooldown)?;
        let key = pbkdf2_sha1_key(&password, 1003);
        match decrypt_cbc_prefixed(encrypted, prefix, &key) {
            Ok(plaintext) => return Ok(plaintext),
            Err(primary_err) => {
                if prefix == V11_PREFIX {
                    let linux_style_key = pbkdf2_sha1_key(&password, 1);
                    if let Ok(plaintext) = decrypt_cbc_prefixed(encrypted, prefix, &linux_style_key)
                    {
                        return Ok(plaintext);
                    }
                }
                if let Some(plaintext) = try_decrypt_with_fallback_macos_services(
                    encrypted, prefix, data_root, mode, cooldown,
                ) {
                    return Ok(plaintext);
                }
                return Err(primary_err);
            }
        }
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
const GITHUB_COPILOT_SESSION_SCOPES: &[&str] = &["read:user", "user:email", "repo", "workflow"];

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
    match conn.query_row("SELECT value FROM ItemTable WHERE key = ?", [key], |row| {
        row.get(0)
    }) {
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
    .map_err(|e| {
        format!(
            "Failed to write github.auth to {}: {}",
            db_path.display(),
            e
        )
    })?;

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
        paths.push(crate::modules::vscode_paths::vscode_state_db_path(
            data_root,
        ));
        if let Some(shared_path) =
            crate::modules::vscode_paths::vscode_shared_storage_db_path(data_root)
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

fn try_decode_json_byte_array(arr: &[serde_json::Value]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(arr.len());
    for v in arr {
        let n = v.as_u64()?;
        if n > 255 {
            return None;
        }
        bytes.push(n as u8);
    }
    Some(bytes)
}

fn try_decode_prefixed_hex_ciphertext(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim();
    let hex_str = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex_str.len() < 38 || hex_str.len() % 2 != 0 {
        return None;
    }
    if !hex_str.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Vec::with_capacity(hex_str.len() / 2);
    for chunk in hex_str.as_bytes().chunks_exact(2) {
        let s = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(s, 16).ok()?);
    }
    detect_prefix(&out).map(|_| out)
}

fn try_decode_prefixed_base64_ciphertext(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim();
    if trimmed.len() < 24 {
        return None;
    }
    let bytes = general_purpose::STANDARD.decode(trimmed).ok()?;
    if bytes.len() < 19 {
        return None;
    }
    detect_prefix(&bytes).map(|_| bytes)
}

fn is_buffer_object_value(parsed: &serde_json::Value) -> bool {
    parsed.get("type").and_then(|value| value.as_str()) == Some("Buffer")
        || (parsed.get("type").is_none() && parsed.get("data").is_some_and(serde_json::Value::is_array))
}

fn decode_encrypted_bytes_to_utf8(
    encrypted_bytes: &[u8],
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
    passive_cooldown: bool,
) -> Result<String, String> {
    if detect_prefix(encrypted_bytes).is_none() {
        if let Ok(plain) = std::str::from_utf8(encrypted_bytes) {
            let trimmed = plain.trim();
            if (trimmed.starts_with('{') || trimmed.starts_with('['))
                && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
            {
                return Ok(plain.to_string());
            }
        }
    }
    let decrypted = decrypt_secret_payload_with_mode_and_cooldown(
        encrypted_bytes,
        data_root,
        mode,
        passive_cooldown,
    )?;
    String::from_utf8(decrypted).map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))
}

fn decode_secret_storage_value_with_mode(
    raw_value: &str,
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
) -> Result<String, String> {
    decode_secret_storage_value_with_mode_and_cooldown(raw_value, data_root, mode, false)
}

fn decode_secret_storage_value_with_mode_and_cooldown(
    raw_value: &str,
    data_root: Option<&Path>,
    mode: SafeStorageReadMode,
    passive_cooldown: bool,
) -> Result<String, String> {
    let parsed: serde_json::Value = match serde_json::from_str(raw_value) {
        Ok(value) => value,
        Err(_) => {
            if let Some(encrypted_bytes) = try_decode_prefixed_hex_ciphertext(raw_value)
                .or_else(|| try_decode_prefixed_base64_ciphertext(raw_value))
            {
                return decode_encrypted_bytes_to_utf8(
                    &encrypted_bytes,
                    data_root,
                    mode,
                    passive_cooldown,
                );
            }
            return Ok(raw_value.to_string());
        }
    };

    if is_buffer_object_value(&parsed) {
        let encrypted_bytes = decode_buffer_data(&parsed)?;
        return decode_encrypted_bytes_to_utf8(
            &encrypted_bytes,
            data_root,
            mode,
            passive_cooldown,
        );
    }

    if let Some(arr) = parsed.as_array() {
        if let Some(encrypted_bytes) =
            try_decode_json_byte_array(arr).filter(|bytes| detect_prefix(bytes).is_some())
        {
            return decode_encrypted_bytes_to_utf8(
                &encrypted_bytes,
                data_root,
                mode,
                passive_cooldown,
            );
        }
    }

    if let Some(value) = parsed.as_str() {
        let trimmed = value.trim();
        if trimmed.starts_with('{') {
            if let Ok(inner) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if is_buffer_object_value(&inner) {
                    let encrypted_bytes = decode_buffer_data(&inner)?;
                    return decode_encrypted_bytes_to_utf8(
                        &encrypted_bytes,
                        data_root,
                        mode,
                        passive_cooldown,
                    );
                }
            }
        }
        if let Some(encrypted_bytes) = try_decode_prefixed_hex_ciphertext(trimmed)
            .or_else(|| try_decode_prefixed_base64_ciphertext(trimmed))
        {
            return decode_encrypted_bytes_to_utf8(
                &encrypted_bytes,
                data_root,
                mode,
                passive_cooldown,
            );
        }
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
    if let Some(shared_path) =
        crate::modules::vscode_paths::vscode_shared_storage_db_path(data_root)
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

pub(crate) fn read_qoder_secret_snapshot<const N: usize>(
    db_path: &Path,
    keys: [&str; N],
) -> Result<[Option<String>; N], String> {
    read_qoder_secret_snapshot_with_cooldown(db_path, keys, false)
}

pub(crate) fn read_qoder_secret_snapshot_passive<const N: usize>(
    db_path: &Path,
    keys: [&str; N],
) -> Result<[Option<String>; N], String> {
    read_qoder_secret_snapshot_with_cooldown(db_path, keys, true)
}

fn read_qoder_secret_snapshot_with_cooldown<const N: usize>(
    db_path: &Path,
    keys: [&str; N],
    passive_cooldown: bool,
) -> Result<[Option<String>; N], String> {
    let mut values: [Option<String>; N] = std::array::from_fn(|_| None);
    if !db_path.try_exists().map_err(|error| error.to_string())? {
        return Ok(values);
    }
    let data_root = resolve_data_root_from_state_db_path(db_path)?;
    let mut conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(2)).map_err(|error| error.to_string())?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    for (key, value) in keys.into_iter().zip(values.iter_mut()) {
        *value = match tx.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| row.get(0)) {
            Ok(value) => Some(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => return Err(error.to_string()),
        };
    }
    tx.commit().map_err(|error| error.to_string())?;
    for value in values.iter_mut().flatten() {
        *value = decode_secret_storage_value_with_mode_and_cooldown(
            value,
            Some(data_root),
            SafeStorageReadMode::QoderOnly,
            passive_cooldown,
        )
        .map_err(|error| format!("QODER_STORAGE_UNAVAILABLE: {error}"))?;
    }
    Ok(values)
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
        "scopes": GITHUB_COPILOT_SESSION_SCOPES,
        "accessToken": token,
        "account": {
            "label": username,
            "id": user_id
        }
    });

    let mut replaced = false;
    for session in &mut sessions {
        if is_same_github_account_session(session, username, user_id)
            && github_session_scopes_match(session, GITHUB_COPILOT_SESSION_SCOPES)
        {
            *session = new_session.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        sessions.push(new_session);
    }

    Ok((serde_json::Value::Array(sessions), existing_prefix))
}

fn is_same_github_account_session(
    session: &serde_json::Value,
    username: &str,
    github_user_id: &str,
) -> bool {
    let account = &session["account"];
    account["label"].as_str() == Some(username) || account["id"].as_str() == Some(github_user_id)
}

fn github_session_scopes_match(session: &serde_json::Value, expected_scopes: &[&str]) -> bool {
    let Some(scopes) = session["scopes"].as_array() else {
        return false;
    };
    if scopes.len() != expected_scopes.len() {
        return false;
    }

    let mut actual = scopes
        .iter()
        .filter_map(|scope| scope.as_str())
        .collect::<Vec<_>>();
    if actual.len() != expected_scopes.len() {
        return false;
    }

    let mut expected = expected_scopes.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    actual == expected
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

pub(crate) fn qoder_storage_ready(data_root: &Path) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let path = data_root.join("Local State");
        match path.try_exists() {
            Ok(false) => return Ok(false),
            Err(error) => return Err(format!("QODER_STORAGE_UNAVAILABLE: {}", error)),
            Ok(true) => {}
        }
        match get_windows_encryption_key(Some(data_root)) {
            Ok(_) => return Ok(true),
            Err(error) if error == "Cannot find os_crypt.encrypted_key in Local State" => {
                return Ok(false);
            }
            Err(error) => return Err(format!("QODER_STORAGE_UNAVAILABLE: {}", error)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match get_macos_safe_storage_password(Some(data_root), SafeStorageReadMode::QoderOnly) {
            Ok(_) => Ok(true),
            Err(_) if !data_root.join("User/globalStorage/state.vscdb").is_file() => Ok(false),
            Err(error) => Err(format!("QODER_STORAGE_UNAVAILABLE: {error}")),
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = data_root;
        Ok(true)
    }
}

pub fn inject_qoder_secret_snapshot(
    db_path: &Path,
    entries: &[(&str, &str)],
) -> Result<(), String> {
    inject_secret_batch_with_mode(db_path, entries, SafeStorageReadMode::QoderOnly)
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
    inject_secret_batch_with_mode(db_path, &[(db_key, plaintext)], mode)
}

fn inject_secret_batch_with_mode(
    db_path: &Path,
    entries: &[(&str, &str)],
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

    let mut conn = rusqlite::Connection::open(db_path)
        .map_err(|e| format!("Failed to open state.vscdb: {}", e))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("Failed to init ItemTable: {}", e))?;

    let mut prepared = Vec::with_capacity(entries.len());
    // Finish keychain/encryption work before taking the SQLite write transaction.
    for &(db_key, plaintext) in entries {
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

        prepared.push((db_key.to_string(), buffer_str));
    }
    write_secret_snapshot(&mut conn, &prepared)
}

fn write_secret_snapshot(
    conn: &mut Connection,
    entries: &[(String, String)],
) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (key, value) in entries {
        tx.execute(
            "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn qoder_safe_storage_includes_native_and_legacy_keychain_items() {
        let candidates = build_macos_safe_storage_candidates(None, SafeStorageReadMode::QoderOnly);
        assert_eq!(
            candidates[0],
            ("Qoder Safe Storage".into(), Some("Qoder Key".into()))
        );
        assert!(candidates.contains(&("Qoder Safe Storage".into(), Some("Qoder".into()))));
        assert!(candidates.contains(&(
            "Qoder App Safe Storage".into(),
            Some("Qoder App Key".into())
        )));
        assert!(candidates.contains(&("Qoder Safe Storage".into(), None)));
        let unique: HashSet<_> = candidates.iter().collect();
        assert_eq!(unique.len(), candidates.len());
        assert!(!build_macos_safe_storage_candidates(None, SafeStorageReadMode::Default)
            .iter()
            .any(|(service, _)| service.starts_with("Qoder")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keychain_probe_caches_password_across_repeated_reads() {
        let cache = Mutex::new(HashMap::new());
        let probe_lock = Mutex::new(());
        let key = build_macos_safe_storage_cache_key(None, SafeStorageReadMode::QoderOnly);
        let candidates = build_macos_safe_storage_candidates(None, SafeStorageReadMode::QoderOnly);
        let mut calls = 0usize;

        let first = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key.clone(),
            &candidates,
            MACOS_KEYCHAIN_INTERACTIVE_COOLDOWN,
            Instant::now,
            |service, account| {
                calls += 1;
                if service == "Qoder Safe Storage" && account == Some("Qoder Key") {
                    MacosKeychainReadOutcome::Found("cached-secret".to_string())
                } else {
                    MacosKeychainReadOutcome::NotFound
                }
            },
        )
        .unwrap();
        assert_eq!(first, "cached-secret");
        assert_eq!(calls, 1);

        let second = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key,
            &candidates,
            MACOS_KEYCHAIN_INTERACTIVE_COOLDOWN,
            Instant::now,
            |_service, _account| {
                calls += 1;
                MacosKeychainReadOutcome::AccessBlocked("should not probe".to_string())
            },
        )
        .unwrap();
        assert_eq!(second, "cached-secret");
        assert_eq!(calls, 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keychain_probe_stops_on_access_blocked_and_enters_cooldown() {
        let cache = Mutex::new(HashMap::new());
        let probe_lock = Mutex::new(());
        let key = build_macos_safe_storage_cache_key(None, SafeStorageReadMode::QoderOnly);
        let candidates = build_macos_safe_storage_candidates(None, SafeStorageReadMode::QoderOnly);
        let mut probed = Vec::new();

        let err = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key.clone(),
            &candidates,
            Duration::from_secs(600),
            Instant::now,
            |service, account| {
                probed.push((service.to_string(), account.map(str::to_string)));
                if service == "Qoder Safe Storage" && account == Some("Qoder Key") {
                    MacosKeychainReadOutcome::AccessBlocked(
                        "Keychain access was canceled by user".to_string(),
                    )
                } else {
                    MacosKeychainReadOutcome::Found("should-never-reach".to_string())
                }
            },
        )
        .unwrap_err();
        assert!(err.contains("canceled"));
        assert_eq!(
            probed,
            vec![("Qoder Safe Storage".to_string(), Some("Qoder Key".to_string()))]
        );

        let err_during_cooldown = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key.clone(),
            &candidates,
            Duration::from_secs(600),
            Instant::now,
            |service, account| {
                probed.push((service.to_string(), account.map(str::to_string)));
                MacosKeychainReadOutcome::Found("should-not-probe".to_string())
            },
        )
        .unwrap_err();
        assert!(err_during_cooldown.contains("canceled"));
        assert_eq!(probed.len(), 1);

        let recovered_with_zero_cooldown = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key,
            &candidates,
            Duration::ZERO,
            Instant::now,
            |service, account| {
                probed.push((service.to_string(), account.map(str::to_string)));
                if service == "Qoder Safe Storage" && account == Some("Qoder Key") {
                    MacosKeychainReadOutcome::Found("recovered-secret".to_string())
                } else {
                    MacosKeychainReadOutcome::NotFound
                }
            },
        )
        .unwrap();
        assert_eq!(recovered_with_zero_cooldown, "recovered-secret");
        assert_eq!(probed.len(), 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keychain_probe_continues_on_not_found_without_long_cooldown() {
        let cache = Mutex::new(HashMap::new());
        let probe_lock = Mutex::new(());
        let key = build_macos_safe_storage_cache_key(None, SafeStorageReadMode::QoderOnly);
        let candidates = build_macos_safe_storage_candidates(None, SafeStorageReadMode::QoderOnly);
        let mut first_round = Vec::new();

        let err = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key.clone(),
            &candidates,
            Duration::from_secs(600),
            Instant::now,
            |service, account| {
                first_round.push((service.to_string(), account.map(str::to_string)));
                MacosKeychainReadOutcome::NotFound
            },
        )
        .unwrap_err();
        assert!(err.contains("Failed to read Safe Storage password"));
        assert!(first_round.contains(&("Qoder Safe Storage".to_string(), None)));

        let skip_after_none_candidates = vec![
            ("Test Safe Storage".to_string(), None),
            ("Test Safe Storage".to_string(), Some("Unused".to_string())),
        ];
        let mut skipped_round = Vec::new();
        let _ = probe_macos_keychain_candidates_with(
            &skip_after_none_candidates,
            |service, account| {
                skipped_round.push((service.to_string(), account.map(str::to_string)));
                MacosKeychainReadOutcome::NotFound
            },
        );
        assert_eq!(
            skipped_round,
            vec![("Test Safe Storage".to_string(), None)]
        );

        let ok = resolve_macos_safe_storage_password_with(
            &cache,
            &probe_lock,
            key,
            &candidates,
            Duration::from_secs(600),
            Instant::now,
            |service, account| {
                if service == "Qoder Safe Storage" && account == Some("Qoder Key") {
                    MacosKeychainReadOutcome::Found("created-secret".to_string())
                } else {
                    MacosKeychainReadOutcome::NotFound
                }
            },
        )
        .unwrap();
        assert_eq!(ok, "created-secret");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_security_output_classification_distinguishes_not_found_and_blocked() {
        use std::os::unix::process::ExitStatusExt;

        let ok_output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"base64pass==\n".to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_macos_security_result(Ok(ok_output)),
            MacosKeychainReadOutcome::Found("base64pass==".to_string())
        );

        let not_found_output = std::process::Output {
            status: std::process::ExitStatus::from_raw(44 << 8),
            stdout: Vec::new(),
            stderr: b"security: SecKeychainSearchCopyNext: The specified item could not be found in the keychain.\n".to_vec(),
        };
        assert_eq!(
            classify_macos_security_result(Ok(not_found_output)),
            MacosKeychainReadOutcome::NotFound
        );

        let canceled_output = std::process::Output {
            status: std::process::ExitStatus::from_raw(128 << 8),
            stdout: Vec::new(),
            stderr: b"User canceled the operation.\n".to_vec(),
        };
        assert!(matches!(
            classify_macos_security_result(Ok(canceled_output)),
            MacosKeychainReadOutcome::AccessBlocked(_)
        ));

        let timed_out = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        assert!(matches!(
            classify_macos_security_result(Err(timed_out)),
            MacosKeychainReadOutcome::AccessBlocked(_)
        ));
    }

    #[test]
    fn decode_secret_storage_value_handles_plaintext_json_with_data_and_unencrypted_buffer() {
        let plain_api_json = r#"{"code":0,"data":{"used":12,"total":100}}"#;
        assert_eq!(
            decode_secret_storage_value_with_mode(
                plain_api_json,
                None,
                SafeStorageReadMode::QoderOnly
            )
            .unwrap(),
            plain_api_json
        );

        let plain_inner = r#"{"token":"qoder-tok","email":"user@example.com"}"#;
        let buffer_json = serde_json::json!({
            "type": "Buffer",
            "data": plain_inner.as_bytes()
        })
        .to_string();
        assert_eq!(
            decode_secret_storage_value_with_mode(
                &buffer_json,
                None,
                SafeStorageReadMode::QoderOnly
            )
            .unwrap(),
            plain_inner
        );

        assert!(try_decode_prefixed_hex_ciphertext("deadbeef").is_none());
        assert!(try_decode_prefixed_base64_ciphertext("eyJ0b2tlbiI6Inh5eiJ9").is_none());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn cbc_v10_round_trip_matches_chromium_safe_storage_parameters() {
        let key = pbkdf2_sha1_key("sample-keychain-password==", 1003);
        let plaintext = br#"{"token":"abc","email":"qoder@example.com"}"#;
        let encrypted = encrypt_cbc_prefixed(V10_PREFIX, &key, plaintext).unwrap();
        assert!(encrypted.starts_with(V10_PREFIX));
        let decrypted = decrypt_cbc_prefixed(&encrypted, V10_PREFIX, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn qoder_secret_snapshot_rolls_back_all_keys_on_failure() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value TEXT); INSERT INTO ItemTable VALUES('identity','old'); INSERT INTO ItemTable VALUES('enterprise','old'); CREATE TRIGGER reject_quota BEFORE INSERT ON ItemTable WHEN NEW.key = 'credit' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        let fields = [
            ("identity".into(), "new".into()),
            ("enterprise".into(), "null".into()),
            ("credit".into(), "new-quota".into()),
        ];
        assert!(write_secret_snapshot(&mut conn, &fields).is_err());
        let old: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'identity'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old, "old");
        let old: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'enterprise'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old, "old");
        conn.execute_batch("DROP TRIGGER reject_quota").unwrap();
        write_secret_snapshot(&mut conn, &fields).unwrap();
        let new: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'identity'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new, "new");
    }

    fn sessions_array(value: serde_json::Value) -> Vec<serde_json::Value> {
        value.as_array().cloned().unwrap_or_default()
    }

    #[test]
    fn github_auth_sessions_use_exact_copilot_scopes() {
        let (sessions, _) =
            build_github_auth_sessions(None, None, "octocat", "ghu_new", Some("123")).unwrap();
        let sessions = sessions_array(sessions);

        assert_eq!(sessions.len(), 1);
        assert!(github_session_scopes_match(
            &sessions[0],
            GITHUB_COPILOT_SESSION_SCOPES
        ));
        assert_eq!(sessions[0]["accessToken"].as_str(), Some("ghu_new"));
    }

    #[test]
    fn github_auth_sessions_preserve_plain_github_session_and_add_copilot_session() {
        let existing = serde_json::json!([
            {
                "id": "plain-session",
                "scopes": ["read:user", "user:email"],
                "accessToken": "ghu_plain",
                "account": { "label": "octocat", "id": "123" }
            }
        ])
        .to_string();

        let (sessions, _) = build_github_auth_sessions(
            Some(&existing),
            None,
            "octocat",
            "ghu_copilot",
            Some("123"),
        )
        .unwrap();
        let sessions = sessions_array(sessions);

        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .any(|session| session["id"].as_str() == Some("plain-session")
                && session["accessToken"].as_str() == Some("ghu_plain")));
        assert!(sessions.iter().any(|session| github_session_scopes_match(
            session,
            GITHUB_COPILOT_SESSION_SCOPES
        ) && session["accessToken"].as_str()
            == Some("ghu_copilot")));
    }

    #[test]
    fn github_auth_sessions_replace_existing_copilot_session_for_same_account() {
        let existing = serde_json::json!([
            {
                "id": "old-copilot-session",
                "scopes": ["workflow", "repo", "user:email", "read:user"],
                "accessToken": "ghu_old",
                "account": { "label": "octocat", "id": "123" }
            }
        ])
        .to_string();

        let (sessions, _) =
            build_github_auth_sessions(Some(&existing), None, "octocat", "ghu_new", Some("123"))
                .unwrap();
        let sessions = sessions_array(sessions);

        assert_eq!(sessions.len(), 1);
        assert!(github_session_scopes_match(
            &sessions[0],
            GITHUB_COPILOT_SESSION_SCOPES
        ));
        assert_eq!(sessions[0]["accessToken"].as_str(), Some("ghu_new"));
    }
}
