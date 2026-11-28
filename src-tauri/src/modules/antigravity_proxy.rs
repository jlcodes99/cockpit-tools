//! Antigravity client exits. The encrypted store owns credentials; commands expose metadata only.
//! Runtime projections are token-free and are never written into the Codex account store.
use super::{
    account, atomic_write, codex_proxy_catalog as catalog, codex_proxy_catalog_binding as binding,
    secure_account_storage,
};
use crate::models::{
    account::Account,
    codex::{CodexAccount, CodexTokens},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

const PREFIX: &str = "__antigravity_proxy__:";
const FILE: &str = "antigravity-proxy-routes.json";
const STORAGE: &str = "ANTIGRAVITY_PROXY_STORAGE";
const STALE: &str = "ANTIGRAVITY_PROXY_STALE";
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static WRITES: Mutex<()> = Mutex::new(());
static LOCKS: LazyLock<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static IO: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Setting {
    snapshot: Option<String>,
    reference: Option<super::codex_unified_proxy::Reference>,
    error: Option<String>,
    revision: u64,
}
#[derive(Serialize, Deserialize)]
struct Store {
    version: u32,
    targets: BTreeMap<String, Setting>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            targets: BTreeMap::new(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub account_id: String,
    pub binding: Option<Value>,
    pub error: Option<String>,
    pub legacy_bound: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub target_id: String,
    pub name: String,
}

fn normalize_id(raw: &str) -> Result<String, String> {
    let id = raw.trim();
    if id.is_empty()
        || id.len() > 256
        || id.contains(['/', '\\', ':'])
        || matches!(id, "." | "..")
        || id.chars().any(char::is_control)
    {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    Ok(id.to_owned())
}
pub fn route_target(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX)
}
pub fn route_id(id: &str) -> String {
    format!("{PREFIX}{id}")
}
pub(crate) fn route_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id.to_owned())
        .or_default()
        .clone()
}
fn path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir().map_err(|_| STORAGE)?.join(FILE))
}
fn read() -> Result<Store, String> {
    let path = path()?;
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Ok(meta) if meta.is_file() && meta.len() <= MAX_BYTES => (),
        _ => return Err(STORAGE.into()),
    }
    let key = path
        .parent()
        .ok_or(STORAGE)?
        .join("secure-account-storage.key");
    let store: Store =
        secure_account_storage::read_account_file_readonly(&path, &key).map_err(|_| STORAGE)?;
    if store.version != 1 || store.targets.len() > 1024 {
        return Err(STORAGE.into());
    }
    for (id, setting) in &store.targets {
        normalize_id(id).map_err(|_| STORAGE)?;
        match (&setting.snapshot, &setting.reference) {
            (None, None) if setting.error.is_none() => (),
            (Some(raw), Some(reference)) => {
                let decoded = binding::decode(raw).map_err(|_| STORAGE)?;
                if decoded.source_id != reference.source_id || decoded.item_id != reference.item_id
                {
                    return Err(STORAGE.into());
                }
            }
            _ => return Err(STORAGE.into()),
        }
    }
    Ok(store)
}
fn mutate<T>(change: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let _write = WRITES.lock().map_err(|_| STORAGE)?;
    let path = path()?;
    let parent = path.parent().ok_or(STORAGE)?;
    std::fs::create_dir_all(parent).map_err(|_| STORAGE)?;
    let lock_path = parent.join("antigravity-proxy-routes.lock");
    if lock_path.is_symlink() {
        return Err(STORAGE.into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| STORAGE)?;
    lock.try_lock_exclusive().map_err(|_| "CATALOG_BUSY")?;
    let mut store = read()?;
    let result = change(&mut store)?;
    if store.targets.len() > 1024 {
        return Err(STORAGE.into());
    }
    let encrypted =
        secure_account_storage::serialize_account_file("antigravity-proxy-routes", &store)
            .map_err(|_| STORAGE)?;
    if encrypted.len() as u64 > MAX_BYTES {
        return Err(STORAGE.into());
    }
    atomic_write::write_secret_string_atomic(&path, &encrypted).map_err(|_| STORAGE)?;
    Ok(result)
}
async fn io<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = tokio::time::timeout(Duration::from_secs(3), IO.clone().acquire_owned())
        .await
        .map_err(|_| "PROXY_RUNTIME_READ_TIMEOUT")?
        .map_err(|_| STORAGE)?;
    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        }),
    )
    .await
    .map_err(|_| "PROXY_RUNTIME_READ_TIMEOUT")?
    .map_err(|_| STORAGE.to_string())?
}
fn read_index() -> Result<crate::models::account::AccountIndex, String> {
    use super::antigravity_account_index_store::{AccountIndexStore, IndexRead};
    let store = AccountIndexStore::new(account::get_data_dir().map_err(|_| STORAGE)?);
    match store.load().map_err(|_| STORAGE)? {
        IndexRead::Active(index) | IndexRead::Legacy(index) => Ok(index),
        IndexRead::Missing => Ok(crate::models::account::AccountIndex::default()),
        IndexRead::InvalidLegacy(_) => Err(STORAGE.into()),
    }
}
fn checked_account(id: &str) -> Result<Account, String> {
    normalize_id(id)?;
    // Read-only membership lookup: never acquire the index write lock while holding
    // the proxy store lock (account deletion acquires those locks in the opposite order).
    // An orphan detail file is not proof of ownership after deletion.
    if !read_index()?.accounts.iter().any(|a| a.id == id) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    account::load_account(id).map_err(|_| "PROXY_ACCOUNT_UNSUPPORTED".into())
}
async fn stored(id: &str) -> Result<(Account, Option<Setting>), String> {
    let id = normalize_id(id)?;
    io(move || Ok((checked_account(&id)?, read()?.targets.remove(&id)))).await
}
async fn check_reference(setting: &Setting) -> Result<(), String> {
    if setting.error.is_some() {
        return Err(STALE.into());
    }
    if let Some(reference) = &setting.reference {
        if !catalog::reference_exists(&reference.source_id, &reference.item_id).await? {
            return Err(STALE.into());
        }
    }
    Ok(())
}
fn view(id: String, legacy: bool, setting: Option<&Setting>, error: Option<String>) -> View {
    View {
        account_id: id,
        binding: setting
            .and_then(|s| s.snapshot.as_deref())
            .and_then(binding::summary),
        error,
        legacy_bound: setting.is_none() && legacy,
    }
}
pub async fn get(account_id: String) -> Result<View, String> {
    let id = normalize_id(&account_id)?;
    let (account, setting) = stored(&id).await?;
    let error = match &setting {
        Some(s) => check_reference(s).await.err(),
        None => None,
    };
    Ok(view(
        id,
        account
            .egress_proxy_url
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty()),
        setting.as_ref(),
        error,
    ))
}
pub async fn list() -> Result<Vec<View>, String> {
    let ids = io(|| {
        Ok(read_index()?
            .accounts
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>())
    })
    .await?;
    let mut views = Vec::with_capacity(ids.len());
    for id in ids {
        views.push(get(id).await?);
    }
    Ok(views)
}

