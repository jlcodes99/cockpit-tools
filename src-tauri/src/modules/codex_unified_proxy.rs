//! 统一代理出口（全局唯一，一期只支持"全部可用账号统一"）。
//!
//! 账号自身的绑定仍然保存在加密账号文件里；统一代理只保存「引用 + 快照」：
//! - 引用（source/item/group/selections）用于订阅刷新或改名后重建快照；
//! - 快照是 `cockpit-proxy://` 编码的完整绑定，让启动、切号、sidecar 组装这些
//!   热路径只用内存状态，既不做磁盘 IO，也不做网络请求。
//!
//! 快照里含节点凭据，因此状态文件与 `codex-proxy-sources.json` 使用同一套本地加密
//! 存储（`secure-account-storage.key`），不额外落下明文副本。
use crate::modules::{
    account, atomic_write, codex_proxy_catalog_binding as binding, logger,
    secure_account_storage,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

pub const MODE_OFF: &str = "off";
pub const MODE_ALL_ACCOUNTS: &str = "all_accounts";
pub const STALE_SOURCE_REMOVED: &str = "CATALOG_NOT_FOUND";
pub const STALE_SOURCE_CHANGED: &str = "CATALOG_CHANGED";

const STORE_FILE: &str = "codex-unified-proxy.json";
const STORE_VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 2 * 1024 * 1024;
const RESYNC_COMMIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 代理资源引用：只含资源标识，不含任何节点凭据。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub source_id: String,
    pub source_name: String,
    pub item_id: String,
    #[serde(default)]
    pub group_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub selections: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    // Required fields keep incomplete JSON/encryption envelopes from being
    // mistaken for a valid, explicitly disabled plaintext configuration.
    version: u32,
    mode: String,
    #[serde(default)]
    reference: Option<Reference>,
    #[serde(default)]
    snapshot: Option<String>,
    #[serde(default)]
    stale_error: Option<String>,
    #[serde(default)]
    updated_at: i64,
}

/// 命令层使用的只读快照。
#[derive(Debug, Clone)]
pub struct UnifiedProxyState {
    pub mode: String,
    pub reference: Option<Reference>,
    pub snapshot: Option<String>,
    pub stale_error: Option<String>,
    pub updated_at: i64,
}

impl Default for UnifiedProxyState {
    fn default() -> Self {
        Self {
            mode: MODE_OFF.to_string(),
            reference: None,
            snapshot: None,
            stale_error: None,
            updated_at: 0,
        }
    }
}

