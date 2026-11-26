//! Native WorkBuddy ACP account metadata and request headers.
//! The source account remains in encrypted WorkBuddy storage and is loaded only at request time.

use crate::models::{codex::CodexAccount, workbuddy::WorkbuddyAccount};
use axum::http::{HeaderMap, HeaderValue};
use std::sync::Arc;

pub(super) fn validate_reference(account: &CodexAccount) -> Result<(), String> {
    if !crate::modules::codex_account::is_workbuddy_upstream_provider(account)
        || account.api_base_url.as_deref()
            != Some(crate::modules::codex_account::WORKBUDDY_ACP_BASE_URL)
    {
        return Err("WORKBUDDY_INVALID_REFERENCE".to_string());
    }
    Ok(())
}

pub(super) fn route(
    account: &CodexAccount,
    base_url: &str,
    model: String,
    permit: Option<Arc<dyn Send + Sync>>,
) -> Result<super::native_contract::Route, super::native_contract::ServiceError> {
    validate_reference(account).map_err(|_| {
        super::native_error(
            400,
            "invalid_workbuddy_account",
            "WorkBuddy ACP requires a linked account with the native internal route",
        )
    })?;
    if base_url != crate::modules::codex_account::WORKBUDDY_ACP_BASE_URL {
        return Err(super::native_error(
            400,
            "invalid_upstream",
            "WorkBuddy ACP requires the exact native internal route",
        ));
    }
    // Required by the shared Route shape, but never used as an HTTP transport:
    // the ACP executor loads the latest source account and creates its client.
    let client = reqwest::Client::builder().no_proxy().build().map_err(|_| {
        super::native_error(
            503,
            "workbuddy_unavailable",
            "Unable to prepare WorkBuddy route",
        )
    })?;
    Ok(super::native_contract::Route {
        account_id: account.id.clone(),
        continuity_account_id: None,
        owner_id: super::native_account_owner(account, base_url),
        installation_profile: None,
        transport_identity: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
        official_fallback_failure: None,
        base_url: base_url.to_string(),
        model,
        // No source token (or fabricated OpenAI key) enters the generic route.
        headers: HeaderMap::new(),
        client,
        proxy_url: None,
        proxy_route: None,
        chatgpt: false,
        chatgpt_plan: false,
        wire_api: super::native_contract::WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit,
    })
}

pub(super) fn credential_headers(source: &WorkbuddyAccount) -> Result<HeaderMap, String> {
    if source.access_token.trim().is_empty() {
        return Err("WorkBuddy access token is empty".to_string());
    }
    let mut headers = crate::modules::workbuddy_oauth::billing_headers(
        source.access_token.trim(),
        source.uid.as_deref(),
        source.enterprise_id.as_deref(),
        source.domain.as_deref(),
        "en",
    )
    .map_err(|error| error.replace(source.access_token.trim(), "<redacted>"))?;
    if let Some(value) = headers.get_mut("authorization") {
        let raw = value.as_bytes().to_vec();
        let mut sensitive =
            HeaderValue::from_bytes(&raw).map_err(|_| "Invalid WorkBuddy authorization header")?;
        sensitive.set_sensitive(true);
        *value = sensitive;
    }
    Ok(headers)
}

pub(super) fn bind_owner(account: &mut CodexAccount, source: &WorkbuddyAccount) {
    account.user_id = source.uid.clone();
    account.account_id = Some(source.id.clone());
    account.openai_api_key = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_binding_does_not_copy_token() {
        let mut account = CodexAccount::new_api_key(
            "linked".into(),
            "fixture@example.invalid".into(),
            String::new(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some(crate::modules::codex_account::WORKBUDDY_ACP_BASE_URL.into()),
            Some("workbuddy_acp".into()),
            Some("WorkBuddy".into()),
            vec!["workbuddy-default".into()],
        );
        let source: WorkbuddyAccount = serde_json::from_value(serde_json::json!({
            "id":"workbuddy-source",
            "email":"fixture@example.invalid",
            "access_token":"secret-token",
            "created_at":1,
            "last_used":1
        }))
        .unwrap();
        bind_owner(&mut account, &source);
        assert_eq!(account.account_id.as_deref(), Some("workbuddy-source"));
        assert!(account.openai_api_key.is_none());
        assert!(!serde_json::to_string(&account)
            .unwrap()
            .contains("secret-token"));
    }
}
