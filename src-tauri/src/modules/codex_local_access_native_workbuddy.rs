//! Native WorkBuddy ACP account metadata and request headers.
//! The source account remains in encrypted WorkBuddy storage and is loaded only at request time.

use crate::models::{codex::CodexAccount, workbuddy::WorkbuddyAccount};
use axum::http::{HeaderMap, HeaderValue};

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
