//! Explicit associations only. A matching email prevents an obvious wrong-account
//! selection; it does not establish equality of workspaces or cross-client subs.
use super::{
    http, storage,
    types::{ChatgptPlanAccountSummary, Registration, SourceBinding},
};
use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexAuthMode};
use crate::modules::codex_account;
use sha2::{Digest, Sha256};

#[path = "chatgpt_plan_binding_cache.rs"]
mod cache;

pub(super) async fn list_accounts() -> Result<Vec<ChatgptPlanAccountSummary>, String> {
    let (accounts, pending) = storage::blocking(|| {
        let mut accounts = Vec::new();
        let mut pending = Vec::new();
        for record in storage::load_all()? {
            let (summary, job) = cache::cached_summary(&record)?;
            accounts.push(summary);
            if let Some(job) = job {
                pending.push(job);
            }
        }
        Ok((accounts, pending))
    })
    .await?;
    cache::schedule(pending);
    Ok(accounts)
}

pub(super) fn checked_summary(record: &Registration) -> Result<ChatgptPlanAccountSummary, String> {
    let mut summary = record.summary.clone();
    summary.source_binding_valid = false;
    summary.source_binding_error = None;
    if record.summary.source_account_id.is_some() || record.source_binding.is_some() {
        let job = cache::job(record)?;
        let outcome = association_outcome(record);
        return cache::finish(job, outcome).map(|(summary, _)| summary);
    }
    Ok(summary)
}

fn ready_summary(record: &Registration) -> Result<ChatgptPlanAccountSummary, String> {
    let summary = checked_summary(record)?;
    if !summary.source_binding_valid {
        return Err(summary
            .source_binding_error
            .unwrap_or_else(|| "CHATGPT_PLAN_SOURCE_CHANGED".into()));
    }
    Ok(summary)
}

fn association_outcome(record: &Registration) -> Result<(), String> {
    let id = record
        .summary
        .source_account_id
        .as_deref()
        .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
    let id = source_id(id)?;
    validate_record_source(record, &id)
}

pub(super) fn validate_bound_record(record: &Registration) -> Result<(), String> {
    if record.summary.source_account_id.is_none() && record.source_binding.is_none() {
        return if record.source_binding_required { Err("CHATGPT_PLAN_BINDING_REQUIRED".into()) } else { Ok(()) };
    }
    association_outcome(record)
}

fn source_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err("CHATGPT_PLAN_SOURCE_NOT_FOUND".into());
    }
    Ok(value.into())
}

fn identity_field(value: Option<&str>) -> Result<String, String> {
    value
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| "CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN".into())
}

fn email(value: &str) -> Result<String, String> {
    let value = value.trim();
    let mut parts = value.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if local.is_empty()
        || domain.is_empty()
        || parts.next().is_some()
        || value.len() > 320
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN".into());
    }
    Ok(value.to_ascii_lowercase())
}

pub(crate) fn source_identity_fingerprint(account: &CodexAccount) -> Result<String, String> {
    source_identity(account).map(|(_, fingerprint)| fingerprint)
}

fn source_identity(account: &CodexAccount) -> Result<(String, String), String> {
    if account.auth_mode != CodexAuthMode::OAuth
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
        || account.api_provider_mode != CodexApiProviderMode::OpenaiBuiltin
        || account
            .api_provider_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || account
            .upstream_grok_account_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || account.id.starts_with("codex_grok_")
        || account
            .openai_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Err("CHATGPT_PLAN_SOURCE_UNSUPPORTED".into());
    }
    let email = email(&account.email)?;
    let user_id = identity_field(account.user_id.as_deref())?;
    let account_id = identity_field(account.account_id.as_deref())?;
    let organization_id = account
        .organization_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if organization_id.is_some_and(|value| value.len() > 512 || value.chars().any(char::is_control))
    {
        return Err("CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN".into());
    }
    // Token generation, access/refresh/ID tokens, expiry and reauth flags are
    // deliberately absent. A normal refresh must preserve this association.
    let identity = serde_json::json!({
        "version":1,"source_account_id":account.id,"auth_kind":"codex_oauth",
        "email":email,"user_id":user_id,"account_id":account_id,"organization_id":organization_id,
    });
    Ok((
        email,
        format!("{:x}", Sha256::digest(identity.to_string().as_bytes())),
    ))
}

fn load_source(id: &str) -> Result<CodexAccount, String> {
    match codex_account::account_storage_status(id).map_err(|_| "CHATGPT_PLAN_STORAGE")? {
        codex_account::CodexAccountStorageStatus::Present => {}
        codex_account::CodexAccountStorageStatus::Missing
        | codex_account::CodexAccountStorageStatus::Deleted => {
            return Err("CHATGPT_PLAN_SOURCE_NOT_FOUND".into());
        }
    }
    let account = codex_account::load_account(id).ok_or("CHATGPT_PLAN_SOURCE_NOT_FOUND")?;
    if account.id != id {
        return Err("CHATGPT_PLAN_SOURCE_CHANGED".into());
    }
    Ok(account)
}

