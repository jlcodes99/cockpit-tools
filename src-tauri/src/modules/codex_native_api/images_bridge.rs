//! A forced Responses image tool executes against the selected Images source.
use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    response::{IntoResponse, Response},
};
use serde_json::{json, Map, Value};

use super::super::{
    contract::{ImageRouteRequest, ServiceError},
    events::SseDecoder,
    http::{self, Input},
    server::{downstream_response, AppState},
};
use super::{
    assets, native_from_events,
    transport::{is_sse, Execution},
    ImageOperation, ImageRequest, ImageResponseFormat,
};

#[path = "images_bridge_stream.rs"]
mod streaming;

pub(crate) fn is_forced_generation(body: &Value) -> bool {
    body.pointer("/tool_choice/type").and_then(Value::as_str) == Some("image_generation")
}

pub(crate) async fn responses(
    state: Arc<AppState>,
    mut input: Input,
) -> Result<Response, ServiceError> {
    let parsed = prepare_request(&input.body);
    let mut request = match parsed {
        Ok(value) => value,
        Err(error) => {
            input.tracker.fail(&error).await;
            return Err(error);
        }
    };
    let route_request = ImageRouteRequest {
        model: request.model.clone(),
        responses_model: input
            .body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        edit: request.operation == ImageOperation::Edit,
        target_account_id: input
            .headers
            .get("x-cockpit-target-account-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        preferred_account_id: None,
        force_refresh: false,
        excluded_account_ids: Vec::new(),
    };
    let selected = super::super::execution::within(
        &state,
        state.host.image_route(&input.caller, route_request.clone()),
    )
    .await;
    let selected = match selected {
        Ok(value) => value,
        Err(error) => {
            input.tracker.fail(&error).await;
            return Err(error);
        }
    };
    if selected.route.chatgpt {
        return http::dispatch_selected(state, input, selected, route_request).await;
    }
    if external_history_is_opaque(&input.body, &input.headers) {
        let error=ServiceError::new(409,"continuation_not_supported","An external Images source needs the complete prompt and input images; opaque Responses continuation is not supported");
        input.tracker.fail(&error).await;
        return Err(error);
    }
    if let Err(error) = super::super::execution::prepare(&state, request.prepare_urls(&state)).await
    {
        input.tracker.fail(&error).await;
        return Err(error);
    }
    let response_model = input.body["model"].as_str().unwrap_or_default().to_owned();
    let client_stream = input
        .body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    request.stream = client_stream;
    request.original["stream"] = json!(client_stream);
    let mut execution = Execution {
        budget: super::super::execution::current_or_new(&state).await?,
        state,
        caller: input.caller,
        headers: input.headers,
        request,
        selected,
        route_request,
        tracker: input.tracker,
        usage: None,
        deadline: None,
        owner_committed: false,
    };
    execution.record_route();
    let (upstream, _) = match execution.send(false).await {
        Ok(value) => value,
        Err(error) => {
            execution.tracker.fail(&error).await;
            return Err(error);
        }
    };
    if !upstream.status().is_success() {
        let status = upstream.status();
        let headers = upstream.headers().clone();
        let bytes = execution.read_bounded(upstream).await?;
        execution
            .tracker
            .complete(
                status.as_u16(),
                false,
                None,
                Some("image_upstream_error".into()),
            )
            .await;
        return Ok(downstream_response(status, &headers, Body::from(bytes)));
    }
    let upstream_headers = upstream.headers().clone();
    let id = format!("resp_{}", uuid::Uuid::now_v7().simple());
    let created = chrono::Utc::now().timestamp();
    if !client_stream {
        let images = match collect(&mut execution, upstream).await {
            Ok(value) => value,
            Err(error) => {
                execution.tracker.fail(&error).await;
                return Err(error);
            }
        };
        let response = completed_response(&id, created, &response_model, &images);
        execution
            .tracker
            .complete(200, true, images.get("usage").cloned(), None)
            .await;
        return Ok(super::transport::with_upstream_headers(
            axum::Json(response).into_response(),
            &upstream_headers,
        ));
    }
    Ok(super::transport::with_upstream_headers(
        streaming::response(execution, upstream, id, created, response_model),
        &upstream_headers,
    ))
}

async fn collect(
    execution: &mut Execution,
    upstream: reqwest::Response,
) -> Result<Value, ServiceError> {
    let sse = is_sse(upstream.headers());
    let bytes = execution.read_bounded(upstream).await?;
    let mut images = if sse {
        let mut decoder = SseDecoder::new(execution.event_limit());
        let mut events = decoder.push(&bytes)?;
        events.extend(decoder.finish()?);
        if decoder.has_partial_event() {
            return Err(ServiceError::new(
                502,
                "invalid_image_response",
                "Image stream ended inside an event",
            ));
        }
        native_from_events(&events)?
    } else {
        serde_json::from_slice(&bytes).map_err(|_| {
            ServiceError::new(
                502,
                "invalid_image_response",
                "Image upstream returned invalid JSON",
            )
        })?
    };
    execution.usage = images.get("usage").cloned();
    execution.tracker.capture_usage(&execution.usage);
    assets::normalize_output(
        &execution.state,
        &mut images,
        ImageResponseFormat::Base64,
        execution.response_limit(),
        execution.asset_budget(),
    )
    .await?;
    Ok(images)
}

fn encode(event: Value) -> Bytes {
    Bytes::from(format!(
        "event: {}\ndata: {event}\n\n",
        event["type"].as_str().unwrap_or("message")
    ))
}

fn completed_response(id: &str, created: i64, model: &str, images: &Value) -> Value {
    let output=images["data"].as_array().into_iter().flatten().map(|image|{
        let mut item=json!({"id":format!("ig_{}",uuid::Uuid::now_v7().simple()),"type":"image_generation_call","status":"completed","result":image["b64_json"]});
        for name in ["revised_prompt","output_format","quality","size","background"] {
            if let Some(value)=image.get(name).or_else(||images.get(name)){item[name]=value.clone();}
        }
        item
    }).collect::<Vec<_>>();
    let mut response = json!({"id":id,"object":"response","created_at":created,"model":model,"status":"completed","output":output,"error":null,"incomplete_details":null});
    if let Some(usage) = images.get("usage") {
        response["usage"] = usage.clone();
    }
    response
}

fn prepare_request(body: &Value) -> Result<ImageRequest, ServiceError> {
    let bad = |message: &str| ServiceError::new(400, "invalid_image_tool_request", message);
    if !is_forced_generation(body) {
        return Err(bad("An image_generation tool choice is required"));
    }
    if body
        .get("model")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(bad("A Responses model is required"));
    }
    let tool = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|tool| tool.get("type").and_then(Value::as_str) == Some("image_generation"));
    let mut prompt = Vec::new();
    let mut images = Vec::new();
    if let Some(instructions) = body
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        prompt.push(instructions.to_owned());
    }
    let input = body
        .get("input")
        .ok_or_else(|| bad("An image prompt is required"))?;
    if let Some(text) = input.as_str() {
        prompt.push(text.to_owned());
    } else if let Some(items) = input.as_array() {
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
                continue;
            }
            let content = item.get("content").unwrap_or(item);
            if let Some(text) = content.as_str() {
                prompt.push(text.to_owned());
                continue;
            }
            let chunks: Vec<&Value> = if let Some(chunks) = content.as_array() {
                chunks.iter().collect()
            } else {
                vec![content]
            };
            for chunk in chunks {
                match chunk.get("type").and_then(Value::as_str) {
                    Some("input_text" | "output_text") => {
                        if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                            prompt.push(text.to_owned());
                        }
                    }
                    Some("input_image") => images.push(chunk.clone()),
                    _ => {}
                }
            }
        }
    } else {
        return Err(bad("Responses input must be text or message items"));
    }
    let mut image_body = Map::new();
    image_body.insert("prompt".into(), json!(prompt.join("\n\n")));
    image_body.insert("stream".into(), json!(false));
    image_body.insert("response_format".into(), json!("b64_json"));
    if let Some(tool) = tool {
        for name in [
            "model",
            "size",
            "quality",
            "background",
            "output_format",
            "output_compression",
            "moderation",
            "input_fidelity",
            "partial_images",
        ] {
            if let Some(value) = tool.get(name) {
                image_body.insert(name.into(), value.clone());
            }
        }
        if let Some(mask) = tool.get("input_image_mask") {
            image_body.insert("mask".into(), mask.clone());
        }
    }
    let edit = !images.is_empty();
    if edit {
        image_body.insert("images".into(), json!(images));
    }
    ImageRequest::from_json(
        if edit {
            ImageOperation::Edit
        } else {
            ImageOperation::Generation
        },
        Value::Object(image_body),
    )
}

