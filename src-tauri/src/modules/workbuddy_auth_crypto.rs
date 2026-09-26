//! WorkBuddy 5.6 field encryption, verified against the installed desktop client.
//! The official Electron native binding provides the build key in Node mode;
//! no GUI, Keychain access, network, keyblob creation or persistent key copy.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

const KEY_TIMEOUT: Duration = Duration::from_secs(5);
const KEY_TTL: Duration = Duration::from_secs(60);
const PREPARING: &str = "WorkBuddy 官方加密密钥正在准备，请稍后重试";
const KEY_MISMATCH: &str = "WorkBuddy 官方加密密钥已变化，正在重新准备，请稍后重试";
const INVALID: &str =
    "WorkBuddy 官方加密字段校验失败，已停止覆盖，请更新或重新登录官方客户端后重试";
const UNSUPPORTED: &str = "WorkBuddy 官方加密格式不受支持，已停止覆盖，请更新 Cockpit Tools 后重试";
// Keep the payload confined to an anonymous pipe. Never log stdout/stderr or
// pass key material in arguments/environment. Hash the base64 TEXT, not bytes.
const KEY_SCRIPT: &str = r#"
try {
  const c = require('crypto');
  const p = JSON.parse(process._linkedBinding('electron_browser_workbuddy_storage').loggerGet());
  if (p.version !== 1 || typeof p.atRestSecretKey !== 'string') process.exit(2);
  const b = Buffer.from(p.atRestSecretKey, 'base64');
  if (b.length !== 32 || b.toString('base64') !== p.atRestSecretKey || b.every(x => x === 0)) process.exit(2);
  const key = c.createHash('sha256').update(p.atRestSecretKey, 'utf8').digest();
  process.stdout.write(key.toString('base64'));
  key.fill(0); b.fill(0);
} catch (_) { process.exit(2); }
"#;

struct OfficialKey {
    bytes: [u8; 32],
    id: String,
}

impl OfficialKey {
    fn new(bytes: [u8; 32]) -> Self {
        let id = format!("{:x}", Sha256::digest(bytes))[..16].to_owned();
        Self { bytes, id }
    }

    fn aad(&self) -> Vec<u8> {
        let mut bytes = b"WB-AAD\0\x01".to_vec();
        for value in ["WBEV1", "sym-v1"] {
            bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&(self.id.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.id.as_bytes());
        bytes.extend_from_slice(&[2, 0, 0]); // field framing, no sequence/final
        bytes
    }

    fn open(&self, wrapper: &Value) -> Result<String, String> {
        let obj = wrapper.as_object().ok_or(INVALID)?;
        if obj.len() != 2 || obj.get("$wbEncrypted") != Some(&json!(1)) {
            return Err(UNSUPPORTED.into());
        }
        let encoded = obj.get("envelope").and_then(Value::as_str).ok_or(INVALID)?;
        let envelope: Envelope =
            serde_json::from_slice(&decode_base64(encoded)?).map_err(|_| INVALID.to_string())?;
        if envelope.suite != 1 {
            return Err(UNSUPPORTED.into());
        }
        if envelope.key_id != self.id {
            return Err(KEY_MISMATCH.into());
        }
        let nonce = decode_base64(&envelope.nonce)?;
        let tag = decode_base64(&envelope.auth_tag)?;
        if nonce.len() != 12 || tag.len() != 16 {
            return Err(INVALID.into());
        }
        let mut ciphertext = decode_base64(&envelope.ciphertext)?;
        ciphertext.extend_from_slice(&tag);
        let cipher = Aes256Gcm::new_from_slice(&self.bytes).map_err(|_| INVALID)?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &self.aad(),
                },
            )
            .map_err(|_| INVALID)?;
        String::from_utf8(plaintext).map_err(|_| INVALID.into())
    }

    fn seal(&self, plaintext: &str) -> Result<Value, String> {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(&self.bytes).map_err(|_| INVALID)?;
        let sealed = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &self.aad(),
                },
            )
            .map_err(|_| INVALID)?;
        let split = sealed.len() - 16;
        let envelope = Envelope {
            suite: 1,
            key_id: self.id.clone(),
            nonce: STANDARD.encode(nonce),
            auth_tag: STANDARD.encode(&sealed[split..]),
            ciphertext: STANDARD.encode(&sealed[..split]),
        };
        let bytes = serde_json::to_vec(&envelope).map_err(|_| INVALID)?;
        Ok(json!({ "$wbEncrypted": 1, "envelope": STANDARD.encode(bytes) }))
    }
}

