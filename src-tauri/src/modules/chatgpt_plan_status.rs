//! Authorization health is independent from a billable model test.
use super::{storage, types::Registration, ChatgptPlanAccountSummary};
use sha2::{Digest, Sha256};
use tauri::Emitter;

pub(crate) fn token_fingerprint(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub(super) fn requires_reauthorization(error: &str) -> bool {
    matches!(
        error,
        "CHATGPT_PLAN_REAUTH_REQUIRED"
            | "CHATGPT_PLAN_DISCONNECTED"
            | "CHATGPT_PLAN_PERMISSION_REQUIRED"
            | "CHATGPT_PLAN_IDENTITY_MISMATCH"
    )
}

pub(super) fn apply_result(record: &mut Registration, result: &Result<(), String>) {
    record.summary.last_checked_at = Some(chrono::Utc::now().timestamp());
    record.summary.check_error = result.as_ref().err().cloned();
    if result
        .as_ref()
        .err()
        .is_some_and(|error| requires_reauthorization(error))
    {
        record.summary.needs_reauth = true;
    }
    record.summary.updated_at = chrono::Utc::now().timestamp();
}

pub(super) fn emit_updated() {
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("chatgpt-plan-updated", ());
        let _ = app.emit("codex-local-access-state-updated", ());
    }
}

pub(super) fn save_record(record: &Registration) -> Result<(), String> {
    storage::save(record)?;
    crate::modules::codex_account::update_chatgpt_plan_projection_health(&record.summary)
}

pub(super) async fn record_result(
    id: &str,
    authorization_revision: Option<&str>,
    fingerprint: Option<&str>,
    result: Result<(), String>,
    model_catalog: Option<Vec<String>>,
) -> Result<ChatgptPlanAccountSummary, String> {
    let guard = storage::lock(id).await?;
    let id = id.to_owned();
    let revision = authorization_revision.map(str::to_owned);
    let fingerprint = fingerprint.map(str::to_owned);
    let summary = storage::blocking(move || {
        // Keep the process/file lock with the actual worker if its caller is cancelled.
        let _guard = guard;
        let mut record = storage::load(&id)?;
        if revision
            .as_ref()
            .is_some_and(|revision| revision != &record.authorization_revision)
        {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        if let Some(fingerprint) = fingerprint {
            if !record.summary.connected || record.encrypted_credentials.is_none() {
                return Ok(record.summary);
            }
            if token_fingerprint(&record.credentials()?.access_token) != fingerprint {
                return Ok(record.summary); // a stale request cannot disable a replacement token
            }
        }
        apply_result(&mut record, &result);
        if let Some(models) = model_catalog {
            record.model_catalog = models;
        }
        save_record(&record)?;
        Ok(record.summary)
    })
    .await?;
    emit_updated();
    Ok(summary)
}

/// Final inference evidence only. Transport failures and caller/model errors
/// never revoke a grant; an old access-token response cannot change a new grant.
pub(crate) async fn observe_inference_result(
    id: &str,
    revision: &str,
    fingerprint: &str,
    status: u16,
    code: Option<&str>,
) {
    let error = match code {
        Some("CHATGPT_PLAN_REAUTH_REQUIRED" | "CHATGPT_PLAN_PERMISSION_REQUIRED") => code,
        // Classified upstream configuration/availability failures do not revoke
        // the user's grant even when that upstream uses HTTP 401.
        Some(code) if code.starts_with("CHATGPT_PLAN_") => None,
        Some("upstream_unavailable" | "upstream_stream_error") => None,
        _ if status == 401 => Some("CHATGPT_PLAN_REAUTH_REQUIRED"),
        _ => None,
    };
    if let Some(error) = error {
        if let Err(error) = record_result(
            id,
            Some(revision),
            Some(fingerprint),
            Err(error.into()),
            None,
        )
        .await
        {
            crate::modules::logger::log_warn(&format!(
                "[ChatGPT Plan] Cannot save authorization health: {error}"
            ));
        }
    }
}

/// GET /models is a non-generating permission check. Failure is returned as
/// saved state so the dialog can distinguish retryable checks from reauth.
pub async fn check_authorization(id: &str) -> Result<ChatgptPlanAccountSummary, String> {
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    let starting = storage::read(id).await?;
    let authorization_revision = starting.authorization_revision.clone();
    let fingerprint = starting
        .credentials()
        .ok()
        .map(|credentials| token_fingerprint(&credentials.access_token));
    let result = super::list_models(id).await.map(|_| ());
    if result.is_ok() {
        super::official_health::reset_after_check(
            id,
            &authorization_revision,
            fingerprint.as_deref(),
            started_at_ms,
        )
        .await?;
    }
    // list_models and refresh persist exactly once using the actual token and
    // authorization epoch. Never re-save their error without that token fence.
    let summary = super::get_account(id).await?;
    if let Err(error) = result {
        if !summary.needs_reauth && summary.check_error.as_deref() != Some(error.as_str()) {
            return Err(error);
        }
    }
    Ok(summary)
}

pub(crate) async fn remember_service_mode(
    id: &str,
    source: Option<&str>,
    mode: &str,
) -> Result<(), String> {
    remember_mode(id, source, mode, false).await
}

pub(crate) async fn remember_service_mode_if_unset(
    id: &str,
    source: Option<&str>,
    mode: &str,
) -> Result<(), String> {
    remember_mode(id, source, mode, true).await
}

async fn remember_mode(
    id: &str,
    source: Option<&str>,
    mode: &str,
    only_unset: bool,
) -> Result<(), String> {
    let guard = storage::lock(id).await?;
    let id = id.to_owned();
    let source = source.map(str::to_owned);
    let mode = mode.to_owned();
    storage::blocking(move || {
        let _guard = guard;
        let mut record = storage::load(&id)?;
        if only_unset && record.summary.last_service_mode.is_some() {
            return Ok(());
        }
        if source.is_some() && record.summary.source_account_id != source {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        record.summary.last_service_mode = Some(mode);
        record.summary.updated_at = chrono::Utc::now().timestamp();
        storage::save(&record)
    })
    .await?;
    emit_updated();
    Ok(())
}
