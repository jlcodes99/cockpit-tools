//! ZCode 凭证读取与解密。
//!
//! 逻辑移植自 zcode-switch（zcrypto.rs / store.rs）：读取 ZCode 桌面端
//! `~/.zcode/v2/credentials.json`（支持 `enc:v1:` AES-256-GCM 加密值）
//! 以及 zcode-switch 的账号快照 `~/.zcode-switch/accounts/*.json`。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::modules::logger;

pub const ENC_PREFIX: &str = "enc:v1:";

pub fn node_platform_for(os: &str) -> &'static str {
    match os {
        "windows" => "win32",
        "macos" => "darwin",
        _ => "linux",
    }
}

fn pick_username(username: Option<&str>, user: Option<&str>, logname: Option<&str>) -> String {
    username
        .or(user)
        .or(logname)
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(not(windows))]
fn passwd_username() -> Option<String> {
    let out = std::process::Command::new("id").arg("-un").output().ok()?;
    let n = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!n.is_empty()).then_some(n)
}

fn compose_fallback_secret(platform: &str, home: &str, username: &str) -> String {
    format!(
        "zcode-credential-fallback:{}:{}:{}",
        platform, home, username
    )
}

pub fn default_secret(home: &Path) -> String {
    if let Ok(s) = std::env::var("ZCODE_CREDENTIAL_SECRET") {
        return s;
    }
    #[cfg(windows)]
    let primary = std::env::var("USERNAME").ok();
    #[cfg(not(windows))]
    let primary = passwd_username().or_else(|| std::env::var("USER").ok());
    let username = pick_username(
        primary.as_deref(),
        std::env::var("USER").ok().as_deref(),
        std::env::var("LOGNAME").ok().as_deref(),
    );
    compose_fallback_secret(
        node_platform_for(std::env::consts::OS),
        &home.display().to_string(),
        &username,
    )
}

fn derive_key(secret: &str) -> [u8; 32] {
    let d = Sha256::digest(secret.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn is_encrypted(v: &str) -> bool {
    v.starts_with(ENC_PREFIX)
}

pub fn decrypt_with_secret(value: &str, secret: &str) -> Result<String, String> {
    let body = value
        .strip_prefix(ENC_PREFIX)
        .ok_or_else(|| "不是 enc:v1 格式".to_string())?;
    let parts: Vec<&str> = body.split('.').collect();
    if parts.len() != 3 {
        return Err("enc:v1 格式不正确".to_string());
    }
    let nonce_b = URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|e| format!("nonce 解码失败: {e}"))?;
    let tag_b = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| format!("tag 解码失败: {e}"))?;
    let ct_b = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|e| format!("密文解码失败: {e}"))?;
    if nonce_b.len() != 12 {
        return Err("nonce 长度异常".to_string());
    }
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("密钥初始化失败: {e}"))?;
    let mut buf = ct_b.clone();
    buf.extend_from_slice(&tag_b);
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce_b), buf.as_slice())
        .map_err(|_| "解密失败（密钥不匹配或数据损坏）".to_string())?;
    Ok(String::from_utf8_lossy(&pt).to_string())
}

pub fn safe_decrypt(value: Option<&str>, secret: &str) -> Option<String> {
    let v = value?;
    if is_encrypted(v) {
        decrypt_with_secret(v, secret).ok()
    } else {
        Some(v.to_string())
    }
}

pub fn decrypt_json_opt(value: Option<&str>, secret: &str) -> Option<Value> {
    let v = value?;
    let plain = if is_encrypted(v) {
        decrypt_with_secret(v, secret).ok()?
    } else {
        v.to_string()
    };
    serde_json::from_str(&plain).ok()
}

pub fn decode_jwt(jwt: &str) -> Option<Value> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    if payload.is_empty() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ZcodeIdentity {
    pub provider: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub user_id: Option<String>,
}

impl ZcodeIdentity {
    pub fn label(&self) -> Option<String> {
        self.display_name
            .clone()
            .or_else(|| self.username.clone())
            .or_else(|| self.email.clone())
    }
}

