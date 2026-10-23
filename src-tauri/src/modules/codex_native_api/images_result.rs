use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};

use super::{ImageOperation, ServiceError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageResponseFormat {
    Base64,
    Url,
}

pub(crate) struct ImageAccumulator {
    operation: ImageOperation,
    format: ImageResponseFormat,
    output_format: String,
    created: i64,
    collected: BTreeMap<u64, Value>,
    item_metadata: BTreeMap<u64, Map<String, Value>>,
    output_indexes: BTreeMap<u64, u64>,
    item_indexes: BTreeMap<String, u64>,
    next_image_index: u64,
    metadata: Map<String, Value>,
    limit: usize,
    pub terminal: bool,
    pub usage: Option<Value>,
    failed: bool,
}

impl ImageAccumulator {
    pub(crate) fn new(operation: ImageOperation, format: ImageResponseFormat) -> Self {
        Self {
            operation,
            format,
            output_format: "png".into(),
            created: chrono::Utc::now().timestamp(),
            collected: BTreeMap::new(),
            item_metadata: BTreeMap::new(),
            output_indexes: BTreeMap::new(),
            item_indexes: BTreeMap::new(),
            next_image_index: 0,
            metadata: Map::new(),
            limit: 32 * 1024 * 1024,
            terminal: false,
            usage: None,
            failed: false,
        }
    }

    pub(crate) fn with_output_format(mut self, format: &str) -> Self {
        self.output_format = format.to_owned();
        self
    }
    pub(crate) fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Converts only protocol image events. Text/tool payloads are never searched
    /// for image-like strings, and an image item alone is not a successful response.
    pub(crate) fn observe(&mut self, event: &Value) -> Result<Vec<Value>, ServiceError> {
        if self.terminal {
            return Ok(Vec::new());
        }
        let response = event.get("response").unwrap_or(event);
        if let Some(usage) = image_usage(response) {
            self.usage = Some(usage.clone());
        }
        if let Some(created) = response
            .get("created_at")
            .or_else(|| response.get("created"))
            .and_then(Value::as_i64)
        {
            self.created = created;
        }
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "error" | "response.failed" | "response.incomplete" => self.fail(event),
            "response.image_generation_call.partial_image" => {
                let Some(value) = event.get("partial_image_b64").and_then(Value::as_str) else {
                    return Err(invalid_image(
                        "Partial image event did not contain image bytes",
                    ));
                };
                let mut out = image_data(
                    value,
                    self.format,
                    event
                        .get("output_format")
                        .and_then(Value::as_str)
                        .unwrap_or(&self.output_format),
                    self.limit,
                )?;
                out["type"] = json!(format!("{}.partial_image", self.operation.event_prefix()));
                out["created_at"] = json!(self.created);
                for name in [
                    "partial_image_index",
                    "size",
                    "quality",
                    "background",
                    "output_format",
                ] {
                    if let Some(value) = event.get(name) {
                        out[name] = value.clone();
                    }
                }
                if let Some(index) = self.image_index(
                    event.get("output_index").and_then(Value::as_u64),
                    event.get("item_id").and_then(Value::as_str),
                )? {
                    out["output_index"] = json!(index);
                }
                Ok(vec![out])
            }
            "response.output_item.added" => {
                if let Some(item) = event
                    .get("item")
                    .filter(|item| item["type"] == "image_generation_call")
                {
                    self.image_index(
                        event.get("output_index").and_then(Value::as_u64),
                        item.get("id").and_then(Value::as_str),
                    )?;
                }
                Ok(Vec::new())
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    self.collect_item(
                        item,
                        event
                            .get("output_index")
                            .and_then(Value::as_u64)
                            .unwrap_or(self.collected.len() as u64),
                    )?;
                }
                Ok(Vec::new())
            }
            "response.completed" | "response.done" => {
                if response
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status != "completed")
                {
                    return self.fail(event);
                }
                self.complete(response)
            }
            _ if response.get("object").and_then(Value::as_str) == Some("response")
                && response.get("status").and_then(Value::as_str) == Some("completed") =>
            {
                self.complete(response)
            }
            _ => Ok(Vec::new()),
        }
    }

    // Responses indexes include messages and other tools. Bind only observed image
    // identities to a stable, compact image index shared by partial and final events.
    fn image_index(
        &mut self,
        output: Option<u64>,
        item_id: Option<&str>,
    ) -> Result<Option<u64>, ServiceError> {
        let item_id = item_id.filter(|id| !id.is_empty());
        if output.is_none() && item_id.is_none() {
            return Ok(None);
        }
        let by_output = output.and_then(|index| self.output_indexes.get(&index).copied());
        let by_item = item_id.and_then(|id| self.item_indexes.get(id).copied());
        if by_output
            .zip(by_item)
            .is_some_and(|(left, right)| left != right)
        {
            return Err(invalid_image(
                "Image output identity changed during generation",
            ));
        }
        let index = by_item.or(by_output).unwrap_or(self.next_image_index);
        if index >= 1024
            || item_id.is_some_and(|id| id.len() > 1024)
            || (by_output.is_none() && output.is_some() && self.output_indexes.len() >= 1024)
            || (by_item.is_none() && item_id.is_some() && self.item_indexes.len() >= 1024)
        {
            return Err(invalid_image("Image output identity exceeds its limit"));
        }
        self.next_image_index = self.next_image_index.max(index + 1);
        if let Some(output) = output {
            self.output_indexes.insert(output, index);
        }
        if let Some(id) = item_id {
            self.item_indexes.insert(id.to_owned(), index);
        }
        Ok(Some(index))
    }

    fn collect_item(&mut self, item: &Value, index: u64) -> Result<(), ServiceError> {
        if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
            return Ok(());
        }
        let Some(result) = item
            .get("result")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let item_id = item.get("id").and_then(Value::as_str);
        if self.output_indexes.contains_key(&index)
            || item_id.is_some_and(|id| self.item_indexes.contains_key(id))
        {
            self.image_index(Some(index), item_id)?;
        }
        let output_format = item
            .get("output_format")
            .and_then(Value::as_str)
            .unwrap_or(&self.output_format);
        let mut data = image_data(result, self.format, output_format, self.limit)?;
        if let Some(revised) = item.get("revised_prompt") {
            data["revised_prompt"] = revised.clone();
        }
        let mut metadata = Map::new();
        metadata.insert("output_format".into(), json!(output_format));
        for name in ["size", "quality", "background", "output_format"] {
            if let Some(value) = item.get(name) {
                metadata.insert(name.into(), value.clone());
            }
        }
        self.item_metadata.insert(index, metadata);
        self.collected.insert(index, data);
        let bytes: usize = self
            .collected
            .values()
            .map(|value| {
                value
                    .get("b64_json")
                    .or_else(|| value.get("url"))
                    .and_then(Value::as_str)
                    .map_or(0, str::len)
            })
            .sum();
        if bytes > self.limit {
            return Err(invalid_image(
                "Image response exceeds the configured size limit",
            ));
        }
        Ok(())
    }

    fn complete(&mut self, response: &Value) -> Result<Vec<Value>, ServiceError> {
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            // The completed response is authoritative when it includes images.
            if output.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("image_generation_call")
                    && item
                        .get("result")
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
            }) {
                self.collected.clear();
                self.item_metadata.clear();
                for (index, item) in output.iter().enumerate() {
                    self.collect_item(item, index as u64)?;
                }
            }
        }
        if self.collected.is_empty() {
            return Err(invalid_image("Upstream completed without an image result"));
        }
        for index in self.collected.keys().copied().collect::<Vec<_>>() {
            self.image_index(Some(index), None)?;
        }
        self.terminal = true;
        self.metadata = self
            .item_metadata
            .values()
            .next()
            .cloned()
            .unwrap_or_default();
        let mut events = Vec::new();
        for (key, item) in &self.collected {
            let mut event = item.clone();
            event["type"] = json!(format!("{}.completed", self.operation.event_prefix()));
            event["created_at"] = json!(self.created);
            event["output_index"] = json!(self.output_indexes[key]);
            if let Some(metadata) = self.item_metadata.get(key) {
                for (name, value) in metadata {
                    event[name] = value.clone();
                }
            }
            if let Some(usage) = &self.usage {
                event["usage"] = usage.clone();
            }
            events.push(event);
        }
        Ok(events)
    }

    fn fail(&mut self, event: &Value) -> Result<Vec<Value>, ServiceError> {
        self.failed = true;
        self.terminal = true;
        Err(image_failure(event))
    }

    pub(crate) fn finish(&self) -> Result<Value, ServiceError> {
        if !self.terminal || self.failed || self.collected.is_empty() {
            return Err(invalid_image(
                "Image stream ended before successful completion",
            ));
        }
        let mut output = Value::Object(self.metadata.clone());
        output["created"] = json!(self.created);
        output["data"] = Value::Array(self.collected.values().cloned().collect());
        if let Some(usage) = &self.usage {
            output["usage"] = usage.clone();
        }
        Ok(output)
    }
}

