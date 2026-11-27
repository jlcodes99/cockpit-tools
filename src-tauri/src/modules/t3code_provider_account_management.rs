use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    extensions, get_provider_workspace, live, load_persisted_bindings,
    prepare_provider_binding_entry, read_t3_settings_json, remove_provider_status_cache,
    reserve_provider_write, resolve_state_dir_for_t3_instance, resolve_t3_running_for_instance,
    save_persisted_bindings, set_provider_entry_enabled, t3code_app_injection,
    write_t3_settings_json, PersistedProviderBinding, T3CodeBindableAccountView,
    T3CodeProviderInstanceView, T3CodeProviderSwitchView, T3CodeProviderWorkspaceView,
};
use crate::models::t3code::{T3CodeDefaultSelectionState, T3CodePendingDefaultView};
use crate::modules::{atomic_write, logger};
use live::rollback::ProviderProfileSnapshot;

const PENDING_DEFAULTS_FILE: &str = "t3code_cockpit_pending_defaults.json";
const DEFAULT_RECHECK_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefaultSelection {
    source_id: String,
    account_id: String,
    generation: String,
    #[serde(flatten)]
    status: T3CodePendingDefaultView,
}

type DefaultSelections = HashMap<String, DefaultSelection>;

fn load_default_selections(state_dir: &Path) -> Result<DefaultSelections, String> {
    match fs::read(state_dir.join(PENDING_DEFAULTS_FILE)) {
        Ok(data) => serde_json::from_slice(&data).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn save_default_selections(state_dir: &Path, selections: &DefaultSelections) -> Result<(), String> {
    let content = serde_json::to_string(selections).map_err(|error| error.to_string())?;
    atomic_write::write_string_atomic(&state_dir.join(PENDING_DEFAULTS_FILE), &content)
}

pub(super) fn remove_copied_qoder_default_selections(state_dir: &Path) -> Result<(), String> {
    let mut selections = load_default_selections(state_dir)?;
    let previous = selections.len();
    selections.retain(|platform, _| extensions::QoderProvider::from_platform(platform).is_none());
    if selections.len() != previous {
        save_default_selections(state_dir, &selections)?;
    }
    Ok(())
}

pub(super) fn show_pending_defaults(state_dir: &Path, rows: &mut [T3CodeProviderInstanceView]) {
    match load_default_selections(state_dir) {
        Ok(selections) => set_pending_default_rows(&selections, rows),
        Err(error) => logger::log_warn(&format!(
            "[T3 Code] Cannot read saved default accounts: {error}"
        )),
    }
}

fn set_pending_default_rows(
    selections: &DefaultSelections,
    rows: &mut [T3CodeProviderInstanceView],
) {
    for row in rows {
        row.pending_default = selections.get(&row.driver).and_then(|selection| {
            (row.bound_account_id.as_ref() == Some(&selection.account_id)
                && (row.instance_id == selection.source_id || row.instance_id == row.driver))
                .then(|| selection.status.clone())
        });
    }
}

enum ProviderEdit {
    Upsert {
        id: String,
        entry: Value,
        binding: PersistedProviderBinding,
    },
    Remove(String),
}

impl ProviderEdit {
    fn id(&self) -> &str {
        match self {
            Self::Upsert { id, .. } | Self::Remove(id) => id,
        }
    }

    fn matches_live(&self, entries: &Value, generation: &str) -> bool {
        match self {
            Self::Upsert { id, entry, .. } => entries.get(id).is_some_and(|accepted| {
                live::has_generation(accepted, generation)
                    && accepted["enabled"] == entry["enabled"]
            }),
            Self::Remove(id) => entries.get(id).is_none(),
        }
    }
}

fn save_edits(state_dir: &Path, edits: &[ProviderEdit]) -> Result<(), String> {
    let mut settings = read_t3_settings_json(state_dir)?;
    let entries = settings
        .as_object_mut()
        .expect("validated settings")
        .entry("providerInstances")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let previous = load_persisted_bindings(state_dir);
    let mut bindings = previous.clone();
    for edit in edits {
        match edit {
            ProviderEdit::Upsert { id, entry, binding } => {
                entries.insert(id.clone(), entry.clone());
                bindings.insert(id.clone(), binding.clone());
            }
            ProviderEdit::Remove(id) => {
                entries.remove(id);
                bindings.remove(id);
            }
        }
    }
    let unlinked = edits
        .iter()
        .filter_map(|edit| match edit {
            ProviderEdit::Upsert { id, binding, .. } if binding.unlinked => Some(id),
            ProviderEdit::Remove(id) => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !unlinked.is_empty() {
        if let Some(projects) = settings
            .get_mut("projectSettingsOverrides")
            .and_then(Value::as_object_mut)
        {
            for project in projects.values_mut() {
                let Some(instances) = project
                    .get_mut("providerInstances")
                    .and_then(Value::as_object_mut)
                else {
                    continue;
                };
                for id in &unlinked {
                    if let Some(override_entry) =
                        instances.get_mut(*id).and_then(Value::as_object_mut)
                    {
                        // A project-level enable would keep the removed account routable.
                        override_entry.remove("enabled");
                    }
                }
            }
        }
    }
    save_persisted_bindings(state_dir, &bindings)?;
    if let Err(error) = write_t3_settings_json(state_dir, &settings) {
        return match save_persisted_bindings(state_dir, &previous) {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}\n{restore}")),
        };
    }
    Ok(())
}

async fn control_for_changes(
    workspace: &T3CodeProviderWorkspaceView,
    state_dir: &Path,
    providers: &[T3CodeProviderInstanceView],
) -> Result<Option<live::ProviderControl>, String> {
    if !workspace.t3_running {
        return Ok(None);
    }
    let mut control =
        match live::ProviderControl::connect(&workspace.t3_instance_id, state_dir).await {
            Ok(control) => control,
            Err(error) => {
                crate::modules::logger::log_warn(&format!(
                    "[T3 Code] Provider settings will apply at next launch: {error}"
                ));
                None
            }
        };
    for provider in providers.iter().filter(|provider| provider.enabled) {
        if let Some(control) = control.as_mut() {
            control.prepare_switch(state_dir, provider).await?;
        } else {
            live::wait_for_active_runs(state_dir, &provider.instance_id).await?;
        }
    }
    Ok(control)
}

async fn applied_in_t3(
    control: Option<live::ProviderControl>,
    edits: &[ProviderEdit],
    generation: &str,
) -> bool {
    let Some(mut control) = control else {
        return false;
    };
    // The local T3 settings watcher publishes the two edited entries together.
    // Acknowledge the config update only. Model discovery runs separately.
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let Ok(settings) = control
                .call("server.getSettings", json!({}), Duration::from_secs(2))
                .await
            else {
                return false;
            };
            let instances = &settings["providerInstances"];
            if edits
                .iter()
                .all(|edit| edit.matches_live(instances, generation))
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    })
    .await
    .unwrap_or(false)
}

fn account_for_provider<'a>(
    accounts: &'a [T3CodeBindableAccountView],
    provider: &T3CodeProviderInstanceView,
) -> Option<&'a T3CodeBindableAccountView> {
    accounts
        .iter()
        .find(|account| Some(account.id.as_str()) == provider.bound_account_id.as_deref())
}