fn registration_email(record: &Registration) -> Result<String, String> {
    if record.summary.subject.trim().is_empty()
        || record.summary.client_id.trim().is_empty()
        || record.summary.client_id == "dynamic_agent_client"
    {
        return Err("CHATGPT_PLAN_IDENTITY_MISMATCH".into());
    }
    email(record.summary.email.as_deref().unwrap_or_default())
}

fn registration_identity(record: &Registration) -> Result<String, String> {
    if !record.summary.connected {
        return Err("CHATGPT_PLAN_DISCONNECTED".into());
    }
    if record.summary.needs_reauth {
        return Err("CHATGPT_PLAN_REAUTH_REQUIRED".into());
    }
    let email = registration_email(record)?;
    let credentials = record.credentials()?;
    if credentials.access_token.trim().is_empty()
        || credentials.refresh_token.trim().is_empty()
        || credentials.id_token.trim().is_empty()
    {
        return Err("CHATGPT_PLAN_REAUTH_REQUIRED".into());
    }
    http::scopes(&credentials.scopes.join(" "))?;
    Ok(email)
}

pub(super) fn validate_record_source(record: &Registration, id: &str) -> Result<(), String> {
    let binding = record
        .source_binding
        .as_ref()
        .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
    if record.summary.source_account_id.as_deref() != Some(id) || binding.source_account_id != id {
        return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
    }
    let registration_email = registration_email(record)?;
    let source = load_source(id)?;
    let (source_email, fingerprint) = source_identity(&source)?;
    if source_email != registration_email {
        return Err("CHATGPT_PLAN_SOURCE_EMAIL_MISMATCH".into());
    }
    if fingerprint != binding.identity_fingerprint {
        return Err("CHATGPT_PLAN_SOURCE_CHANGED".into());
    }
    Ok(())
}

pub async fn validate_source_binding(
    registration_id: &str,
    source_account_id: &str,
) -> Result<ChatgptPlanAccountSummary, String> {
    let source = source_id(source_account_id)?;
    let _guard = storage::lock(registration_id).await?;
    let registration = registration_id.to_owned();
    storage::blocking(move || {
        let record = storage::load(&registration)?;
        validate_record_source(&record, &source)?;
        registration_identity(&record)?;
        ready_summary(&record)
    })
    .await
}

pub async fn bind_account(
    registration_id: &str,
    source_account_id: &str,
) -> Result<ChatgptPlanAccountSummary, String> {
    let source = source_id(source_account_id)?;
    let _associations = storage::binding_lock().await?;
    let _registration = storage::lock(registration_id).await?;
    let accounts = storage::list().await?;
    if accounts.iter().any(|account| {
        account.id != registration_id
            && account.source_account_id.as_deref() == Some(source.as_str())
    }) {
        return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
    }
    let registration = registration_id.to_owned();
    storage::blocking(move || {
        let _associations = _associations;
        let _registration = _registration;
        let mut record = storage::load(&registration)?;
        let registration_email = registration_identity(&record)?;
        if record
            .summary
            .source_account_id
            .as_deref()
            .is_some_and(|id| id != source)
            || record
                .source_binding
                .as_ref()
                .is_some_and(|binding| binding.source_account_id != source)
        {
            return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
        }
        if record.source_binding.is_some() || record.summary.source_account_id.is_some() {
            validate_record_source(&record, &source)?;
            return ready_summary(&record);
        }
        let account = load_source(&source)?;
        let (source_email, fingerprint) = source_identity(&account)?;
        if source_email != registration_email {
            return Err("CHATGPT_PLAN_SOURCE_EMAIL_MISMATCH".into());
        }
        codex_account::set_chatgpt_plan_projection_source(&registration, &source)?;
        record.summary.source_account_id = Some(source.clone());
        record.source_binding_required = true;
        record.source_binding = Some(SourceBinding {
            source_account_id: source,
            identity_fingerprint: fingerprint,
        });
        record.authorization_revision = uuid::Uuid::new_v4().to_string();
        record.revision = uuid::Uuid::new_v4().to_string();
        record.summary.updated_at = chrono::Utc::now().timestamp();
        storage::save(&record)?;
        ready_summary(&record)
    })
    .await
}

/// Opaque, local-only proof of the identity and grant selected before a service
/// transaction. It deliberately carries no credentials and is never serialized.
#[derive(Clone)]
pub struct SourceBindingSnapshot {
    registration_id: String,
    source_account_id: String,
    identity_fingerprint: String,
    authorization_revision: String,
}

