//! Native-only WorkBuddy ACP request adapter for the text-only MVP.

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;

use super::{
    http::HttpResponseStream,
    mapping::Prepared,
    server::AppState,
    upstream::{ByteStream, CanonicalStream},
};
use crate::modules::codex_account;
use crate::modules::workbuddy_model_probe::{self, ProbeAccount};

pub(super) fn is_route(route: &super::contract::Route) -> bool {
    route.base_url == codex_account::WORKBUDDY_ACP_BASE_URL
}

pub(super) async fn open_text_stream(
    state: &Arc<AppState>,
    prepared: &Prepared,
    _compact: bool,
) -> Result<HttpResponseStream, super::contract::ServiceError> {
    let prompt = extract_text_prompt(&prepared.body)?;
    let model = prepared.route.model.trim();
    if model.is_empty() {
        return Err(error(
            400,
            "model_required",
            "WorkBuddy ACP requires a model",
        ));
    }
    let source = codex_account::load_account(&prepared.route.account_id).ok_or_else(|| {
        error(
            401,
            "workbuddy_reauthorization_required",
            "The linked WorkBuddy account is unavailable; sign in again",
        )
    })?;
    if !codex_account::is_workbuddy_upstream_provider(&source) {
        return Err(error(
            400,
            "invalid_workbuddy_account",
            "The selected account is not a WorkBuddy ACP account",
        ));
    }
    let source = codex_account::workbuddy_upstream_account(&source).ok_or_else(|| {
        error(
            401,
            "workbuddy_reauthorization_required",
            "The linked WorkBuddy account is unavailable; sign in again",
        )
    })?;
    if source.access_token.trim().is_empty()
        || source
            .expires_at
            .is_some_and(|expires_at| expires_at <= chrono::Utc::now().timestamp_millis())
        || source.status.as_deref().is_some_and(|status| {
            matches!(status, "expired" | "reauth_required" | "reauthorization_required")
        })
    {
        return Err(error(
            401,
            "workbuddy_reauthorization_required",
            "WorkBuddy authorization is unavailable; sign in again",
        ));
    }
    let probe_account = ProbeAccount::from_workbuddy(&source);
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 WorkBuddy Native ACP Gateway")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| {
            error(
                503,
                "workbuddy_unavailable",
                "Unable to prepare WorkBuddy client",
            )
        })?;
    let base = workbuddy_model_probe::WORKBUDDY_WEB_ENDPOINT;
    let conversation_id =
        workbuddy_model_probe::create_conversation(&client, base, &probe_account, model, &prompt)
            .await
            .map_err(map_error)?;
    let response_future = async {
        let (api_url, session_token, session_id) =
            workbuddy_model_probe::fetch_conversation_session(
                &client,
                base,
                &conversation_id,
                &probe_account,
            )
            .await
            .map_err(map_error)?;
        let text = workbuddy_model_probe::read_conversation_reply(
            &client,
            &api_url,
            &session_token,
            &session_id,
            model,
            &prompt,
            source.enterprise_id.as_deref(),
            45,
        )
        .await
        .map_err(map_error)?;
        Ok::<String, super::contract::ServiceError>(text)
    };
    let mut stop = state.stop.clone();
    let response = tokio::select! {
        _ = stop.wait() => Err(error(503, "service_stopping", "Native Gateway is stopping")),
        response = response_future => response,
    };
    let cleanup =
        workbuddy_model_probe::delete_conversation(&client, base, &probe_account, &conversation_id)
            .await;
    if let Err(cleanup_error) = cleanup {
        crate::modules::logger::log_warn(&format!(
            "WorkBuddy ACP conversation cleanup failed: {}",
            cleanup_error.replace(source.access_token.trim(), "<redacted>")
        ));
    }
    let text = response?;
    let event = format_response_events(model, &text);
    let bytes: ByteStream = stream::iter(vec![Ok(Bytes::from(event))]).boxed();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(HttpResponseStream {
        status: StatusCode::OK,
        headers,
        bytes: stream::empty().boxed(),
        is_sse: true,
        canonical: Some(CanonicalStream {
            bytes,
            is_sse: true,
        }),
    })
}

