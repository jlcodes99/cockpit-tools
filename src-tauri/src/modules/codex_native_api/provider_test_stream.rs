use std::time::Instant;

use serde_json::Value;

use super::{diagnostics::error_object, events::SseDecoder};

pub(crate) async fn read(
    mut response: reqwest::Response,
    started: Instant,
) -> Result<(Value, Option<u64>), String> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with("text/event-stream") {
        let body = response.json().await.map_err(|error| error.to_string())?;
        return Ok((body, None));
    }

    let mut decoder = SseDecoder::new(4 * 1024 * 1024);
    let mut first_text_ms = None;
    loop {
        let chunk = response.chunk().await.map_err(|error| error.to_string())?;
        let arrived = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let events = match &chunk {
            Some(bytes) => decoder.push(bytes),
            None => decoder.finish(),
        }
        .map_err(|error| error.message)?;
        for mut event in events {
            match event["type"].as_str() {
                Some("response.output_text.delta" | "response.output_text.done") => {
                    if event
                        .get("delta")
                        .or_else(|| event.get("text"))
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                    {
                        first_text_ms.get_or_insert(arrived);
                    }
                }
                Some("response.completed" | "response.incomplete") => {
                    let body = event["response"].take();
                    return Ok((body, first_text_ms));
                }
                Some("response.failed" | "error") => {
                    let error = error_object(&event);
                    return Err(error["message"]
                        .as_str()
                        .unwrap_or("Provider stream failed")
                        .to_string());
                }
                _ => {}
            }
        }
        if chunk.is_none() {
            return Err("Provider stream ended without a completion event".to_string());
        }
    }
}