impl Drop for OfficialKey {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    suite: u32,
    key_id: String,
    nonce: String,
    auth_tag: String,
    ciphertext: String,
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, String> {
    if encoded.len() > 16 * 1024 * 1024 {
        return Err(INVALID.into());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| INVALID)?;
    if STANDARD.encode(&bytes) != encoded {
        return Err(INVALID.into());
    }
    Ok(bytes)
}

#[derive(Default)]
struct KeyState {
    key: Option<Arc<OfficialKey>>,
    loaded_at: Option<Instant>,
    loading: bool,
    failure: Option<(Instant, String)>,
}
static KEY_STATE: LazyLock<(Mutex<KeyState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(KeyState::default()), Condvar::new()));

fn load_official_key() -> Result<OfficialKey, String> {
    let executable = super::process::resolve_workbuddy_launch_path()?;
    let mut command = Command::new(executable);
    command
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env_remove("NODE_OPTIONS")
        .args(["-e", KEY_SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    run_key_command(command, KEY_TIMEOUT)
}

fn run_key_command(command: Command, timeout: Duration) -> Result<OfficialKey, String> {
    use tokio::io::AsyncReadExt;
    // The timeout covers both child exit and pipe reads (including inherited
    // stdout handles), and dropping a timed-out child kills the helper.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| INVALID)?;
    let mut output = runtime.block_on(async {
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|_| "无法读取 WorkBuddy 官方密钥，请检查客户端安装路径".to_string())?;
        let stdout = child.stdout.take().ok_or(INVALID)?;
        tokio::time::timeout(timeout, async {
            let read = async {
                let mut output = Vec::new();
                stdout.take(65).read_to_end(&mut output).await?;
                Ok::<_, std::io::Error>(output)
            };
            let (status, output) = tokio::try_join!(child.wait(), read).map_err(|_| INVALID)?;
            if !status.success() {
                return Err(
                    "当前 WorkBuddy 客户端未提供可用的官方密钥接口，已停止覆盖，请更新客户端后重试"
                        .to_string(),
                );
            }
            Ok(output)
        })
        .await
        .map_err(|_| "读取 WorkBuddy 官方密钥超时，请重试".to_string())?
    })?;
    let result = (|| {
        if output.len() != 44 {
            return Err(INVALID.to_string());
        }
        let encoded = std::str::from_utf8(&output).map_err(|_| INVALID)?;
        let mut decoded = decode_base64(encoded)?;
        let result = <[u8; 32]>::try_from(decoded.as_slice())
            .map(OfficialKey::new)
            .map_err(|_| INVALID.to_string());
        decoded.fill(0);
        result
    })();
    output.fill(0);
    result
}

fn finish_load(result: Result<OfficialKey, String>) -> Result<(), String> {
    let (lock, changed) = &*KEY_STATE;
    let mut state = lock.lock().map_err(|_| INVALID)?;
    state.loading = false;
    let result = match result {
        Ok(key) => {
            state.key = Some(Arc::new(key));
            state.loaded_at = Some(Instant::now());
            state.failure = None;
            Ok(())
        }
        Err(error) => {
            state.failure = Some((Instant::now(), error.clone()));
            Err(error)
        }
    };
    changed.notify_all();
    result
}

/// Only call on a blocking worker. Concurrent preparations share one helper,
/// and failures remain retryable without restarting the host.
pub(crate) fn prepare() -> Result<(), String> {
    #[cfg(test)]
    if TEST_KEY.with(|key| key.borrow().is_some()) {
        return Ok(());
    }
    prepare_with_loader(load_official_key)
}

fn prepare_with_loader(load: impl FnOnce() -> Result<OfficialKey, String>) -> Result<(), String> {
    let (lock, changed) = &*KEY_STATE;
    let mut state = lock.lock().map_err(|_| INVALID)?;
    if state.loading {
        let (next, wait) = changed
            .wait_timeout_while(state, KEY_TIMEOUT + Duration::from_secs(1), |s| s.loading)
            .map_err(|_| INVALID)?;
        state = next;
        if wait.timed_out() && state.loading {
            return Err(PREPARING.into());
        }
    }
    if let Some((at, error)) = &state.failure {
        if at.elapsed() < Duration::from_secs(1) {
            return Err(error.clone());
        }
    }
    if state.loaded_at.is_some_and(|at| at.elapsed() < KEY_TTL) && state.key.is_some() {
        return Ok(());
    }
    state.loading = true;
    drop(state);
    finish_load(load())
}

/// UI/tray reads never synchronously wait for an external process.
fn cached_key() -> Result<Arc<OfficialKey>, String> {
    #[cfg(test)]
    if let Some(key) = TEST_KEY.with(|key| key.borrow().clone()) {
        return Ok(key);
    }
    let (lock, _) = &*KEY_STATE;
    let mut state = lock.lock().map_err(|_| INVALID)?;
    if let Some(key) = &state.key {
        return Ok(key.clone());
    }
    if let Some((at, error)) = &state.failure {
        if at.elapsed() < Duration::from_secs(1) {
            return Err(error.clone());
        }
    }
    if !state.loading {
        state.loading = true;
        drop(state);
        if std::thread::Builder::new()
            .name("workbuddy-auth-key".into())
            .spawn(|| {
                let _ = finish_load(load_official_key());
            })
            .is_err()
        {
            let _ = finish_load(Err(PREPARING.into()));
        }
    }
    Err(PREPARING.into())
}

pub(crate) fn contains_encrypted_wrapper(value: &Value) -> bool {
    match value {
        Value::Object(obj) => {
            obj.contains_key("$wbEncrypted") || obj.values().any(contains_encrypted_wrapper)
        }
        Value::Array(items) => items.iter().any(contains_encrypted_wrapper),
        _ => false,
    }
}

fn decode_field(value: &mut Value, name: &str, key: &OfficialKey) -> Result<(), String> {
    if let Some(field) = value.get_mut(name) {
        if field.get("$wbEncrypted").is_some() {
            *field = Value::String(key.open(field)?);
        }
    }
    Ok(())
}

fn decode_with_key(value: &mut Value, key: &OfficialKey) -> Result<(), String> {
    if let Some(auth) = value.get_mut("auth") {
        for name in ["accessToken", "refreshToken"] {
            decode_field(auth, name, key)?;
        }
    }
    if let Some(account) = value.get_mut("account") {
        for name in ["phoneNumber", "departmentFullName", "nickname"] {
            decode_field(account, name, key)?;
        }
    }
    for name in ["accounts", "allAccounts"] {
        if let Some(items) = value.get_mut(name).and_then(Value::as_array_mut) {
            for item in items {
                for field in ["phoneNumber", "departmentFullName", "nickname"] {
                    decode_field(item, field, key)?;
                }
            }
        }
    }
    // Unknown encrypted paths cannot be safely round-tripped by this policy.
    // Refuse the transaction rather than persisting unknown secrets as plaintext.
    if contains_encrypted_wrapper(value) {
        return Err(UNSUPPORTED.into());
    }
    Ok(())
}

fn invalidate_key(key: &Arc<OfficialKey>) {
    let (lock, _) = &*KEY_STATE;
    if let Ok(mut state) = lock.lock() {
        if state
            .key
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, key))
        {
            state.key = None;
            state.loaded_at = None;
            state.failure = Some((Instant::now(), KEY_MISMATCH.into()));
        }
    }
    let _ = cached_key(); // subsequent reads retry after backoff; never block a UI read
}

