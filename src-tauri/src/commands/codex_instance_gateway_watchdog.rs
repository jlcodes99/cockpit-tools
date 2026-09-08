use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify;

use super::codex_instance_gateway_watchdog_state::{process_context, Lease, WatchState};
use crate::models::{DefaultInstanceSettings, InstanceProfile, InstanceStore};
use crate::modules;

#[derive(Clone)]
enum Target {
    Default(DefaultInstanceSettings),
    Instance(InstanceProfile),
}

#[derive(Default)]
struct Control {
    app: Option<AppHandle>,
    initialized: bool,
    state: WatchState,
    targets: BTreeMap<String, Target>,
}
static CONTROL: LazyLock<Mutex<Control>> = LazyLock::new(|| Mutex::new(Control::default()));
static CHANGED: LazyLock<Notify> = LazyLock::new(Notify::new);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
fn now() -> u64 {
    START.elapsed().as_secs()
}

fn enabled_targets(store: &InstanceStore) -> BTreeMap<String, Target> {
    let mut targets = BTreeMap::new();
    if store
        .default_settings
        .model_routing
        .as_ref()
        .is_some_and(|r| r.enabled)
    {
        targets.insert(
            "__default__".into(),
            Target::Default(store.default_settings.clone()),
        );
    }
    for instance in &store.instances {
        if instance.model_routing.as_ref().is_some_and(|r| r.enabled) {
            targets.insert(instance.id.clone(), Target::Instance(instance.clone()));
        }
    }
    targets
}

fn update(control: &mut Control, store: &InstanceStore) -> bool {
    let targets = enabled_targets(store);
    let signatures = targets
        .iter()
        .map(|(id, target)| {
            let signature = match target {
                Target::Default(s) => serde_json::to_string(&(
                    &s.bind_account_id,
                    s.follow_local_account,
                    &s.model_routing,
                )),
                Target::Instance(s) => {
                    serde_json::to_string(&(&s.user_data_dir, &s.bind_account_id, &s.model_routing))
                }
            }
            .expect("routing settings are serializable");
            (id.clone(), signature)
        })
        .collect();
    control.targets = targets;
    control.initialized = true;
    control.state.update(signatures)
}

// Called only after successful persistence. No file reads, process scans or
// await while the instance store's writer lock is held.
pub(crate) fn instance_store_saved(store: &InstanceStore) {
    let mut c = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
    let changed = update(&mut c, store);
    let start = c.app.is_some() && c.state.arm();
    drop(c);
    if changed {
        CHANGED.notify_one();
    }
    if start {
        tauri::async_runtime::spawn(scheduler());
    }
}

pub fn start_mixed_model_gateway_watchdog(app: AppHandle) {
    {
        let mut c = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
        if c.app.is_some() {
            return;
        }
        c.app = Some(app);
    }
    // One startup read, not a permanent disabled polling loop. A concurrent
    // successful save wins over this potentially stale disk snapshot.
    tauri::async_runtime::spawn_blocking(|| {
        let snapshot = modules::codex_instance::load_instance_store();
        let mut c = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
        if !c.initialized {
            match snapshot {
                Ok(store) => {
                    update(&mut c, &store);
                }
                Err(error) => modules::logger::log_warn(&format!(
                    "[MixedModelRouting] 读取待恢复实例失败: {error}"
                )),
            }
        }
        let start = c.state.arm();
        drop(c);
        if start {
            tauri::async_runtime::spawn(scheduler());
        }
    });
}

fn current(lease: &Lease) -> bool {
    !modules::app_lifecycle::is_shutdown_started()
        && CONTROL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .state
            .current(lease)
}

struct Completion {
    lease: Lease,
    failed: bool,
}
impl Drop for Completion {
    fn drop(&mut self) {
        CONTROL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .state
            .complete(&self.lease, now(), self.failed);
        CHANGED.notify_one();
    }
}

async fn scheduler() {
    loop {
        let jobs = {
            let mut c = CONTROL.lock().unwrap_or_else(|e| e.into_inner());
            if c.state
                .retire_if_idle(modules::app_lifecycle::is_shutdown_started())
            {
                return;
            }
            c.state
                .due(now())
                .into_iter()
                .filter_map(|lease| {
                    Some((
                        lease.clone(),
                        c.targets.get(&lease.key)?.clone(),
                        c.app.clone()?,
                    ))
                })
                .collect::<Vec<_>>()
        };
        for (lease, target, app) in jobs {
            let mut task = tauri::async_runtime::spawn_blocking(move || {
                let mut completion = Completion {
                    lease: lease.clone(),
                    failed: true,
                };
                // This worker owns synchronous profile/process I/O. Never poll
                // this large transaction on the WebView or async worker stack.
                completion.failed =
                    tauri::async_runtime::block_on(check_profile(lease, target, app)).is_err();
            });
            tauri::async_runtime::spawn(async move {
                if tokio::time::timeout(Duration::from_secs(30), &mut task)
                    .await
                    .is_err()
                {
                    modules::logger::log_warn(
                        "[MixedModelRouting] 后台检查超时，等待原任务结束后再检查该实例",
                    );
                    // Keep its flight reserved; never detach and schedule duplicate
                    // blocking I/O or abort a partially applied activation.
                    let _ = task.await;
                }
            });
        }
        tokio::select! {
            _ = CHANGED.notified() => {},
            _ = tokio::time::sleep(Duration::from_secs(10)) => {},
        }
    }
}

