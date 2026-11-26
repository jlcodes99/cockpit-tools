//! Explicit WorkBuddy native-delegate boundary. This path never enters the
//! generic account selector, HTTP retry engine or conversation rebuild logic.
use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use serde_json::{json, Value};

use super::{
    contract::{ServiceError, WorkbuddyDelegateSelection},
    http::Input,
    server::{downstream_response, AppState},
    workbuddy_delegate_protocol::{self as protocol, ToolCatalog},
    workbuddy_delegate_runtime::{self as runtime, LaunchConfig, Owner},
};

pub(super) const BASE_URL: &str = "workbuddy-delegate://native";

/// Reserved, server-issued references must never fall through to another
/// provider after key reconfiguration, process loss or gateway restart.
pub(super) fn has_native_reference(body: &Value) -> bool {
    body.get("previous_response_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with("resp_wb_"))
        || body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    ["call_id", "id"].iter().any(|field| {
                        item.get(field).and_then(Value::as_str).is_some_and(|id| {
                            id.starts_with("call_wb_")
                                || id.starts_with("resp_wb_")
                                || id.starts_with("msg_wb_")
                        })
                    })
                })
            })
}

fn has_assistant_history(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("role").and_then(Value::as_str) == Some("assistant")
                    || item.get("type").and_then(Value::as_str) == Some("reasoning")
            })
        })
}

fn unsupported(message: &str) -> ServiceError {
    ServiceError::new(400, "workbuddy_delegate_unsupported", message)
}

fn validate_request(body: &Value, compact: bool, effort: &str) -> Result<(), ServiceError> {
    if compact {
        return Err(unsupported("WorkBuddy native-delegate does not support /responses/compact; start a new thread if its context is full"));
    }
    if body.get("conversation").is_some_and(|v| !v.is_null()) {
        return Err(unsupported("Use the pinned native-delegate thread or previous_response_id, not a hosted conversation"));
    }
    for key in ["background", "store"] {
        if body.get(key).and_then(Value::as_bool) == Some(true) {
            return Err(unsupported(
                "Hosted background execution and stored responses are not supported",
            ));
        }
    }
    if body
        .get("truncation")
        .and_then(Value::as_str)
        .is_some_and(|s| s != "disabled")
    {
        return Err(unsupported(
            "Automatic truncation cannot rebuild a live native-delegate session",
        ));
    }
    if body
        .get("tool_choice")
        .is_some_and(|v| !v.is_null() && v.as_str() != Some("auto"))
    {
        return Err(unsupported(
            "Native-delegate currently supports tool_choice=auto only",
        ));
    }
    if body
        .get("service_tier")
        .and_then(Value::as_str)
        .is_some_and(|s| !matches!(s, "auto" | "default"))
    {
        return Err(unsupported(
            "OpenAI service tiers do not select a WorkBuddy entitlement",
        ));
    }
    if body.get("max_output_tokens").is_some_and(|v| !v.is_null()) {
        return Err(unsupported(
            "Native-delegate cannot honor max_output_tokens on the installed CLI",
        ));
    }
    if body
        .pointer("/text/format/type")
        .and_then(Value::as_str)
        .is_some_and(|s| s != "text")
    {
        return Err(unsupported(
            "Structured output formats are not supported by this native bridge",
        ));
    }
    if let Some(requested) = body.pointer("/reasoning/effort").filter(|v| !v.is_null()) {
        if requested.as_str() != Some(effort) {
            return Err(ServiceError::new(400, "workbuddy_reasoning_effort_mismatch", "The requested reasoning effort must match this dedicated profile's verified WorkBuddy effort"));
        }
    }
    ToolCatalog::from_tools(body.get("tools"))?;
    protocol::input_items(body)?;
    Ok(())
}

pub(super) async fn dispatch(
    state: Arc<AppState>,
    mut input: Input,
    selection: WorkbuddyDelegateSelection,
    compact: bool,
) -> Result<Response, ServiceError> {
    let result = dispatch_inner(&state, &mut input, selection, compact).await;
    if let Err(error) = &result {
        input.tracker.fail(error).await;
    }
    result
}

