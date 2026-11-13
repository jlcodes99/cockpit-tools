//! AES-256-GCM envelopes for provider account detail files (#1104).
//!
//! Index/summary files stay plaintext; only per-account detail JSON is encrypted
//! at rest. Legacy plaintext files still load and are rewritten encrypted on the
//! next save (or when rotation is due).

use aes_gcm::{Aes256Gcm, KeyInit};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::account_file_crypto::{
    decrypt_envelope, lock_account_file_for_write, read_key, read_or_create_key_at, serialize_with_cipher, SecureAccountEnvelope,
    KEY_FILE, ROTATION_SECONDS,
};

fn key_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(KEY_FILE))
}

fn read_or_create_key() -> Result<[u8; 32], String> {
    let path = key_path()?;
    read_or_create_key_at(&path)
}

fn cipher() -> Result<Aes256Gcm, String> {
    Aes256Gcm::new_from_slice(&read_or_create_key()?)
        .map_err(|e| format!("初始化账号详情加密失败: {}", e))
}

/// Recovery must not create a missing key, rotate credentials or restore a
/// backup over the source file. The caller supplies the key belonging to the
/// account directory, so background work cannot drift to another profile.
pub(crate) fn read_account_file_readonly<T: DeserializeOwned>(
    path: &Path,
    key_path: &Path,
) -> Result<T, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("读取账号详情失败: {}", e))?;
    if let Ok(envelope) = serde_json::from_str::<SecureAccountEnvelope>(&content) {
        let cipher = Aes256Gcm::new_from_slice(&read_key(key_path)?)
            .map_err(|e| format!("初始化账号详情加密失败: {}", e))?;
        return decrypt_envelope(&envelope, cipher);
    }
    serde_json::from_str(&content).map_err(|e| format!("解析账号详情失败: {}", e))
}

pub fn serialize_account_file<T: Serialize>(kind: &str, account: &T) -> Result<String, String> {
    let plaintext =
        serde_json::to_vec(account).map_err(|e| format!("序列化账号详情失败: {}", e))?;
    serialize_with_cipher(kind, &plaintext, cipher()?)
}

pub(crate) struct LocalVaultCipher([u8; 32]);

impl LocalVaultCipher {
    pub(crate) fn open(root: &Path, initialize: bool) -> Result<Self, String> {
        let path = local_vault_key_path(root);
        let key = if initialize {
            read_or_create_key_at(&path)?
        } else {
            read_key(&path)?
        };
        Ok(Self(key))
    }

    fn cipher(&self, kind: &str) -> Result<Aes256Gcm, String> {
        Aes256Gcm::new_from_slice(&self.fingerprint(b"cockpit-local-vault-encryption:", kind.as_bytes()))
            .map_err(|_| "无法初始化本地密钥库加密".to_string())
    }

    pub(crate) fn seal<T: Serialize>(&self, kind: &str, value: &T) -> Result<String, String> {
        let bytes = serde_json::to_vec(value).map_err(|_| "序列化本地密钥库失败".to_string())?;
        serialize_with_cipher(kind, &bytes, self.cipher(kind)?)
    }

    pub(crate) fn unseal<T: DeserializeOwned>(&self, kind: &str, value: &str) -> Result<T, String> {
        let envelope: SecureAccountEnvelope =
            serde_json::from_str(value).map_err(|_| "本地密钥库格式无效".to_string())?;
        if envelope.kind != kind {
            return Err("本地密钥库类型无效".into());
        }
        decrypt_envelope(&envelope, self.cipher(kind)?)
    }

    pub(crate) fn fingerprint(&self, domain: &[u8], value: &[u8]) -> [u8; 32] {
        use ring::hmac::{Context, Key, HMAC_SHA256};
        let key = Key::new(HMAC_SHA256, &self.0);
        let mut context = Context::with_key(&key);
        context.update(domain);
        context.update(value);
        let mut fingerprint = [0; 32];
        fingerprint.copy_from_slice(context.sign().as_ref());
        fingerprint
    }
}

pub(crate) fn local_vault_key_path(root: &Path) -> std::path::PathBuf {
    root.join(KEY_FILE)
}