fn default_entry(entries: &Value, driver: &str) -> Result<Value, String> {
    let raw_driver = extensions::t3_raw_driver_for_platform(driver)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let entry = entries
        .get(driver)
        .cloned()
        .unwrap_or_else(|| json!({"driver": raw_driver, "config": {}}));
    if entry["driver"] != raw_driver {
        return Err(live::message("switchTargetChanged"));
    }
    Ok(entry)
}

async fn prepare_edit(
    state_dir: &Path,
    id: &str,
    driver: &str,
    account: &T3CodeBindableAccountView,
    entry: Value,
    generation: &str,
    connected: bool,
) -> Result<ProviderEdit, String> {
    let mut entry = live::with_generation(entry, generation)?;
    let mut binding =
        prepare_provider_binding_entry(state_dir, id, driver, &account.id, &mut entry, false)
            .await?;
    if connected {
        binding.live_generation = Some(generation.to_owned());
    }
    Ok(ProviderEdit::Upsert {
        id: id.to_owned(),
        entry,
        binding,
    })
}

fn parallel_name(driver: &str, account: &T3CodeBindableAccountView) -> String {
    let name = account
        .display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| account.email.split('@').next().unwrap_or(&account.email));
    format!("{} · {name}", extensions::default_display_name(driver))
}