impl UnifiedProxyState {
    pub fn active(&self) -> bool {
        self.mode == MODE_ALL_ACCOUNTS
            && self
                .snapshot
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

impl From<Stored> for UnifiedProxyState {
    fn from(value: Stored) -> Self {
        let mode = if value.mode == MODE_ALL_ACCOUNTS {
            MODE_ALL_ACCOUNTS.to_string()
        } else {
            MODE_OFF.to_string()
        };
        Self {
            mode,
            reference: value.reference,
            snapshot: value.snapshot,
            stale_error: value.stale_error,
            updated_at: value.updated_at,
        }
    }
}

impl From<&UnifiedProxyState> for Stored {
    fn from(value: &UnifiedProxyState) -> Self {
        Self {
            version: STORE_VERSION,
            mode: value.mode.clone(),
            reference: value.reference.clone(),
            snapshot: value.snapshot.clone(),
            stale_error: value.stale_error.clone(),
            updated_at: value.updated_at,
        }
    }
}

const STORAGE_ERROR: &str = "UNIFIED_PROXY_STORAGE";
const LOADING_ERROR: &str = "UNIFIED_PROXY_LOADING";

#[derive(Default)]
struct StateCache {
    state: Option<UnifiedProxyState>,
    error: Option<String>,
    revision: u64,
}
impl StateCache {
    fn current(&self) -> Result<UnifiedProxyState, String> {
        if let Some(error) = &self.error { return Err(error.clone()); }
        self.state.clone().ok_or_else(|| LOADING_ERROR.to_string())
    }
    fn publish(&mut self, next: UnifiedProxyState) {
        self.state = Some(next);
        self.error = None;
        self.revision = self.revision.wrapping_add(1);
    }
    fn finish_read(&mut self, revision: u64, result: Result<UnifiedProxyState, String>) {
        // A write completed while disk was being read: never replace its newer state.
        if self.revision != revision { return; }
        match result {
            Ok(next) => self.publish(next),
            Err(error) => { self.error = Some(error); self.revision = self.revision.wrapping_add(1); }
        }
    }
}
static STATE: LazyLock<RwLock<StateCache>> = LazyLock::new(|| RwLock::new(StateCache::default()));
static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static READ_LOCK: LazyLock<Arc<tokio::sync::Mutex<()>>> = LazyLock::new(|| Arc::new(tokio::sync::Mutex::new(())));

fn store_path() -> Result<PathBuf, String> {
    // Use the same directory resolver as account encryption and honor isolated
    // test/profile overrides for both the state file and its encryption key.
    Ok(account::get_data_dir().map_err(|_| STORAGE_ERROR)?.join(STORE_FILE))
}

/// Only an absent file means disabled. An unreadable file must stay distinguishable and retryable.
fn read_store(path: &Path) -> Result<UnifiedProxyState, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(UnifiedProxyState::default()),
        Err(_) => return Err(STORAGE_ERROR.into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_STORE_BYTES { return Err(STORAGE_ERROR.into()); }
    let key_path = path.parent().ok_or(STORAGE_ERROR)?.join("secure-account-storage.key");
    let stored: Stored = secure_account_storage::read_account_file_readonly(path, &key_path)
        .map_err(|_| STORAGE_ERROR)?;
    if stored.version != STORE_VERSION || !matches!(stored.mode.as_str(), MODE_OFF | MODE_ALL_ACCOUNTS) {
        return Err(STORAGE_ERROR.into());
    }
    let state = UnifiedProxyState::from(stored);
    if state.mode == MODE_ALL_ACCOUNTS && (!state.active() || state.reference.is_none()) {
        return Err(STORAGE_ERROR.into());
    }
    Ok(state)
}

/// Cold reads/retries run outside the async worker, with one disk reader even after a timeout.
/// Successfully loaded settings stay in memory; a failed read never becomes a cached "off".
pub async fn ensure_loaded() -> Result<UnifiedProxyState, String> {
    if let Ok(state) = current() { return Ok(state); }
    let read_guard = tokio::time::timeout(std::time::Duration::from_secs(4), READ_LOCK.clone().lock_owned())
        .await.map_err(|_| "UNIFIED_PROXY_TIMEOUT")?;
    if let Ok(state) = current() { return Ok(state); }
    let revision = STATE.read().map_err(|_| STORAGE_ERROR)?.revision;
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), tokio::task::spawn_blocking(move || {
        // Keep the permit in the worker: timing out must not start overlapping disk reads.
        let _read_guard = read_guard;
        store_path().and_then(|path| read_store(&path))
    })).await.map_err(|_| "UNIFIED_PROXY_TIMEOUT".to_string())
        .and_then(|result| result.map_err(|_| STORAGE_ERROR.to_string()))
        .and_then(|result| result);
    if result.is_err() { logger::log_warn("[UnifiedProxy] 统一代理状态读取失败，保留错误并等待重试"); }
    let mut cache = STATE.write().map_err(|_| STORAGE_ERROR)?;
    cache.finish_read(revision, result);
    cache.current()
}

#[cfg(test)]
pub(crate) fn reset_cache() {
    *STATE.write().unwrap_or_else(|error| error.into_inner()) = StateCache::default();
}

/// Synchronous consumer tests supply an explicitly prepared snapshot without
/// reading a developer's files or inheriting another test's shared settings.
#[cfg(test)]
pub(crate) struct TestCacheGuard {
    previous: StateCache,
    _env_lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl TestCacheGuard {
    pub(crate) fn new() -> Self {
        let env_lock = super::test_support::env_lock()
            .lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::mem::take(&mut *STATE.write().unwrap());
        Self { previous, _env_lock: env_lock }
    }

    pub(crate) fn prepare_disabled(&self) {
        STATE.write().unwrap().publish(UnifiedProxyState::default());
    }
}

#[cfg(test)]
impl Drop for TestCacheGuard {
    fn drop(&mut self) {
        *STATE.write().unwrap_or_else(|error| error.into_inner()) =
            std::mem::take(&mut self.previous);
    }
}

fn write_store(path: &Path, state: &UnifiedProxyState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "UNIFIED_PROXY_STORAGE".to_string())?;
    }
    let payload = Stored::from(state);
    let encrypted = secure_account_storage::serialize_account_file("codex-unified-proxy", &payload)
        .map_err(|_| "UNIFIED_PROXY_STORAGE".to_string())?;
    atomic_write::write_secret_string_atomic(path, &encrypted)
        .map_err(|_| "UNIFIED_PROXY_STORAGE".to_string())
}