/// Image tool usage is distinct from the enclosing text model's token usage.
/// Older upstreams only provide response.usage; retain that compatibility fallback.
pub(super) fn image_usage(response: &Value) -> Option<&Value> {
    response
        .pointer("/tool_usage/image_gen")
        .filter(|value| value.is_object())
        .or_else(|| response.get("usage").filter(|value| !value.is_null()))
}

pub(super) fn image_failure(event: &Value) -> ServiceError {
    let upstream = super::super::upstream_errors::chat_failure(event, 502);
    let details = event
        .pointer("/response/error")
        .or_else(|| event.get("error"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| super::super::upstream_errors::flat_error_details(event));
    ServiceError::new(
        upstream.as_ref().map_or(502, |error| error.status),
        "image_generation_failed",
        upstream
            .as_ref()
            .map(|error| error.message.as_str())
            .unwrap_or("Image generation did not complete"),
    )
    .with_credential_scope(
        upstream
            .as_ref()
            .is_some_and(|error| error.credential_scope),
    )
    .with_retry_after(super::super::events::retry_after_event(event))
    .with_error_details(details)
}

fn image_data(
    result: &str,
    format: ImageResponseFormat,
    output_format: &str,
    limit: usize,
) -> Result<Value, ServiceError> {
    if result.len() > limit {
        return Err(invalid_image(
            "Image result exceeds the configured size limit",
        ));
    }
    let is_url = result.starts_with("https://") || result.starts_with("http://");
    match (format, is_url) {
        (ImageResponseFormat::Url, true) => {
            super::assets::checked_url(result).map_err(|error| invalid_image(error.message))?;
            Ok(json!({"url":result}))
        }
        (ImageResponseFormat::Base64, true) => Err(ServiceError::new(
            502,
            "image_download_required",
            "Upstream returned an image URL where encoded image bytes were required",
        )),
        (ImageResponseFormat::Base64, false) => {
            let encoded = if result.starts_with("data:image/") {
                result
                    .split_once(";base64,")
                    .ok_or_else(|| invalid_image("Upstream returned an invalid image data URL"))?
                    .1
            } else {
                result
            };
            validate_base64(encoded)?;
            Ok(json!({"b64_json":encoded}))
        }
        (ImageResponseFormat::Url, false) => {
            if result.starts_with("data:image/") {
                let encoded = result
                    .split_once(";base64,")
                    .ok_or_else(|| invalid_image("Upstream returned an invalid image data URL"))?
                    .1;
                validate_base64(encoded)?;
                return Ok(json!({"url":result}));
            }
            validate_base64(result)?;
            let mime = match output_format {
                "jpeg" | "jpg" => "image/jpeg",
                "webp" => "image/webp",
                _ => "image/png",
            };
            Ok(json!({"url":format!("data:{mime};base64,{result}")}))
        }
    }
}

fn validate_base64(value: &str) -> Result<(), ServiceError> {
    if value.is_empty() || STANDARD.decode(value).is_err() {
        return Err(invalid_image(
            "Upstream returned invalid base64 image bytes",
        ));
    }
    Ok(())
}

fn invalid_image(message: impl Into<String>) -> ServiceError {
    ServiceError::new(502, "invalid_image_response", message)
}

/// Combines independent, requested image operations without dropping their usage.
pub(crate) fn merge_responses(responses: &[Value]) -> Result<Value, ServiceError> {
    let mut combined = responses
        .first()
        .cloned()
        .ok_or_else(|| invalid_image("No image operations completed"))?;
    let mut data = Vec::new();
    let mut usage = Value::Null;
    for response in responses {
        let images = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_image("Invalid image result data"))?;
        data.extend(images.iter().cloned());
        if let Some(value) = response.get("usage") {
            add_usage(&mut usage, value);
        }
    }
    combined["data"] = Value::Array(data);
    if !usage.is_null() {
        combined["usage"] = usage;
    }
    Ok(combined)
}

