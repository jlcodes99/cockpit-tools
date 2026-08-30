//! 只读的账号详情文件读取器（支持 AES-256-GCM 信封与历史明文）。
//!
//! GUI 侧（src-tauri secure_account_storage）是加密读写的权威实现；本模块只做
//! 只读镜像，供容量快照等 CLI 场景读取账号数据：
//! - 绝不创建密钥、绝不写回/迁移文件；
//! - 解密只发生在进程内存中，调用方不得把明文写入日志或输出。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};
use serde::de::DeserializeOwned;
use std::path::Path;

const KEY_FILE: &str = "secure-account-storage.key";
const ENVELOPE_VERSION: u32 = 1;
const ALGORITHM: &str = "AES-256-GCM";

#[derive(Debug, Clone, serde::Deserialize)]
struct SecureAccountEnvelope {
    version: u32,
    #[allow(dead_code)]
    kind: String,
    algorithm: String,
    #[allow(dead_code)]
    key_id: String,
    nonce: String,
    ciphertext: String,
    #[allow(dead_code)]
    encrypted_at: i64,
}

fn key_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::modules::config::get_data_dir()?.join(KEY_FILE))
}

/// 只读取密钥；密钥不存在时直接报错，绝不创建新密钥。
fn read_key() -> Result<[u8; 32], String> {
    let path = key_path()?;
    let raw =
        std::fs::read_to_string(&path).map_err(|_| "账号详情加密密钥不存在或不可读".to_string())?;
    let bytes = general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| "账号详情加密密钥格式无效".to_string())?;
    if bytes.len() != 32 {
        return Err("账号详情加密密钥长度无效".to_string());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn looks_like_envelope(content: &str) -> bool {
    content.trim_start().starts_with('{') && content.contains("\"ciphertext\"")
}

/// 读取单个账号详情：优先按 AES-256-GCM 信封解密，否则按历史明文 JSON 解析。
pub fn read_account_detail<T: DeserializeOwned>(_path: &Path, content: &str) -> Result<T, String> {
    if looks_like_envelope(content) {
        let envelope: SecureAccountEnvelope =
            serde_json::from_str(content).map_err(|_| "账号详情信封格式无效".to_string())?;
        if envelope.algorithm != ALGORITHM || envelope.version != ENVELOPE_VERSION {
            return Err("账号详情加密版本不受支持".to_string());
        }
        let nonce = general_purpose::STANDARD
            .decode(envelope.nonce.trim())
            .map_err(|_| "账号详情 nonce 无效".to_string())?;
        if nonce.len() != 12 {
            return Err("账号详情 nonce 长度无效".to_string());
        }
        let ciphertext = general_purpose::STANDARD
            .decode(envelope.ciphertext.trim())
            .map_err(|_| "账号详情密文无效".to_string())?;
        let cipher = Aes256Gcm::new_from_slice(&read_key()?)
            .map_err(|_| "初始化账号详情解密失败".to_string())?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| "解密账号详情失败（密钥可能不匹配）".to_string())?;
        return serde_json::from_slice::<T>(&plaintext)
            .map_err(|e| format!("解析账号详情明文失败: {}", e));
    }

    serde_json::from_str(content).map_err(|e| format!("解析账号详情失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct DemoAccount {
        id: String,
        secret: String,
    }

    #[test]
    fn decrypts_envelope_and_reads_legacy_plaintext_without_writes() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!(
            "cockpit-secure-reader-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        // 用与 GUI 相同的信封格式手工构造密文（测试自持，不依赖真实密钥）
        let account = DemoAccount {
            id: "a1".to_string(),
            secret: "token-secret-value".to_string(),
        };
        let plaintext = serde_json::to_vec(&account).unwrap();
        let key_bytes: [u8; 32] = core::array::from_fn(|i| i as u8);
        let nonce_bytes: [u8; 12] = core::array::from_fn(|i| (i * 3) as u8);
        let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .unwrap();
        let envelope = serde_json::json!({
            "version": 1,
            "kind": "demo",
            "algorithm": "AES-256-GCM",
            "key_id": "local-secure-account-storage-v1",
            "nonce": general_purpose::STANDARD.encode(nonce_bytes),
            "ciphertext": general_purpose::STANDARD.encode(ciphertext),
            "encrypted_at": 0,
        })
        .to_string();

        // 密钥文件就位后可解密
        std::fs::write(
            dir.join(KEY_FILE),
            general_purpose::STANDARD.encode(key_bytes),
        )
        .unwrap();
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &dir);
        let decoded: DemoAccount =
            read_account_detail(Path::new("unused"), &envelope).expect("decrypt");
        assert_eq!(decoded, account);

        // 历史明文也能读取
        let plain = serde_json::to_string_pretty(&account).unwrap();
        let legacy: DemoAccount = read_account_detail(Path::new("unused"), &plain).expect("legacy");
        assert_eq!(legacy, account);

        // 缺少密钥时不创建、只报错
        std::fs::remove_file(dir.join(KEY_FILE)).unwrap();
        let missing_key: Result<DemoAccount, String> =
            read_account_detail(Path::new("unused"), &envelope);
        assert!(missing_key.is_err());

        std::env::remove_var("COCKPIT_TOOLS_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.join(KEY_FILE).exists());
    }
}
