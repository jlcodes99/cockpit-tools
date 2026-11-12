//! The desktop client's paginated banked-reset history contract.
use super::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryEvent {
    pub id: String,
    pub kind: String,
    pub occurred_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryPage {
    pub events: Vec<HistoryEvent>,
    pub next_cursor: Option<String>,
}

fn message(key: &str) -> String {
    crate::modules::i18n::translate(
        &crate::modules::config::get_user_config().language,
        &format!("codex.resetHistory.{key}"),
        &[],
    )
}

async fn request(account: &CodexAccount, url: &str) -> Result<HistoryPage, String> {
    let response = send_codex_api_request(account, Method::GET, url, None).await?;
    if !response.status.is_success() {
        // Preserve the status for the existing 401 refresh path, without exposing credentials.
        return Err(format!(
            "{}: HTTP {}",
            message("loadFailed"),
            response.status
        ));
    }
    let mut page: HistoryPage = serde_json::from_str(&response.body)
        .map_err(|error| format!("{}: {error}", message("loadFailed")))?;
    page.next_cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
    Ok(page)
}

pub async fn fetch(account_id: &str, cursor: Option<String>) -> Result<HistoryPage, String> {
    let mut account = codex_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err(message("officialUnavailable"));
    }
    let mut url = reqwest::Url::parse(&format!("{RESET_CREDITS_URL}/history"))
        .map_err(|error| error.to_string())?;
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
        url.query_pairs_mut().append_pair("cursor", &cursor);
    }
    // Same account/proxy/Agent Identity path as reset-credit details. Only refresh
    // an expired token or an explicit 401, retaining the same pagination cursor.
    if !account.is_agent_identity_auth()
        && crate::modules::codex_oauth::is_token_expired(&account.tokens.access_token)
    {
        refresh(&mut account).await?;
    }
    match request(&account, url.as_str()).await {
        Err(error) if !account.is_agent_identity_auth() && is_unauthorized_error(&error) => {
            refresh(&mut account).await?;
            request(&account, url.as_str()).await
        }
        result => result,
    }
}

async fn refresh(account: &mut CodexAccount) -> Result<(), String> {
    refresh_account_tokens(account, "reset history").await?;
    sync_subscription_expiry_from_current_id_token(account);
    let saved = account.clone();
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::save_account_preserving_egress_proxy(&saved).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_reset_official_history_keeps_events_and_opaque_cursor() {
        let page: HistoryPage = serde_json::from_value(serde_json::json!({
            "events": [
                {"id":"grant-1", "kind":"granted", "occurred_at":"2026-11-01T19:55:00Z"},
                {"id":"use-1", "kind":"redeemed", "occurred_at":"2026-11-02T01:00:00Z"}
            ], "next_cursor":"opaque+/=cursor"
        }))
        .unwrap();
        assert_eq!(page.events[0].kind, "granted");
        assert_eq!(page.events[1].kind, "redeemed");
        assert_eq!(page.next_cursor.as_deref(), Some("opaque+/=cursor"));
        assert!(
            serde_json::from_value::<HistoryPage>(serde_json::json!({"error":"not history"}))
                .is_err()
        );
    }
}