fn publish_unlocked(mut next: UnifiedProxyState) -> Result<UnifiedProxyState, String> {
    let previous = STATE.read().map_err(|_| STORAGE_ERROR)?.state.as_ref().map_or(0, |state| state.updated_at);
    next.updated_at = next.updated_at.max(previous.saturating_add(1));
    write_store(&store_path()?, &next)?;
    STATE.write().map_err(|_| STORAGE_ERROR)?.publish(next.clone());
    Ok(next)
}

fn publish(next: UnifiedProxyState) -> Result<UnifiedProxyState, String> {
    let _guard = WRITE_LOCK.lock().map_err(|_| STORAGE_ERROR)?;
    publish_unlocked(next)
}

/// Memory-only lookup. Call ensure_loaded on asynchronous entry paths before consuming it.
pub fn current() -> Result<UnifiedProxyState, String> {
    STATE.read().map_err(|_| STORAGE_ERROR)?.current()
}

pub fn effective_snapshot() -> Result<Option<String>, String> {
    let state = current()?;
    Ok(if state.active() { state.snapshot } else { None })
}

pub fn active() -> Result<bool, String> { Ok(current()?.active()) }

pub fn reference() -> Result<Option<Reference>, String> { Ok(current()?.reference) }

pub fn matches_source(source_id: &str) -> Result<bool, String> {
    Ok(reference()?.is_some_and(|reference| reference.source_id == source_id))
}

/// Summary only: never send the credential-bearing binding snapshot to the frontend.
pub fn snapshot_summary() -> Result<Option<serde_json::Value>, String> {
    Ok(effective_snapshot()?.and_then(|snapshot| binding::summary(&snapshot)))
}

pub fn enable(reference: Reference, snapshot: String) -> Result<UnifiedProxyState, String> {
    let snapshot = snapshot.trim().to_string();
    if snapshot.is_empty() || !snapshot.starts_with(binding::PREFIX) {
        return Err("UNIFIED_PROXY_INVALID".into());
    }
    publish(UnifiedProxyState {
        mode: MODE_ALL_ACCOUNTS.to_string(),
        reference: Some(reference),
        snapshot: Some(snapshot),
        stale_error: None,
        updated_at: now_millis(),
    })
}

pub fn disable() -> Result<UnifiedProxyState, String> {
    publish(UnifiedProxyState {
        mode: MODE_OFF.to_string(),
        reference: None,
        snapshot: None,
        stale_error: None,
        updated_at: now_millis(),
    })
}

/// 引用的来源被删除：直接关闭统一代理，避免账号继续使用已删除来源的出口。
pub fn disable_with_stale(error: &str) -> Result<UnifiedProxyState, String> {
    publish(UnifiedProxyState {
        mode: MODE_OFF.to_string(),
        reference: None,
        snapshot: None,
        stale_error: Some(error.to_string()),
        updated_at: now_millis(),
    })
}

/// 订阅刷新后重建失败等场景：保留最后一次可用快照，只标记待更新。
pub fn mark_stale(error: Option<&str>) -> Result<UnifiedProxyState, String> {
    let mut next = current()?;
    next.stale_error = error.map(str::to_string);
    next.updated_at = now_millis();
    publish(next)
}

