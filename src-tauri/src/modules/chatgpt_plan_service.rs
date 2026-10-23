//! Explicit activation of independent ChatGPT plan registrations in the API pool.
use crate::models::codex_local_access::{CodexLocalAccessGatewayMode, CodexLocalAccessState};
use crate::modules::{chatgpt_plan, codex_account, codex_local_access};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
use tauri::Emitter;

static ACTION_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn action_lock(id: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    codex_account::chatgpt_plan_projection_id(id)?;
    Ok(ACTION_LOCKS
        .lock()
        .map_err(|_| "CHATGPT_PLAN_STORAGE_BUSY")?
        .entry(id.into())
        .or_default()
        .clone())
}

#[derive(Serialize)]
pub struct ChatgptPlanServiceAccount {
    #[serde(flatten)]
    pub account: chatgpt_plan::ChatgptPlanAccountSummary,
    pub in_service: bool,
}

fn emit_service_state_updated() {
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("codex-local-access-state-updated", ());
    }
}

pub async fn list_accounts() -> Result<Vec<ChatgptPlanServiceAccount>, String> {
    let accounts = chatgpt_plan::list_accounts().await?;
    let (_, ids) = codex_local_access::chatgpt_plan_pool_snapshot().await?;
    let bindings = codex_local_access::chatgpt_plan_selected_bindings().await?;
    Ok(accounts
        .into_iter()
        .map(|mut account| {
            schedule_source_origin_backfill(&account);
            let in_service = codex_account::chatgpt_plan_projection_id(&account.id)
                .is_ok_and(|id| ids.contains(&id))
                || account.source_account_id.as_ref().is_some_and(|source| {
                    ids.contains(source) && bindings.get(source) == Some(&account.id)
                });
            if account.last_service_mode.is_none() {
                if let Some(mode) = inferred_service_mode(&account, &ids, &bindings) {
                    account.last_service_mode = Some(mode.into());
                    persist_inferred_mode_in_background(&account, mode);
                }
            }
            ChatgptPlanServiceAccount {
                account,
                in_service,
            }
        })
        .collect())
}

/// An explicit catalog refresh also updates an existing service reference.
/// Failure leaves the last known catalog and pool membership intact.
pub async fn list_models(id: &str) -> Result<Vec<chatgpt_plan::ChatgptPlanModel>, String> {
    let lock = action_lock(id)?;
    let _guard = lock.lock().await;
    let models = chatgpt_plan::list_models(id).await?;
    Ok(models)
}

pub async fn check_authorization(
    id: &str,
) -> Result<chatgpt_plan::ChatgptPlanAccountSummary, String> {
    let lock = action_lock(id)?;
    let _guard = lock.lock().await;
    chatgpt_plan::check_authorization(id).await
}

pub async fn join(id: &str, model: &str) -> Result<(), String> {
    let lock = action_lock(id)?;
    let _guard = lock.lock().await;
    let summary = chatgpt_plan::get_account(id).await?;
    if let Some(source) = summary.source_account_id.as_deref() {
        set_official_mode_locked(source, id, Some(model), false).await?;
        emit_service_state_updated();
        return Ok(());
    }
    Err("CHATGPT_PLAN_BINDING_REQUIRED".into())
}

async fn source_registration(
    source: &str,
    registration: Option<&str>,
) -> Result<Option<chatgpt_plan::ChatgptPlanAccountSummary>, String> {
    if let Some(id) = registration {
        let account = chatgpt_plan::get_account(id).await?;
        if account.source_account_id.as_deref() != Some(source) {
            return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
        }
        return Ok(Some(account));
    }
    let mut matches = chatgpt_plan::list_accounts()
        .await?
        .into_iter()
        .filter(|account| account.source_account_id.as_deref() == Some(source));
    let first = matches.next();
    if matches.next().is_some() {
        return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
    }
    Ok(first)
}

/// Select the credential used by this card in the current API service. Neither
/// authorization is copied into the other credential store or client profile.
pub async fn set_service_mode(
    source: &str,
    registration: Option<&str>,
    mode: &str,
    model: Option<&str>,
) -> Result<CodexLocalAccessState, String> {
    set_service_mode_with_intent(source, registration, mode, model, "join").await
}

