//! Card-specific desktop egress. Never consumed by model/upstream routing.
use super::{
    account, atomic_write, codex_account_proxy, codex_proxy_catalog_binding as binding,
    secure_account_storage,
};
use crate::models::codex::{CodexAccount, CodexTokens};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

pub const ROUTE_PREFIX: &str = "__client_proxy__:";
const STORE: &str = "codex-client-proxies.json";
const ERROR: &str = "CLIENT_PROXY_STORAGE";
static WRITES: Mutex<()> = Mutex::new(());
static IO: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Inherit,
    BoundOauth,
    Independent,
    Direct,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Setting {
    mode: Mode,
    snapshot: Option<String>,
    reference: Option<super::codex_unified_proxy::Reference>,
    stale_error: Option<String>,
    revision: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct Store {
    version: u32,
    targets: BTreeMap<String, Setting>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub target_id: String,
    pub mode: Mode,
    pub binding: Option<serde_json::Value>,
    pub stale_error: Option<String>,
}

fn path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir().map_err(|_| ERROR)?.join(STORE))
}
fn read() -> Result<Store, String> {
    let path = path()?;
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Store {
                version: 1,
                ..Default::default()
            })
        }
        Ok(meta) if meta.is_file() && meta.len() <= 4 * 1024 * 1024 => (),
        _ => return Err(ERROR.into()),
    }
    let key = path
        .parent()
        .ok_or(ERROR)?
        .join("secure-account-storage.key");
    let store: Store =
        secure_account_storage::read_account_file_readonly(&path, &key).map_err(|_| ERROR)?;
    if store.version != 1 || store.targets.len() > 1024 {
        return Err(ERROR.into());
    }
    for setting in store.targets.values() {
        if setting.mode == Mode::Independent
            && setting
                .snapshot
                .as_deref()
                .is_none_or(|v| v.trim().is_empty())
        {
            return Err(ERROR.into());
        }
    }
    Ok(store)
}
fn write(store: &Store) -> Result<(), String> {
    let path = path()?;
    if store.targets.len() > 1024 {
        return Err(ERROR.into());
    }
    let encrypted = secure_account_storage::serialize_account_file("codex-client-proxies", store)
        .map_err(|_| ERROR)?;
    if encrypted.len() > 4 * 1024 * 1024 {
        return Err(ERROR.into());
    }
    atomic_write::write_secret_string_atomic(&path, &encrypted).map_err(|_| ERROR.to_string())
}
async fn io<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = tokio::time::timeout(Duration::from_secs(3), IO.clone().acquire_owned())
        .await
        .map_err(|_| "CLIENT_PROXY_TIMEOUT")?
        .map_err(|_| ERROR)?;
    tokio::time::timeout(
        Duration::from_secs(4),
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        }),
    )
    .await
    .map_err(|_| "CLIENT_PROXY_TIMEOUT")?
    .map_err(|_| ERROR.to_string())?
}
pub fn normalize_target(raw: &str) -> Result<String, String> {
    let target = super::codex_instance::parse_provider_gateway_bind_account_id(raw)
        .unwrap_or_else(|| raw.trim().to_string());
    if target.is_empty()
        || target.contains(['/', '\\'])
        || matches!(target.as_str(), "." | "..")
        || target.len() > 256
        || target.chars().any(char::is_control)
        || target.starts_with(ROUTE_PREFIX)
    {
        return Err("CLIENT_PROXY_TARGET_UNSUPPORTED".into());
    }
    Ok(target)
}
pub fn route_id(target: &str) -> String {
    format!("{ROUTE_PREFIX}{target}")
}
pub fn route_target(id: &str) -> Option<&str> {
    id.strip_prefix(ROUTE_PREFIX)
}
pub(crate) async fn target_account(target: &str) -> Result<Option<CodexAccount>, String> {
    if super::codex_instance::is_api_service_bind_account_id(target) {
        return Ok(None);
    }
    let target = target.to_owned();
    let account = io(move || {
        super::codex_account::load_account(&target)
            .ok_or_else(|| "CLIENT_PROXY_TARGET_UNSUPPORTED".into())
    })
    .await?;
    if !account.is_api_key_auth() {
        return Err("CLIENT_PROXY_TARGET_UNSUPPORTED".into());
    }
    Ok(Some(account))
}
async fn setting(target: &str) -> Result<Setting, String> {
    let target = target.to_owned();
    io(move || Ok(read()?.targets.remove(&target).unwrap_or_default())).await
}
pub(crate) async fn modes() -> Result<BTreeMap<String, Mode>, String> {
    io(|| {
        Ok(read()?
            .targets
            .into_iter()
            .map(|(id, setting)| (id, setting.mode))
            .collect())
    })
    .await
}

