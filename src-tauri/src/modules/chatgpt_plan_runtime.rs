use super::{binding, storage};
use crate::models::codex::{CodexAccount, CodexAuthMode};
use crate::modules::codex_account;

/// Blocking metadata read; callers run this on a bounded worker. No network,
/// no credential refresh, and no writes to the source account are performed.
pub(crate) fn official_runtime_overlay(
    mut source: CodexAccount,
    registration: &str,
) -> CodexAccount {
    let source_id = source.id.clone();
    source.official_registration_id = Some(registration.into());
    source.api_provider_id = Some(codex_account::CHATGPT_PLAN_PROVIDER_ID.into());
    source.auth_mode = CodexAuthMode::Apikey;
    source.openai_api_key = Some("chatgpt-plan-managed".into());
    source.tokens.access_token.clear();
    source.tokens.id_token.clear();
    source.tokens.refresh_token = None;
    source.api_base_url = Some("https://api.openai.com/v1".into());
    source.api_wire_api = Some("responses".into());
    source.api_supports_websockets = false;
    source.api_supports_vision = true;
    source.api_model_catalog.clear();
    source.api_model_mappings.clear();
    source.agent_identity = None;
    source.quota = None;
    source.quota_error = None;
    source.requires_reauth = true;
    source.reauth_reason = Some("CHATGPT_PLAN_BINDING_REQUIRED".into());
    if let Ok(record) = storage::load(registration) {
        if binding::validate_record_source(&record, &source_id).is_ok() {
            source.requires_reauth = !record.summary.connected || record.summary.needs_reauth;
            source.reauth_reason = source.requires_reauth.then(|| {
                record
                    .summary
                    .check_error
                    .clone()
                    .unwrap_or_else(|| "CHATGPT_PLAN_REAUTH_REQUIRED".into())
            });
            source.api_model_catalog = record.model_catalog;
            if source.api_model_catalog.is_empty() {
                source.api_model_catalog = codex_account::chatgpt_plan_projection_id(registration)
                    .ok()
                    .and_then(|id| codex_account::load_account(&id))
                    .map(|account| account.api_model_catalog)
                    .unwrap_or_default();
            }
        }
    }
    source
}

pub(crate) async fn authorization_epoch_matches(
    id: &str,
    source: Option<&str>,
    revision: &str,
) -> bool {
    let id = id.to_owned();
    let source = source.map(str::to_owned);
    let revision = revision.to_owned();
    storage::blocking(move || {
        let record = storage::load(&id)?;
        if let Some(source) = source {
            binding::validate_record_source(&record, &source)?;
        }
        Ok(record.authorization_revision == revision
            && record.summary.connected
            && !record.summary.needs_reauth)
    })
    .await
    .unwrap_or(false)
}