/// Association-only proof for returning to the original Codex credential.
/// Official token revocation, expiry or missing consent does not alter it.
#[derive(Clone)]
pub struct SourceAssociationSnapshot {
    registration_id: String,
    source_account_id: String,
    identity_fingerprint: String,
    registration_subject: String,
    registration_client_id: String,
}

pub async fn validate_source_association_snapshot(
    registration_id: &str,
    source_account_id: &str,
) -> Result<SourceAssociationSnapshot, String> {
    let source = source_id(source_account_id)?;
    let _guard = storage::lock(registration_id).await?;
    let registration = registration_id.to_owned();
    storage::blocking(move || {
        let record = storage::load(&registration)?;
        validate_record_source(&record, &source)?;
        let binding = record
            .source_binding
            .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
        Ok(SourceAssociationSnapshot {
            registration_id: registration,
            source_account_id: source,
            identity_fingerprint: binding.identity_fingerprint,
            registration_subject: record.summary.subject,
            registration_client_id: record.summary.client_id,
        })
    })
    .await
}

pub(crate) fn validate_source_association_snapshot_for_commit(
    snapshot: &SourceAssociationSnapshot,
) -> Result<(), String> {
    let record = storage::load(&snapshot.registration_id)?;
    if record.summary.subject != snapshot.registration_subject
        || record.summary.client_id != snapshot.registration_client_id
        || record.summary.source_account_id.as_deref() != Some(snapshot.source_account_id.as_str())
        || !record.source_binding.as_ref().is_some_and(|binding| {
            binding.source_account_id == snapshot.source_account_id
                && binding.identity_fingerprint == snapshot.identity_fingerprint
        })
    {
        return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
    }
    validate_record_source(&record, &snapshot.source_account_id)
}

pub async fn validate_source_binding_snapshot(
    registration_id: &str,
    source_account_id: &str,
) -> Result<SourceBindingSnapshot, String> {
    let source = source_id(source_account_id)?;
    let _guard = storage::lock(registration_id).await?;
    let registration = registration_id.to_owned();
    storage::blocking(move || {
        let record = storage::load(&registration)?;
        validate_record_source(&record, &source)?;
        registration_identity(&record)?;
        let binding = record
            .source_binding
            .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
        Ok(SourceBindingSnapshot {
            registration_id: registration,
            source_account_id: source,
            identity_fingerprint: binding.identity_fingerprint,
            authorization_revision: record.authorization_revision,
        })
    })
    .await
}

/// Runs on the service's blocking transaction worker immediately before CAS.
/// Reauthorization/disconnect or local ID reuse cannot bless the old snapshot.
pub(crate) fn validate_source_binding_snapshot_for_commit(
    snapshot: &SourceBindingSnapshot,
) -> Result<(), String> {
    let record = storage::load(&snapshot.registration_id)?;
    if record.authorization_revision != snapshot.authorization_revision
        || !record.source_binding.as_ref().is_some_and(|binding| {
            binding.source_account_id == snapshot.source_account_id
                && binding.identity_fingerprint == snapshot.identity_fingerprint
        })
    {
        return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
    }
    validate_record_source(&record, &snapshot.source_account_id)?;
    registration_identity(&record)?;
    Ok(())
}

/// Repair/reassociation primitive for the service transaction layer. Callers
/// first remove official service references; this never deletes either login.
pub async fn unbind_account(
    registration_id: &str,
    source_account_id: &str,
) -> Result<ChatgptPlanAccountSummary, String> {
    let source = source_id(source_account_id)?;
    let _associations = storage::binding_lock().await?;
    let _registration = storage::lock(registration_id).await?;
    let registration = registration_id.to_owned();
    storage::blocking(move || {
        let _associations = _associations;
        let _registration = _registration;
        let mut record = storage::load(&registration)?;
        if record
            .summary
            .source_account_id
            .as_deref()
            .is_some_and(|id| id != source)
            || record
                .source_binding
                .as_ref()
                .is_some_and(|binding| binding.source_account_id != source)
        {
            return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
        }
        if record.summary.source_account_id.is_some() || record.source_binding.is_some() {
            record.source_binding_required = true;
            record.summary.source_account_id = None;
            record.source_binding = None;
            record.authorization_revision = uuid::Uuid::new_v4().to_string();
            record.revision = uuid::Uuid::new_v4().to_string();
            record.summary.updated_at = chrono::Utc::now().timestamp();
            storage::save(&record)?;
        }
        Ok(record.summary)
    })
    .await
}

#[cfg(test)]
#[path = "chatgpt_plan_binding_tests.rs"]
mod tests;
