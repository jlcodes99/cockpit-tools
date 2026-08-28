//! AES-256-GCM envelopes for provider account detail files (#1104).
//!
//! Index/summary files stay plaintext; only per-account detail JSON is encrypted
//! at rest. Legacy plaintext files still load and are rewritten encrypted on the
//! next save (or when rotation is due).

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const KEY_FILE: &str = "secure-account-storage.key";
const VERSION: u32 = 1;
const ROTATION_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecureAccountEnvelope {
    version: u32,
    kind: String,
    algorithm: String,
    key_id: String,
    nonce: String,
    ciphertext: String,
    encrypted_at: i64,
}

fn key_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(KEY_FILE))
}

fn read_or_create_key() -> Result<[u8; 32], String> {
    let path = key_path()?;
    if path.exists() {
        let raw =
            fs::read_to_string(&path).map_err(|e| format!("读取账号详情加密密钥失败: {}", e))?;
        let bytes = general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|e| format!("解析账号详情加密密钥失败: {}", e))?;
        if bytes.len() != 32 {
            return Err("账号详情加密密钥长度无效".to_string());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(key);
    }

    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let encoded = general_purpose::STANDARD.encode(key);
    crate::modules::atomic_write::write_string_atomic(&path, &encoded)
        .map_err(|e| format!("写入账号详情加密密钥失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

fn cipher() -> Result<Aes256Gcm, String> {
    Aes256Gcm::new_from_slice(&read_or_create_key()?)
        .map_err(|e| format!("初始化账号详情加密失败: {}", e))
}

pub fn serialize_account_file<T: Serialize>(kind: &str, account: &T) -> Result<String, String> {
    let plaintext =
        serde_json::to_vec(account).map_err(|e| format!("序列化账号详情失败: {}", e))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher()?
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
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

/// Returns `(account, needs_rewrite)` where `needs_rewrite` is true for legacy
/// plaintext files or envelopes past the rotation window.
pub fn deserialize_account_file<T: DeserializeOwned>(
    path: &Path,
    content: &str,
) -> Result<(T, bool), String> {
    if let Ok(envelope) = serde_json::from_str::<SecureAccountEnvelope>(content) {
        if envelope.version != VERSION {
            return Err("账号详情加密版本不受支持".to_string());
        }
        let nonce = general_purpose::STANDARD
            .decode(envelope.nonce.trim())
            .map_err(|e| format!("解析账号详情 nonce 失败: {}", e))?;
        if nonce.len() != 12 {
            return Err("账号详情 nonce 长度无效".to_string());
        }
        let ciphertext = general_purpose::STANDARD
            .decode(envelope.ciphertext.trim())
            .map_err(|e| format!("解析账号详情密文失败: {}", e))?;
        let plaintext = cipher()?
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|e| format!("解密账号详情失败: {:?}", e))?;
        let value = serde_json::from_slice::<T>(&plaintext)
            .map_err(|e| format!("解析账号详情明文失败: {}", e))?;
        let needs_rotation =
            chrono::Utc::now().timestamp() - envelope.encrypted_at > ROTATION_SECONDS;
        return Ok((value, needs_rotation));
    }

    let value = crate::modules::atomic_write::parse_json_with_auto_restore::<T>(path, content)
        .map_err(|e| format!("解析账号详情失败: {}", e))?;
    Ok((value, true))
}

/// Decodes only an AES-GCM envelope. Transfer imports must never accept a
/// plaintext credential payload, even though ordinary persisted files retain a
/// legacy migration path through `deserialize_account_file`.
pub fn deserialize_encrypted_account_file<T: DeserializeOwned>(
    expected_kind: &str,
    content: &str,
) -> Result<T, String> {
    let envelope = serde_json::from_str::<SecureAccountEnvelope>(content)
        .map_err(|_| "encrypted account transfer required".to_string())?;
    if envelope.version != VERSION
        || envelope.algorithm != "AES-256-GCM"
        || envelope.kind != expected_kind
    {
        return Err("unsupported encrypted account transfer".to_string());
    }
    let nonce = general_purpose::STANDARD
        .decode(envelope.nonce.trim())
        .map_err(|e| format!("解析账号详情 nonce 失败: {}", e))?;
    if nonce.len() != 12 {
        return Err("账号详情 nonce 长度无效".to_string());
    }
    let ciphertext = general_purpose::STANDARD
        .decode(envelope.ciphertext.trim())
        .map_err(|e| format!("解析账号详情密文失败: {}", e))?;
    let plaintext = cipher()?
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| "encrypted account transfer cannot be opened on this device".to_string())?;
    serde_json::from_slice::<T>(&plaintext)
        .map_err(|e| format!("解析账号详情明文失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct DemoAccount {
        id: String,
        secret: String,
    }

    #[test]
    fn roundtrip_encrypt_decrypt_and_legacy_plaintext() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!(
            "secure-account-storage-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let account = DemoAccount {
            id: "a1".into(),
            secret: "tok-secret".into(),
        };
        let path = dir.join("a1.json");
        let enc = serialize_account_file("demo", &account).expect("encrypt");
        assert!(enc.contains("AES-256-GCM"));
        assert!(!enc.contains("tok-secret"));
        fs::write(&path, &enc).expect("write");

        let (decoded, needs_rotation) =
            deserialize_account_file::<DemoAccount>(&path, &enc).expect("decrypt");
        assert_eq!(decoded, account);
        assert!(!needs_rotation);

        let plain = serde_json::to_string_pretty(&account).unwrap();
        let (legacy, needs_migration) =
            deserialize_account_file::<DemoAccount>(&path, &plain).expect("legacy");
        assert_eq!(legacy, account);
        assert!(needs_migration);

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