fn external_history_is_opaque(body: &Value, headers: &axum::http::HeaderMap) -> bool {
    fn opaque(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.iter().any(|(key, value)| {
                    (key == "encrypted_content" && !value.is_null())
                        || (key == "encrypted_function_args"
                            && !value.is_null()
                            && value
                                .as_array()
                                .is_none_or(|arguments| !arguments.is_empty()))
                        || opaque(value)
                }) || object.get("type").and_then(Value::as_str) == Some("item_reference")
                    || super::super::agent_communication::encrypted_message(value).is_some()
            }
            Value::Array(items) => items.iter().any(opaque),
            _ => false,
        }
    }
    body.get("previous_response_id")
        .is_some_and(|value| !value.is_null())
        || body
            .get("conversation")
            .is_some_and(|value| !value.is_null())
        || body
            .get("conversation_id")
            .is_some_and(|value| !value.is_null())
        || headers.contains_key("conversation_id")
        || headers.contains_key("x-codex-turn-state")
        || opaque(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_image_tool_choice_enters_the_bridge() {
        assert!(!is_forced_generation(
            &json!({"tools":[{"type":"image_generation"}],"tool_choice":"auto"})
        ));
        assert!(!is_forced_generation(
            &json!({"tool_choice":{"type":"function","name":"image_generation"}})
        ));
        assert!(is_forced_generation(
            &json!({"tool_choice":{"type":"image_generation"}})
        ));
    }
    #[test]
    fn external_history_guard_distinguishes_ciphertext_from_explicit_plaintext_arguments() {
        let headers = axum::http::HeaderMap::new();
        let mut body = json!({"input":[{"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"agent","arguments":"{\"message\":\"task text\"}","encrypted_function_args":[]}]});
        assert!(!external_history_is_opaque(&body, &headers));
        body["input"][0]
            .as_object_mut()
            .unwrap()
            .remove("encrypted_function_args");
        assert!(external_history_is_opaque(&body, &headers));
        body["input"][0]["encrypted_function_args"] = json!(["message"]);
        assert!(external_history_is_opaque(&body, &headers));
        assert!(external_history_is_opaque(
            &json!({"conversation_id":"server-conversation"}),
            &headers
        ));
    }

    #[test]
    fn forced_image_edit_preserves_tool_controls_and_input() {
        let request=prepare_request(&json!({"model":"gpt-5","instructions":"Keep proportions","input":[{"role":"user","content":[{"type":"input_text","text":"Make it blue"},{"type":"input_image","image_url":"data:image/png;base64,AA=="}]}],"tools":[{"type":"image_generation","model":"gpt-image-2.5-flare","quality":"high","input_image_mask":{"image_url":"data:image/png;base64,AQ=="}}],"tool_choice":{"type":"image_generation"}})).unwrap();
        assert_eq!(request.operation, ImageOperation::Edit);
        assert_eq!(request.model.as_deref(), Some("gpt-image-2.5-flare"));
        assert_eq!(request.prompt, "Keep proportions\n\nMake it blue");
        assert_eq!(request.native_json("selected")["quality"], "high");
        assert!(request.mask.is_some());
    }
}