pub fn identity_with_secret(creds: &Value, secret: &str) -> ZcodeIdentity {
    let mut id = ZcodeIdentity {
        provider: "bigmodel".into(),
        ..Default::default()
    };
    let Some(map) = creds.as_object() else {
        return id;
    };
    if let Some(ap) = map.get("oauth:active_provider").and_then(|v| v.as_str()) {
        let plain = if is_encrypted(ap) {
            decrypt_with_secret(ap, secret).ok()
        } else {
            Some(ap.to_string())
        };
        if let Some(p) = plain.filter(|p| !p.is_empty()) {
            id.provider = p;
        }
    }
    let ui_key = format!("oauth:{}:user_info", id.provider);
    let ui = map
        .get(ui_key.as_str())
        .and_then(|v| v.as_str())
        .and_then(|v| decrypt_json_opt(Some(v), secret));
    if let Some(ui) = ui {
        id.username = ui
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        id.display_name = ui
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(String::from);
        id.email = ui
            .get("email")
            .and_then(|v| v.as_str())
            .or_else(|| {
                ui.get("rawProfile")
                    .and_then(|r| r.get("email"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);
        if let Some(uid) = ui.get("id").filter(|v| !v.is_null()) {
            id.user_id = uid
                .as_str()
                .map(String::from)
                .or_else(|| serde_json::to_string(uid).ok());
        }
    }
    if id.user_id.is_none() {
        let at_key = format!("oauth:{}:access_token", id.provider);
        if let Some(at) = map.get(at_key.as_str()).and_then(|v| v.as_str()) {
            let plain = if is_encrypted(at) {
                decrypt_with_secret(at, secret).ok()
            } else {
                Some(at.to_string())
            };
            if let Some(jwt) = plain.as_deref().and_then(decode_jwt) {
                id.user_id = jwt
                    .get("user_id")
                    .or_else(|| jwt.get("sub"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }
    }
    id
}

// ---------------------------------------------------------------------------
// 本机 ZCode 数据目录
// ---------------------------------------------------------------------------

pub fn home_dir() -> PathBuf {
    std::env::var("ZCODE_SWITCH_HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn bootstrap_data_base_dir(home: &Path) -> Option<PathBuf> {
    std::fs::read_to_string(home.join(".zcode").join("v2").join("setting.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| {
            v.get("dataBaseDir")
                .and_then(|d| d.as_str())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
}

fn abs_env_path(name: &str) -> Option<PathBuf> {
    let raw = std::env::var(name).ok()?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let p = PathBuf::from(t);
    p.is_absolute().then_some(p)
}

/// ZCode 数据根目录：ZCODE_DATA_BASE_DIR env → setting.json dataBaseDir → home。
/// （zcode-switch 还支持 ZCODE_SWITCH_DATA_ROOT，此处不对该私有变量做兼容。）
pub fn resolve_data_root(home: &Path) -> PathBuf {
    if let Some(d) = bootstrap_data_base_dir(home) {
        return d;
    }
    if let Some(d) = abs_env_path("ZCODE_DATA_BASE_DIR") {
        return d;
    }
    home.to_path_buf()
}

pub fn zcode_v2_dir() -> Option<PathBuf> {
    let home = home_dir();
    let dir = resolve_data_root(&home).join(".zcode").join("v2");
    dir.exists().then_some(dir)
}

pub fn new_gen_provider_config() -> bool {
    zcode_v2_dir()
        .map(|dir| dir.join("provider_config.json").exists())
        .unwrap_or(false)
}

pub fn read_live_credentials() -> Option<Value> {
    let dir = zcode_v2_dir()?;
    let content = std::fs::read_to_string(dir.join("credentials.json")).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn read_live_config() -> Option<Value> {
    let dir = zcode_v2_dir()?;
    let content = std::fs::read_to_string(dir.join("config.json")).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn device_mid() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let dir = zcode_v2_dir()?;
            let content = std::fs::read_to_string(dir.join("telemetry-state.json")).ok()?;
            let v: Value = serde_json::from_str(&content).ok()?;
            v.get("deviceMid")
                .and_then(|m| m.as_str())
                .map(String::from)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// zcode-switch 账号快照
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeSwitchSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub credentials: Value,
    #[serde(default)]
    pub config: Option<Value>,
}

fn zcode_switch_accounts_dir() -> Option<PathBuf> {
    let dir = home_dir().join(".zcode-switch").join("accounts");
    dir.is_dir().then_some(dir)
}

/// 读取本机 zcode-switch 已保存的账号快照（按创建时间排序）。
pub fn list_switch_snapshots() -> Vec<ZcodeSwitchSnapshot> {
    let Some(dir) = zcode_switch_accounts_dir() else {
        return vec![];
    };
    let mut out = vec![];
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<ZcodeSwitchSnapshot>(&raw) {
                Ok(snap) => out.push(snap),
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Zcode Account] 跳过无法解析的快照: path={}, error={}",
                        path.display(),
                        err
                    ));
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    out
}
