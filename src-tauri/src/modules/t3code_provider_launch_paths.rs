use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::{
    expand_tilde_path, extensions, live, load_persisted_bindings, read_t3_settings_json,
    remove_provider_status_cache, reserve_provider_write, resolve_state_dir_for_t3_instance,
    resolve_t3_running_for_instance, write_t3_settings_json, DRIVER_ANTIGRAVITY, DRIVER_CLAUDE,
};
use crate::commands::cli_launch_path::{executable_file, resolve_claude_cli};
use crate::modules::{codebuddy_cli, logger, t3code_app_injection};

pub(super) const BORROWED_PATH: &str = "COCKPIT_T3_RUNTIME_PATH";

fn managed_antigravity_runtime(state_dir: &Path) -> Option<PathBuf> {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return None,
    };
    let root = state_dir
        .parent()?
        .join("tools/antigravity-acp")
        .join(format!("{platform}-{arch}"));
    let active: Value = serde_json::from_slice(&fs::read(root.join("active.json")).ok()?).ok()?;
    let release_id = active["releaseId"].as_str()?;
    if release_id.len() != 64 || !release_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let active_dir = root.join("versions").join(release_id);
    let record: Value =
        serde_json::from_slice(&fs::read(active_dir.join(".install-complete.json")).ok()?).ok()?;
    if record["releaseId"] != release_id {
        return None;
    }
    let name = record["executable"]["name"].as_str()?;
    if !matches!(name, "agy.exe" | "antigravity" | "agy") {
        return None;
    }
    let path = active_dir.join(name);
    executable_file(&path).then_some(path)
}

fn find_antigravity_cli() -> Option<PathBuf> {
    crate::commands::account::cli_launch::find_cli()
}

pub(super) fn resolve_antigravity_for_probe(
    state_dir: &Path,
    configured: Option<&str>,
) -> Option<PathBuf> {
    if let Some(path) = configured.map(str::trim).filter(|path| !path.is_empty()) {
        let path = expand_tilde_path(path);
        return executable_file(&path).then_some(path);
    }
    managed_antigravity_runtime(state_dir).or_else(find_antigravity_cli)
}

fn borrowed_path(entry: &Value) -> Option<&str> {
    entry["environment"].as_array()?.iter().find_map(|item| {
        (item["name"] == BORROWED_PATH)
            .then(|| item["value"].as_str())
            .flatten()
    })
}

fn set_path_marker(entry: &mut Value, path: &str) {
    if !entry["environment"].is_array() {
        entry["environment"] = json!([]);
    }
    if let Some(environment) = entry["environment"].as_array_mut() {
        environment.retain(|item| item["name"] != BORROWED_PATH);
        environment.push(json!({ "name": BORROWED_PATH, "value": path }));
    }
}

fn configure_native_path(
    entry: &mut Value,
    driver: &str,
    fallback: impl FnOnce() -> Option<PathBuf>,
) -> bool {
    if !entry.is_object()
        || entry
            .get("config")
            .is_some_and(|config| !config.is_object())
    {
        return false;
    }
    let current = entry["config"]["binaryPath"].as_str().unwrap_or_default();
    let native_default = if driver == DRIVER_CLAUDE {
        "claude"
    } else {
        ""
    };
    let previous = borrowed_path(entry);

    // A path entered in T3 Code takes precedence over Cockpit's recorded fallback.
    if !current.trim().is_empty() && current != native_default && previous != Some(current) {
        return clear_borrowed_path(entry);
    }
    let resolved = fallback();
    let path = resolved.as_deref().and_then(Path::to_str);
    if path == previous && (path.is_none() || path == Some(current)) {
        return false;
    }
    clear_borrowed_path(entry);
    if let Some(path) = path {
        entry["config"]["binaryPath"] = json!(path);
        set_path_marker(entry, path);
    }
    true
}

fn clear_borrowed_path(entry: &mut Value) -> bool {
    let Some(previous) = borrowed_path(entry) else {
        return false;
    };
    if entry["config"]["binaryPath"] == previous {
        if let Some(config) = entry["config"].as_object_mut() {
            config.remove("binaryPath");
        }
    }
    if let Some(environment) = entry["environment"].as_array_mut() {
        environment.retain(|item| item["name"] != BORROWED_PATH);
    }
    true
}

pub(super) fn apply_native_path(state_dir: &Path, entry: &mut Value, driver: &str) {
    if let Some(family @ (RuntimeFamily::Antigravity | RuntimeFamily::Claude)) =
        RuntimeFamily::for_driver(driver)
    {
        configure_native_path(entry, driver, || family.resolve(state_dir));
    }
}

