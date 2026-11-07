//! Apply the selected account's installation at the final protocol boundary.
use axum::http::HeaderMap;
use serde_json::Value;

use super::contract::{Route, ServiceError, WireApi};

pub(super) fn apply(
    route: &Route,
    headers: &mut HeaderMap,
    body: &mut Value,
) -> Result<(), ServiceError> {
    // Chat and xAI have their own wire contracts. Their transports are still
    // account-scoped, but they must not acquire unsupported Codex JSON fields.
    if route.chatgpt_plan
        || route.wire_api != WireApi::Responses
        || route.headers.contains_key("x-xai-token-auth")
    {
        return Ok(());
    }
    if let Some(profile) = &route.installation_profile {
        crate::modules::codex_api_identity::apply_headers(profile, headers)
            .and_then(|_| crate::modules::codex_api_identity::apply_body(profile, body))
            .map_err(|message| ServiceError::new(400, "invalid_protocol_identity", message))?;
    }
    Ok(())
}