pub(crate) fn decode_session(value: &Value) -> Result<Value, String> {
    let mut decoded = value.clone();
    if contains_encrypted_wrapper(value) {
        let key = cached_key()?;
        if let Err(error) = decode_with_key(&mut decoded, &key) {
            if error == KEY_MISMATCH {
                invalidate_key(&key);
            }
            return Err(error);
        }
    }
    Ok(decoded)
}

fn encode_account(value: &mut Value, key: &OfficialKey) -> Result<(), String> {
    for name in ["phoneNumber", "departmentFullName", "nickname"] {
        encode_field(value, name, key)?;
    }
    Ok(())
}

fn encode_field(value: &mut Value, name: &str, key: &OfficialKey) -> Result<(), String> {
    if let Some(field) = value.get_mut(name) {
        if let Some(plaintext) = field.as_str() {
            *field = key.seal(plaintext)?;
        } else if !field.is_null() {
            return Err(INVALID.into());
        }
    }
    Ok(())
}

fn encode_with_key(value: &Value, key: &OfficialKey) -> Result<Value, String> {
    if contains_encrypted_wrapper(value) {
        return Err(INVALID.into());
    }
    let mut encoded = value.clone();
    if let Some(auth) = encoded.get_mut("auth") {
        for name in ["accessToken", "refreshToken"] {
            encode_field(auth, name, key)?;
        }
    }
    if let Some(account) = encoded.get_mut("account") {
        encode_account(account, key)?;
    }
    for name in ["accounts", "allAccounts"] {
        if let Some(items) = encoded.get_mut(name).and_then(Value::as_array_mut) {
            for item in items {
                encode_account(item, key)?;
            }
        }
    }
    Ok(encoded)
}