fn add_usage(target: &mut Value, incoming: &Value) {
    match (target, incoming) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                add_usage(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (Value::Number(target), Value::Number(source)) => {
            if let (Some(left), Some(right)) = (target.as_u64(), source.as_u64()) {
                *target = left.saturating_add(right).into();
            }
        }
        (target @ Value::Null, value) => *target = value.clone(),
        _ => {}
    }
}

pub(crate) fn native_to_events(
    response: &Value,
    operation: ImageOperation,
) -> Result<Vec<Value>, ServiceError> {
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .filter(|data| !data.is_empty())
        .ok_or_else(|| invalid_image("Native Images response did not contain image data"))?;
    let mut events = Vec::new();
    for (index, item) in data.iter().enumerate() {
        if !item.is_object()
            || (item.get("b64_json").and_then(Value::as_str).is_none()
                && item.get("url").and_then(Value::as_str).is_none())
        {
            return Err(invalid_image(
                "Native Images response contains an invalid image",
            ));
        }
        let mut event = item.clone();
        event["type"] = json!(format!("{}.completed", operation.event_prefix()));
        event["output_index"] = json!(index);
        if let Some(created) = response.get("created") {
            event["created_at"] = created.clone();
        }
        for key in ["background", "output_format", "quality", "size"] {
            if let Some(value) = response.get(key) {
                event[key] = value.clone();
            }
        }
        // The native response usage covers the entire request, not every output item.
        if index + 1 == data.len() {
            if let Some(usage) = response.get("usage") {
                event["usage"] = usage.clone();
            }
        }
        events.push(event);
    }
    Ok(events)
}