pub async fn set_service_mode_with_intent(
    source: &str,
    registration: Option<&str>,
    mode: &str,
    model: Option<&str>,
    intent: &str,
) -> Result<CodexLocalAccessState, String> {
    if !matches!(intent, "join" | "switch") {
        return Err("CHATGPT_PLAN_SERVICE_MODE_INVALID".into());
    }
    let require_existing = intent == "switch";
    if !matches!(mode, "codex" | "official") {
        return Err("CHATGPT_PLAN_SERVICE_MODE_INVALID".into());
    }
    let source = source.trim();
    if source.is_empty()
        || source.len() > 256
        || source
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        || source.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
    {
        return Err("CHATGPT_PLAN_SOURCE_INVALID".into());
    }
    let registration = source_registration(source, registration).await?;
    let state = if let Some(registration) = registration {
        let lock = action_lock(&registration.id)?;
        let _guard = lock.lock().await;
        // Resolve again after waiting, including disconnected bindings when
        // returning to Codex. A broken official token must not trap that path.
        let latest = source_registration(source, Some(&registration.id))
            .await?
            .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
        if mode == "official" {
            set_official_mode_locked(source, &registration.id, model, require_existing).await?
        } else {
            if !latest.source_binding_valid {
                return Err(latest
                    .source_binding_error
                    .unwrap_or_else(|| "CHATGPT_PLAN_SOURCE_CHANGED".into()));
            }
            let projection = codex_account::chatgpt_plan_projection_id(&registration.id)?;
            let association_snapshot =
                chatgpt_plan::validate_source_association_snapshot(&registration.id, source)
                    .await?;
            let state = codex_local_access::switch_chatgpt_plan_service_member_intent(
                source,
                Some(&projection),
                false,
                None,
                Some(association_snapshot),
                require_existing,
                false,
            )
            .await?;
            remember_committed_mode(&registration.id, Some(source), "codex").await?;
            state
        }
    } else {
        if mode == "official" {
            return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
        }
        codex_local_access::switch_chatgpt_plan_service_member_intent(
            source,
            None,
            false,
            None,
            None,
            require_existing,
            false,
        )
        .await?
    };
    emit_service_state_updated();
    Ok(state)
}

async fn set_official_mode_locked(
    source: &str,
    id: &str,
    _model: Option<&str>,
    require_existing: bool,
) -> Result<CodexLocalAccessState, String> {
    let (mode, _) = codex_local_access::chatgpt_plan_pool_snapshot().await?;
    if mode != CodexLocalAccessGatewayMode::Native {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    chatgpt_plan::access_token(id, false).await?;
    chatgpt_plan::validate_source_binding(id, source).await?;
    let binding_snapshot = chatgpt_plan::validate_source_binding_snapshot(id, source).await?;
    let projection = codex_account::chatgpt_plan_projection_id(id)?;
    let state = codex_local_access::switch_chatgpt_plan_service_member_intent(
        source,
        Some(&projection),
        true,
        Some(binding_snapshot),
        None,
        require_existing,
        false,
    )
    .await?;
    remember_committed_mode(id, Some(source), "official").await?;
    refresh_catalog_in_background(id);
    Ok(state)
}

pub async fn remove_from_service(id: &str) -> Result<(), String> {
    let lock = action_lock(id)?;
    let _guard = lock.lock().await;
    remove_locked(id).await
}

async fn remove_locked(id: &str) -> Result<(), String> {
    let projection = codex_account::chatgpt_plan_projection_id(id)?;
    let bindings = codex_local_access::chatgpt_plan_selected_bindings().await?;
    if let Some((source, _)) = bindings
        .iter()
        .find(|(_, selected)| selected.as_str() == id)
    {
        codex_local_access::remove_local_access_account(source).await?;
    } else {
        codex_local_access::remove_local_access_account(&projection).await?;
    }
    emit_service_state_updated();
    Ok(())
}

pub async fn disconnect(id: &str) -> Result<chatgpt_plan::ChatgptPlanDisconnectResult, String> {
    let result = manage_authorization(id, "pause", true, false).await?;
    Ok(chatgpt_plan::ChatgptPlanDisconnectResult {
        remote_revoked: result.remote_revoked,
    })
}

/// Detach a broken/deleted source association without deleting either login.
/// Removing only the official member first also revokes its explicit key scope.
pub async fn unbind(id: &str) -> Result<chatgpt_plan::ChatgptPlanAccountSummary, String> {
    Ok(manage_authorization(id, "pause", false, true)
        .await?
        .authorization)
}

include!("chatgpt_plan_service_release.rs");
include!("chatgpt_plan_service_origins.rs");

fn existing_projection_models(id: &str) -> Vec<String> {
    codex_account::chatgpt_plan_projection_id(id)
        .ok()
        .and_then(|projection| codex_account::load_account(&projection))
        .map(|account| account.api_model_catalog)
        .unwrap_or_default()
}

static CATALOG_REFRESHES: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

struct CatalogRefresh(String);
impl Drop for CatalogRefresh {
    fn drop(&mut self) {
        if let Ok(mut pending) = CATALOG_REFRESHES.lock() {
            pending.remove(&self.0);
        }
    }
}
pub(crate) fn refresh_catalog_in_background(id: &str) {
    if !CATALOG_REFRESHES
        .lock()
        .map(|mut pending| pending.insert(id.to_owned()))
        .unwrap_or(false)
    {
        return;
    }
    let id = id.to_owned();
    tokio::spawn(async move {
        let _guard = CatalogRefresh(id.clone());
        // The existing client and token helpers bound network waits. Keeping
        // this single-flight guard through completion prevents duplicate work.
        if let Err(error) = list_models(&id).await {
            crate::modules::logger::log_info(&format!(
                "[ChatGPT Plan] Background catalog update failed: {error}"
            ));
        }
    });
}

fn inferred_service_mode(
    account: &chatgpt_plan::ChatgptPlanAccountSummary,
    ids: &[String],
    bindings: &HashMap<String, String>,
) -> Option<&'static str> {
    if codex_account::chatgpt_plan_projection_id(&account.id).is_ok_and(|id| ids.contains(&id))
        || account
            .source_account_id
            .as_ref()
            .is_some_and(|source| ids.contains(source) && bindings.get(source) == Some(&account.id))
    {
        Some("official")
    } else if account
        .source_account_id
        .as_ref()
        .is_some_and(|id| ids.contains(id))
    {
        Some("codex")
    } else {
        None
    }
}