pub(crate) fn encode_session(value: &Value) -> Result<Value, String> {
    encode_with_key(value, cached_key()?.as_ref())
}

#[cfg(test)]
thread_local! {
    static TEST_KEY: std::cell::RefCell<Option<Arc<OfficialKey>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_key(f: impl FnOnce()) {
    struct Restore(Option<Arc<OfficialKey>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_KEY.with(|key| *key.borrow_mut() = self.0.take());
        }
    }
    let _restore =
        Restore(TEST_KEY.with(|key| key.replace(Some(Arc::new(OfficialKey::new([7; 32]))))));
    f();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent Node crypto fixture using WorkBuddy's exact authenticated
    // context framing, synthetic key [7;32] and nonce [3;12]. No user secrets.
    fn fixture() -> Value {
        let envelope = r#"{"suite":1,"keyId":"4bb06f8e4e3a7715","nonce":"AwMDAwMDAwMDAwMD","authTag":"8mXI80XfjtTaNENkKD5E5Q==","ciphertext":"Q5fbdy9aO28OLyg5hXoZuWbUT3o="}"#;
        json!({"$wbEncrypted":1,"envelope":STANDARD.encode(envelope)})
    }

    #[test]
    fn matches_official_node_framing() {
        assert_eq!(
            OfficialKey::new([7; 32]).open(&fixture()).unwrap(),
            "fixture-token-测试"
        );
    }

    #[test]
    fn rejects_wrong_key_tampering_and_unknown_formats() {
        assert!(OfficialKey::new([8; 32]).open(&fixture()).is_err());
        let key = OfficialKey::new([7; 32]);
        for field in ["nonce", "authTag", "ciphertext"] {
            let mut e: Value = serde_json::from_slice(
                &STANDARD
                    .decode(fixture()["envelope"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            e[field] = json!(STANDARD.encode([0u8; 16]));
            assert!(key.open(&json!({"$wbEncrypted":1,"envelope":STANDARD.encode(serde_json::to_vec(&e).unwrap())})).is_err());
        }
        let mut wrapper = fixture();
        wrapper["scheme"] = json!("asym-v1");
        assert!(key.open(&wrapper).is_err());
        wrapper.as_object_mut().unwrap().remove("scheme");
        wrapper["$wbEncrypted"] = json!(2);
        assert!(key.open(&wrapper).is_err());
    }

    #[test]
    fn encrypts_official_fields_and_preserves_session_shape() {
        with_test_key(|| {
            let plain = json!({
                "auth":{"accessToken":"access","refreshToken":"refresh","expiresAt":42},
                "account":{"uid":"one","nickname":"测试","phoneNumber":"","departmentFullName":"team"},
                "accounts":[{"uid":"other","nickname":"Other","lastLogin":false}],
                "allAccounts":[{"uid":"one","nickname":"测试"}],
                "unknown":{"revision":1}
            });
            let encrypted = encode_session(&plain).unwrap();
            assert!(encrypted["auth"]["accessToken"].is_object());
            assert!(encrypted["accounts"][0]["nickname"].is_object());
            assert!(encrypted["account"]["phoneNumber"].is_object());
            assert_eq!(encrypted["auth"]["expiresAt"], 42);
            assert_eq!(decode_session(&encrypted).unwrap(), plain);
            assert_ne!(encode_session(&plain).unwrap(), encrypted); // fresh nonces
        });
    }

    #[test]
    fn unknown_encrypted_paths_are_rejected_without_exposing_plaintext() {
        let mut session = json!({"unknown": {"secret": fixture()}});
        let original = session.clone();
        assert!(decode_with_key(&mut session, &OfficialKey::new([7; 32])).is_err());
        assert_eq!(session, original);
    }

    /// Explicit opt-in local diagnostic; reads only, never writes official state.
    #[test]
    #[ignore = "requires an installed WorkBuddy and explicit local auth path"]
    fn installed_client_readonly_compatibility() {
        let executable =
            std::env::var_os("WORKBUDDY_TEST_EXECUTABLE").expect("explicit executable");
        let auth_path = std::env::var_os("WORKBUDDY_TEST_AUTH_PATH").expect("explicit auth path");
        let mut command = Command::new(executable);
        command
            .env("ELECTRON_RUN_AS_NODE", "1")
            .env_remove("NODE_OPTIONS")
            .args(["-e", KEY_SCRIPT])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let key = run_key_command(command, KEY_TIMEOUT).unwrap();
        let original = std::fs::read(auth_path).unwrap();
        let encrypted: Value = serde_json::from_slice(&original).unwrap();
        assert!(contains_encrypted_wrapper(&encrypted));
        let mut decoded = encrypted.clone();
        assert!(
            decode_with_key(&mut decoded, &key).is_ok(),
            "official ciphertext must authenticate"
        );
        assert!(decoded["auth"]["accessToken"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        let mut roundtrip = encode_with_key(&decoded, &key).unwrap();
        assert!(contains_encrypted_wrapper(&roundtrip));
        assert!(decode_with_key(&mut roundtrip, &key).is_ok());
        assert!(roundtrip == decoded, "round-trip must preserve every field");
    }

    #[test]
    fn plaintext_read_does_not_need_client_or_key() {
        let plain = json!({"auth":{"accessToken":"legacy-token"}});
        assert_eq!(decode_session(&plain).unwrap(), plain);
    }

    #[test]
    fn concurrent_preparation_is_single_flight_and_failures_can_retry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Other account tests may have already prepared the shared cache. This
        // test exercises the cold-load path, so start with an empty cache.
        *KEY_STATE.0.lock().unwrap() = KeyState::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let calls = calls.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    prepare_with_loader(|| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(40));
                        Ok(OfficialKey::new([7; 32]))
                    })
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        {
            let mut state = KEY_STATE.0.lock().unwrap();
            *state = KeyState::default();
        }
        assert!(prepare_with_loader(|| Err("synthetic failure".into())).is_err());
        {
            let mut state = KEY_STATE.0.lock().unwrap();
            state.failure.as_mut().unwrap().0 = Instant::now() - Duration::from_secs(2);
        }
        prepare_with_loader(|| Ok(OfficialKey::new([7; 32]))).unwrap();
        *KEY_STATE.0.lock().unwrap() = KeyState::default();
    }

    #[cfg(unix)]
    #[test]
    fn inherited_stdout_cannot_outlive_helper_deadline() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 1 & exit 0"])
            .stdout(Stdio::piped());
        let start = Instant::now();
        assert!(run_key_command(command, Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < Duration::from_millis(700));
    }

    #[cfg(unix)]
    #[test]
    fn key_helper_timeout_is_bounded_and_retryable() {
        let mut command = Command::new("/bin/sleep");
        command.arg("2").stdout(Stdio::piped());
        let start = Instant::now();
        assert!(run_key_command(command, Duration::from_millis(40)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut command = Command::new("/usr/bin/printf");
        command.arg(STANDARD.encode([7; 32])).stdout(Stdio::piped());
        assert_eq!(
            run_key_command(command, Duration::from_secs(1)).unwrap().id,
            OfficialKey::new([7; 32]).id
        );
    }
}