pub(crate) fn native_from_events(events: &[Value]) -> Result<Value, ServiceError> {
    let mut output = json!({"created":chrono::Utc::now().timestamp(),"data":[]});
    for event in events {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "error" {
            return Err(image_failure(event));
        }
        if let Some(usage) = event.get("usage") {
            output["usage"] = usage.clone();
        }
        if !matches!(kind, "image_generation.completed" | "image_edit.completed") {
            continue;
        }
        let mut item = Map::new();
        for key in ["b64_json", "url", "revised_prompt"] {
            if let Some(value) = event.get(key) {
                item.insert(key.into(), value.clone());
            }
        }
        if !item.contains_key("b64_json") && !item.contains_key("url") {
            return Err(invalid_image(
                "Native completed image event did not contain image data",
            ));
        }
        output["data"]
            .as_array_mut()
            .expect("initialized array")
            .push(Value::Object(item));
        if let Some(created) = event.get("created_at") {
            output["created"] = created.clone();
        }
        for key in ["background", "output_format", "quality", "size"] {
            if let Some(value) = event.get(key) {
                output[key] = value.clone();
            }
        }
    }
    if output["data"].as_array().is_none_or(Vec::is_empty) {
        return Err(invalid_image(
            "Native image stream ended without completed image data",
        ));
    }
    Ok(output)
}