pub(super) fn t3_explicit_runtime_path(entry: &Value, driver: &str) -> bool {
    if !matches!(driver, DRIVER_ANTIGRAVITY | DRIVER_CLAUDE) {
        return false;
    }
    entry["config"]["binaryPath"].as_str().is_some_and(|path| {
        !path.trim().is_empty()
            && (driver != DRIVER_CLAUDE || path != "claude")
            && borrowed_path(entry) != Some(path)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RuntimeFamily {
    Antigravity,
    Claude,
    CodeBuddy,
}

impl RuntimeFamily {
    const ALL: [Self; 3] = [Self::Antigravity, Self::Claude, Self::CodeBuddy];

    fn for_driver(driver: &str) -> Option<Self> {
        match driver {
            DRIVER_ANTIGRAVITY => Some(Self::Antigravity),
            DRIVER_CLAUDE => Some(Self::Claude),
            extensions::PLATFORM_CODEBUDDY | extensions::PLATFORM_CODEBUDDY_CN => {
                Some(Self::CodeBuddy)
            }
            _ => None,
        }
    }

    fn resolve(self, state_dir: &Path) -> Option<PathBuf> {
        match self {
            Self::Antigravity if managed_antigravity_runtime(state_dir).is_none() => {
                find_antigravity_cli()
            }
            Self::Antigravity => None,
            Self::Claude => resolve_claude_cli().ok(),
            Self::CodeBuddy => codebuddy_cli::find_cli(),
        }
    }
}

fn sync_paths(
    state_dir: &Path,
    families: &HashSet<RuntimeFamily>,
) -> Result<Option<Vec<String>>, String> {
    let needed: HashSet<_> = load_persisted_bindings(state_dir)
        .into_values()
        .filter(|binding| !binding.unlinked)
        .filter_map(|binding| RuntimeFamily::for_driver(&binding.driver))
        .filter(|family| families.contains(family))
        .collect();
    if needed.is_empty() {
        return Ok(Some(Vec::new()));
    }
    // Search the platform paths before reserving the T3 settings write.
    let candidates: HashMap<_, _> = needed
        .into_iter()
        .map(|family| (family, family.resolve(state_dir)))
        .collect();

    let Ok(_reservation) = reserve_provider_write(state_dir) else {
        return Ok(None);
    };
    let bindings = load_persisted_bindings(state_dir);
    let mut settings = read_t3_settings_json(state_dir)?;
    let Some(entries) = settings["providerInstances"].as_object_mut() else {
        return Ok(Some(Vec::new()));
    };
    let mut changed = Vec::new();
    for (id, entry) in entries {
        let Some(binding) = bindings.get(id).filter(|binding| !binding.unlinked) else {
            continue;
        };
        let driver = binding.driver.as_str();
        if entry["enabled"] == false
            || entry["config"]["enabled"] == false
            || extensions::t3_raw_driver_for_platform(driver) != entry["driver"].as_str()
            || id.contains(['/', '\\'])
            || matches!(id.as_str(), "." | "..")
        {
            continue;
        }
        let Some(family) = RuntimeFamily::for_driver(driver) else {
            continue;
        };
        let Some(candidate) = candidates.get(&family) else {
            continue;
        };
        let changed_entry = if family == RuntimeFamily::CodeBuddy {
            match extensions::update_codebuddy_runtime(
                state_dir,
                id,
                driver,
                entry,
                candidate.as_deref(),
            ) {
                Ok(Some(file_changed)) => {
                    let path = candidate
                        .as_deref()
                        .and_then(Path::to_str)
                        .unwrap_or("codebuddy");
                    let settings_changed = borrowed_path(entry) != Some(path);
                    if settings_changed {
                        set_path_marker(entry, path);
                    }
                    file_changed || settings_changed
                }
                Ok(None) => false,
                Err(error) => {
                    logger::log_warn(&format!(
                        "[T3 Code] Could not update CodeBuddy runtime for {id}: {error}"
                    ));
                    false
                }
            }
        } else {
            configure_native_path(entry, driver, || candidate.clone())
        };
        if changed_entry {
            changed.push(id.clone());
        }
    }
    if !changed.is_empty() {
        write_t3_settings_json(state_dir, &settings)?;
        for id in &changed {
            remove_provider_status_cache(state_dir, id);
        }
    }
    Ok(Some(changed))
}

pub(crate) fn sync_saved_cli_path(app: &str) {
    let family = match app {
        "antigravity_cli" => RuntimeFamily::Antigravity,
        "claude_cli" => RuntimeFamily::Claude,
        "codebuddy_cli" => RuntimeFamily::CodeBuddy,
        _ => return,
    };
    schedule("__default__", [family].into());
}

pub(crate) fn schedule_sync(t3_instance_id: Option<&str>) {
    schedule(
        t3_instance_id.unwrap_or("__default__"),
        RuntimeFamily::ALL.into(),
    );
}

/// Resolve paths outside the account operation and coalesce requests per T3 Code process.
fn schedule(instance_id: &str, mut families: HashSet<RuntimeFamily>) {
    type Jobs = HashMap<String, HashSet<RuntimeFamily>>;
    static JOBS: std::sync::LazyLock<Mutex<Jobs>> = std::sync::LazyLock::new(Default::default);
    let instance_id = instance_id.to_owned();
    if let Ok(mut jobs) = JOBS.lock() {
        if let Some(pending) = jobs.get_mut(&instance_id) {
            pending.extend(families);
            return;
        }
        jobs.insert(instance_id.clone(), HashSet::new());
    } else {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = sync_and_refresh(&instance_id, &families).await {
                logger::log_warn(&format!("[T3 Code] Runtime path sync failed: {error}"));
            }
            let Ok(mut jobs) = JOBS.lock() else { return };
            families = jobs
                .get_mut(&instance_id)
                .map(std::mem::take)
                .unwrap_or_default();
            if families.is_empty() {
                jobs.remove(&instance_id);
                break;
            }
        }
    });
}

async fn sync_and_refresh(
    instance_id: &str,
    families: &HashSet<RuntimeFamily>,
) -> Result<(), String> {
    let id = instance_id.to_owned();
    let (_, state_dir) =
        tokio::task::spawn_blocking(move || resolve_state_dir_for_t3_instance(Some(&id)))
            .await
            .map_err(|error| error.to_string())??;
    // A switch may have reserved provider settings. Retry only the contested write.
    for attempt in 0..2 {
        let dir = state_dir.clone();
        let families = families.clone();
        let Some(ids) = tokio::task::spawn_blocking(move || sync_paths(&dir, &families))
            .await
            .map_err(|error| error.to_string())??
        else {
            if attempt == 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            continue;
        };
        if ids.is_empty() {
            return Ok(());
        }
        for id in &ids {
            t3code_app_injection::forget_provider_status(instance_id, id);
        }
        let id = instance_id.to_owned();
        if tokio::task::spawn_blocking(move || resolve_t3_running_for_instance(&id))
            .await
            .map_err(|error| error.to_string())?
        {
            live::schedule_model_refreshes(instance_id, &state_dir, ids.iter().map(String::as_str));
        }
        t3code_app_injection::notify_provider_binding_changed(instance_id);
        return Ok(());
    }
    Err(live::message("switchBusy"))
}

pub async fn install_antigravity_runtime(provider_id: &str) -> Result<(), String> {
    let id = provider_id.to_owned();
    let (t3_id, state_dir) = tokio::task::spawn_blocking(move || {
        let (t3_id, state_dir) = resolve_state_dir_for_t3_instance(None)?;
        let settings = read_t3_settings_json(&state_dir)?;
        let entry = &settings["providerInstances"][&id];
        if entry["driver"] != DRIVER_ANTIGRAVITY && !(id == DRIVER_ANTIGRAVITY && entry.is_null()) {
            return Err(live::message("switchTargetChanged"));
        }
        Ok::<_, String>((t3_id, state_dir))
    })
    .await
    .map_err(|error| error.to_string())??;
    let mut control = live::ProviderControl::connect(&t3_id, &state_dir)
        .await?
        .ok_or_else(|| live::message("switchControlUnavailable"))?;
    // T3 Code manages the download and refreshes the providers when installation completes.
    control
        .call(
            "provider.install.start",
            json!({ "instanceId": provider_id }),
            Duration::from_secs(8),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_and_releases_only_cockpit_provided_paths() {
        let mut provider = json!({
            "config": { "homePath": "separate-account" },
            "environment": [{ "name": "OTHER_VARIABLE", "value": "preserved" }],
        });
        let first = PathBuf::from("/first/agy");
        assert!(configure_native_path(
            &mut provider,
            DRIVER_ANTIGRAVITY,
            || Some(first.clone())
        ));
        assert_eq!(provider["config"]["binaryPath"], first.to_str().unwrap());
        assert!(!t3_explicit_runtime_path(&provider, DRIVER_ANTIGRAVITY));
        assert!(!configure_native_path(
            &mut provider,
            DRIVER_ANTIGRAVITY,
            || Some(first)
        ));

        let second = PathBuf::from("/second/agy");
        assert!(configure_native_path(
            &mut provider,
            DRIVER_ANTIGRAVITY,
            || Some(second.clone())
        ));
        assert_eq!(provider["config"]["binaryPath"], second.to_str().unwrap());
        assert!(configure_native_path(
            &mut provider,
            DRIVER_ANTIGRAVITY,
            || None
        ));
        assert!(provider["config"].get("binaryPath").is_none());
        assert_eq!(provider["config"]["homePath"], "separate-account");
        assert_eq!(provider["environment"][0]["value"], "preserved");

        provider["config"]["binaryPath"] = json!("/user/agy");
        assert!(!configure_native_path(
            &mut provider,
            DRIVER_ANTIGRAVITY,
            || { panic!("T3 Code's custom path already takes precedence") }
        ));
        assert!(t3_explicit_runtime_path(&provider, DRIVER_ANTIGRAVITY));
        assert_eq!(provider["config"]["binaryPath"], "/user/agy");
    }
}
