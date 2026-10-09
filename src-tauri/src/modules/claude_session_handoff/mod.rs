//! Explicit local Code handoff. Authentication stays with the existing account switcher.
mod catalog;
pub mod engine;
mod runtime;
mod storage_contract;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use crate::modules::{account, backup_storage, claude_account, provider_current_state};
use engine::{Identity, Preview, Roots, RunSummary};

static PROFILE_OPERATION: Mutex<()> = Mutex::new(());
static APPROVED_PREVIEWS: Mutex<std::collections::BTreeMap<String, PreviewContext>> =
    Mutex::new(std::collections::BTreeMap::new());

#[derive(Clone, PartialEq, Eq)]
struct PreviewContext {
    source: Identity,
    target: Identity,
    accounts: Vec<Identity>,
    archive: String,
    roots: [PathBuf; 3],
    app: PathBuf,
    allow_bundled_update: bool,
    created: std::time::Instant,
}

fn remember_preview(
    fingerprint: &str,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    archive: &str,
    roots: &[PathBuf; 3],
    app: &std::path::Path,
) -> Result<(), String> {
    let mut previews = APPROVED_PREVIEWS.lock().map_err(|_| "HANDOFF_BUSY")?;
    previews.retain(|_, context| context.created.elapsed() < Duration::from_secs(600));
    if previews.len() >= 32 {
        if let Some(oldest) = previews
            .iter()
            .min_by_key(|(_, context)| context.created)
            .map(|(key, _)| key.clone())
        {
            previews.remove(&oldest);
        }
    }
    previews.insert(
        fingerprint.into(),
        PreviewContext {
            source: source.clone(),
            target: target.clone(),
            accounts: accounts.to_vec(),
            archive: archive.into(),
            roots: roots.clone(),
            app: app.to_owned(),
            // The dialog explains that a bundled update triggered by normal
            // shutdown can settle and be rechecked before any publication.
            allow_bundled_update: true,
            created: std::time::Instant::now(),
        },
    );
    Ok(())
}

fn require_preview_context(
    fingerprint: &str,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    archive: &str,
    roots: &[PathBuf; 3],
    app: &std::path::Path,
) -> Result<bool, String> {
    let previews = APPROVED_PREVIEWS.lock().map_err(|_| "HANDOFF_BUSY")?;
    let context = previews.get(fingerprint).ok_or("PREVIEW_CHANGED")?;
    if context.source != *source
        || context.target != *target
        || context.accounts != accounts
        || context.roots != *roots
        || context.app != app
        || context.created.elapsed() >= Duration::from_secs(600)
    {
        return Err("PREVIEW_CHANGED".into());
    }
    if context.archive != archive {
        return Err("DESKTOP_CONTRACT_CHANGED".into());
    }
    Ok(context.allow_bundled_update)
}

fn contract_after_shutdown(
    initial: &storage_contract::Contract,
    shutdown: &runtime::ShutdownReceipt,
    allow_bundled_update: bool,
    app: &std::path::Path,
) -> Result<storage_contract::Contract, String> {
    if !initial.belongs_to_app(app) {
        return Err("DESKTOP_CONTRACT_CHANGED".into());
    }
    if shutdown.bundled_update_settled && allow_bundled_update {
        let final_contract = shutdown
            .settled_contract
            .as_ref()
            .ok_or("DESKTOP_CONTRACT_CHANGED")?
            .clone();
        if !final_contract.belongs_to_app(app) {
            return Err("DESKTOP_CONTRACT_CHANGED".into());
        }
        final_contract.assert_unchanged()?;
        tracing::info!(
            initial_archive = initial.fingerprint,
            final_archive = final_contract.fingerprint,
            "Claude handoff rechecked storage after controlled bundled update"
        );
        return Ok(final_contract);
    }
    initial.assert_unchanged()?;
    Ok(initial.clone())
}

fn approval_token() -> String {
    // A new preview must never silently rebind an older approval to a new App.
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(uuid::Uuid::new_v4().as_bytes()))
}

fn root_binding(roots: &Roots) -> [PathBuf; 3] {
    [
        roots.records.clone(),
        roots.pool.clone(),
        roots.state.clone(),
    ]
}

pub(crate) struct ProfileOperation {
    _guard: MutexGuard<'static, ()>,
}

