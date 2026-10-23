//! Continue recovery after exposing only bounded Responses control metadata.
use crate::modules::codex_native_api::deadline::Deadline;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn recover(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    fence: &mut super::super::bootstrap::Fence,
    frame: &SseFrame,
    previous_wait: Option<std::time::Duration>,
    policy: &ExecutionPolicy,
    deadline: Deadline,
    streaming: bool,
) -> Result<Option<HttpResponseStream>, ServiceError> {
    let original = frame.identity_event.as_ref().or(frame.value.as_ref());
    let Some((status, _)) = fence.observe(
        original,
        frame.bytes.len(),
        super::super::grok_errors::applies(&prepared.route),
    ) else {
        return Ok(None);
    };
    let event = original.expect("rejection has an event");
    let mut error = super::super::output_error::event_failure(event, status);
    error.retry_after = previous_wait.max(error.retry_after);
    if !super::super::retry::recover_before_output(
        state,
        prepared,
        tracker,
        event,
        status,
        previous_wait,
        policy,
        deadline,
    )
    .await?
    {
        return Ok(None);
    }
    let attempts = tracker.upstream_attempt_count();
    let remaining = super::super::retry::remaining_credential_policy(prepared, tracker, policy);
    let mut opened = match open_response_stream(
        state, prepared, false, streaming, tracker, &remaining, deadline,
    )
    .await
    {
        Ok(opened) => opened,
        Err(_failure)
            if !state.stop.stopped()
                && tokio::time::Instant::now() < deadline
                && tracker.upstream_attempt_count() == attempts =>
        {
            return Err(error)
        }
        Err(failure) => return Err(failure),
    };
    if opened.status.is_success() && !opened.is_sse {
        let bytes = read_bounded(state, opened.bytes, tracker, policy, deadline).await?;
        let mut response: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ServiceError::new(
                502,
                "invalid_upstream_response",
                "Upstream did not return a valid Responses object",
            )
        })?;
        if super::super::upstream_errors::chat_failure(&response, 502).is_some() {
            return Err(super::super::output_error::event_failure(&response, 502));
        }
        if response.get("status").and_then(Value::as_str) != Some("completed")
            || !response
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
            || !response.get("output").is_some_and(Value::is_array)
        {
            return Err(ServiceError::new(
                502,
                "invalid_upstream_response",
                "Upstream did not return a completed Responses object",
            ));
        }
        super::super::output_usage::normalize_usage_details(&mut response);
        let event = serde_json::json!({"type":"response.completed","response":response});
        let bytes = super::super::output_error::event_sse(&event);
        opened.bytes = stream::once(async move { Ok(bytes) }).boxed();
        opened.is_sse = true;
        opened.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
    }
    if !opened.status.is_success() {
        let status = if opened.status.is_success() {
            502
        } else {
            opened.status.as_u16()
        };
        let header_wait = retry_after_headers(&opened.headers);
        let bytes = read_bounded(state, opened.bytes, tracker, policy, deadline).await?;
        let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        tracker.capture_upstream_error(status, &value);
        let error_value = super::super::diagnostics::error_object(&value).clone();
        let (message, scope) =
            super::super::diagnostics::upstream_error_metadata_status(status, &value);
        return Err(ServiceError::new(
            status,
            "upstream_http_error",
            message.unwrap_or_else(|| "Upstream rejected the recovery request".into()),
        )
        .with_retry_after(header_wait.max(super::super::events::retry_after_event(&value)))
        .with_error_details(error_value)
        .with_credential_scope(scope));
    }
    *fence = super::super::bootstrap::Fence::new(!prepared.route.chatgpt_plan);
    Ok(Some(opened))
}