/// Refresh and import snapshots may predate a completed switch. Merge only this
/// monotonic metadata under the ordinary account-file write lock; credentials
/// and quota fields keep their existing ownership and replacement semantics.
pub(crate) fn write_account_preserving_switch_time<T: Serialize>(
    path: &Path,
    kind: &str,
    account: &T,
) -> Result<(), String> {
    let _account_lock = lock_account_file_for_write(path)?;
    let mut next = serde_json::to_value(account).map_err(|error| error.to_string())?;
    crate::modules::atomic_write::update_string_atomic(path, |current| {
        if let Some(current) = current {
            let current = current.strip_prefix('\u{feff}').unwrap_or(current);
            let envelope: serde_json::Value = serde_json::from_str(current)
                .map_err(|error| format!("读取账号切号记录失败: {error}"))?;
            let previous = if envelope.get("ciphertext").is_some() {
                deserialize_encrypted_snapshot::<serde_json::Value>(current)?
            } else {
                envelope
            };
            if previous.get("id") == next.get("id") {
                let timestamp = previous.get("last_switched_at").and_then(serde_json::Value::as_i64)
                    .max(next.get("last_switched_at").and_then(serde_json::Value::as_i64));
                if let Some(timestamp) = timestamp {
                    next["last_switched_at"] = timestamp.into();
                }
            }
        }
        serialize_account_file(kind, &next)
    })
}

/// Decode an immutable encrypted snapshot without restoring backups, rotating
/// storage, or creating a replacement key when the original key is missing.
pub(crate) fn deserialize_encrypted_snapshot<T: DeserializeOwned>(
    content: &str,
) -> Result<T, String> {
    deserialize_encrypted_snapshot_with_rotation(content).map(|(value, _)| value)
}

/// Decode without restoring files or creating a new key. Callers can validate a
/// backup completely before deciding whether it is safe to restore.
pub(crate) fn deserialize_encrypted_snapshot_with_rotation<T: DeserializeOwned>(
    content: &str,
) -> Result<(T, bool), String> {
    let envelope: SecureAccountEnvelope = serde_json::from_str(content)
        .map_err(|error| format!("Invalid encrypted account snapshot: {error}"))?;
    let cipher = Aes256Gcm::new_from_slice(&read_key(&key_path()?)?)
        .map_err(|error| format!("Cannot initialize snapshot decryption: {error}"))?;
    let value = decrypt_envelope(&envelope, cipher)?;
    let needs_rotation = chrono::Utc::now().timestamp() - envelope.encrypted_at > ROTATION_SECONDS;
    Ok((value, needs_rotation))
}

/// Returns `(account, needs_rewrite)` where `needs_rewrite` is true for legacy
/// plaintext files or envelopes past the rotation window.
pub fn deserialize_account_file<T: DeserializeOwned>(
    path: &Path,
    content: &str,
) -> Result<(T, bool), String> {
    if let Ok(envelope) = serde_json::from_str::<SecureAccountEnvelope>(content) {
        let cipher = Aes256Gcm::new_from_slice(&read_key(&key_path()?)?)
            .map_err(|error| format!("初始化账号详情解密失败: {error}"))?;
        let value = decrypt_envelope(&envelope, cipher)?;
        let needs_rotation =
            chrono::Utc::now().timestamp() - envelope.encrypted_at > ROTATION_SECONDS;
        return Ok((value, needs_rotation));
    }

    let value = crate::modules::atomic_write::parse_json_with_auto_restore::<T>(path, content)
        .map_err(|e| format!("解析账号详情失败: {}", e))?;
    Ok((value, true))
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
    fn concurrent_first_key_creation_never_replaces_the_winner() {
        let dir = std::env::temp_dir().join(format!("storage-key-race-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(KEY_FILE);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    read_or_create_key_at(&path)
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let key = read_key(&path).unwrap();
        assert!(results.iter().any(Result::is_ok));
        for result in results {
            match result {
                Ok(created) => assert_eq!(created, key),
                Err(error) => assert_eq!(error, "账号详情加密密钥正在初始化，请重试"),
            }
        }
        assert_eq!(read_or_create_key_at(&path).unwrap(), key);
        fs::remove_dir_all(dir).unwrap();
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

        let switched = serde_json::json!({"id":"a1", "last_switched_at": 200, "secret":"new"});
        write_account_preserving_switch_time(&path, "demo", &switched).unwrap();
        let stale = serde_json::json!({"id":"a1", "last_switched_at":100, "secret":"refreshed"});
        write_account_preserving_switch_time(&path, "demo", &stale).unwrap();
        let saved: serde_json::Value = deserialize_encrypted_snapshot(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["last_switched_at"], 200);
        assert_eq!(saved["secret"], "refreshed");
        write_account_preserving_switch_time(&path, "demo", &serde_json::json!({"id":"a1", "secret":"imported"})).unwrap();
        let saved: serde_json::Value = deserialize_encrypted_snapshot(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["last_switched_at"], 200);

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