async fn read_bound_oauth_with_timeout(
    reader: impl std::future::Future<Output = Result<Option<String>, String>>,
    deadline: Duration,
) -> Result<Option<String>, String> {
    tokio::time::timeout(deadline, reader)
        .await
        .map_err(|_| "CLIENT_PROXY_TIMEOUT")?
        .map_err(|_| ERROR.to_string())
}

async fn bound_account_id(target: &str) -> Result<String, String> {
    match target_account(target).await? {
        Some(account) => account.bound_oauth_account_id,
        None => {
            read_bound_oauth_with_timeout(
                super::codex_local_access::client_proxy_bound_oauth_account_id(),
                Duration::from_secs(7),
            )
            .await?
        }
    }
    .filter(|id| !id.trim().is_empty())
    .ok_or_else(|| "CLIENT_PROXY_BOUND_OAUTH_MISSING".to_string())
}

async fn bound_snapshot(target: &str) -> Result<String, String> {
    let bound = bound_account_id(target).await?;
    let account = super::codex_proxy_runtime::load(&bound)
        .await
        .map_err(|error| {
            if error == "PROXY_ACCOUNT_UNSUPPORTED" {
                "CLIENT_PROXY_BOUND_OAUTH_MISSING".to_string()
            } else {
                error
            }
        })?;
    if !codex_account_proxy::eligible(&account) {
        return Err("CLIENT_PROXY_BOUND_OAUTH_MISSING".into());
    }
    codex_account_proxy::configured_url(&account)?
        .map(|v| v.into_owned())
        .ok_or_else(|| "CLIENT_PROXY_BOUND_PROXY_MISSING".into())
}
fn summary(raw: &str) -> Option<serde_json::Value> {
    let mut account = synthetic("summary", Some(raw.to_string()));
    account.egress_proxy_disabled = false;
    serde_json::to_value(account)
        .ok()?
        .get("egress_proxy")
        .cloned()
        .filter(|v| !v.is_null())
}
fn view(target: String, setting: &Setting, raw: Option<&str>) -> View {
    let mut safe = raw.and_then(summary);
    if let (Some(value), Some(reference)) = (
        safe.as_mut().and_then(|v| v.as_object_mut()),
        setting.reference.as_ref(),
    ) {
        value.insert("selections".into(), serde_json::json!(reference.selections));
    }
    View {
        target_id: target,
        mode: setting.mode,
        binding: safe,
        stale_error: setting.stale_error.clone(),
    }
}
async fn validate_reference(setting: &Setting) -> Result<(), String> {
    if let Some(reference) = &setting.reference {
        // A deleted source must fail closed even if marking its stored snapshot stale failed.
        super::codex_proxy_catalog::snapshot_with_group(
            reference.source_id.clone(),
            reference.item_id.clone(),
            reference.selections.clone(),
            reference.group_id.clone(),
        )
        .await
        .map_err(|_| "CLIENT_PROXY_STALE")?;
    }
    Ok(())
}
pub async fn get(raw_target: String) -> Result<View, String> {
    let target = normalize_target(&raw_target)?;
    target_account(&target).await?;
    let setting = setting(&target).await?;
    if setting.mode == Mode::BoundOauth {
        return match bound_snapshot(&target).await {
            Ok(raw) => Ok(view(target, &setting, Some(&raw))),
            Err(error) => {
                let mut v = view(target, &setting, None);
                v.stale_error = Some(error);
                Ok(v)
            }
        };
    }
    let mut response = view(target, &setting, setting.snapshot.as_deref());
    if setting.mode == Mode::Independent && validate_reference(&setting).await.is_err() {
        response.stale_error = Some("CLIENT_PROXY_STALE".into());
    }
    Ok(response)
}