/// Share this gate with Desktop profile injection, so a handoff cannot race a switch.
pub(crate) fn profile_operation() -> Result<ProfileOperation, String> {
    PROFILE_OPERATION
        .try_lock()
        .map(|guard| ProfileOperation { _guard: guard })
        .map_err(|_| "HANDOFF_BUSY".into())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountChoice {
    id: String,
    label: String,
    eligible: bool,
    reason: Option<String>,
    identity: Option<Identity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    supported: bool,
    reason: Option<String>,
    desktop_version: Option<String>,
    uses_saved_account_index: bool,
    accounts: Vec<AccountChoice>,
    runs: Vec<RunSummary>,
    current_account_id: Option<String>,
    current_identity: Option<Identity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffPreview {
    #[serde(flatten)]
    preview: Preview,
    desktop_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    run: RunSummary,
    reopened: bool,
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_switched: Option<bool>,
}

fn handoff_state_dir() -> Result<PathBuf, String> {
    Ok(crate::modules::data_paths::without_compatibility_alias(
        &account::resolve_data_dir()?.join("claude_session_handoff"),
    ))
}

fn roots() -> Result<Roots, String> {
    runtime::supported_profile()?;
    let home = dirs::home_dir().ok_or("HOME_UNAVAILABLE")?;
    Ok(Roots {
        records: claude_account::get_default_claude_desktop_user_data_dir()?
            .join("claude-code-sessions"),
        pool: home.join(".claude/projects"),
        // Durable journals are not pruned by the configurable behavior-backup policy.
        state: handoff_state_dir()?,
    })
}

/// Desktop profile changes must not consume a partially written sidebar.
pub(crate) fn require_no_pending_before_switch() -> Result<(), String> {
    let state = handoff_state_dir()?;
    engine::require_no_pending_state(&state)
}

pub(crate) fn require_no_pending_for_profile(
    target: &std::path::Path,
    default: &std::path::Path,
    state: &std::path::Path,
) -> Result<(), String> {
    let same_profile = target == default
        || target
            .canonicalize()
            .ok()
            .zip(default.canonicalize().ok())
            .is_some_and(|(target, default)| target == default);
    if same_profile {
        engine::require_no_pending_state(state)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn require_default_profile_ready(target: &std::path::Path) -> Result<(), String> {
    require_no_pending_for_profile(
        target,
        &claude_account::get_default_claude_desktop_user_data_dir()?,
        &handoff_state_dir()?,
    )
}

fn identity(account: &catalog::SavedAccount, roots: &Roots) -> Result<Identity, String> {
    let normalize = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(|id| id.hyphenated().to_string())
            .ok_or_else(|| "ACCOUNT_IDENTITY_MISSING".to_string())
    };
    let identity = Identity {
        account: normalize(&account.account_uuid)?,
        org: normalize(&account.organization_uuid)?,
    };
    let dir = roots.records.join(&identity.account).join(&identity.org);
    runtime::regular_directory(&dir)?;
    Ok(identity)
}

fn selection(
    roots: &Roots,
    source_id: &str,
    target_id: &str,
) -> Result<(Identity, Identity), String> {
    if source_id == target_id {
        return Err("SAME_ACCOUNT".into());
    }
    // Resolve only IDs present in Cockpit's saved account list, never IPC-provided paths.
    let accounts = catalog::accounts()?;
    let source = accounts
        .iter()
        .find(|a| a.id == source_id)
        .ok_or("ACCOUNT_NOT_FOUND")?;
    let target = accounts
        .iter()
        .find(|a| a.id == target_id)
        .ok_or("ACCOUNT_NOT_FOUND")?;
    let result = (identity(source, roots)?, identity(target, roots)?);
    if result.0 == result.1 {
        return Err("SAME_ACCOUNT".into());
    }
    Ok(result)
}

fn continuity_accounts(roots: &Roots) -> Result<Vec<Identity>, String> {
    let mut identities = std::collections::BTreeMap::new();
    for account in catalog::accounts()? {
        match identity(&account, roots) {
            Ok(identity) => {
                identities.insert(format!("{}/{}", identity.account, identity.org), identity);
            }
            Err(code) if code == "ACCOUNT_IDENTITY_MISSING" => (),
            Err(code) if code == "SIDEBAR_NOT_INITIALIZED" => {
                // An account that never initialized Code has no sidebar to collect.
                // Do not treat an existing but unreadable namespace as empty.
                if let (Some(account), Some(org)) =
                    (&account.account_uuid, &account.organization_uuid)
                {
                    if roots.records.join(account).join(org).exists() {
                        return Err(code);
                    }
                }
            }
            Err(code) => return Err(code),
        }
    }
    Ok(identities.into_values().collect())
}

pub fn status() -> Result<Status, String> {
    let profile = roots();
    let version = runtime::desktop_version();
    let supported = profile
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
        .and_then(|_| runtime::app_bundle())
        .and_then(|app| storage_contract::inspect(&app).map(|_| ()));
    let accounts: Vec<AccountChoice> = catalog::accounts()?
        .into_iter()
        .map(|a| {
            let eligible = profile
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|r| identity(&a, r));
            AccountChoice {
                id: a.id,
                label: a.email,
                eligible: eligible.is_ok(),
                reason: eligible.as_ref().err().cloned(),
                identity: eligible.ok(),
            }
        })
        .collect();
    let current = runtime::current_identity().ok().flatten();
    let current_account_id = accounts
        .iter()
        .find(|account| account.identity.as_ref() == current.as_ref() && account.identity.is_some())
        .map(|account| account.id.clone());
    let runs = match &profile {
        Ok(r) => engine::list_runs(r)?,
        Err(_) => Vec::new(),
    };
    Ok(Status {
        supported: supported.is_ok(),
        reason: supported.err(),
        desktop_version: version.ok(),
        uses_saved_account_index: catalog::external_index()?.is_some(),
        accounts,
        runs,
        current_account_id,
        current_identity: current,
    })
}

pub fn preview(source_id: &str, target_id: &str) -> Result<HandoffPreview, String> {
    let _operation = profile_operation()?;
    let roots = roots()?;
    let app = runtime::app_bundle()?;
    let contract = storage_contract::inspect(&app)?;
    let (source, target) = selection(&roots, source_id, target_id)?;
    let accounts = continuity_accounts(&roots)?;
    let unknown_fields = engine::unknown_persisted_fields(&contract.projected_fields);
    let mut preview = engine::preview_continuity_with_fields(
        &roots,
        &source,
        &target,
        &accounts,
        &unknown_fields,
    )?;
    contract.assert_unchanged()?;
    preview.fingerprint = approval_token();
    remember_preview(
        &preview.fingerprint,
        &source,
        &target,
        &accounts,
        &contract.fingerprint,
        &root_binding(&roots),
        &app,
    )?;
    let version = runtime::desktop_version().unwrap_or_default();
    Ok(HandoffPreview {
        preview,
        desktop_version: version,
    })
}

pub fn apply(
    source_id: &str,
    target_id: &str,
    fingerprint: &str,
    version: &str,
) -> Result<Outcome, String> {
    let _operation = profile_operation()?;
    apply_locked(
        source_id,
        target_id,
        fingerprint,
        version,
        true,
        &mut |_, _, _| {},
    )
}

pub fn apply_and_switch(
    source_id: &str,
    target_id: &str,
    fingerprint: &str,
    version: &str,
) -> Result<Outcome, String> {
    apply_and_switch_observed(
        source_id,
        target_id,
        fingerprint,
        version,
        &mut |_, _, _| {},
    )
}

pub fn apply_and_switch_observed(
    source_id: &str,
    target_id: &str,
    fingerprint: &str,
    version: &str,
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<Outcome, String> {
    let _operation = profile_operation()?;
    if catalog::external_index()?.is_some() {
        return Err("ACCOUNT_INDEX_UNAVAILABLE".into());
    }
    let target = claude_account::load_account(target_id).ok_or("ACCOUNT_NOT_FOUND")?;
    if target.auth_mode != crate::models::claude::ClaudeAuthMode::DesktopOAuth {
        return Err("ACCOUNT_NOT_ELIGIBLE".into());
    }
    let current_roots = roots()?;
    let target_identity = identity(
        &catalog::SavedAccount {
            id: target.id.clone(),
            email: target.email.clone(),
            account_uuid: target.account_uuid.clone(),
            organization_uuid: target.organization_uuid.clone(),
        },
        &current_roots,
    )?;
    if selection(&current_roots, source_id, target_id)?.1 != target_identity {
        return Err("ACCOUNT_IDENTITY_CHANGED".into());
    }
    let mut outcome = apply_locked(source_id, target_id, fingerprint, version, false, progress)?;
    // The run has committed. Preserve its receipt even if sign-in restoration fails.
    if let Some(warning) = outcome.warning.take() {
        outcome.warning = Some(warning);
        outcome.account_switched = Some(false);
        return Ok(outcome);
    }
    progress("switching", 0, 0);
    // Authentication restoration belongs to Cockpit's existing switcher. A
    // private diagnostic log is optional UI evidence, never a required success
    // signal: current Desktop builds may omit initialization messages entirely.
    match claude_account::inject_to_claude_with_profile_gate(target_id, &_operation) {
        Ok(()) => {
            progress("confirming", 0, 0);
            let launch = runtime::app_bundle().and_then(|app| runtime::wait_for_launch(&app));
            if !preserve_committed_launch_receipt(&mut outcome, launch) {
                return Ok(outcome);
            }
            outcome.account_switched = Some(true);
            outcome.reopened = true;
            if provider_current_state::set_current_account_id(
                "claude_desktop_account",
                Some(target_id),
            )
            .is_err()
            {
                outcome.warning = Some("CURRENT_STATE_UPDATE_FAILED".into());
            }
        }
        Err(_) => {
            outcome.account_switched = Some(false);
            outcome.warning = Some("ACCOUNT_SWITCH_UNCERTAIN".into());
        }
    }
    progress("complete", 0, 0);
    Ok(outcome)
}

fn preserve_committed_launch_receipt(outcome: &mut Outcome, launch: Result<(), String>) -> bool {
    if let Err(reason) = launch {
        outcome.account_switched = Some(false);
        outcome.reopened = false;
        outcome.warning = Some(reason);
        false
    } else {
        true
    }
}

fn apply_locked(
    source_id: &str,
    target_id: &str,
    fingerprint: &str,
    _version: &str,
    reopen_after_apply: bool,
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<Outcome, String> {
    progress("checking", 0, 0);
    let roots = roots()?;
    let app = runtime::app_bundle()?;
    let contract = storage_contract::inspect(&app)?;
    let unknown_fields = engine::unknown_persisted_fields(&contract.projected_fields);
    catalog::assert_other_manager_closed()?;
    let (source, target) = selection(&roots, source_id, target_id)?;
    let accounts = continuity_accounts(&roots)?;
    let allow_bundled_update = require_preview_context(
        fingerprint,
        &source,
        &target,
        &accounts,
        &contract.fingerprint,
        &root_binding(&roots),
        &app,
    )?;
    // The user's scope is the complete saved-account catalog, not a fixed list
    // of byte images. Desktop can legitimately flush newer state on quit.
    // Validate the preview token, then freeze a fresh complete plan after quit.
    let plan_preview = engine::preview_continuity_with_fields(
        &roots,
        &source,
        &target,
        &accounts,
        &unknown_fields,
    )?;
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("PREVIEW_CHANGED".into());
    }
    if plan_preview.missing > 0 {
        return Err("UNRESOLVED_SOURCE_ROWS".into());
    }
    // A single Desktop sidebar row cannot expose both independently advanced
    // CLI branches. Do not offer a partial result as a complete handoff, and
    // do not quit Desktop merely to discover this conflict after the fact.
    if plan_preview.stale > 0 || plan_preview.replaced_branches > 0 {
        return Err("UNRESOLVED_ACTIVE_BRANCHES".into());
    }
    contract.assert_unchanged()?;
    progress("stopping", 0, 0);
    let shutdown = runtime::quit_and_settle(&app, allow_bundled_update, &mut || {
        progress("updating", 0, 0)
    })?;
    let was_running = shutdown.was_running;
    let contract = runtime::assert_publication_quiet(&app)
        .and_then(|_| {
            if runtime::app_bundle()? != app
                || root_binding(&self::roots()?) != root_binding(&roots)
            {
                return Err("DESKTOP_CONTRACT_CHANGED".into());
            }
            if selection(&roots, source_id, target_id)? != (source.clone(), target.clone())
                || continuity_accounts(&roots)? != accounts
            {
                return Err("ACCOUNT_IDENTITY_CHANGED".into());
            }
            contract_after_shutdown(&contract, &shutdown, allow_bundled_update, &app)
        })
        .map_err(|code| {
            if was_running {
                let _ = runtime::reopen_if_closed(&app);
            }
            code
        })?;
    // This final binding is distinct from the immutable initial preview. It may
    // be renewed only after the approved shutdown update, never after writes.
    let unknown_fields = engine::unknown_persisted_fields(&contract.projected_fields);
    progress("checking", 0, 0);
    let mut guard = || {
        catalog::assert_other_manager_closed()?;
        runtime::assert_publication_quiet(&app)?;
        contract.assert_unchanged()?;
        if runtime::app_bundle()? != app || root_binding(&self::roots()?) != root_binding(&roots) {
            return Err("DESKTOP_CONTRACT_CHANGED".into());
        }
        if selection(&roots, source_id, target_id)? != (source.clone(), target.clone()) {
            return Err("ACCOUNT_IDENTITY_CHANGED".into());
        }
        if continuity_accounts(&roots)? != accounts {
            return Err("ACCOUNT_IDENTITY_CHANGED".into());
        }
        Ok(())
    };
    // No handoff write exists yet. If an immediate post-quit check rejects the
    // operation, restore the user's Desktop session before surfacing the error.
    if let Err(code) = guard() {
        if was_running {
            let _ = runtime::reopen_if_closed(&app);
        }
        return Err(code);
    }
    let frozen_preview = engine::preview_continuity_with_fields(
        &roots,
        &source,
        &target,
        &accounts,
        &unknown_fields,
    )
    .and_then(|preview| {
        if preview.missing > 0 || preview.stale > 0 || preview.replaced_branches > 0 {
            Err("UNRESOLVED_SOURCE_ROWS".into())
        } else {
            Ok(preview)
        }
    })
    .map_err(|code| {
        if was_running {
            let _ = runtime::reopen_if_closed(&app);
        }
        code
    })?;
    let backup = backup_storage::transaction_recovery_dir(
        "claude-handoff",
        &uuid::Uuid::new_v4().to_string(),
    )
    .map(|path| crate::modules::data_paths::without_compatibility_alias(&path))
    .map_err(|code| {
        if was_running {
            let _ = runtime::reopen_if_closed(&app);
        }
        code
    })?;
    let mut published_run = None;
    let mut quick_guard = || {
        catalog::assert_other_manager_closed()?;
        runtime::assert_publication_quiet(&app)?;
        contract.assert_unchanged_fast()
    };
    let attempt = engine::apply_continuity_with_fields_and_progress(
        &roots,
        &source,
        &target,
        &accounts,
        &unknown_fields,
        &frozen_preview.fingerprint,
        &backup,
        &mut guard,
        Some(&mut quick_guard),
        &mut |id| published_run = Some(id.to_owned()),
        progress,
    );
    let run = match attempt {
        Ok(run) => run,
        Err(code) => {
            tracing::warn!(
                "Claude handoff rejected: code={code}, journal_published={}",
                published_run.is_some()
            );
            if let Some(id) = published_run {
                if let Err(reason) =
                    engine::record_failure(&roots, &id, &runtime::diagnostic_code(&code))
                {
                    tracing::warn!("Claude handoff failure receipt unavailable: code={reason}");
                }
                // A failed copy is not a usable account switch. While its own
                // images and backups are still intact, undo it before letting
                // Desktop consume the partially written destination namespace.
                let mut recovery_guard = || {
                    catalog::assert_other_manager_closed()?;
                    runtime::assert_quiet()
                };
                let recovered = runtime::wait_until_quiet(Duration::from_secs(15))
                    .and_then(|_| engine::rollback(&roots, &id, &mut recovery_guard));
                if let Err(reason) = recovered {
                    tracing::warn!("Claude handoff recovery required: code={reason}");
                    return Err("RECOVERY_REQUIRED".into());
                }
            }
            if was_running {
                runtime::reopen_if_closed(&app)?;
            }
            return Err(code);
        }
    };
    // A reopen failure must not turn a committed handoff into an apparent failed transfer.
    Ok(finish(
        run,
        was_running && reopen_after_apply,
        &app,
        guard(),
    ))
}

pub fn rollback(run_id: &str) -> Result<Outcome, String> {
    let _operation = profile_operation()?;
    catalog::assert_other_manager_closed()?;
    let roots = roots()?;
    let app = runtime::app_bundle()?;
    runtime::quit_normally(&app)?;
    let run = engine::rollback(&roots, run_id, &mut || {
        catalog::assert_other_manager_closed()?;
        runtime::assert_quiet()
    })?;
    // Recovery leaves the app stopped: do not automatically dispatch restored quit markers.
    Ok(Outcome {
        run,
        reopened: false,
        warning: Some("RECOVERED_APP_CLOSED".into()),
        account_switched: None,
    })
}

fn finish(
    run: RunSummary,
    was_running: bool,
    app: &PathBuf,
    checked: Result<(), String>,
) -> Outcome {
    let result = checked.and_then(|_| {
        if was_running {
            runtime::reopen(app)
        } else {
            Ok(())
        }
    });
    Outcome {
        run,
        reopened: was_running && result.is_ok(),
        warning: result.err(),
        account_switched: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_run_is_retained_when_post_switch_app_configuration_is_unavailable() {
        let run: RunSummary = serde_json::from_value(serde_json::json!({
            "id":"run-00000000-0000-4000-8000-000000000001","state":"applied","createdAt":1,
            "created":2,"updated":3,"source":{"account":"source","org":"org"},
            "target":{"account":"target","org":"org"},"backupDir":"/synthetic/backup"
        }))
        .unwrap();
        let mut outcome = Outcome {
            run,
            reopened: false,
            warning: None,
            account_switched: None,
        };
        assert!(!preserve_committed_launch_receipt(
            &mut outcome,
            Err("APP_PATH_NOT_FOUND".into())
        ));
        let receipt = serde_json::to_value(outcome).unwrap();
        assert_eq!(receipt["run"]["state"], "applied");
        assert_eq!(receipt["run"]["created"], 2);
        assert_eq!(receipt["warning"], "APP_PATH_NOT_FOUND");
        assert_eq!(receipt["accountSwitched"], false);
    }

    #[test]
    fn approval_binds_archive_roots_and_roster_without_version_allowlist() {
        let identity = |n| Identity {
            account: format!("00000000-0000-4000-8000-{n:012}"),
            org: "00000000-0000-4000-8000-000000000009".into(),
        };
        let (source, target) = (identity(1), identity(2));
        let accounts = vec![source.clone(), target.clone()];
        let roots = [
            PathBuf::from("/records"),
            PathBuf::from("/pool"),
            PathBuf::from("/state"),
        ];
        let app = std::path::Path::new("/synthetic/Claude.app");
        let old_token = approval_token();
        let new_token = approval_token();
        assert_ne!(old_token, new_token);
        remember_preview(
            &old_token,
            &source,
            &target,
            &accounts,
            "archive-A",
            &roots,
            app,
        )
        .unwrap();
        remember_preview(
            &new_token,
            &source,
            &target,
            &accounts,
            "archive-B",
            &roots,
            app,
        )
        .unwrap();
        assert!(require_preview_context(
            &old_token,
            &source,
            &target,
            &accounts,
            "archive-A",
            &roots,
            app
        )
        .is_ok());
        // Identical sidebar data after an update cannot rebind the prior token.
        assert_eq!(
            require_preview_context(
                &old_token,
                &source,
                &target,
                &accounts,
                "archive-B",
                &roots,
                app
            )
            .unwrap_err(),
            "DESKTOP_CONTRACT_CHANGED"
        );
        assert!(require_preview_context(
            &new_token,
            &source,
            &target,
            &accounts,
            "archive-B",
            &roots,
            app
        )
        .is_ok());
        assert_eq!(
            require_preview_context(
                &new_token,
                &source,
                &identity(3),
                &accounts,
                "archive-B",
                &roots,
                app
            )
            .unwrap_err(),
            "PREVIEW_CHANGED"
        );
        assert_eq!(
            require_preview_context(
                &new_token,
                &source,
                &target,
                &[source.clone(), target.clone(), identity(3)],
                "archive-B",
                &roots,
                app
            )
            .unwrap_err(),
            "PREVIEW_CHANGED"
        );
        let changed = [
            PathBuf::from("/other-records"),
            roots[1].clone(),
            roots[2].clone(),
        ];
        assert_eq!(
            require_preview_context(
                &new_token,
                &source,
                &target,
                &accounts,
                "archive-B",
                &changed,
                app
            )
            .unwrap_err(),
            "PREVIEW_CHANGED"
        );
        assert_eq!(
            require_preview_context(
                &new_token,
                &source,
                &target,
                &accounts,
                "archive-B",
                &roots,
                std::path::Path::new("/other/Claude.app")
            )
            .unwrap_err(),
            "PREVIEW_CHANGED"
        );
        assert_eq!(
            require_preview_context(
                "unknown",
                &source,
                &target,
                &accounts,
                "archive-B",
                &roots,
                app
            )
            .unwrap_err(),
            "PREVIEW_CHANGED"
        );
    }

    #[test]
    fn profile_gate_blocks_a_competing_switch_until_handoff_finishes() {
        let operation = profile_operation().unwrap();
        let blocked = std::thread::spawn(|| profile_operation().err())
            .join()
            .unwrap();
        assert_eq!(blocked.as_deref(), Some("HANDOFF_BUSY"));
        drop(operation);
        assert!(profile_operation().is_ok());
    }
}
