use super::super::chat::{ChatContext, ChatFlavor, ChatStreamAdapter, StreamOutput};
use super::{output, search, upstream, CompatContext, CompatFlavor, CompatFrame, ServiceError};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
struct Tool {
    id: String,
    name: String,
    args: String,
    block: Option<usize>,
}
pub struct CompatStreamAdapter {
    context: CompatContext,
    inner: ChatStreamAdapter,
    started: bool,
    finished: bool,
    blocks: BTreeMap<String, usize>,
    tools: BTreeMap<usize, Tool>,
    next_block: usize,
    created: String,
    reasoning_key: String,
    signed_reasoning: BTreeSet<String>,
    closed_blocks: BTreeSet<usize>,
    searches: BTreeSet<String>,
    response_id: String,
    ollama_text: String,
    ollama_thinking: String,
    reasoning_signatures: BTreeMap<String, String>,
    reasoning_keys: BTreeMap<usize, String>,
    signature_bytes: usize,
}
impl CompatStreamAdapter {
    pub fn new(context: CompatContext) -> Self {
        let inner = ChatStreamAdapter::new(ChatContext {
            model: context.model.clone(),
            flavor: ChatFlavor::ChatCompletions,
            include_usage: true,
            legacy_functions: false,
            tool_names: Default::default(),
        });
        Self {
            context,
            inner,
            started: false,
            finished: false,
            blocks: BTreeMap::new(),
            tools: BTreeMap::new(),
            next_block: 0,
            created: chrono::Utc::now().to_rfc3339(),
            reasoning_key: "reasoning_content".into(),
            signed_reasoning: BTreeSet::new(),
            closed_blocks: BTreeSet::new(),
            searches: BTreeSet::new(),
            response_id: String::new(),
            ollama_text: String::new(),
            ollama_thinking: String::new(),
            reasoning_signatures: BTreeMap::new(),
            reasoning_keys: BTreeMap::new(),
            signature_bytes: 0,
        }
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    fn frame(&self, data: Value) -> CompatFrame {
        CompatFrame {
            event: if self.context.flavor == CompatFlavor::Claude {
                data["type"].as_str().map(str::to_string)
            } else {
                None
            },
            data,
        }
    }
    fn plain(&self, message: Value) -> Value {
        json!({"model":self.context.model,"created_at":self.created,"message":message,"done":false})
    }
    fn gemini(&self, parts: Value) -> Value {
        json!({"candidates":[{"index":0,"content":{"role":"model","parts":parts}}],"modelVersion":self.context.model,"responseId":self.response_id,"createTime":self.created})
    }
    fn emit_text(&mut self, key: &str, text: &str, out: &mut Vec<CompatFrame>) {
        if text.is_empty() {
            return;
        }
        match self.context.flavor {
            CompatFlavor::Claude => {
                let block_key = if key == "reasoning_content" {
                    self.reasoning_key.clone()
                } else {
                    key.to_owned()
                };
                let block = if let Some(index) = self
                    .blocks
                    .get(&block_key)
                    .filter(|index| !self.closed_blocks.contains(index))
                {
                    *index
                } else {
                    let index = self.next_block;
                    self.next_block += 1;
                    self.blocks.insert(block_key, index);
                    let content = if key == "reasoning_content" {
                        json!({"type":"thinking","thinking":"","signature":""})
                    } else {
                        json!({"type":"text","text":""})
                    };
                    out.push(self.frame(
                        json!({"type":"content_block_start","index":index,"content_block":content}),
                    ));
                    index
                };
                let delta = if key == "reasoning_content" {
                    json!({"type":"thinking_delta","thinking":text})
                } else {
                    json!({"type":"text_delta","text":text})
                };
                out.push(
                    self.frame(json!({"type":"content_block_delta","index":block,"delta":delta})),
                );
            }
            CompatFlavor::Gemini => {
                let mut part = json!({"text":text});
                if key == "reasoning_content" {
                    part["thought"] = json!(true);
                }
                out.push(self.frame(self.gemini(json!([part]))));
            }
            CompatFlavor::Ollama => {
                if key == "reasoning_content" {
                    self.ollama_thinking.push_str(text);
                } else {
                    self.ollama_text.push_str(text);
                }
                let mut message = json!({"role":"assistant","content":""});
                message[if key == "reasoning_content" {
                    "thinking"
                } else {
                    "content"
                }] = json!(text);
                out.push(self.frame(self.plain(message)));
            }
        }
    }
    fn reasoning_signature(
        &mut self,
        item: &Value,
        output_index: Option<usize>,
        out: &mut Vec<CompatFrame>,
    ) {
        if self.context.flavor != CompatFlavor::Claude {
            return;
        }
        let key = item["id"]
            .as_str()
            .map(|id| format!("reasoning:{id}"))
            .or_else(|| output_index.map(|index| format!("reasoning-index:{index}")))
            .unwrap_or_else(|| self.reasoning_key.clone());
        if self.signed_reasoning.contains(&key) {
            return;
        }
        let fallback = self.reasoning_signatures.remove(&key);
        if let Some(signature) = &fallback {
            self.signature_bytes = self.signature_bytes.saturating_sub(signature.len());
        }
        let signature = item["encrypted_content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(fallback.as_deref())
            .unwrap_or("");
        if signature.is_empty() {
            return;
        }
        let mut block = self.blocks.get(&key).copied().or_else(|| {
            self.blocks
                .get(&self.reasoning_key)
                .copied()
                .filter(|index| !self.closed_blocks.contains(index))
        });
        if block.is_none() && !signature.is_empty() {
            let index = self.next_block;
            self.next_block += 1;
            self.blocks.insert(key.clone(), index);
            out.push(self.frame(json!({"type":"content_block_start","index":index,"content_block":{"type":"thinking","thinking":"","signature":""}})));
            block = Some(index);
        }
        if let Some(index) = block {
            if !signature.is_empty() {
                out.push(self.frame(json!({"type":"content_block_delta","index":index,"delta":{"type":"signature_delta","signature":signature}})));
            }
            if self.closed_blocks.insert(index) {
                out.push(self.frame(json!({"type":"content_block_stop","index":index})));
            }
            self.signed_reasoning.insert(key);
        }
    }
    fn hosted_search(&mut self, item: &Value, out: &mut Vec<CompatFrame>) {
        if self.context.flavor != CompatFlavor::Claude {
            return;
        }
        let Some(id) = item["id"].as_str() else {
            return;
        };
        let blocks = search::claude_blocks(item);
        if blocks.is_empty() || !self.searches.insert(id.to_owned()) {
            return;
        }
        for content in blocks {
            let index = self.next_block;
            self.next_block += 1;
            out.push(self.frame(
                json!({"type":"content_block_start","index":index,"content_block":content}),
            ));
            out.push(self.frame(json!({"type":"content_block_stop","index":index})));
            self.closed_blocks.insert(index);
        }
    }
    fn emit_tool(&mut self, call: &Value, out: &mut Vec<CompatFrame>) -> Result<(), ServiceError> {
        if call.get("custom").is_some() {
            return Err(upstream(
                "This client protocol supports JSON function tools only",
            ));
        }
        let index = call["index"].as_u64().unwrap_or(0) as usize;
        let tool = self.tools.entry(index).or_default();
        if let Some(id) = call["id"].as_str() {
            tool.id = id.to_string();
        }
        if let Some(name) = call["function"]["name"].as_str() {
            tool.name.push_str(name);
        }
        let args = call["function"]["arguments"].as_str().unwrap_or("");
        tool.args.push_str(args);
        if self.context.flavor == CompatFlavor::Claude {
            let mut data = Vec::new();
            let name = self.context.tool_name(&tool.name);
            let block = if let Some(b) = tool.block {
                b
            } else {
                if tool.id.is_empty() || tool.name.is_empty() {
                    return Ok(());
                }
                let b = self.next_block;
                self.next_block += 1;
                tool.block = Some(b);
                data.push(json!({"type":"content_block_start","index":b,"content_block":{"type":"tool_use","id":tool.id,"name":name,"input":{}}}));
                b
            };
            if !args.is_empty() {
                data.push(json!({"type":"content_block_delta","index":block,"delta":{"type":"input_json_delta","partial_json":args}}));
            }
            for event in data {
                out.push(self.frame(event));
            }
        }
        Ok(())
    }
    fn emit_images(&mut self, images: &Value, out: &mut Vec<CompatFrame>) {
        for image in images.as_array().into_iter().flatten() {
            let Some((mime, data)) = image
                .pointer("/image_url/url")
                .and_then(Value::as_str)
                .and_then(|url| url.strip_prefix("data:"))
                .and_then(|url| url.split_once(";base64,"))
            else {
                continue;
            };
            match self.context.flavor {
                CompatFlavor::Claude => {
                    let index = self.next_block;
                    self.next_block += 1;
                    out.push(self.frame(json!({"type":"content_block_start","index":index,"content_block":{"type":"image","source":{"type":"base64","media_type":mime,"data":data}}})));
                    out.push(self.frame(json!({"type":"content_block_stop","index":index})));
                    self.closed_blocks.insert(index);
                }
                CompatFlavor::Gemini => out.push(
                    self.frame(self.gemini(json!([{"inlineData":{"mimeType":mime,"data":data}}]))),
                ),
                CompatFlavor::Ollama => {
                    out.push(self.frame(
                        self.plain(json!({"role":"assistant","content":"","images":[data]})),
                    ))
                }
            }
        }
    }
    fn terminal(
        &mut self,
        response: &Value,
        out: &mut Vec<CompatFrame>,
    ) -> Result<(), ServiceError> {
        for (index, item) in response["output"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            match item["type"].as_str() {
                Some("reasoning") => self.reasoning_signature(item, Some(index), out),
                Some("web_search_call") => self.hosted_search(item, out),
                _ => {}
            }
        }
        let mut tool_parts = Vec::new();
        let mut ollama_calls = Vec::new();
        for tool in self.tools.values() {
            let args: Value = serde_json::from_str(&tool.args)
                .map_err(|_| upstream("Tool stream ended with invalid JSON arguments"))?;
            if !args.is_object() {
                return Err(upstream("Tool arguments must be a JSON object"));
            }
            let name = self.context.tool_name(&tool.name);
            tool_parts.push(json!({"functionCall":{"id":tool.id,"name":name,"args":args}}));
            ollama_calls.push(
                json!({"id":tool.id,"type":"function","function":{"name":name,"arguments":args}}),
            );
        }
        let usage = output::usage(&response["usage"], self.context.flavor);
        let reason = output::stop_reason(response, self.context.flavor, !self.tools.is_empty());
        match self.context.flavor {
            CompatFlavor::Claude => {
                for index in 0..self.next_block {
                    if self.closed_blocks.insert(index) {
                        out.push(self.frame(json!({"type":"content_block_stop","index":index})));
                    }
                }
                out.push(self.frame(json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":response.get("stop_sequence").cloned().unwrap_or(Value::Null)},"usage":usage})));
                out.push(self.frame(json!({"type":"message_stop"})));
            }
            CompatFlavor::Gemini => {
                let mut frame = self.gemini(json!(tool_parts));
                frame["candidates"][0]["finishReason"] = json!(reason);
                frame["usageMetadata"] = usage;
                frame["responseId"] = response["id"].clone();
                out.push(self.frame(frame));
            }
            CompatFlavor::Ollama => {
                let mut message = json!({"role":"assistant","content":self.ollama_text});
                if !self.ollama_thinking.is_empty() {
                    message["thinking"] = json!(self.ollama_thinking);
                }
                if !ollama_calls.is_empty() {
                    message["tool_calls"] = json!(ollama_calls);
                }
                let mut frame = self.plain(message);
                frame["done"] = json!(true);
                frame["done_reason"] = json!(reason);
                for key in [
                    "total_duration",
                    "load_duration",
                    "prompt_eval_duration",
                    "eval_duration",
                ] {
                    frame[key] = json!(0);
                }
                frame
                    .as_object_mut()
                    .unwrap()
                    .extend(usage.as_object().unwrap().clone());
                out.push(self.frame(frame));
            }
        }
        self.finished = true;
        Ok(())
    }
    pub fn push(&mut self, event: &Value) -> Result<Vec<CompatFrame>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        let kind = event["type"].as_str().unwrap_or("");
        if kind.contains("reasoning")
            || event.pointer("/item/type").and_then(Value::as_str) == Some("reasoning")
        {
            if let Some(id) = event["item_id"]
                .as_str()
                .or_else(|| event.pointer("/item/id").and_then(Value::as_str))
            {
                self.reasoning_key = format!("reasoning:{id}");
                if let Some(index) = event["output_index"].as_u64() {
                    self.reasoning_keys
                        .insert(index as usize, self.reasoning_key.clone());
                }
            } else if let Some(index) = event["output_index"].as_u64() {
                self.reasoning_key = self
                    .reasoning_keys
                    .get(&(index as usize))
                    .cloned()
                    .unwrap_or_else(|| format!("reasoning-index:{index}"));
            }
        }
        let mut out = Vec::new();
        // Terminal-only responses must retain each item's thinking/signature
        // association instead of merging all final reasoning into one block.
        if matches!(kind, "response.completed" | "response.incomplete") {
            if let Some(response) = event.get("response") {
                if !self.started {
                    out.extend(self.push(&json!({"type":"response.created","response":response}))?);
                }
                for (index, item) in response["output"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    out.extend(self.push(&json!({"type":"response.output_item.done","output_index":index,"item":item}))?);
                }
            }
        }
        if self.context.flavor == CompatFlavor::Claude
            && kind == "response.output_item.added"
            && event.pointer("/item/type").and_then(Value::as_str) == Some("reasoning")
        {
            if let Some(signature) = event
                .pointer("/item/encrypted_content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                let previous = self
                    .reasoning_signatures
                    .get(&self.reasoning_key)
                    .map_or(0, String::len);
                self.signature_bytes = self
                    .signature_bytes
                    .saturating_sub(previous)
                    .saturating_add(signature.len());
                if self.signature_bytes > 32 * 1024 * 1024
                    || self.reasoning_signatures.len() >= 4096
                {
                    return Err(upstream("Thinking signatures exceed bounded adapter state"));
                }
                self.reasoning_signatures
                    .insert(self.reasoning_key.clone(), signature.to_owned());
            }
        }
        let chunks = self.inner.push(event)?;
        if !self.started {
            self.started = true;
            if let Some(response) = event.get("response") {
                self.created = output::timestamp(response);
                self.response_id = response["id"].as_str().unwrap_or("").to_owned();
                if let Some(model) = response["model"].as_str() {
                    self.context.model = model.to_string();
                }
            }
            if self.context.flavor == CompatFlavor::Claude {
                let id = event
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
                out.push(self.frame(json!({"type":"message_start","message":{"id":format!("msg_{id}"),"type":"message","role":"assistant","model":self.context.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}})));
            }
        }
        if self.context.flavor == CompatFlavor::Claude
            && kind == "response.reasoning_summary_part.added"
            && self
                .blocks
                .get(&self.reasoning_key)
                .is_some_and(|index| !self.closed_blocks.contains(index))
        {
            self.emit_text("reasoning_content", "\n\n", &mut out);
        }
        for chunk in chunks {
            match chunk {
                StreamOutput::Event(value) => {
                    if let Some(choices) = value["choices"].as_array() {
                        for choice in choices {
                            let delta = &choice["delta"];
                            for key in ["content", "refusal", "reasoning_content"] {
                                if let Some(text) = delta[key].as_str() {
                                    self.emit_text(key, text, &mut out);
                                }
                            }
                            self.emit_images(&delta["images"], &mut out);
                            if let Some(calls) = delta["tool_calls"].as_array() {
                                for call in calls {
                                    self.emit_tool(call, &mut out)?;
                                }
                            }
                        }
                    }
                }
                StreamOutput::Done => {
                    self.terminal(
                        event
                            .get("response")
                            .ok_or_else(|| upstream("Terminal event is missing response"))?,
                        &mut out,
                    )?;
                }
            }
        }
        if kind == "response.output_item.done" {
            if let Some(item) = event.get("item") {
                match item["type"].as_str() {
                    Some("reasoning") => self.reasoning_signature(
                        item,
                        event["output_index"].as_u64().map(|index| index as usize),
                        &mut out,
                    ),
                    Some("web_search_call") => self.hosted_search(item, &mut out),
                    _ => {}
                }
            }
        }
        Ok(out)
    }
    pub fn finish(&mut self) -> Result<Vec<CompatFrame>, ServiceError> {
        self.inner.finish()?;
        if self.finished {
            Ok(Vec::new())
        } else {
            Err(upstream("Responses stream ended without a terminal event"))
        }
    }
}