async fn check_profile(lease: Lease, target: Target, app: AppHandle) -> Result<(), String> {
    if !current(&lease) {
        return Ok(());
    }
    let (profile_dir, account_id, routing, last_pid, is_default) = match target {
        Target::Default(s) => (
            modules::codex_instance::get_default_codex_home()?,
            super::codex_instance::resolve_default_account_id(&s),
            s.model_routing,
            s.last_pid,
            true,
        ),
        Target::Instance(s) => (
            PathBuf::from(s.user_data_dir),
            s.bind_account_id,
            s.model_routing,
            s.last_pid,
            false,
        ),
    };
    let (Some(account_id), Some(routing)) = (account_id, routing.filter(|r| r.enabled)) else {
        return Ok(());
    };
    if !modules::instance::is_profile_initialized(&profile_dir) || !current(&lease) {
        return Ok(());
    }
    let path = profile_dir.to_string_lossy();
    let running =
        modules::process::resolve_codex_pid(last_pid, process_context(is_default, &path)).is_some();
    if !current(&lease) {
        return Ok(());
    }
    if running && !modules::codex_local_access::profile_uses_mixed_model_gateway(&profile_dir)? {
        return Ok(());
    }
    let healthy =
        modules::codex_local_access::mixed_model_gateway_runtime_is_healthy(&profile_dir).await;
    let managed = running
        || modules::codex_local_access::mixed_model_gateway_runtime_is_managed(&profile_dir).await;
    if healthy && managed {
        return Ok(());
    }
    if !current(&lease) {
        return Ok(());
    }
    if lease.failures >= 3 {
        return Err("mixed routing recovery suppressed".into());
    }
    let result = modules::codex_local_access::ensure_mixed_model_gateway_for_dir_if_current(
        &profile_dir,
        &account_id,
        &routing,
        || current(&lease),
    )
    .await;
    if !current(&lease) {
        return Ok(());
    }
    match result {
        Ok(()) => {
            modules::logger::log_info(&format!(
                "[MixedModelRouting] 本地服务已恢复: profile={}",
                profile_dir.display()
            ));
            Ok(())
        }
        Err(error) => {
            let failures = lease.failures.saturating_add(1);
            modules::logger::log_warn(&format!(
                "[MixedModelRouting] 本地服务恢复失败: profile={} attempt={} error={}",
                profile_dir.display(),
                failures,
                error
            ));
            if failures >= 3 {
                let rollback_error =
                    modules::codex_local_access::fallback_mixed_model_gateway_if_current(
                        &profile_dir,
                        || current(&lease),
                    )
                    .await
                    .err();
                if current(&lease) {
                    let rollback_failed = rollback_error.is_some();
                    let _ = app.emit(
                        "codex:mixed-model-routing-unavailable",
                        serde_json::json!({
                            "profileDir": profile_dir, "error": error,
                            "rollbackError": rollback_error, "fallback": "official",
                        }),
                    );
                    let body = if rollback_failed {
                        "本地分流服务连续恢复失败，且自动恢复官方配置未完全成功。请打开 Cockpit Tools 检查。"
                    } else {
                        "本地分流服务连续恢复失败，已回退官方配置。请重新启动 Codex 后继续使用。"
                    };
                    if let Err(error) = app
                        .notification()
                        .builder()
                        .title("Codex 本地分流服务不可用")
                        .body(body)
                        .show()
                    {
                        modules::logger::log_warn(&format!(
                            "[MixedModelRouting] 系统通知发送失败: {error}"
                        ));
                    }
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CodexInstanceModelRouting;

    #[test]
    fn disabled_store_has_no_targets_or_timer() {
        let store = InstanceStore::new();
        let mut c = Control::default();
        update(&mut c, &store);
        assert!(c.targets.is_empty());
        assert!(!c.state.arm());
    }

    #[test]
    fn saved_routing_wins_and_pid_updates_do_not_restart_work() {
        let mut store = InstanceStore::new();
        store.default_settings.model_routing = Some(CodexInstanceModelRouting {
            enabled: true,
            ..Default::default()
        });
        let mut c = Control::default();
        assert!(update(&mut c, &store));
        assert!(c.state.arm());
        let lease = c.state.due(0).remove(0);
        store.default_settings.last_pid = Some(123);
        assert!(!update(&mut c, &store));
        assert!(c.state.current(&lease));
        store
            .default_settings
            .model_routing
            .as_mut()
            .unwrap()
            .enabled = false;
        assert!(update(&mut c, &store));
        assert!(c.initialized); // startup snapshot must not overwrite this save
        assert!(!c.state.current(&lease));
        assert!(c.state.retire_if_idle(false));
    }

    #[tokio::test]
    async fn disabled_or_stale_recovery_returns_before_account_or_profile_io() {
        let path = std::path::Path::new("/nonexistent-cockpit-watchdog-test");
        let disabled = CodexInstanceModelRouting {
            enabled: false,
            ..Default::default()
        };
        assert!(
            modules::codex_local_access::ensure_mixed_model_gateway_for_dir_if_current(
                path,
                "missing-account",
                &disabled,
                || panic!("disabled route must short circuit"),
            )
            .await
            .is_ok()
        );
        let enabled = CodexInstanceModelRouting {
            enabled: true,
            ..Default::default()
        };
        assert!(
            modules::codex_local_access::ensure_mixed_model_gateway_for_dir_if_current(
                path,
                "missing-account",
                &enabled,
                || false,
            )
            .await
            .is_ok()
        );
    }
}
