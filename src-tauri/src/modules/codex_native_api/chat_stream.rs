use super::output::{chat_image, native_finish_reason, url_citations};
use super::{
    chat_usage, finish_reason, output::completion_id, upstream_error, ChatContext, ChatFlavor,
    ServiceError, StreamOutput,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Tool {
    index: usize,
    id: String,
    name: String,
    arguments: String,
    emitted: bool,
    custom: bool,
}

pub struct ChatStreamAdapter {
    context: ChatContext,
    id: String,
    created: Value,
    role_sent: bool,
    finished: bool,
    text: BTreeMap<(usize, usize, String), String>,
    tools: BTreeMap<usize, Tool>,
    service_tier: Option<Value>,
    item_indices: BTreeMap<String, usize>,
    last_tool: Option<usize>,
    text_offsets: BTreeMap<(usize, usize), i64>,
    text_chars: i64,
    citations: BTreeSet<String>,
    images: BTreeMap<String, [u8; 32]>,
}

impl ChatStreamAdapter {
    pub fn new(context: ChatContext) -> Self {
        Self {
            id: completion_id(&json!({}), context.flavor),
            context,
            created: json!(chrono::Utc::now().timestamp()),
            role_sent: false,
            finished: false,
            text: BTreeMap::new(),
            tools: BTreeMap::new(),
            service_tier: None,
            item_indices: BTreeMap::new(),
            last_tool: None,
            text_offsets: BTreeMap::new(),
            text_chars: 0,
            citations: BTreeSet::new(),
            images: BTreeMap::new(),
        }
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    fn chunk(&self, delta: Value, reason: Option<&str>) -> Value {
        let choice = if self.context.flavor == ChatFlavor::Completions {
            json!({"index":0,"text":delta.get("content").cloned().unwrap_or(json!("")),"finish_reason":reason,"native_finish_reason":reason,"logprobs":null})
        } else {
            json!({"index":0,"delta":delta,"finish_reason":reason,"native_finish_reason":reason,"logprobs":null})
        };
        let mut result = json!({"id":self.id,"object":if self.context.flavor==ChatFlavor::Completions {"text_completion"} else {"chat.completion.chunk"},"created":self.created,"model":self.context.model,"choices":[choice]});
        if self.context.include_usage {
            result["usage"] = Value::Null;
        }
        if let Some(tier) = &self.service_tier {
            result["service_tier"] = tier.clone();
        }
        result
    }
    fn text(
        &mut self,
        output: usize,
        part: usize,
        key: &str,
        value: &str,
        full: bool,
    ) -> Result<Option<Value>, ServiceError> {
        let entry = self
            .text
            .entry((output, part, key.to_string()))
            .or_default();
        let suffix = if full {
            value
                .strip_prefix(entry.as_str())
                .ok_or_else(|| upstream_error("Final response text disagrees with streamed text"))?
                .to_string()
        } else {
            value.to_string()
        };
        entry.push_str(&suffix);
        if key == "content" {
            self.text_offsets
                .entry((output, part))
                .or_insert(self.text_chars);
            self.text_chars += suffix.chars().count() as i64;
        }
        if suffix.is_empty() {
            Ok(None)
        } else {
            let mut delta = json!({});
            delta[if key == "reasoning_text" {
                "reasoning_content"
            } else {
                key
            }] = json!(suffix);
            Ok(Some(self.chunk(delta, None)))
        }
    }
    fn tool(
        &mut self,
        output: usize,
        item: &Value,
        full: bool,
    ) -> Result<Vec<Value>, ServiceError> {
        if self.context.flavor == ChatFlavor::Completions {
            return Err(upstream_error(
                "Legacy Completions cannot stream tool calls",
            ));
        }
        if self.context.legacy_functions
            && (item["type"] == "custom_tool_call"
                || (!self.tools.is_empty() && !self.tools.contains_key(&output)))
        {
            return Err(upstream_error(
                "Legacy function_call cannot represent multiple or custom tool calls",
            ));
        }
        self.last_tool = Some(output);
        let next_index = self.tools.len();
        let tool = self.tools.entry(output).or_insert_with(|| Tool {
            index: next_index,
            ..Tool::default()
        });
        tool.custom = item.get("type").and_then(Value::as_str) == Some("custom_tool_call");
        if let Some(id) = item.get("call_id").and_then(Value::as_str) {
            tool.id = id.to_string();
        }
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            tool.name = self.context.tool_name(name).to_string();
        }
        let value = item
            .get(if tool.custom { "input" } else { "arguments" })
            .and_then(Value::as_str)
            .unwrap_or(if full { &tool.arguments } else { "" });
        let suffix = if full {
            value.strip_prefix(&tool.arguments).ok_or_else(|| {
                upstream_error("Final tool arguments disagree with streamed arguments")
            })?
        } else {
            value
        }
        .to_string();
        tool.arguments.push_str(&suffix);
        let mut updates = Vec::new();
        if !tool.emitted && !tool.name.is_empty() && !tool.id.is_empty() {
            let mut details = json!({"name":tool.name});
            details[if tool.custom { "input" } else { "arguments" }] = json!(tool.arguments);
            let delta = if self.context.legacy_functions {
                json!({"function_call":details})
            } else {
                let mut call = json!({"index":tool.index,"id":tool.id,"type":if tool.custom{"custom"}else{"function"}});
                call[if tool.custom { "custom" } else { "function" }] = details;
                json!({"tool_calls":[call]})
            };
            tool.emitted = true;
            updates.push(delta);
        } else if tool.emitted && !suffix.is_empty() {
            let mut details = json!({});
            details[if tool.custom { "input" } else { "arguments" }] = json!(suffix);
            let delta = if self.context.legacy_functions {
                json!({"function_call":details})
            } else {
                let mut call = json!({"index":tool.index});
                call[if tool.custom { "custom" } else { "function" }] = details;
                json!({"tool_calls":[call]})
            };
            updates.push(delta);
        }
        Ok(updates
            .into_iter()
            .map(|delta| self.chunk(delta, None))
            .collect())
    }
    fn item(
        &mut self,
        output: usize,
        item: &Value,
        full: bool,
    ) -> Result<Vec<Value>, ServiceError> {
        let mut result = Vec::new();
        match item.get("type").and_then(Value::as_str) {
            Some("function_call" | "custom_tool_call") => {
                result.extend(self.tool(output, item, full)?)
            }
            Some("message") => {
                for (part, content) in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let (key, value) = match content.get("type").and_then(Value::as_str) {
                        Some("output_text") => ("content", content.get("text")),
                        Some("refusal") => ("refusal", content.get("refusal")),
                        _ => continue,
                    };
                    if let Some(chunk) = self.text(
                        output,
                        part,
                        key,
                        value.and_then(Value::as_str).unwrap_or_default(),
                        full,
                    )? {
                        result.push(chunk);
                    }
                }
            }
            Some("reasoning") => {
                for (field, key) in [
                    ("summary", "reasoning_content"),
                    ("content", "reasoning_text"),
                ] {
                    for (part, content) in item
                        .get(field)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        if let Some(chunk) = self.text(
                            output,
                            part,
                            key,
                            content["text"].as_str().unwrap_or_default(),
                            full,
                        )? {
                            result.push(chunk);
                        }
                    }
                }
            }
            Some("image_generation_call") => {
                if let Some(chunk) = self.image(
                    item["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("output:{output}")),
                    item["result"].as_str(),
                    item["output_format"].as_str(),
                )? {
                    result.push(chunk);
                }
            }
            _ => {}
        }
        Ok(result)
    }
    fn image(
        &mut self,
        key: String,
        data: Option<&str>,
        format: Option<&str>,
    ) -> Result<Option<Value>, ServiceError> {
        let Some(data) = data.filter(|data| !data.is_empty()) else {
            return Ok(None);
        };
        let hash: [u8; 32] = Sha256::digest(data.as_bytes()).into();
        if self.images.get(&key) == Some(&hash) {
            return Ok(None);
        }
        self.images.insert(key, hash);
        Ok(Some(self.chunk(
            json!({"images":[chat_image(data, format, 0)]}),
            None,
        )))
    }
    fn annotations(
        &mut self,
        event: &Value,
        index: usize,
        part: usize,
    ) -> Result<Option<Value>, ServiceError> {
        let mut offset = self.text_offsets.get(&(index, part)).copied().unwrap_or(0);
        let mut annotations = Vec::new();
        for value in [
            event.get("annotation"),
            event.get("annotations"),
            event.pointer("/part/annotations"),
            event.pointer("/item/annotations"),
        ] {
            annotations.extend(url_citations(value, offset, &mut self.citations));
        }
        for (part, content) in event
            .pointer("/item/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            offset = self
                .text_offsets
                .get(&(index, part))
                .copied()
                .unwrap_or(offset);
            annotations.extend(url_citations(
                content.get("annotations"),
                offset,
                &mut self.citations,
            ));
            offset += content["text"].as_str().unwrap_or("").chars().count() as i64;
        }
        if annotations.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.chunk(json!({"annotations":annotations}), None)))
    }
    pub fn push(&mut self, event: &Value) -> Result<Vec<StreamOutput>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if let Some(error) = super::super::upstream_errors::chat_failure(event, 502) {
            return Err(error);
        }
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut output = Vec::new();
        if let Some(response) = event.get("response") {
            if !self.role_sent {
                self.id = completion_id(response, self.context.flavor);
                if let Some(created) = response.get("created_at") {
                    self.created = created.clone();
                }
                if let Some(model) = response.get("model").and_then(Value::as_str) {
                    self.context.model = model.to_string();
                }
            }
            if let Some(tier) = response
                .get("service_tier")
                .filter(|value| value.as_str().is_some_and(|tier| !tier.trim().is_empty()))
            {
                self.service_tier = Some(tier.clone());
            }
        }
        if !self.role_sent {
            self.role_sent = true;
            if self.context.flavor == ChatFlavor::ChatCompletions {
                output.push(StreamOutput::Event(
                    self.chunk(json!({"role":"assistant","content":""}), None),
                ));
            }
        }
        let item_id = event["item_id"]
            .as_str()
            .or_else(|| event.pointer("/item/id").and_then(Value::as_str));
        let index = event["output_index"]
            .as_u64()
            .map(|index| index as usize)
            .or_else(|| item_id.and_then(|id| self.item_indices.get(id).copied()))
            .or_else(|| {
                if kind.contains("arguments") || kind.contains("tool_call_input") {
                    self.last_tool
                } else {
                    None
                }
            })
            .unwrap_or(0);
        if let Some(id) = item_id {
            if !self.item_indices.contains_key(id) {
                self.item_indices.insert(id.to_owned(), index);
            }
        }
        let part = event
            .get("content_index")
            .or_else(|| event.get("summary_index"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let mut chunks = match kind {
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta" => {
                let key = match kind {
                    "response.output_text.delta" => "content",
                    "response.refusal.delta" => "refusal",
                    "response.reasoning_text.delta" => "reasoning_text",
                    _ => "reasoning_content",
                };
                self.text(
                    index,
                    part,
                    key,
                    event
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    false,
                )?
                .into_iter()
                .collect()
            }
            "response.output_item.added" | "response.output_item.done" => self.item(
                index,
                event
                    .get("item")
                    .ok_or_else(|| upstream_error("Output event has no item"))?,
                true,
            )?,
            "response.output_text.done"
            | "response.refusal.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done" => {
                let key = match kind {
                    "response.output_text.done" => "content",
                    "response.refusal.done" => "refusal",
                    "response.reasoning_text.done" => "reasoning_text",
                    _ => "reasoning_content",
                };
                if let Some(value) = event
                    .get(if key == "refusal" { "refusal" } else { "text" })
                    .and_then(Value::as_str)
                {
                    self.text(index, part, key, value, true)?
                        .into_iter()
                        .collect()
                } else {
                    Vec::new()
                }
            }
            "response.image_generation_call.partial_image" => self
                .image(
                    item_id
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("output:{index}")),
                    event["partial_image_b64"].as_str(),
                    event["output_format"].as_str(),
                )?
                .into_iter()
                .collect(),
            "response.function_call_arguments.delta"
            | "response.custom_tool_call_input.delta"
            | "response.function_call_arguments.done"
            | "response.custom_tool_call_input.done" => {
                let custom = kind.starts_with("response.custom_tool_call_input.");
                let full = kind.ends_with(".done");
                let mut item = json!({"type":if custom{"custom_tool_call"}else{"function_call"}});
                item[if custom { "input" } else { "arguments" }] = event
                    .get(if full {
                        if custom {
                            "input"
                        } else {
                            "arguments"
                        }
                    } else {
                        "delta"
                    })
                    .cloned()
                    .unwrap_or(json!(""));
                self.tool(index, &item, full)?
            }
            "response.completed" | "response.incomplete" => {
                let response = event
                    .get("response")
                    .ok_or_else(|| upstream_error("Terminal event has no response"))?;
                let mut chunks = Vec::new();
                for (i, item) in response
                    .get("output")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    chunks.extend(self.item(i, item, true)?);
                    if let Some(annotation) = self.annotations(&json!({"item":item}), i, 0)? {
                        chunks.push(annotation);
                    }
                }
                if self.tools.values().any(|tool| !tool.emitted) {
                    return Err(upstream_error(
                        "Tool stream ended without a call ID or name",
                    ));
                }
                let mut terminal = self.chunk(
                    json!({}),
                    Some(finish_reason(
                        response,
                        !self.tools.is_empty(),
                        self.context.legacy_functions,
                    )),
                );
                let reason = terminal["choices"][0]["finish_reason"]
                    .as_str()
                    .unwrap_or("stop");
                terminal["choices"][0]["native_finish_reason"] =
                    json!(native_finish_reason(response, reason));
                if !self.context.include_usage {
                    if let Some(usage) = response.get("usage").filter(|usage| !usage.is_null()) {
                        terminal["usage"] = chat_usage(usage);
                    }
                }
                chunks.push(terminal);
                if self.context.include_usage {
                    let mut usage = self.chunk(json!({}), None);
                    usage["choices"] = json!([]);
                    usage["usage"] = response
                        .get("usage")
                        .filter(|v| !v.is_null())
                        .map(chat_usage)
                        .unwrap_or(Value::Null);
                    chunks.push(usage);
                }
                self.finished = true;
                chunks
            }
            "response.failed" | "error" => {
                return Err(upstream_error(
                    event
                        .pointer("/error/message")
                        .or_else(|| event.pointer("/response/error/message"))
                        .and_then(Value::as_str)
                        .unwrap_or("The upstream stream failed"),
                ))
            }
            _ => Vec::new(),
        };
        if matches!(
            kind,
            "response.output_text.annotation.added"
                | "response.output_text.done"
                | "response.content_part.done"
                | "response.output_item.done"
        ) {
            if let Some(annotation) = self.annotations(event, index, part)? {
                chunks.push(annotation);
            }
        }
        if kind == "response.output_text.delta" {
            if let Some(logprobs) = event.get("logprobs").filter(|v| v.is_array()) {
                if let Some(chunk) = chunks.first_mut() {
                    chunk["choices"][0]["logprobs"] = json!({"content":logprobs,"refusal":null});
                }
            }
        }
        output.extend(chunks.into_iter().map(StreamOutput::Event));
        if self.finished {
            output.push(StreamOutput::Done);
        }
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Vec<StreamOutput>, ServiceError> {
        if self.finished {
            Ok(Vec::new())
        } else {
            Err(upstream_error(
                "Responses stream ended before a terminal event",
            ))
        }
    }
}