pub async fn bind(
    account_id: String,
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<View, String> {
    // Serialize a new binding with the cross-account no-restart safety check:
    // it must not become configured between that check and credential injection.
    let _switch = account::try_begin_antigravity_switch(&account::ANTIGRAVITY_SWITCH_GATE)?;
    let id = normalize_id(&account_id)?;
    stored(&id).await?;
    let _source = catalog::SourceGuard::new(source_id.clone())?;
    let snapshot = catalog::snapshot_with_group(
        source_id.clone(),
        item_id,
        selections.clone(),
        group_id.clone(),
    )
    .await?;
    let decoded = binding::decode(&snapshot)?;
    let reference = super::codex_unified_proxy::Reference {
        source_id,
        source_name: decoded.source_name,
        item_id: decoded.item_id,
        group_id,
        name: decoded.name,
        selections,
    };
    let runtime_id = route_id(&id);
    let candidate =
        super::codex_proxy_runtime::prepare_client_binding(&runtime_id, Some(&snapshot)).await?;
    let guard = tokio::time::timeout(
        Duration::from_secs(10),
        route_lock(&runtime_id).lock_owned(),
    )
    .await
    .map_err(|_| "CATALOG_BUSY")?;
    // Install before the durable write: listener failures leave the previous binding intact.
    super::codex_proxy_desktop_router::ensure_account_entry(&runtime_id).await?;
    let setting = Setting {
        snapshot: Some(snapshot.clone()),
        reference: Some(reference),
        error: None,
        revision: 0,
    };
    let write_id = id.clone();
    let returned = setting.clone();
    let guard = io(move || {
        mutate(|store| {
            checked_account(&write_id)?;
            let mut setting = setting;
            setting.revision = store
                .targets
                .get(&write_id)
                .map_or(1, |s| s.revision.wrapping_add(1));
            store.targets.insert(write_id, setting);
            Ok(())
        })?;
        Ok(guard)
    })
    .await?;
    super::codex_proxy_runtime::publish_client_binding(&runtime_id, Some(&snapshot), candidate)
        .await?;
    drop(guard);
    Ok(view(id, false, Some(&returned), None))
}
pub async fn clear(account_id: String) -> Result<View, String> {
    let id = normalize_id(&account_id)?;
    stored(&id).await?;
    let runtime_id = route_id(&id);
    let candidate = super::codex_proxy_runtime::prepare_client_binding(&runtime_id, None).await?;
    let guard = tokio::time::timeout(
        Duration::from_secs(10),
        route_lock(&runtime_id).lock_owned(),
    )
    .await
    .map_err(|_| "CATALOG_BUSY")?;
    let write_id = id.clone();
    let guard = io(move || {
        mutate(|store| {
            checked_account(&write_id)?;
            let revision = store
                .targets
                .get(&write_id)
                .map_or(1, |s| s.revision.wrapping_add(1));
            // Presence is a tombstone for legacy URLs: stale account refreshes cannot resurrect them.
            store.targets.insert(
                write_id,
                Setting {
                    revision,
                    ..Default::default()
                },
            );
            Ok(())
        })?;
        Ok(guard)
    })
    .await?;
    super::codex_proxy_runtime::publish_client_binding(&runtime_id, None, candidate).await?;
    drop(guard);
    Ok(view(id, false, Some(&Setting::default()), None))
}

fn projection(id: &str, raw: Option<String>) -> CodexAccount {
    let mut account = CodexAccount::new(
        route_id(id),
        "antigravity-proxy@local.invalid".into(),
        CodexTokens {
            access_token: String::new(),
            refresh_token: None,
            id_token: String::new(),
        },
    );
    // Explicitly disabled means 'no route' internally, NOT a direct launch. It fences Codex unified fallback.
    account.egress_proxy_disabled = raw.is_none();
    account.egress_proxy_url = raw;
    account
}
pub(crate) async fn load_route(id: &str) -> Result<CodexAccount, String> {
    let (_, setting) = stored(id).await?;
    let Some(setting) = setting else {
        return Ok(projection(id, None));
    };
    check_reference(&setting).await?;
    Ok(projection(id, setting.snapshot))
}
pub async fn has_managed_binding(account_id: &str) -> Result<bool, String> {
    let (_, setting) = stored(account_id).await?;
    Ok(setting.is_some_and(|setting| setting.snapshot.is_some() || setting.error.is_some()))
}

pub async fn launch_url(account_id: &str) -> Result<Option<String>, String> {
    let (account, setting) = stored(account_id).await?;
    let Some(setting) = setting else {
        return account
            .egress_proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(super::codex_account_proxy::normalize_direct_proxy)
            .transpose();
    };
    check_reference(&setting).await?;
    if setting.snapshot.is_none() {
        return Ok(None);
    }
    let id = route_id(&account.id);
    super::codex_proxy_runtime::desktop_target(&id)
        .await?
        .ok_or(STALE)?;
    let port = super::codex_proxy_desktop_router::ensure(&id)
        .await?
        .ok_or(STALE)?;
    // HTTP CONNECT is supported by both the Go CLI and the desktop's proxy environment.
    Ok(Some(format!("http://127.0.0.1:{port}")))
}

pub async fn dependencies(source_id: &str) -> Result<Vec<Dependency>, String> {
    let source_id = source_id.to_owned();
    io(move || {
        let settings = read()?
            .targets
            .into_iter()
            .filter(|(_, s)| {
                s.reference
                    .as_ref()
                    .is_some_and(|r| r.source_id == source_id)
            })
            .collect::<Vec<_>>();
        if settings.is_empty() {
            return Ok(Vec::new());
        }
        let index = read_index()?;
        let names: BTreeMap<_, _> = index
            .accounts
            .into_iter()
            .map(|a| (a.id, a.name.unwrap_or(a.email)))
            .collect();
        Ok(settings
            .into_iter()
            .filter_map(|(id, _)| {
                names.get(&id).map(|name| Dependency {
                    target_id: route_id(&id),
                    name: format!("Antigravity · {name}"),
                })
            })
            .collect())
    })
    .await
}
/// Refresh checks identity only. Saved credentials and policy members never change implicitly.
pub async fn resync_source(source_id: &str, removed: bool) -> Result<(), String> {
    let source_id = source_id.to_owned();
    let source_for_read = source_id.clone();
    let settings = io(move || {
        Ok(read()?
            .targets
            .into_iter()
            .filter(|(_, s)| {
                s.reference
                    .as_ref()
                    .is_some_and(|r| r.source_id == source_for_read)
            })
            .collect::<Vec<_>>())
    })
    .await?;
    for (id, setting) in settings {
        let reference = setting.reference.as_ref().ok_or(STORAGE)?;
        let missing =
            removed || !catalog::reference_exists(&reference.source_id, &reference.item_id).await?;
        if !missing {
            continue;
        }
        let runtime_id = route_id(&id);
        let guard = route_lock(&runtime_id).lock_owned().await;
        let candidate =
            super::codex_proxy_runtime::prepare_client_binding(&runtime_id, None).await?;
        let write_id = id.clone();
        let revision = setting.revision;
        let (guard, changed) = io(move || {
            let changed = mutate(|store| {
                if let Some(current) = store
                    .targets
                    .get_mut(&write_id)
                    .filter(|s| s.revision == revision)
                {
                    current.error = Some(STALE.into());
                    current.revision = current.revision.wrapping_add(1);
                    return Ok(true);
                }
                Ok(false)
            })?;
            Ok((guard, changed))
        })
        .await?;
        if changed {
            // Do not remove the slot: a concurrently prepared candidate may already
            // reference it. Replacing its owner lease preserves old in-flight streams.
            super::codex_proxy_runtime::publish_client_binding(&runtime_id, None, candidate)
                .await?;
        }
        drop(guard);
    }
    Ok(())
}
/// Called after account index removal; remaining detail files no longer prove membership.
pub(crate) fn account_deleted(account_id: &str) -> Result<(), String> {
    super::codex_proxy_runtime::release_deleted_account(&route_id(account_id));
    if !path()?.try_exists().map_err(|_| STORAGE)? {
        return Ok(());
    }
    mutate(|store| {
        store.targets.remove(account_id);
        Ok(())
    })
}

#[cfg(test)]
#[path = "antigravity_proxy_tests.rs"]
mod regression_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_identity_is_distinct_and_rejects_path_injection() {
        assert_eq!(route_target(&route_id("a")), Some("a"));
        assert_ne!(
            route_id("a"),
            super::super::codex_client_proxy::route_id("a")
        );
        for id in ["", "../x", "a/b", "a\\b", "a:b", "\n"] {
            assert!(normalize_id(id).is_err());
        }
    }
    #[test]
    fn token_free_projection_fences_codex_unified_fallback() {
        let account = projection("example", None);
        assert!(account.tokens.access_token.is_empty());
        assert!(account.tokens.id_token.is_empty());
        assert!(account.tokens.refresh_token.is_none());
        assert_eq!(
            super::super::codex_account_proxy::configured_url(&account).unwrap(),
            None
        );
        assert!(account.egress_proxy_disabled);
    }
    #[test]
    fn tombstone_hides_legacy_bound_state() {
        let default = Setting::default();
        assert!(view("a".into(), true, None, None).legacy_bound);
        let cleared = view("a".into(), true, Some(&default), None);
        assert!(!cleared.legacy_bound);
        assert!(cleared.binding.is_none());
    }
    #[test]
    fn view_never_serializes_snapshot_or_legacy_url() {
        let setting = Setting {
            snapshot: Some("http://user:TOPSECRET@proxy.invalid:8080".into()),
            ..Default::default()
        };
        let json =
            serde_json::to_string(&view("a".into(), true, Some(&setting), Some(STALE.into())))
                .unwrap();
        for secret in ["TOPSECRET", "proxy.invalid", "snapshot", "http://"] {
            assert!(!json.contains(secret));
        }
    }
}
