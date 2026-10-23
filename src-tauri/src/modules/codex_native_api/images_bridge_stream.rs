//! Turns native Images SSE into the Responses image tool event sequence.
use std::collections::{BTreeMap, VecDeque};

use axum::{
    body::{Body, Bytes},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{stream, stream::BoxStream, StreamExt};
use serde_json::{json, Value};

use super::{
    assets, collect, completed_response, encode, is_sse, native_from_events, Execution,
    ImageResponseFormat, ServiceError, SseDecoder,
};

pub(super) fn response(
    execution: Execution,
    upstream: reqwest::Response,
    id: String,
    created: i64,
    model: String,
) -> Response {
    let envelope = json!({"id":id,"object":"response","created_at":created,"model":model,"status":"in_progress","output":[]});
    let pending = VecDeque::from([
        encode(json!({"type":"response.created","sequence_number":0,"response":envelope})),
        encode(json!({"type":"response.in_progress","sequence_number":1,"response":envelope})),
    ]);
    let limit = execution.event_limit();
    let (stream, json) = if is_sse(upstream.headers()) {
        (Some(upstream.bytes_stream().boxed()), None)
    } else {
        (None, Some(upstream))
    };
    let state = BridgeStream {
        execution,
        stream,
        json,
        decoder: SseDecoder::new(limit),
        received: 0,
        pending,
        id,
        created,
        model,
        sequence: 2,
        items: BTreeMap::new(),
        completed: Vec::new(),
        finished: false,
    };
    let stream = stream::unfold(state, |mut current| async move {
        loop {
            if let Some(bytes) = current.pending.pop_front() {
                return Some((Ok::<_, std::io::Error>(bytes), current));
            }
            if current.finished {
                return None;
            }
            if let Err(error) = current.advance().await {
                current.execution.tracker.fail(&error).await;
                let envelope = json!({"id":current.id,"object":"response","status":"failed","output":[],"error":super::super::super::output_error::json_error(&error)["error"]});
                current.push(json!({"type":"response.failed","response":envelope}));
                current.finished = true;
            }
        }
    });
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

struct BridgeStream {
    execution: Execution,
    stream: Option<BoxStream<'static, Result<Bytes, reqwest::Error>>>,
    json: Option<reqwest::Response>,
    decoder: SseDecoder,
    received: usize,
    pending: VecDeque<Bytes>,
    id: String,
    created: i64,
    model: String,
    sequence: u64,
    items: BTreeMap<u64, String>,
    completed: Vec<Value>,
    finished: bool,
}

impl BridgeStream {
    fn push(&mut self, mut event: Value) {
        event["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        self.pending.push_back(encode(event));
    }
    fn start_item(&mut self, index: u64) -> Result<String, ServiceError> {
        if index >= 10 {
            return Err(ServiceError::new(
                502,
                "invalid_image_response",
                "Image output index exceeds the supported limit",
            ));
        }
        if let Some(id) = self.items.get(&index) {
            return Ok(id.clone());
        }
        let id = format!("ig_{}", uuid::Uuid::now_v7().simple());
        self.items.insert(index, id.clone());
        self.push(json!({"type":"response.output_item.added","output_index":index,"item":{"type":"image_generation_call","id":id,"status":"in_progress"}}));
        self.push(json!({"type":"response.image_generation_call.in_progress","output_index":index,"item_id":id}));
        Ok(id)
    }
    async fn advance(&mut self) -> Result<(), ServiceError> {
        if let Some(response) = self.json.take() {
            let images = collect(&mut self.execution, response).await?;
            return self.finish(images).await;
        }
        let Some(upstream) = self.stream.as_mut() else {
            return Err(ServiceError::new(
                502,
                "invalid_image_response",
                "Image stream is unavailable",
            ));
        };
        let next = self.execution.read_next(upstream).await?;
        let eof = next.is_none();
        let events = if let Some(bytes) = next {
            self.execution.tracker.first_response();
            self.received = self.received.saturating_add(bytes.len());
            if self.received > self.execution.response_limit() {
                return Err(ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Image stream exceeds its size limit",
                ));
            }
            self.decoder.push(&bytes)?
        } else {
            self.decoder.finish()?
        };
        for event in events {
            if let Some(usage) = event.get("usage") {
                self.execution.usage = Some(usage.clone());
                self.execution.tracker.capture_usage(&self.execution.usage);
            }
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if kind == "error" {
                return Err(super::super::result::image_failure(&event));
            }
            if kind.ends_with(".partial_image") {
                let encoded = event
                    .get("b64_json")
                    .or_else(|| event.get("partial_image_b64"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ServiceError::new(
                            502,
                            "invalid_image_response",
                            "Partial image has no image bytes",
                        )
                    })?;
                if encoded.is_empty() || STANDARD.decode(encoded).is_err() {
                    return Err(ServiceError::new(
                        502,
                        "invalid_image_response",
                        "Partial image contains invalid image bytes",
                    ));
                }
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let item_id = self.start_item(index)?;
                self.push(json!({"type":"response.image_generation_call.partial_image","output_index":index,"item_id":item_id,"partial_image_index":event.get("partial_image_index").cloned().unwrap_or(json!(0)),"partial_image_b64":encoded}));
            }
            if kind.ends_with(".completed") {
                self.completed.push(event);
            }
        }
        if eof {
            let mut images = native_from_events(&self.completed)?;
            assets::normalize_output(
                &self.execution.state,
                &mut images,
                ImageResponseFormat::Base64,
                self.execution.response_limit(),
                self.execution.asset_budget(),
            )
            .await?;
            return self.finish(images).await;
        }
        Ok(())
    }
    async fn finish(&mut self, images: Value) -> Result<(), ServiceError> {
        let mut response = completed_response(&self.id, self.created, &self.model, &images);
        for (index, item) in response["output"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            let id = self.start_item(index as u64)?;
            item["id"] = json!(id);
            self.push(json!({"type":"response.image_generation_call.completed","output_index":index,"item_id":id}));
            self.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        }
        self.push(json!({"type":"response.completed","response":response}));
        self.execution
            .tracker
            .complete(200, true, images.get("usage").cloned(), None)
            .await;
        self.finished = true;
        Ok(())
    }
}