pub async fn save(
    raw_target: String,
    mode: Mode,
    proxy_url: Option<String>,
    source_id: Option<String>,
    item_id: Option<String>,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<View, String> {
    let target = normalize_target(&raw_target)?;
    target_account(&target).await?;
    let source_for_guard = source_id.clone().or_else(|| {
        proxy_url
            .as_deref()
            .and_then(|raw| binding::decode(raw).ok())
            .map(|value| value.source_id)
    });
    let _source_guard = source_for_guard
        .map(super::codex_proxy_catalog::SourceGuard::new)
        .transpose()?;
    let (snapshot, reference) = if mode == Mode::Independent {
        if let Some(source) = source_id {
            if proxy_url.is_some() {
                return Err("PROXY_INVALID_URL".into());
            }
            let raw = super::codex_proxy_catalog::snapshot_with_group(
                source.clone(),
                item_id.ok_or("CLIENT_PROXY_STALE")?,
                selections.clone(),
                group_id.clone(),
            )
            .await?;
            let decoded = binding::decode(&raw)?;
            let reference = super::codex_unified_proxy::Reference {
                source_id: source,
                source_name: decoded.source_name,
                item_id: decoded.item_id,
                group_id,
                name: decoded.name,
                selections,
            };
            (Some(raw), Some(reference))
        } else {
            let raw = match proxy_url.filter(|v| !v.trim().is_empty()) {
                Some(raw) => raw,
                None => {
                    let old = setting(&target).await?;
                    if old.mode != Mode::Independent || old.reference.is_some() {
                        return Err("PROXY_INVALID_URL".into());
                    }
                    old.snapshot.ok_or("PROXY_INVALID_URL")?
                }
            };
            (
                Some(super::codex_proxy_runtime::normalize_binding(&raw)?),
                reference_for_snapshot(&raw).await?,
            )
        }
    } else {
        (None, None)
    };
    let effective = if mode == Mode::BoundOauth {
        Some(bound_snapshot(&target).await?)
    } else {
        snapshot.clone()
    };
    let id = route_id(&target);
    let candidate =
        super::codex_proxy_runtime::prepare_client_binding(&id, effective.as_deref()).await?;
    let lock = super::codex_account::codex_token_lock_for(&id);
    let token_guard = tokio::time::timeout(Duration::from_secs(10), lock.lock_owned())
        .await
        .map_err(|_| "CLIENT_PROXY_TIMEOUT")?;
    let target_for_write = target.clone();
    let next = Setting {
        mode,
        snapshot,
        reference,
        stale_error: None,
        revision: 0,
    };
    let returned = next.clone();
    // Keep the transaction lock in the worker: cancelled callers cannot overlap a later save.
    let token_guard = io(move || {
        let _write = WRITES.lock().map_err(|_| ERROR)?;
        let mut store = read()?;
        let mut next = next;
        next.revision = store
            .targets
            .get(&target_for_write)
            .map_or(1, |old| old.revision.wrapping_add(1));
        store.targets.insert(target_for_write, next);
        write(&store)?;
        Ok(token_guard)
    })
    .await?;
    super::codex_proxy_runtime::publish_client_binding(&id, effective.as_deref(), candidate)
        .await?;
    drop(token_guard);
    Ok(view(target, &returned, effective.as_deref()))
}

fn synthetic(id: &str, raw: Option<String>) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.to_string(),
        "client-proxy@local.invalid".into(),
        CodexTokens {
            access_token: String::new(),
            refresh_token: None,
            id_token: String::new(),
        },
    );
    account.egress_proxy_disabled = false;
    account.egress_proxy_url = raw;
    account
}
/// Synthetic identities exist only in the desktop router, never in account files or upstream pools.
pub(crate) async fn load_route(target: &str) -> Result<CodexAccount, String> {
    target_account(target).await?;
    let setting = setting(target).await?;
    if setting.stale_error.is_some() {
        return Err("CLIENT_PROXY_STALE".into());
    }
    let raw = match setting.mode {
        Mode::Independent => {
            validate_reference(&setting).await?;
            Some(setting.snapshot.ok_or("CLIENT_PROXY_STALE")?)
        }
        Mode::BoundOauth => Some(bound_snapshot(target).await?),
        Mode::Inherit | Mode::Direct => None,
    };
    let mut account = synthetic(&route_id(target), raw);
    account.egress_proxy_disabled = setting.mode == Mode::Direct;
    Ok(account)
}
pub async fn resolve(target: &str) -> Result<super::process::CodexLaunchProxy, String> {
    use super::process::CodexLaunchProxy;
    let target = normalize_target(target)?;
    target_account(&target).await?;
    match setting(&target).await?.mode {
        Mode::Inherit => {
            let account = load_route(&target).await?;
            super::codex_proxy_runtime::ensure_account_proxy_state(&account).await?;
            if codex_account_proxy::configured_url(&account)?.is_none() {
                return Ok(CodexLaunchProxy::Inherit);
            }
            resolve_prepared_route(&target, &account).await
        }
        Mode::Direct => Ok(CodexLaunchProxy::Direct),
        Mode::BoundOauth | Mode::Independent => {
            let account = load_route(&target).await?;
            resolve_prepared_route(&target, &account).await
        }
    }
}