async fn dispatch_inner(
    state: &Arc<AppState>,
    input: &mut Input,
    selection: WorkbuddyDelegateSelection,
    compact: bool,
) -> Result<Response, ServiceError> {
    if selection.route.base_url != BASE_URL || selection.route.account_id.is_empty() {
        return Err(ServiceError::new(
            500,
            "invalid_workbuddy_delegate_grant",
            "The native-delegate route is not configured correctly",
        ));
    }
    validate_request(&input.body, compact, &selection.effort)?;
    if input.raw_chat.is_some() {
        return Err(unsupported("Use HTTP Responses with the dedicated native-delegate profile; chat compatibility conversion is not supported"));
    }
    let requested_model = input
        .body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    input
        .tracker
        .routed(&input.caller, &selection.route, requested_model);
    input.tracker.request_metadata(&input.body);
    // Reuse the durable identity ownership/checkpoint observer, but never use a
    // rewritten/rebuilt history as input to the live actor. Public IDs and the
    // original Codex history remain authoritative at the actor boundary.
    let identity = state.identity.clone();
    let caller_id = input.caller.id.clone();
    let headers = input.headers.clone();
    let body = input.body.clone();
    let mut context = super::blocking::run_tracked(state, move || {
        identity.inspect_for_forwarding(&caller_id, &headers, &body, false)
    })
    .await?;
    let hint = if context.generated_session {
        None
    } else {
        context.thread_id.as_deref()
    };
    let continuation =
        state
            .workbuddy_delegate
            .continuation(&input.caller.id, &input.body, hint)?;
    if continuation.is_none()
        && (has_native_reference(&input.body) || has_assistant_history(&input.body))
    {
        return Err(ServiceError::new(
            409,
            "workbuddy_delegate_session_lost",
            "The live WorkBuddy session is unavailable; history cannot recreate its upstream state. Start a new thread instead of replaying this conversation",
        ));
    }
    let thread_id = if let Some(previous) = continuation {
        if previous.account_id != selection.route.account_id
            || previous.model != selection.route.model
            || previous.source_generation != selection.source_generation
        {
            return Err(ServiceError::new(409, "workbuddy_delegate_source_changed", "The pinned WorkBuddy source or model changed; this live session cannot be migrated or replayed"));
        }
        if context.generated_session {
            let mut headers = input.headers.clone();
            headers.insert(
                "thread-id",
                HeaderValue::from_str(&previous.thread_id).map_err(|_| {
                    ServiceError::new(
                        409,
                        "invalid_workbuddy_thread",
                        "The pinned thread identity is invalid",
                    )
                })?,
            );
            let identity = state.identity.clone();
            let caller_id = input.caller.id.clone();
            let body = input.body.clone();
            context = super::blocking::run_tracked(state, move || {
                identity.inspect_for_forwarding(&caller_id, &headers, &body, false)
            })
            .await?;
        }
        previous.thread_id
    } else {
        context
            .thread_id
            .clone()
            .or_else(|| context.session_id.clone())
            .ok_or_else(|| {
                ServiceError::new(
                    400,
                    "workbuddy_thread_required",
                    "A stable native-delegate thread identity is required",
                )
            })?
    };
    let identity = state.identity.clone();
    let account_id = selection.route.account_id.clone();
    let owner_id = selection.route.owner_id.clone();
    let mut headers = input.headers.clone();
    let mut observed_body = input.body.clone();
    let binding = super::blocking::run_tracked(state, move || {
        identity.rewrite(
            &context,
            &account_id,
            &owner_id,
            &mut headers,
            &mut observed_body,
        )
    })
    .await?;
    input.tracker.check_quota_before_send()?;
    input.tracker.start_turn()?;
    input.tracker.begin_attempt("workbuddy-native-delegate");
    let owner = Owner {
        caller_id: input.caller.id.clone(),
        thread_id,
    };
    let config = LaunchConfig {
        account_id: selection.route.account_id.clone(),
        source_generation: selection.source_generation,
        model: selection.route.model.clone(),
        effort: selection.effort,
        access_token: selection.access_token,
        domestic: selection.domestic,
    };
    let document = state
        .workbuddy_delegate
        .respond(owner, config, &input.body, state.stop.clone())
        .await?;
    let response_id = document.get("id").and_then(Value::as_str).ok_or_else(|| {
        ServiceError::new(
            502,
            "workbuddy_delegate_missing_response_id",
            "The native-delegate response has no server-issued identity",
        )
    })?;
    let delivery = state
        .workbuddy_delegate
        .delivery_guard(&input.caller.id, response_id)?;
    let completed = json!({"type":"response.completed","response":document});
    let mut observation = super::events::ResponseObservation::default();
    observation.observe(&completed);
    if !observation.terminal || !observation.succeeded {
        return Err(ServiceError::new(
            502,
            "workbuddy_delegate_incomplete",
            "The native-delegate turn did not produce a completed response",
        ));
    }
    let identity = state.identity.clone();
    let observed = completed.clone();
    super::blocking::run_tracked(state, move || identity.observe_event(&binding, &observed))
        .await?;
    input.tracker.first_response_event(&completed);
    input
        .tracker
        .record_response_model(&selection.route, &completed);
    // Cached retries retain the public document's real usage, but must not
    // debit that same upstream work a second time in the gateway ledger.
    let usage = if state
        .workbuddy_delegate
        .claim_accounting(&input.caller.id, response_id)?
    {
        document
            .get("usage")
            .filter(|value| value.is_object())
            .cloned()
    } else {
        None
    };
    let stream = input
        .body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let bytes = if stream {
        Bytes::from(protocol::response_events(&document)?)
    } else {
        Bytes::from(serde_json::to_vec(&document).map_err(|_| {
            ServiceError::new(
                500,
                "workbuddy_response_encoding",
                "Unable to encode the native-delegate response",
            )
        })?)
    };
    let mut guard = input.tracker.defer_downstream();
    let body = Body::from_stream(futures_util::stream::once(async move {
        delivery.commit();
        guard.success();
        Ok::<_, std::io::Error>(bytes)
    }));
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(if stream {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    input.tracker.complete(200, true, usage, None).await;
    Ok(downstream_response(StatusCode::OK, &headers, body))
}

pub(crate) async fn workbuddy_delegate_catalog_for_source(
    source: crate::models::workbuddy::WorkbuddyAccount,
) -> Result<Value, String> {
    workbuddy_delegate_catalog_for_model(source, String::new()).await
}

pub(crate) async fn workbuddy_delegate_catalog_for_model(
    source: crate::models::workbuddy::WorkbuddyAccount,
    model_id: String,
) -> Result<Value, String> {
    if source
        .enterprise_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("WorkBuddy enterprise sources are not supported until native CLI tenant pinning can be verified".into());
    }
    if source.access_token.trim().is_empty()
        || source
            .expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
        || source.status.as_deref().is_some_and(|s| {
            matches!(
                s,
                "expired" | "reauth_required" | "reauthorization_required"
            )
        })
    {
        return Err("WorkBuddy authorization is unavailable; sign in again".into());
    }
    // Keep the sender alive until catalog() has finished; channel closure is a
    // cancellation signal. This RPC initializes metadata only, never a prompt.
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    runtime::catalog(
        LaunchConfig {
            account_id: source.id,
            source_generation: "catalog-only".into(),
            model: model_id,
            effort: String::new(),
            access_token: source.access_token,
            domestic: true,
        },
        super::lifecycle::StopSignal(receiver),
    )
    .await
    .map_err(|error| error.message)
}

#[cfg(test)]
#[path = "workbuddy_delegate_http_tests.rs"]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_references_are_structural_not_prompt_text() {
        assert!(has_native_reference(
            &json!({"previous_response_id":"resp_wb_old"})
        ));
        assert!(has_native_reference(
            &json!({"input":[{"type":"function_call_output","call_id":"call_wb_old","output":"ok"}]})
        ));
        assert!(has_native_reference(
            &json!({"input":[{"type":"message","role":"assistant","id":"msg_wb_old","content":"old answer"}]})
        ));
        assert!(!has_native_reference(
            &json!({"input":"please mention resp_wb_fake"})
        ));
        assert!(!has_native_reference(
            &json!({"input":[{"role":"user","content":"call_wb_fake"}]})
        ));
    }

    #[test]
    fn unsupported_hosted_semantics_are_rejected_before_cli_launch() {
        for extra in [
            json!({"conversation":"conv_x"}),
            json!({"background":true}),
            json!({"store":true}),
            json!({"tool_choice":"required"}),
            json!({"truncation":"auto"}),
            json!({"max_output_tokens":64}),
            json!({"text":{"format":{"type":"json_schema"}}}),
        ] {
            let mut body = json!({"model":"workbuddy/fixture","input":"hi"});
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(validate_request(&body, false, "low").is_err(), "{body}");
        }
        assert!(validate_request(&json!({"input":"hi"}), true, "low").is_err());
        assert!(validate_request(
            &json!({"input":"hi","reasoning":{"effort":"high"}}),
            false,
            "low"
        )
        .is_err());
        assert!(validate_request(&json!({"input":"hi","reasoning":{"effort":"low"},"tool_choice":"auto","parallel_tool_calls":true}), false, "low").is_ok());
    }
}