pub async fn promote_provider_account(
    t3_instance_id: Option<&str>,
    source_id: &str,
    account_id: &str,
) -> Result<T3CodeProviderSwitchView, String> {
    let id = t3_instance_id.map(str::to_owned);
    let source_id = source_id.to_owned();
    let account_id = account_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let (id, state_dir) = resolve_state_dir_for_t3_instance(id.as_deref())?;
        let _reservation = reserve_provider_write(&state_dir)?;
        let mut workspace = get_provider_workspace(Some(&id))?;
        let source = workspace
            .provider_instances
            .iter()
            .find(|row| {
                row.instance_id == source_id && row.bound_account_id.as_ref() == Some(&account_id)
            })
            .ok_or_else(|| live::message("switchTargetChanged"))?;
        let driver = source.driver.clone();
        if source_id == driver {
            return Ok(promotion_result(workspace, "defaultAccountSaved"));
        }
        let selection = DefaultSelection {
            source_id,
            account_id,
            generation: uuid::Uuid::new_v4().to_string(),
            status: T3CodePendingDefaultView {
                state: T3CodeDefaultSelectionState::Waiting,
                error: None,
            },
        };
        promotion_targets(&workspace, &driver, &selection)?;
        let mut selections = load_default_selections(&state_dir)?;
        selections.insert(driver, selection);
        save_default_selections(&state_dir, &selections)?;
        set_pending_default_rows(&selections, &mut workspace.provider_instances);
        drop(_reservation);
        schedule_default_changes(id, state_dir);
        Ok(promotion_result(workspace, "defaultSelectionSaved"))
    })
    .await
    .map_err(|error| error.to_string())?
}

struct PromotionTargets<'a> {
    source: &'a T3CodeProviderInstanceView,
    previous_default: Option<&'a T3CodeProviderInstanceView>,
    selected: &'a T3CodeBindableAccountView,
    previous_account: Option<&'a T3CodeBindableAccountView>,
}

fn promotion_targets<'a>(
    workspace: &'a T3CodeProviderWorkspaceView,
    driver: &str,
    selection: &DefaultSelection,
) -> Result<PromotionTargets<'a>, String> {
    let source = workspace
        .provider_instances
        .iter()
        .find(|row| {
            row.instance_id == selection.source_id
                && row.driver == driver
                && row.bound_account_id.as_ref() == Some(&selection.account_id)
        })
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let accounts = match driver {
        super::DRIVER_CODEX => &workspace.codex_accounts,
        super::DRIVER_ANTIGRAVITY => &workspace.antigravity_accounts,
        super::DRIVER_CLAUDE => &workspace.claude_accounts,
        _ => workspace
            .extra_accounts
            .get(driver)
            .ok_or_else(|| live::message("switchTargetChanged"))?,
    };
    let selected = account_for_provider(accounts, source)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let previous_default = workspace
        .provider_instances
        .iter()
        .find(|row| row.driver == driver && row.instance_id == driver);
    let previous_account = previous_default.and_then(|row| account_for_provider(accounts, row));
    if previous_default
        .is_some_and(|row| row.bound_account_id.is_some() || row.bound_account_email.is_some())
        && previous_account.is_none()
    {
        return Err(live::message("defaultAccountUnavailable"));
    }
    Ok(PromotionTargets {
        source,
        previous_default,
        selected,
        previous_account,
    })
}