async fn resolve_prepared_route(
    target: &str,
    account: &CodexAccount,
) -> Result<super::process::CodexLaunchProxy, String> {
    let value = codex_account_proxy::configured_url(account)?.ok_or("CLIENT_PROXY_STALE")?;
    super::codex_proxy_engine_preflight::for_url(
        value.as_ref(),
        super::codex_proxy_engine_preflight::Usage::Desktop,
    )
    .await?;
    let id = route_id(target);
    super::codex_proxy_runtime::desktop_target(&id)
        .await?
        .ok_or("CLIENT_PROXY_STALE")?;
    let url = super::codex_proxy_desktop_router::ensure(&id)
        .await?
        .ok_or("CLIENT_PROXY_STALE")?;
    Ok(super::process::CodexLaunchProxy::Proxy(url))
}

async fn reference_for_snapshot(
    raw: &str,
) -> Result<Option<super::codex_unified_proxy::Reference>, String> {
    if !raw.starts_with(binding::PREFIX) {
        return Ok(None);
    }
    let decoded = binding::decode(raw)?;
    let names = binding::names(raw)?;
    let catalog = serde_json::to_value(super::codex_proxy_catalog::list().await?)
        .map_err(|_| "CATALOG_FAILED")?;
    let source = catalog["sources"]
        .as_array()
        .and_then(|sources| {
            sources
                .iter()
                .find(|source| source["id"] == decoded.source_id)
        })
        .ok_or("CLIENT_PROXY_STALE")?;
    let mut selections = BTreeMap::new();
    for outbound in binding::outbounds(raw)? {
        let tag = outbound["tag"].as_str().unwrap_or_default();
        let selected = if outbound["type"] == "selector" {
            outbound["default"].as_str()
        } else if outbound["type"] == "mihomo-group" && outbound["group"]["type"] == "select" {
            outbound["group"]["proxies"][0].as_str()
        } else {
            None
        };
        if let Some(selected) = selected {
            let group_name = names.get(tag).ok_or("CLIENT_PROXY_STALE")?;
            let group_id = source["groups"]
                .as_array()
                .and_then(|groups| groups.iter().find(|group| group["name"] == *group_name))
                .and_then(|group| group["id"].as_str())
                .ok_or("CLIENT_PROXY_STALE")?;
            let selected_name = names.get(selected).map(String::as_str).unwrap_or(selected);
            selections.insert(group_id.to_string(), selected_name.to_string());
        }
    }
    Ok(Some(super::codex_unified_proxy::Reference {
        source_id: decoded.source_id,
        source_name: decoded.source_name,
        item_id: decoded.item_id,
        group_id: decoded.group_id,
        name: decoded.name,
        selections,
    }))
}

