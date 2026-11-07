//! Recover only listeners still referenced by this host's running desktops.
//! No standalone service, client restart, config rewrite, or upstream probe.
use super::{
    entry, logger, processes, read_registry, registry_path, restore_exact_port, PortRegistry,
};
use crate::models::InstanceStore;
use crate::modules::{app_lifecycle, codex_account_proxy, codex_proxy_runtime, process};
use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};

// A timed-out disk/process scan keeps its permit until the blocking work actually exits.
static SCANS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
static RECOVERY: Mutex<()> = Mutex::const_new(());
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, PartialEq, Eq)]
struct Target {
    account_id: String,
    port: u16,
}

fn select_targets(
    registry: &PortRegistry,
    store: &InstanceStore,
    running: &[processes::RunningEntry],
    matches_profile: impl Fn(&processes::RunningEntry, Option<&str>) -> bool,
) -> Vec<Target> {
    let mut ports = BTreeMap::<u16, Option<String>>::new();
    for entry in running {
        // None is essential: official default desktop processes have no CODEX_HOME.
        // Matching the saved launch PID also keeps dev/release and copied
        // instance settings from adopting each other's running clients.
        let owned = (store.default_settings.last_pid == Some(entry.pid)
            && matches_profile(entry, None))
            || store.instances.iter().any(|instance| {
                instance.last_pid == Some(entry.pid)
                    && !instance.user_data_dir.trim().is_empty()
                    && matches_profile(entry, Some(&instance.user_data_dir))
            });
        if !owned {
            continue;
        }
        // The store's next-launch binding/mode may have changed while the desktop
        // remained alive. Its real port identifies the original account route.
        for account_id in registry
            .ports
            .keys()
            .filter(|id| registry.recorded(id) == Some(entry.proxy_port))
            .cloned()
        {
            ports
                .entry(entry.proxy_port)
                .and_modify(|existing| {
                    if existing.as_ref() != Some(&account_id) {
                        *existing = None;
                    }
                })
                .or_insert(Some(account_id));
        }
    }
    ports
        .into_iter()
        .filter_map(|(port, account_id)| account_id.map(|account_id| Target { account_id, port }))
        .collect()
}

fn collect_targets() -> Result<Vec<Target>, String> {
    let path = registry_path()?;
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > 4 * 1024 * 1024) {
        return Err("PROXY_ENTRY_RECOVERY_FAILED".into());
    }
    let registry = read_registry(&path);
    if registry.ports.is_empty() {
        return Ok(Vec::new());
    }
    // Read only. Recovery must not migrate/quarantine or rewrite the instance store.
    let store_path = path
        .parent()
        .ok_or("PROXY_ENTRY_RECOVERY_FAILED")?
        .join("codex_instances.json");
    if fs::metadata(&store_path).is_ok_and(|meta| meta.len() > 4 * 1024 * 1024) {
        return Err("PROXY_ENTRY_RECOVERY_FAILED".into());
    }
    let bytes = match fs::read(&store_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("PROXY_ENTRY_RECOVERY_FAILED".into()),
    };
    let store: InstanceStore =
        serde_json::from_slice(&bytes).map_err(|_| "PROXY_ENTRY_RECOVERY_FAILED")?;
    let running = processes::running_entries()?;
    Ok(select_targets(
        &registry,
        &store,
        &running,
        |entry, home| process::codex_proxy_profile_matches(entry.profile_dir.as_deref(), home),
    ))
}

async fn scan_targets() -> Result<Vec<Target>, String> {
    let permit = SCANS
        .clone()
        .try_acquire_owned()
        .map_err(|_| "PROXY_ENTRY_RECOVERY_BUSY")?;
    tokio::time::timeout(
        SCAN_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            collect_targets()
        }),
    )
    .await
    .map_err(|_| "PROXY_ENTRY_RECOVERY_TIMEOUT")?
    .map_err(|_| "PROXY_ENTRY_RECOVERY_FAILED")?
}

async fn restore_target(target: &Target) -> Result<(), String> {
    if app_lifecycle::is_shutdown_started() {
        return Err("PROXY_ENTRY_RECOVERY_FAILED".into());
    }
    // This read is bounded and single-flight gated by the runtime read semaphore.
    let account = codex_proxy_runtime::load(&target.account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ENTRY_NOT_RUNNING".into());
    }
    // Recover even after unbinding: an already-running desktop still uses this entry.
    // The existing per-connection resolver handles shared/individual/direct modes.
    restore_exact_port(&target.account_id, target.port).await
}

pub async fn restore_on_startup() {
    let Ok(_guard) = RECOVERY.try_lock() else {
        return;
    };
    let mut targets = match scan_targets().await {
        Ok(targets) => targets,
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexProxy] 桌面代理入口启动恢复扫描失败: {error}"
            ));
            return;
        }
    };
    for attempt in 0..3 {
        let mut failed = Vec::new();
        for target in targets {
            if app_lifecycle::is_shutdown_started() {
                return;
            }
            match restore_target(&target).await {
                Ok(()) => logger::log_info(&format!(
                    "[CodexProxy] 已恢复桌面代理入口: account={}, port={}",
                    target.account_id, target.port
                )),
                Err(error) => {
                    if super::entry_status(&target.account_id).is_none() {
                        entry::recovery_failed(&target.account_id, target.port, &error);
                    }
                    logger::log_warn(&format!("[CodexProxy] 桌面代理入口恢复失败: account={}, port={}, attempt={}, error={error}", target.account_id, target.port, attempt + 1));
                    failed.push(target);
                }
            }
            tokio::task::yield_now().await;
        }
        if failed.is_empty() || attempt == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Recheck real processes before retrying; do not reopen an entry for a
        // desktop that exited or changed profiles during the upgrade window.
        targets = match scan_targets().await {
            Ok(current) => current
                .into_iter()
                .filter(|target| failed.contains(target))
                .collect(),
            Err(error) => {
                logger::log_warn(&format!(
                    "[CodexProxy] 桌面代理入口恢复重试扫描失败: {error}"
                ));
                return;
            }
        };
    }
}

pub async fn restore_account_entry(account_id: &str) -> Result<(), String> {
    let _guard = RECOVERY
        .try_lock()
        .map_err(|_| "PROXY_ENTRY_RECOVERY_BUSY")?;
    let target = scan_targets()
        .await?
        .into_iter()
        .find(|target| target.account_id == account_id)
        .ok_or("PROXY_ENTRY_NOT_RUNNING")?;
    restore_target(&target).await
}

#[cfg(test)]
#[path = "codex_proxy_desktop_recovery_tests.rs"]
mod tests;