async fn apply_default_change(
    workspace: &T3CodeProviderWorkspaceView,
    state_dir: &Path,
    driver: &str,
    selection: &DefaultSelection,
    mut control: Option<live::ProviderControl>,
) -> Result<bool, String> {
    let source_id = &selection.source_id;
    let generation = &selection.generation;
    let PromotionTargets {
        source,
        previous_default,
        selected,
        previous_account,
    } = promotion_targets(workspace, driver, selection)?;

    let settings = read_t3_settings_json(state_dir)?;
    let entries = &settings["providerInstances"];
    let mut default = default_entry(entries, driver)?;
    if default.get("displayName").is_none() {
        default["displayName"] = json!(extensions::default_display_name(driver));
    }
    set_provider_entry_enabled(
        default
            .as_object_mut()
            .ok_or_else(|| live::message("switchTargetChanged"))?,
        true,
    );
    let mut parallel = entries
        .get(source_id)
        .cloned()
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let providers = [Some(source), previous_default]
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    if let Some(control) = control.as_mut() {
        // Both entries were idle when this attempt began. Detach only their old
        // sessions so that their next turn opens with the new binding.
        tokio::time::timeout(Duration::from_secs(5), async {
            for provider in providers.iter().filter(|provider| provider.enabled) {
                control.detach_sessions(state_dir, provider).await?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| live::message("switchDetachTimeout"))??;
    }

    let snapshot_targets = std::iter::once((driver, selected))
        .chain(previous_account.map(|account| (source_id.as_str(), account)))
        .map(|(id, account)| {
            (
                id.to_owned(),
                account.id.clone(),
                account.auth_mode == "oauth",
            )
        })
        .collect::<Vec<_>>();
    let (dir, platform) = (state_dir.to_owned(), driver.to_owned());
    let snapshots = tokio::task::spawn_blocking(move || {
        snapshot_targets
            .into_iter()
            .map(|(id, account_id, oauth)| {
                ProviderProfileSnapshot::capture_for_account(
                    &dir,
                    &id,
                    &platform,
                    oauth,
                    &account_id,
                )
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| error.to_string())??;

    let _pending = live::PendingBinding::keep_visible(state_dir, providers)?;
    let changes = async {
        let primary = prepare_edit(
            state_dir,
            driver,
            driver,
            selected,
            default,
            generation,
            control.is_some(),
        )
        .await?;
        let secondary = if let Some(previous) = previous_account {
            parallel["displayName"] = json!(parallel_name(driver, previous));
            set_provider_entry_enabled(
                parallel
                    .as_object_mut()
                    .ok_or_else(|| live::message("switchTargetChanged"))?,
                previous_default.is_some_and(|row| row.enabled),
            );
            prepare_edit(
                state_dir,
                source_id,
                driver,
                previous,
                parallel,
                generation,
                control.is_some(),
            )
            .await?
        } else {
            ProviderEdit::Remove(source_id.to_owned())
        };
        let dir = state_dir.to_owned();
        tokio::task::spawn_blocking(move || {
            let edits = [primary, secondary];
            save_edits(&dir, &edits)?;
            Ok::<_, String>(edits)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    let edits = match changes {
        Ok(edits) => edits,
        Err(mut error) => {
            let restored = tokio::task::spawn_blocking(move || {
                snapshots
                    .into_iter()
                    .filter_map(|snapshot| snapshot.restore().err())
                    .collect::<Vec<_>>()
            })
            .await;
            match restored {
                Ok(errors) if errors.is_empty() => {}
                Ok(errors) => error.push_str(&format!(
                    "\n{}\n{}",
                    live::message("switchRollbackFailed"),
                    errors.join("\n")
                )),
                Err(detail) => error.push_str(&format!(
                    "\n{}\n{detail}",
                    live::message("switchRollbackFailed")
                )),
            }
            return Err(error);
        }
    };

    let applied = applied_in_t3(control, &edits, generation).await;
    let id = &workspace.t3_instance_id;
    for edit in &edits {
        t3code_app_injection::forget_provider_status(id, edit.id());
    }
    if applied {
        live::schedule_model_refreshes(
            id,
            state_dir,
            edits.iter().filter_map(|edit| {
                matches!(edit, ProviderEdit::Upsert { entry, .. } if entry["enabled"] == true)
                    .then(|| edit.id())
            }),
        );
    }
    Ok(applied)
}

fn promotion_result(workspace: T3CodeProviderWorkspaceView, key: &str) -> T3CodeProviderSwitchView {
    T3CodeProviderSwitchView {
        workspace,
        applied_live: false,
        restart_required: false,
        message: live::message(key),
    }
}

fn selection_is_on_disk(
    state_dir: &Path,
    driver: &str,
    selection: &DefaultSelection,
) -> Result<bool, String> {
    let settings = read_t3_settings_json(state_dir)?;
    let default = &settings["providerInstances"][driver];
    Ok(live::has_generation(default, &selection.generation)
        && load_persisted_bindings(state_dir)
            .get(driver)
            .is_some_and(|binding| {
                !binding.unlinked
                    && binding.driver == driver
                    && binding.account_id == selection.account_id
            }))
}

// The background job reserves writes only when a selection is ready to apply.
// Return true when the job should check again, without keeping the reservation.
async fn process_default_selection(
    instance_id: &str,
    state_dir: &Path,
    driver: &str,
    selection: &DefaultSelection,
    running: bool,
    before_launch: bool,
) -> Result<bool, String> {
    let control = if running {
        match live::default_change_has_active_runs(state_dir, driver, &selection.source_id).await {
            Ok(true) => return Ok(true),
            Ok(false) => live::ProviderControl::connect(instance_id, state_dir).await,
            Err(error) => Err(error),
        }
    } else {
        Ok(None)
    };

    let Ok(reservation) = reserve_provider_write(state_dir) else {
        return Ok(true);
    };
    let (mut selections, workspace, on_disk) = tokio::task::spawn_blocking({
        let (id, dir, driver, selection) = (
            instance_id.to_owned(),
            state_dir.to_owned(),
            driver.to_owned(),
            selection.clone(),
        );
        move || {
            let selections = load_default_selections(&dir)?;
            if !selections.get(&driver).is_some_and(|current| {
                current.generation == selection.generation
                    && (before_launch
                        || current.status.state == T3CodeDefaultSelectionState::Waiting)
            }) {
                return Ok::<_, String>((selections, None, false));
            }
            let workspace = get_provider_workspace(Some(&id));
            let on_disk = selection_is_on_disk(&dir, &driver, &selection)?;
            Ok((selections, Some(workspace), on_disk))
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    let Some(workspace) = workspace else {
        return Ok(false);
    };
    let t3_running = workspace
        .as_ref()
        .map_or(running, |workspace| workspace.t3_running);
    if t3_running && !running {
        // The process started while we were preparing an offline update.
        return Ok(!before_launch);
    }

    let applied = match workspace {
        Err(error) => Err(error),
        Ok(_) if on_disk => Ok(!t3_running),
        Ok(workspace) if !t3_running => {
            apply_default_change(&workspace, state_dir, driver, selection, None)
                .await
                .map(|_| true)
        }
        Ok(workspace) => match control {
            Ok(Some(control)) => {
                match live::default_change_has_active_runs(state_dir, driver, &selection.source_id)
                    .await
                {
                    Ok(true) => return Ok(true),
                    Ok(false) => {
                        apply_default_change(
                            &workspace,
                            state_dir,
                            driver,
                            selection,
                            Some(control),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            Ok(None) => Err(live::message("switchControlUnavailable")),
            Err(error) => Err(error),
        },
    };
    match applied {
        Ok(true) => {
            selections.remove(driver);
        }
        outcome => {
            let error = outcome.err();
            if let Some(error) = &error {
                logger::log_warn(&format!(
                    "[T3 Code] Could not apply saved {driver} default: {error}"
                ));
            }
            if let Some(selection) = selections.get_mut(driver) {
                selection.status = T3CodePendingDefaultView {
                    state: if t3_running {
                        T3CodeDefaultSelectionState::RestartRequired
                    } else {
                        T3CodeDefaultSelectionState::Failed
                    },
                    error,
                };
            }
        }
    }
    let dir = state_dir.to_owned();
    tokio::task::spawn_blocking(move || save_default_selections(&dir, &selections))
        .await
        .map_err(|error| error.to_string())??;
    drop(reservation);
    t3code_app_injection::notify_provider_binding_changed(instance_id);
    Ok(false)
}

pub(crate) fn resume_pending_defaults(workspace: &T3CodeProviderWorkspaceView) {
    if workspace.provider_instances.iter().any(|row| {
        row.pending_default
            .as_ref()
            .is_some_and(|status| status.state == T3CodeDefaultSelectionState::Waiting)
    }) {
        schedule_default_changes(
            workspace.t3_instance_id.clone(),
            PathBuf::from(&workspace.state_dir),
        );
    }
}

fn schedule_default_changes(instance_id: String, state_dir: PathBuf) {
    static JOBS: LazyLock<Mutex<HashMap<PathBuf, bool>>> = LazyLock::new(Default::default);
    let Ok(mut jobs) = JOBS.lock() else { return };
    if let Some(recheck) = jobs.get_mut(&state_dir) {
        *recheck = true;
        return;
    }
    jobs.insert(state_dir.clone(), false);
    drop(jobs);

    tauri::async_runtime::spawn(async move {
        loop {
            let retry = match apply_waiting_defaults(&instance_id, &state_dir).await {
                Ok(retry) => retry,
                Err(error) => {
                    logger::log_warn(&format!(
                        "[T3 Code] Could not process saved default accounts: {error}"
                    ));
                    false
                }
            };
            {
                let Ok(mut jobs) = JOBS.lock() else { return };
                if !jobs.get_mut(&state_dir).is_some_and(std::mem::take) && !retry {
                    jobs.remove(&state_dir);
                    break;
                }
            }
            tokio::time::sleep(DEFAULT_RECHECK_INTERVAL).await;
        }
    });
}

async fn apply_waiting_defaults(instance_id: &str, state_dir: &Path) -> Result<bool, String> {
    let (selections, running) = tokio::task::spawn_blocking({
        let (id, dir) = (instance_id.to_owned(), state_dir.to_owned());
        move || {
            let mut selections = load_default_selections(&dir)?;
            selections.retain(|_, selection| {
                selection.status.state == T3CodeDefaultSelectionState::Waiting
            });
            let running = !selections.is_empty() && resolve_t3_running_for_instance(&id);
            Ok::<_, String>((selections, running))
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    let mut retry = false;
    for (driver, selection) in selections {
        match process_default_selection(instance_id, state_dir, &driver, &selection, running, false)
            .await
        {
            Ok(waiting) => retry |= waiting,
            Err(error) => logger::log_warn(&format!(
                "[T3 Code] Could not process saved {driver} default: {error}"
            )),
        }
    }
    Ok(retry)
}

pub(super) async fn apply_pending_defaults_before_launch(
    instance_id: &str,
    state_dir: &Path,
) -> Result<(), String> {
    let selections = tokio::task::spawn_blocking({
        let dir = state_dir.to_owned();
        move || load_default_selections(&dir)
    })
    .await
    .map_err(|error| error.to_string())??;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    for (driver, selection) in selections {
        while process_default_selection(instance_id, state_dir, &driver, &selection, false, true)
            .await?
        {
            if tokio::time::Instant::now() >= deadline {
                return Err(live::message("switchBusy"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Ok(())
}

pub async fn remove_provider_instance(
    t3_instance_id: Option<&str>,
    instance_id: &str,
    expected_account_id: Option<&str>,
) -> Result<T3CodeProviderSwitchView, String> {
    let id = t3_instance_id.map(str::to_owned);
    let (state_dir, workspace, reservation) = tokio::task::spawn_blocking(move || {
        let (id, state_dir) = resolve_state_dir_for_t3_instance(id.as_deref())?;
        let reservation = reserve_provider_write(&state_dir)?;
        let workspace = get_provider_workspace(Some(&id))?;
        Ok::<_, String>((state_dir, workspace, reservation))
    })
    .await
    .map_err(|error| error.to_string())??;
    let row = workspace
        .provider_instances
        .iter()
        .find(|row| {
            row.instance_id == instance_id && expected_account_id == row.bound_account_id.as_deref()
        })
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let control = control_for_changes(&workspace, &state_dir, std::slice::from_ref(row)).await?;
    let generation = uuid::Uuid::new_v4().to_string();
    let edit = if row.is_builtin_default {
        // T3 recreates a missing built-in provider. Keep its entry disabled and
        // unlink its Cockpit account so the platform can accept another one.
        let settings = read_t3_settings_json(&state_dir)?;
        let mut entry = default_entry(&settings["providerInstances"], &row.driver)?;
        set_provider_entry_enabled(
            entry
                .as_object_mut()
                .ok_or_else(|| live::message("switchTargetChanged"))?,
            false,
        );
        ProviderEdit::Upsert {
            id: instance_id.to_owned(),
            entry: live::with_generation(entry, &generation)?,
            binding: PersistedProviderBinding {
                driver: row.driver.clone(),
                unlinked: true,
                ..Default::default()
            },
        }
    } else {
        ProviderEdit::Remove(instance_id.to_owned())
    };
    save_edits(&state_dir, std::slice::from_ref(&edit))?;
    let clear_selection = (|| {
        let mut selections = load_default_selections(&state_dir)?;
        if selections
            .get(&row.driver)
            .is_some_and(|selection| Some(selection.account_id.as_str()) == expected_account_id)
        {
            selections.remove(&row.driver);
            save_default_selections(&state_dir, &selections)?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = clear_selection {
        logger::log_warn(&format!(
            "[T3 Code] Could not clear the removed account's saved selection: {error}"
        ));
    }
    let applied_live = applied_in_t3(control, &[edit], &generation).await;
    let restart_required = workspace.t3_running && !applied_live;
    let id = workspace.t3_instance_id;
    remove_provider_status_cache(&state_dir, instance_id);
    t3code_app_injection::forget_provider_status(&id, instance_id);
    drop(reservation);
    t3code_app_injection::notify_provider_binding_changed(&id);
    let workspace = tokio::task::spawn_blocking(move || get_provider_workspace(Some(&id)))
        .await
        .map_err(|error| error.to_string())??;
    Ok(T3CodeProviderSwitchView {
        workspace,
        applied_live,
        restart_required,
        message: live::message(if restart_required {
            "removeSaved"
        } else {
            "accountRemoved"
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_default() -> DefaultSelection {
        DefaultSelection {
            source_id: "codex-parallel".into(),
            account_id: "selected".into(),
            generation: "selection-generation".into(),
            status: T3CodePendingDefaultView {
                state: T3CodeDefaultSelectionState::Waiting,
                error: None,
            },
        }
    }

    fn registered(id: &str, account_id: &str) -> ProviderEdit {
        ProviderEdit::Upsert {
            id: id.to_owned(),
            entry: json!({"driver": "codex", "enabled": true, "config": {}}),
            binding: PersistedProviderBinding {
                driver: "codex".to_owned(),
                account_id: account_id.to_owned(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn saving_a_default_change_keeps_both_accounts_and_other_providers() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        write_t3_settings_json(
            state,
            &json!({"providerInstances": {"antigravity": {"driver": "antigravity", "enabled": true}}}),
        )
        .unwrap();
        save_edits(
            state,
            &[
                registered("codex", "original"),
                registered("codex-parallel", "selected"),
            ],
        )
        .unwrap();
        let mut selection = pending_default();
        assert!(!selection_is_on_disk(state, "codex", &selection).unwrap());
        selection.status.state = T3CodeDefaultSelectionState::RestartRequired;
        save_default_selections(state, &[("codex".to_owned(), selection.clone())].into()).unwrap();
        let mut new_default = registered("codex", "selected");
        if let ProviderEdit::Upsert { entry, .. } = &mut new_default {
            *entry = live::with_generation(entry.take(), &selection.generation).unwrap();
        }
        save_edits(
            state,
            &[new_default, registered("codex-parallel", "original")],
        )
        .unwrap();
        let recovered = load_default_selections(state).unwrap();
        assert_eq!(
            recovered["codex"].status.state,
            T3CodeDefaultSelectionState::RestartRequired
        );
        assert!(selection_is_on_disk(state, "codex", &recovered["codex"]).unwrap());
        selection.generation = "older-selection".into();
        assert!(!selection_is_on_disk(state, "codex", &selection).unwrap());

        let bindings = load_persisted_bindings(state);
        assert_eq!(bindings["codex"].account_id, "selected");
        assert_eq!(bindings["codex-parallel"].account_id, "original");
        assert_eq!(bindings.len(), 2);
        let settings = read_t3_settings_json(state).unwrap();
        assert_eq!(
            settings["providerInstances"]["antigravity"]["enabled"],
            true
        );
    }

    #[tokio::test]
    async fn saving_a_default_selection_does_not_wait_on_runs_or_block_provider_writes() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let selection = pending_default();
        save_default_selections(state, &[("codex".to_owned(), selection.clone())].into()).unwrap();
        let db = rusqlite::Connection::open(state.join("statev2.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE orchestration_v2_projection_runs (provider_instance_id TEXT, status TEXT);
             INSERT INTO orchestration_v2_projection_runs VALUES ('codex', 'running'), ('codex-parallel', 'queued');",
        ).unwrap();

        for driver in ["codex", "codex-parallel"] {
            db.execute("UPDATE orchestration_v2_projection_runs SET status = CASE WHEN provider_instance_id = ?1 THEN 'running' ELSE 'queued' END", [driver]).unwrap();
            let waiting = tokio::time::timeout(
                Duration::from_secs(2),
                process_default_selection("fixture", state, "codex", &selection, true, false),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(waiting);
            assert!(reserve_provider_write(state).is_ok());
            assert!(!selection_is_on_disk(state, "codex", &selection).unwrap());
            assert_eq!(
                load_default_selections(state).unwrap()["codex"]
                    .status
                    .state,
                T3CodeDefaultSelectionState::Waiting
            );
        }

        db.execute(
            "UPDATE orchestration_v2_projection_runs SET status = 'completed'",
            [],
        )
        .unwrap();
        assert!(
            !live::default_change_has_active_runs(state, "codex", &selection.source_id)
                .await
                .unwrap()
        );
    }

    #[test]
    fn removing_a_builtin_unlinks_and_disables_it_without_removing_other_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        write_t3_settings_json(
            state,
            &json!({
                "projectSettingsOverrides": {
                    "project-1": {
                        "providerInstances": {
                            "codex": {"enabled": true, "customModels": ["configured-model"]},
                            "codex-parallel": {"enabled": true}
                        }
                    }
                }
            }),
        )
        .unwrap();
        save_edits(
            state,
            &[
                registered("codex", "default"),
                registered("codex-parallel", "parallel"),
            ],
        )
        .unwrap();
        save_edits(
            state,
            &[ProviderEdit::Upsert {
                id: "codex".into(),
                entry: json!({"driver": "codex", "enabled": false, "config": {}}),
                binding: PersistedProviderBinding {
                    driver: "codex".into(),
                    unlinked: true,
                    ..Default::default()
                },
            }],
        )
        .unwrap();

        let settings = read_t3_settings_json(state).unwrap();
        assert_eq!(settings["providerInstances"]["codex"]["enabled"], false);
        assert_eq!(
            settings["providerInstances"]["codex-parallel"]["enabled"],
            true
        );
        let project_instances =
            &settings["projectSettingsOverrides"]["project-1"]["providerInstances"];
        assert!(project_instances["codex"].get("enabled").is_none());
        assert_eq!(
            project_instances["codex"]["customModels"][0],
            "configured-model"
        );
        assert_eq!(project_instances["codex-parallel"]["enabled"], true);
        let bindings = load_persisted_bindings(state);
        assert!(bindings["codex"].unlinked);
        assert!(bindings["codex"].account_id.is_empty());
        assert_eq!(bindings["codex-parallel"].account_id, "parallel");

        save_edits(state, &[ProviderEdit::Remove("codex-parallel".into())]).unwrap();
        assert!(load_persisted_bindings(state)
            .get("codex-parallel")
            .is_none());
        assert!(read_t3_settings_json(state).unwrap()["providerInstances"]
            .get("codex-parallel")
            .is_none());
    }
}