/// Minimal, token-free view for the existing account proxy UI. Not a stored account.
#[derive(Serialize)]
pub struct TargetView {
    #[serde(flatten)]
    pub account: CodexAccount,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_proxy_error: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub egress_proxy_legacy_bound: bool,
}
pub async fn target_view(raw: &str) -> Result<TargetView, String> {
    let target = normalize_target(raw)?;
    let source = target_account(&target).await?;
    let current = setting(&target).await?;
    let (raw, error) = if current.mode == Mode::BoundOauth {
        match bound_snapshot(&target).await {
            Ok(raw) => (Some(raw), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        let error = current.stale_error.clone().or(
            if current.mode == Mode::Independent && validate_reference(&current).await.is_err() {
                Some("CLIENT_PROXY_STALE".into())
            } else {
                None
            },
        );
        (current.snapshot, error)
    };
    let mut account = synthetic(&target, raw);
    account.email.clear();
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    account.egress_proxy_disabled = current.mode == Mode::Direct;
    if current.mode == Mode::BoundOauth {
        account.bound_oauth_account_id = bound_account_id(&target).await.ok();
    }
    if let Some(source) = source {
        account.bound_oauth_account_id = source.bound_oauth_account_id;
        account.email = source.email;
        account.account_name = source.account_name;
        account.api_provider_name = source.api_provider_name;
        account.plan_type = source.plan_type;
        account.created_at = source.created_at;
        account.last_used = source.last_used;
    }
    Ok(TargetView {
        account,
        egress_proxy_error: error,
        egress_proxy_legacy_bound: current.mode == Mode::BoundOauth,
    })
}

pub async fn save_account_binding(
    target: String,
    input: Option<String>,
    disabled: bool,
) -> Result<CodexAccount, String> {
    if disabled && input.is_some() {
        return Err("PROXY_INVALID_URL".into());
    }
    let input = input.filter(|v| !v.trim().is_empty());
    let mode = if disabled {
        Mode::Direct
    } else if input.is_some() {
        Mode::Independent
    } else {
        Mode::Inherit
    };
    save(
        target.clone(),
        mode,
        input,
        None,
        None,
        BTreeMap::new(),
        None,
    )
    .await?;
    Ok(target_view(&target).await?.account)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub target_id: String,
    pub name: String,
}
pub async fn dependencies(source_id: &str) -> Result<Vec<Dependency>, String> {
    let source = source_id.to_owned();
    io(move || {
        Ok(read()?
            .targets
            .into_iter()
            .filter(|(_, s)| s.reference.as_ref().is_some_and(|r| r.source_id == source))
            .map(|(target_id, _)| {
                let name = if super::codex_instance::is_api_service_bind_account_id(&target_id) {
                    String::new()
                } else {
                    super::codex_account::load_account(&target_id)
                        .and_then(|a| a.account_name)
                        .unwrap_or_else(|| target_id.clone())
                };
                Dependency { target_id, name }
            })
            .collect())
    })
    .await
}

fn apply_resync(
    store: &mut Store,
    target: &str,
    revision: u64,
    rebuilt: Result<String, String>,
) -> bool {
    let Some(current) = store
        .targets
        .get_mut(target)
        .filter(|current| current.revision == revision)
    else {
        return false;
    };
    match rebuilt {
        Ok(raw) => {
            current.snapshot = Some(raw);
            current.stale_error = None;
        }
        Err(error) => current.stale_error = Some(error),
    }
    current.revision = current.revision.wrapping_add(1);
    true
}
fn apply_removal(store: &mut Store, target: &str, revision: u64) -> bool {
    let Some(current) = store
        .targets
        .get_mut(target)
        .filter(|current| current.revision == revision)
    else {
        return false;
    };
    // Explicit source deletion has the same confirmed unbind semantics as an
    // ordinary account. Automatic refresh failure retains its stale reference.
    let revision = current.revision.wrapping_add(1);
    *current = Setting {
        revision,
        ..Default::default()
    };
    true
}

/// Refresh failures preserve a stale reference; confirmed source deletion unbinds it.
pub async fn resync_source(source_id: &str, removed: bool) -> Result<(), String> {
    let source = source_id.to_owned();
    let observed = io(move || {
        Ok(read()?
            .targets
            .into_iter()
            .filter(|(_, setting)| {
                setting
                    .reference
                    .as_ref()
                    .is_some_and(|r| r.source_id == source)
            })
            .collect::<Vec<_>>())
    })
    .await?;
    for (target, previous) in observed {
        let reference = previous.reference.as_ref().ok_or("CLIENT_PROXY_STALE")?;
        let rebuilt = if removed {
            Err("CATALOG_NOT_FOUND".to_string())
        } else {
            super::codex_proxy_catalog::snapshot_with_group(
                reference.source_id.clone(),
                reference.item_id.clone(),
                reference.selections.clone(),
                reference.group_id.clone(),
            )
            .await
        };
        let id = route_id(&target);
        let guard = tokio::time::timeout(
            Duration::from_secs(10),
            super::codex_account::codex_token_lock_for(&id).lock_owned(),
        )
        .await
        .map_err(|_| "CLIENT_PROXY_TIMEOUT")?;
        let (changed, guard) = io(move || {
            let _write = WRITES.lock().map_err(|_| ERROR)?;
            let mut store = read()?;
            let changed = if removed {
                apply_removal(&mut store, &target, previous.revision)
            } else {
                apply_resync(&mut store, &target, previous.revision, rebuilt)
            };
            if !changed {
                return Ok((false, guard));
            }
            write(&store)?;
            Ok((true, guard))
        })
        .await?;
        if changed {
            // Publication is under the same target lock as saves. A late source
            // refresh must not stop an engine installed by a newer card setting.
            let candidate = super::codex_proxy_runtime::prepare_client_binding(&id, None).await?;
            super::codex_proxy_runtime::publish_client_binding(&id, None, candidate).await?;
        }
        drop(guard);
    }
    Ok(())
}

#[cfg(test)]
#[path = "codex_client_proxy_tests.rs"]
mod tests;
