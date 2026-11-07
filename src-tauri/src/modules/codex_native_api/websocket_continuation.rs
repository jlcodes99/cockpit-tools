//! One same-account replay for an explicitly rejected, unexecuted continuation.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn rejection(event: &Value) -> bool {
    if !matches!(
        super::super::events::response_terminal_kind(event),
        Some("error" | "response.failed")
    ) || !super::super::bootstrap::pre_execution(event)
    {
        return false;
    }
    let error = super::super::diagnostics::error_object(event);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code == "previous_response_not_found" {
        return true;
    }
    // Some Codex upstreams omit both code and param. Keep the message fallback
    // exact so unrelated malformed requests never enter replay.
    (code.is_empty() || code == "invalid_request_error")
        && error
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|text| {
                matches!(
                    text.trim().trim_end_matches('.'),
                    "Invalid `previous_response_id`" | "Invalid previous_response_id"
                )
            })
}

pub(super) fn initial_rejection(initial: &[UpstreamMessage]) -> Option<Value> {
    let (last, metadata) = initial.split_last()?;
    for message in metadata {
        let UpstreamMessage::Text(text) = message else {
            return None;
        };
        let event = serde_json::from_str::<Value>(text.as_str()).ok()?;
        if !super::super::bootstrap::handshake(&event) {
            return None;
        }
    }
    let UpstreamMessage::Text(text) = last else {
        return None;
    };
    let event = serde_json::from_str::<Value>(text.as_str()).ok()?;
    rejection(&event).then_some(event)
}

pub(super) async fn recover(
    state: &AppState,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    event: &Value,
    deadline: Deadline,
    reason: &'static str,
) -> Result<bool, ServiceError> {
    if prepared.continuation_recovered
        || !rejection(event)
        || !prepared.execution.policy.account_switch_rebuild
        || prepared.route.chatgpt_plan
        || prepared.compact
        || prepared.raw_chat.is_some()
        || prepared.route.wire_api != super::super::contract::WireApi::Responses
    {
        return Ok(false);
    }
    let Some(previous) = prepared
        .replay_body()
        .get("previous_response_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(false);
    };
    let digest = format!("{:x}", Sha256::digest(previous.as_bytes()));
    let result = deadline::timeout_at(deadline, mapping::recover_previous(state, prepared)).await;
    match result {
        Ok(Ok(())) => {
            let error = super::super::output_error::event_failure(event, 400);
            tracker.attempt_error(&error);
            tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
            tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
            tracker.prepared_metadata(prepared);
            tracing::info!(target: "codex_api", "[NativeAPI][continuation] request_id={} reason={} previous_digest={} history=complete recovery=retry_same_account",
                tracker.diagnostic_request_id(), reason, &digest[..16]);
            Ok(true)
        }
        Ok(Err(error)) if !state.stop.stopped() && tokio::time::Instant::now() < deadline => {
            tracing::info!(target: "codex_api", "[NativeAPI][continuation] request_id={} reason={} previous_digest={} recovery=skipped code={}",
                tracker.diagnostic_request_id(), reason, &digest[..16], error.code);
            Ok(false)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(super::super::retry::total_timeout()),
    }
}
