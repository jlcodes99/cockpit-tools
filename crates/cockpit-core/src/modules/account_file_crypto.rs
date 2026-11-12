//! Shared AES-256-GCM account detail envelope and local key handling.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) const KEY_FILE: &str = "secure-account-storage.key";
pub(crate) const LEGACY_TOKEN_KEY_FILE: &str = "account-token.key";
const VERSION: u32 = 1;
pub(crate) const ROTATION_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SecureAccountEnvelope {
    version: u32,
    pub(crate) kind: String,
    algorithm: String,
    key_id: String,
    nonce: String,
    ciphertext: String,
    pub(crate) encrypted_at: i64,
}

pub(crate) fn restore_legacy_antigravity_token(
    root: &Path,
    value: &mut serde_json::Value,
) -> Result<(), String> {
    #[derive(Deserialize)]
    struct LegacyTokenEnvelope {
        version: u32,
        nonce: String,
        ciphertext: String,
    }

    let account = value.as_object_mut().ok_or("账号数据格式无效")?;
    let encrypted = account.remove("token_encrypted");
    if account.contains_key("token") {
        return Ok(());
    }
    let legacy: LegacyTokenEnvelope =
        serde_json::from_value(encrypted.ok_or("账号数据缺少 token")?)
            .map_err(|error| format!("解析历史账号 token 密文失败: {error}"))?;
    if legacy.version != 1 {
        return Err("历史账号 token 加密版本不受支持".into());
    }
    let cipher = Aes256Gcm::new_from_slice(&read_key(&root.join(LEGACY_TOKEN_KEY_FILE))?)
        .map_err(|error| format!("初始化历史账号加密失败: {error}"))?;
    let token = decrypt_payload(cipher, &legacy.nonce, &legacy.ciphertext)?;
    account.insert("token".into(), token);
    Ok(())
}

pub(crate) fn lock_account_file(path: &Path) -> Result<fs::File, String> {
    acquire_account_file_lock(path, Duration::ZERO)
}

pub(crate) fn lock_account_file_for_write(path: &Path) -> Result<fs::File, String> {
    // A foreground edit may arrive just after a background replacement starts.
    // Bound the wait so a slow filesystem never turns contention into a hang.
    acquire_account_file_lock(path, Duration::from_millis(40))
}

fn acquire_account_file_lock(path: &Path, budget: Duration) -> Result<fs::File, String> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| format!("打开账号详情写入锁失败: {error}"))?;
    let deadline = Instant::now() + budget;
    let contended = fs2::lock_contended_error().raw_os_error();
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(error) if error.raw_os_error() == contended && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(format!("获取账号详情写入锁失败: {error}")),
        }
    }
}

pub(crate) fn read_key(path: &Path) -> Result<[u8; 32], String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取账号详情加密密钥失败: {}", e))?;
    let bytes = general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|e| format!("解析账号详情加密密钥失败: {}", e))?;
    bytes
        .try_into()
        .map_err(|_| "账号详情加密密钥长度无效".to_string())
}

pub(crate) fn read_or_create_key_at(path: &Path) -> Result<[u8; 32], String> {
    if path.exists() {
        return read_key(path);
    }
    // Account saves and zero-account proxy imports share this key. Serialize
    // first creation across threads/processes without blocking the caller.
    let lock_path = path.with_extension("key.lock");
    if lock_path.is_symlink() {
        return Err("账号详情加密密钥锁无效".into());
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("创建账号详情加密密钥锁失败: {}", e))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| "账号详情加密密钥正在初始化，请重试".to_string())?;
    if path.exists() {
        return read_key(path);
    }

    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let encoded = general_purpose::STANDARD.encode(key);
    crate::modules::atomic_write::write_secret_string_atomic(path, &encoded)
        .map_err(|e| format!("写入账号详情加密密钥失败: {}", e))?;
    Ok(key)
}

pub(crate) fn decrypt_envelope<T: DeserializeOwned>(
    envelope: &SecureAccountEnvelope,
    cipher: Aes256Gcm,
) -> Result<T, String> {
    if envelope.version != VERSION || envelope.algorithm != "AES-256-GCM" {
        return Err("账号详情加密版本不受支持".to_string());
    }
    decrypt_payload(cipher, &envelope.nonce, &envelope.ciphertext)
}

fn decrypt_payload<T: DeserializeOwned>(
    cipher: Aes256Gcm,
    nonce: &str,
    ciphertext: &str,
) -> Result<T, String> {
    let nonce = general_purpose::STANDARD
        .decode(nonce.trim())
        .map_err(|e| format!("解析账号详情 nonce 失败: {}", e))?;
    if nonce.len() != 12 {
        return Err("账号详情 nonce 长度无效".to_string());
    }
    let ciphertext = general_purpose::STANDARD
        .decode(ciphertext.trim())
        .map_err(|e| format!("解析账号详情密文失败: {}", e))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|e| format!("解密账号详情失败: {:?}", e))?;
    serde_json::from_slice(&plaintext).map_err(|e| format!("解析账号详情明文失败: {}", e))
}

pub(crate) fn serialize_with_cipher(
    kind: &str,
    plaintext: &[u8],
    cipher: Aes256Gcm,
) -> Result<String, String> {
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| format!("加密账号详情失败: {:?}", e))?;
    let envelope = SecureAccountEnvelope {
        version: VERSION,
        kind: kind.to_string(),
        algorithm: "AES-256-GCM".to_string(),
        key_id: "local-secure-account-storage-v1".to_string(),
        nonce: general_purpose::STANDARD.encode(nonce_bytes),
        ciphertext: general_purpose::STANDARD.encode(ciphertext),
        encrypted_at: chrono::Utc::now().timestamp(),
    };
    serde_json::to_string_pretty(&envelope).map_err(|e| format!("序列化账号详情密文失败: {}", e))
}