/// A refresh may finish after the user has changed or disabled the unified exit.
/// Commit only against the exact state observed before the asynchronous rebuild.
fn resync_commit(
    source_id: &str,
    expected_updated_at: i64,
    removed: bool,
    rebuilt: Result<String, String>,
) -> Result<bool, String> {
    let _guard = WRITE_LOCK.lock().map_err(|_| "UNIFIED_PROXY_STORAGE")?;
    let mut next = current()?;
    if !next.active()
        || next.updated_at != expected_updated_at
        || next.reference.as_ref().is_none_or(|reference| reference.source_id != source_id)
    {
        return Ok(false);
    }
    if removed {
        next.mode = MODE_OFF.to_string();
        next.reference = None;
        next.snapshot = None;
        next.stale_error = Some(STALE_SOURCE_REMOVED.to_string());
    } else {
        match rebuilt {
            Ok(snapshot) => {
                next.snapshot = Some(snapshot);
                next.stale_error = None;
            }
            Err(error) => {
                next.stale_error = Some(if error.trim().is_empty() {
                    STALE_SOURCE_CHANGED.to_string()
                } else {
                    error
                });
            }
        }
    }
    next.updated_at = now_millis();
    publish_unlocked(next)?;
    Ok(true)
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 代理资源变更后重建统一代理快照。
///
/// 重建失败只标记待更新并保留最后一次可用出口：账号不会因为一次订阅刷新失败
/// 就被迫直连，也不会被静默改写账号数据。
static RESYNC_COMMIT_LOCK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

async fn finish_resync(
    source_id: String,
    expected_updated_at: i64,
    removed: bool,
    rebuilt: Result<String, String>,
) {
    let Ok(Ok(permit)) = tokio::time::timeout(RESYNC_COMMIT_TIMEOUT, RESYNC_COMMIT_LOCK.acquire()).await else {
        logger::log_warn("[UnifiedProxy] 重新同步仍在写入，本次等待超时，请重试来源更新");
        return;
    };
    let result = tokio::time::timeout(RESYNC_COMMIT_TIMEOUT, tokio::task::spawn_blocking(move || {
        // Single-flight remains held even after the caller stops waiting.
        let _permit = permit;
        let result = resync_commit(&source_id, expected_updated_at, removed, rebuilt);
        if matches!(result, Ok(true)) {
            crate::modules::codex_proxy_runtime::spawn_reload_after_unified_change();
        }
        if result.is_err() {
            logger::log_warn("[UnifiedProxy] 重新同步写入失败，保留原出口，等待重试");
        }
        result
    })).await;
    if result.is_err() {
        logger::log_warn("[UnifiedProxy] 重新同步写入超时，后台完成后仍会更新运行态");
    }
}

pub async fn resync_source(source_id: &str, removed: bool) {
    let Ok(observed) = ensure_loaded().await else { return; };
    if !observed.active() || observed.reference.as_ref().is_none_or(|reference| reference.source_id != source_id) {
        return;
    }
    let source_id = source_id.to_string();
    let expected_updated_at = observed.updated_at;
    if removed {
        finish_resync(source_id, expected_updated_at, true, Ok(String::new())).await;
        return;
    }
    let Some(reference) = observed.reference else {
        return;
    };
    let rebuilt = crate::modules::codex_proxy_catalog::snapshot_with_group(
        reference.source_id.clone(),
        reference.item_id.clone(),
        reference.selections.clone(),
        reference.group_id.clone(),
    )
    .await;
    finish_resync(source_id, expected_updated_at, false, rebuilt).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("unified-proxy-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn reference() -> Reference {
        Reference {
            source_id: "source-id".into(),
            source_name: "Demo".into(),
            item_id: "node-id".into(),
            group_id: None,
            name: "Node".into(),
            selections: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_store_reads_as_disabled() {
        let temp = Temp::new();
        let state = read_store(&temp.0.join("missing.json")).unwrap();
        assert_eq!(state.mode, MODE_OFF);
        assert!(!state.active());
    }

    #[test]
    fn corrupted_store_reports_failure_instead_of_disabling_proxy() {
        let temp = Temp::new();
        let path = temp.0.join("broken.json");
        fs::write(&path, "{not json").unwrap();
        assert_eq!(read_store(&path).unwrap_err(), STORAGE_ERROR);
    }

    #[test]
    fn incomplete_store_or_envelope_cannot_be_read_as_disabled() {
        let temp = Temp::new();
        let path = temp.0.join(STORE_FILE);
        for content in [
            "{}",
            r#"{"version":1}"#,
            r#"{"mode":"off"}"#,
            r#"{"version":1,"kind":"codex-unified-proxy","algorithm":"AES-256-GCM"}"#,
            r#"{"version":1,"mode":"all_accounts","reference":null,"snapshot":null}"#,
        ] {
            fs::write(&path, content).unwrap();
            assert_eq!(read_store(&path).unwrap_err(), STORAGE_ERROR);
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
            assert!(!temp.0.join("secure-account-storage.key").exists());
        }
    }

    #[test]
    fn read_error_preserves_last_snapshot_without_misreporting_off() {
        let mut cache = StateCache::default();
        let previous = UnifiedProxyState { mode: MODE_ALL_ACCOUNTS.into(), reference: Some(reference()),
            snapshot: Some("cockpit-proxy://saved".into()), ..Default::default() };
        cache.publish(previous);
        cache.finish_read(cache.revision, Err(STORAGE_ERROR.into()));
        assert_eq!(cache.current().unwrap_err(), STORAGE_ERROR);
        assert_eq!(cache.state.as_ref().unwrap().snapshot.as_deref(), Some("cockpit-proxy://saved"));
        cache.finish_read(cache.revision, Ok(UnifiedProxyState::default()));
        assert_eq!(cache.current().unwrap().mode, MODE_OFF);
    }

    #[test]
    fn late_disk_result_cannot_replace_a_completed_write() {
        let mut cache = StateCache::default();
        let loading_revision = cache.revision;
        cache.publish(UnifiedProxyState { updated_at: 123, ..Default::default() });
        cache.finish_read(loading_revision, Err(STORAGE_ERROR.into()));
        assert_eq!(cache.current().unwrap().updated_at, 123);
        cache.finish_read(loading_revision, Ok(UnifiedProxyState { updated_at: 1, ..Default::default() }));
        assert_eq!(cache.current().unwrap().updated_at, 123);
    }

    #[tokio::test]
    async fn failed_initial_read_can_retry_after_file_recovery_without_restart() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap_or_else(|error| error.into_inner());
        let temp = Temp::new();
        struct RestoreEnv(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                reset_cache();
                match &self.0 {
                    Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                    None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
                }
                match &self.1 {
                    Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                    None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
                }
            }
        }
        let _restore = RestoreEnv(std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"), std::env::var_os("COCKPIT_TOOLS_DATA_DIR"));
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp.0);
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &temp.0);
        reset_cache();
        let path = store_path().unwrap();
        assert_eq!(path, temp.0.join(STORE_FILE));
        fs::write(&path, "{broken").unwrap();
        assert_eq!(ensure_loaded().await.unwrap_err(), STORAGE_ERROR);
        assert_eq!(effective_snapshot().unwrap_err(), STORAGE_ERROR);
        let restored = Stored { version: STORE_VERSION, mode: MODE_ALL_ACCOUNTS.into(), reference: Some(reference()),
            snapshot: Some("cockpit-proxy://restored".into()), ..Default::default() };
        fs::write(&path, serde_json::to_string(&restored).unwrap()).unwrap();
        assert!(ensure_loaded().await.unwrap().active());
        assert_eq!(effective_snapshot().unwrap().as_deref(), Some("cockpit-proxy://restored"));
    }

    #[test]
    fn only_complete_bindings_activate_the_shared_egress() {
        let snapshot = "cockpit-proxy://demo".to_string();
        let mut state = UnifiedProxyState {
            mode: MODE_ALL_ACCOUNTS.into(),
            reference: Some(reference()),
            snapshot: Some(snapshot.clone()),
            ..Default::default()
        };
        assert!(state.active());
        state.snapshot = Some("   ".into());
        assert!(!state.active());
        state.snapshot = Some(snapshot);
        state.mode = MODE_OFF.into();
        assert!(!state.active());
    }

    /// 磁盘往返走 `secure_account_storage`（写入要求真实密钥），因此这里只覆盖
    /// 旧明文文件的兼容读取；加解密自身由 secure_account_storage 的测试覆盖。
    #[test]
    fn legacy_plaintext_store_still_loads() {
        let temp = Temp::new();
        let path = temp.0.join(STORE_FILE);
        let stored = Stored {
            version: STORE_VERSION,
            mode: MODE_ALL_ACCOUNTS.into(),
            reference: Some(reference()),
            snapshot: Some("cockpit-proxy://demo".into()),
            stale_error: Some(STALE_SOURCE_CHANGED.into()),
            updated_at: 42,
        };
        fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
        let restored = read_store(&path).unwrap();
        assert_eq!(restored.mode, MODE_ALL_ACCOUNTS);
        assert_eq!(restored.reference, Some(reference()));
        assert_eq!(restored.snapshot.as_deref(), Some("cockpit-proxy://demo"));
        assert_eq!(restored.stale_error.as_deref(), Some(STALE_SOURCE_CHANGED));
        assert_eq!(restored.updated_at, 42);
    }

    #[test]
    fn unknown_modes_fall_back_to_off() {
        let stored = Stored {
            version: STORE_VERSION,
            mode: "something-else".into(),
            snapshot: Some("cockpit-proxy://demo".into()),
            ..Default::default()
        };
        let state = UnifiedProxyState::from(stored);
        assert_eq!(state.mode, MODE_OFF);
        assert!(!state.active());
    }

    #[test]
    fn disabled_state_drops_binding_payload() {
        let state = UnifiedProxyState {
            mode: MODE_OFF.into(),
            reference: None,
            snapshot: None,
            ..Default::default()
        };
        assert!(!state.active());
        assert!(state.snapshot.is_none());
    }
}