fn extract_text_prompt(body: &Value) -> Result<String, super::contract::ServiceError> {
    if body.get("tools").is_some_and(|value| !value.is_null())
        || body
            .get("previous_response_id")
            .is_some_and(Value::is_string)
        || body.get("conversation").is_some()
    {
        return Err(error(
            400,
            "workbuddy_text_only",
            "WorkBuddy ACP MVP supports only stateless text input",
        ));
    }
    let input = body
        .get("input")
        .or_else(|| body.get("messages"))
        .ok_or_else(|| error(400, "input_required", "WorkBuddy ACP requires text input"))?;
    let mut parts = Vec::new();
    collect_text(input, &mut parts)?;
    let prompt = parts.join("\n");
    if prompt.trim().is_empty() {
        return Err(error(
            400,
            "input_required",
            "WorkBuddy ACP requires text input",
        ));
    }
    Ok(prompt)
}

fn collect_text(
    value: &Value,
    parts: &mut Vec<String>,
) -> Result<(), super::contract::ServiceError> {
    match value {
        Value::String(text) => {
            parts.push(text.clone());
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                collect_text(item, parts)?;
            }
            Ok(())
        }
        Value::Object(item) => {
            let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(
                kind,
                "input_image" | "image_url" | "image" | "function_call" | "tool_use"
            ) || item.contains_key("tools")
            {
                return Err(error(
                    400,
                    "workbuddy_text_only",
                    "WorkBuddy ACP MVP supports only text input",
                ));
            }
            if let Some(content) = item.get("content") {
                return collect_text(content, parts);
            }
            if let Some(text) = item.get("text") {
                return collect_text(text, parts);
            }
            if kind == "message" || kind == "input_text" || kind.is_empty() {
                return Err(error(
                    400,
                    "workbuddy_text_only",
                    "WorkBuddy ACP received an unsupported text item",
                ));
            }
            Err(error(
                400,
                "workbuddy_text_only",
                "WorkBuddy ACP MVP supports only text input",
            ))
        }
        _ => Err(error(
            400,
            "workbuddy_text_only",
            "WorkBuddy ACP MVP supports only text input",
        )),
    }
}

fn format_response_events(model: &str, text: &str) -> String {
    let response_id = format!("resp_workbuddy_{}", uuid::Uuid::new_v4());
    let created = json!({
        "type": "response.created",
        "response": {"id": response_id, "object": "response", "status": "in_progress", "model": model}
    });
    let delta = json!({"type":"response.output_text.delta","item_id":format!("item_{response_id}"),"output_index":0,"content_index":0,"delta":text});
    let completed = json!({
        "type": "response.completed",
        "response": {"id": response_id, "object": "response", "status": "completed", "model": model,
            "output": [{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}
    });
    [created, delta, completed]
        .into_iter()
        .map(|event| format!("data: {}\n\n", event))
        .collect()
}

fn error(status: u16, code: &'static str, message: &'static str) -> super::contract::ServiceError {
    super::contract::ServiceError::new(status, code, message)
}

fn map_error(value: String) -> super::contract::ServiceError {
    let lower = value.to_ascii_lowercase();
    if value == "WORKBUDDY_PROBE_RATE_LIMITED"
        || lower.contains("429")
        || lower.contains("quota")
        || lower.contains("rate limit")
    {
        return error(429, "rate_limited", "WorkBuddy rate limit or quota reached");
    }
    if value.contains("TIMEOUT") || lower.contains("timed out") || lower.contains("timeout") {
        return error(504, "workbuddy_timeout", "WorkBuddy ACP request timed out");
    }
    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        return error(
            401,
            "workbuddy_reauthorization_required",
            "WorkBuddy authorization is unavailable; sign in again",
        );
    }
    error(
        502,
        "workbuddy_upstream_error",
        "WorkBuddy ACP request failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_responses_text_input_without_secret_material() {
        let prompt = extract_text_prompt(&json!({
            "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}] }]
        })).unwrap();
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn rejects_non_text_mvp_inputs() {
        for body in [
            json!({"input":"hello", "tools": []}),
            json!({"input":[{"type":"input_image","image_url":"https://example.invalid/x"}]}),
            json!({"input":"hello", "previous_response_id":"resp_old"}),
        ] {
            let error = extract_text_prompt(&body).unwrap_err();
            assert_eq!(error.code, "workbuddy_text_only");
        }
    }

    #[test]
    fn canonical_events_have_created_delta_and_completed() {
        let body = format_response_events("workbuddy-default", "hello");
        assert!(body.contains("response.created"));
        assert!(body.contains("response.output_text.delta"));
        assert!(body.contains("response.completed"));
        assert!(body.contains("hello"));
    }
}