static MODE_MIGRATIONS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
struct ModeMigration(String);
impl Drop for ModeMigration {
    fn drop(&mut self) {
        if let Ok(mut pending) = MODE_MIGRATIONS.lock() {
            pending.remove(&self.0);
        }
    }
}
fn persist_inferred_mode_in_background(
    account: &chatgpt_plan::ChatgptPlanAccountSummary,
    mode: &'static str,
) {
    if !MODE_MIGRATIONS
        .lock()
        .map(|mut pending| pending.insert(account.id.clone()))
        .unwrap_or(false)
    {
        return;
    }
    let id = account.id.clone();
    let source = account.source_account_id.clone();
    tokio::spawn(async move {
        let _flight = ModeMigration(id.clone());
        if let Err(error) =
            chatgpt_plan::remember_service_mode_if_unset(&id, source.as_deref(), mode).await
        {
            crate::modules::logger::log_warn(&format!(
                "[ChatGPT Plan] Mode migration deferred: {error}"
            ));
        }
    });
}

/// Removing a member cannot discard the only evidence of its previous mode.
/// Metadata-only and bounded; on contention leave membership intact for retry.
pub(crate) async fn remember_modes_before_removal(
    current: &[String],
    removed: &[String],
) -> Result<(), String> {
    if removed.is_empty() {
        return Ok(());
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let bindings = codex_local_access::chatgpt_plan_selected_bindings().await?;
        for account in chatgpt_plan::list_accounts().await? {
            if account.last_service_mode.is_some() {
                continue;
            }
            let is_removed = account
                .source_account_id
                .as_ref()
                .is_some_and(|id| removed.contains(id))
                || codex_account::chatgpt_plan_projection_id(&account.id)
                    .is_ok_and(|id| removed.contains(&id));
            if is_removed {
                if let Some(mode) = inferred_service_mode(&account, current, &bindings) {
                    chatgpt_plan::remember_service_mode_if_unset(
                        &account.id,
                        account.source_account_id.as_deref(),
                        mode,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?
}

async fn remember_committed_mode(id: &str, source: Option<&str>, mode: &str) -> Result<(), String> {
    if let Err(error) = chatgpt_plan::remember_service_mode(id, source, mode).await {
        emit_service_state_updated();
        crate::modules::logger::log_warn(&format!(
            "[ChatGPT Plan] Service mode committed but preference could not be saved: {error}"
        ));
        return Err("CHATGPT_PLAN_MODE_PREFERENCE_SAVE_FAILED".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn catalog_refresh_pending_for_test(id: &str) -> bool {
    CATALOG_REFRESHES.lock().expect("catalog refresh registry").contains(id)
}
