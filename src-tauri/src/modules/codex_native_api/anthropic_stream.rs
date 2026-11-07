use super::{output_item, upstream, usage, ServiceError, ToolContext};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Block {
    block: Value,
    item: Value,
    output: usize,
    arguments: String,
    custom_input: String,
    done: bool,
}

pub(crate) struct ResponsesStreamAdapter {
    response: Value,
    sequence: u64,
    blocks: BTreeMap<u64, Block>,
    context: ToolContext,
    discovery: super::super::chatgpt_plan_tools::Context,
    started: bool,
    finished: bool,
    stop_reason: String,
    token_usage: Value,
    search_queries: BTreeMap<String, String>,
    search_cache: Option<(
        std::sync::Arc<super::super::anthropic_search_cache::Cache>,
        String,
    )>,
}

impl ResponsesStreamAdapter {
    pub(crate) fn new(model: String, body: &Value) -> Result<Self, ServiceError> {
        let (body, discovery) = super::prepare_client_tools(body)?;
        Ok(Self {
            response: json!({"id":format!("resp_{}",uuid::Uuid::now_v7()),"object":"response","created_at":chrono::Utc::now().timestamp(),"model":model,"status":"in_progress","output":[],"usage":null,"error":null,"incomplete_details":null}),
            sequence: 0,
            blocks: BTreeMap::new(),
            context: ToolContext::for_anthropic(&body),
            discovery,
            started: false,
            finished: false,
            stop_reason: String::new(),
            token_usage: json!({}),
            search_queries: body["input"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "web_search_call")
                .filter_map(|item| {
                    Some((
                        item["id"].as_str()?.to_owned(),
                        item.pointer("/action/query")?.as_str()?.to_owned(),
                    ))
                })
                .collect(),
            search_cache: None,
        })
    }
    pub(in crate::modules::codex_native_api) fn with_search_cache(
        mut self,
        cache: std::sync::Arc<super::super::anthropic_search_cache::Cache>,
        scope: Option<String>,
    ) -> Self {
        self.search_cache = scope.map(|scope| (cache, scope));
        self
    }
    fn event(&mut self, mut event: Value) -> Value {
        event["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        event
    }
    pub(crate) fn push(&mut self, value: &Value) -> Result<Vec<Value>, ServiceError> {
        let mut restored = Vec::new();
        for event in self.push_inner(value)? {
            restored.extend(super::super::chatgpt_plan_tools::restore_event(
                &mut self.discovery,
                event,
            )?);
        }
        Ok(restored)
    }
    fn push_inner(&mut self, value: &Value) -> Result<Vec<Value>, ServiceError> {
        if self.finished {
            return Ok(vec![]);
        }
        let mut out = vec![];
        match value["type"].as_str().unwrap_or("") {
            "ping" => {}
            "error" => {
                return Err(upstream(
                    value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Anthropic stream failed"),
                ))
            }
            "message_start" => {
                if self.started {
                    return Err(upstream("Duplicate Anthropic message_start"));
                }
                if value["message"]["id"].as_str().is_none_or(str::is_empty) {
                    return Err(upstream("Anthropic message_start is missing its id"));
                }
                self.started = true;
                if let Some(id) = value.pointer("/message/id").and_then(Value::as_str) {
                    self.response["id"] = json!(format!("resp_{id}"));
                }
                self.token_usage = value["message"]["usage"].clone();
                out.push(self.event(json!({"type":"response.created","response":self.response})));
                out.push(
                    self.event(json!({"type":"response.in_progress","response":self.response})),
                );
            }
            "content_block_start" => {
                if !self.started {
                    return Err(upstream("Anthropic block arrived before message_start"));
                }
                let index = value["index"]
                    .as_u64()
                    .ok_or_else(|| upstream("Missing Anthropic block index"))?;
                if self.blocks.contains_key(&index) {
                    return Err(upstream("Duplicate Anthropic content block"));
                }
                let block = value["content_block"].clone();
                let output = self
                    .blocks
                    .values()
                    .filter(|block| !block.item.is_null())
                    .count();
                let mut item = if block["type"] == "server_tool_use"
                    && block["name"] == "web_search"
                {
                    super::super::anthropic_search::item(
                        block["id"].as_str().unwrap_or(""),
                        block["input"]["query"].as_str().unwrap_or(""),
                    )
                } else if block["type"] == "web_search_tool_result" {
                    let id = format!("ws_{}", block["tool_use_id"].as_str().unwrap_or(""));
                    if let Some(previous) = self
                        .blocks
                        .values_mut()
                        .find(|b| b.item["type"] == "web_search_call" && b.item["id"] == id)
                    {
                        super::super::anthropic_search::attach_results(
                            &mut previous.item,
                            &block["content"],
                        );
                        self.blocks.insert(
                            index,
                            Block {
                                block,
                                item: Value::Null,
                                output,
                                arguments: String::new(),
                                custom_input: String::new(),
                                done: false,
                            },
                        );
                        return Ok(out);
                    }
                    let mut item = super::super::anthropic_search::item(
                        block["tool_use_id"].as_str().unwrap_or(""),
                        self.search_queries
                            .get(&id)
                            .map(String::as_str)
                            .unwrap_or(""),
                    );
                    super::super::anthropic_search::attach_results(&mut item, &block["content"]);
                    item
                } else {
                    super::output_item_partial(&block, &self.context, true)?
                };
                if item["type"] != "reasoning" {
                    item["status"] = json!("in_progress");
                }
                if block["type"] == "tool_use" {
                    let field = if item["type"] == "custom_tool_call" {
                        "input"
                    } else {
                        "arguments"
                    };
                    item[field] = json!("");
                }
                if block["type"] == "text" {
                    item["content"] = json!([]);
                }
                if item["type"] == "reasoning" {
                    item["summary"] = json!([]);
                    item.as_object_mut().unwrap().remove("encrypted_content");
                }
                out.push(self.event(
                    json!({"type":"response.output_item.added","output_index":output,"item":item}),
                ));
                if block["type"] == "text" {
                    out.push(self.event(json!({"type":"response.content_part.added","output_index":output,"item_id":item["id"],"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})));
                    if let Some(text) = block["text"].as_str().filter(|s| !s.is_empty()) {
                        out.push(self.event(json!({"type":"response.output_text.delta","output_index":output,"item_id":item["id"],"content_index":0,"delta":text})));
                    }
                } else if block["type"] == "thinking" {
                    out.push(self.event(json!({"type":"response.reasoning_summary_part.added","output_index":output,"item_id":item["id"],"summary_index":0,"part":{"type":"summary_text","text":""}})));
                    if let Some(text) = block["thinking"].as_str().filter(|s| !s.is_empty()) {
                        out.push(self.event(json!({"type":"response.reasoning_summary_text.delta","output_index":output,"item_id":item["id"],"summary_index":0,"delta":text})));
                    }
                }
                self.blocks.insert(
                    index,
                    Block {
                        block,
                        item,
                        output,
                        arguments: String::new(),
                        custom_input: String::new(),
                        done: false,
                    },
                );
            }
            "content_block_delta" => {
                let index = value["index"]
                    .as_u64()
                    .ok_or_else(|| upstream("Missing Anthropic delta index"))?;
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or_else(|| upstream("Anthropic delta references an unknown block"))?;
                if block.done {
                    return Err(upstream("Anthropic delta arrived after block completion"));
                }
                let delta = &value["delta"];
                if delta["type"] == "citations_delta" {
                    if !block.block["citations"].is_array() {
                        block.block["citations"] = json!([])
                    }
                    let citations = block.block["citations"].as_array_mut().unwrap();
                    let annotation_index = citations.len();
                    citations.push(delta["citation"].clone());
                    let event = json!({"type":"response.output_text.annotation.added","output_index":block.output,"item_id":block.item["id"],"content_index":0,"annotation_index":annotation_index,"annotation":delta["citation"]});
                    out.push(self.event(event));
                    return Ok(out);
                }
                let (field, key, event_kind) = match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => ("text", "text", "response.output_text.delta"),
                    "thinking_delta" => (
                        "thinking",
                        "thinking",
                        "response.reasoning_summary_text.delta",
                    ),
                    "signature_delta" => ("signature", "signature", ""),
                    "input_json_delta" => {
                        ("", "partial_json", "response.function_call_arguments.delta")
                    }
                    _ => return Err(upstream("Unsupported Anthropic stream delta")),
                };
                let text = delta[key]
                    .as_str()
                    .ok_or_else(|| upstream("Anthropic delta must contain text"))?;
                if field.is_empty() {
                    block.arguments.push_str(text);
                } else {
                    let old = block.block[field].as_str().unwrap_or("");
                    block.block[field] = json!(format!("{old}{text}"));
                }
                if block.item["type"] == "custom_tool_call" && field.is_empty() {
                    let input = super::super::chat_upstream::unwrap_input(&block.arguments, true);
                    let delta = input
                        .strip_prefix(&block.custom_input)
                        .ok_or_else(|| {
                            upstream("Anthropic changed an already streamed custom tool input")
                        })?
                        .to_owned();
                    block.custom_input = input;
                    if !delta.is_empty() {
                        let event = json!({"type":"response.custom_tool_call_input.delta","output_index":block.output,"item_id":block.item["id"],"delta":delta});
                        out.push(self.event(event));
                    }
                    return Ok(out);
                }
                if !event_kind.is_empty()
                    && !matches!(
                        block.item["type"].as_str(),
                        Some("custom_tool_call" | "web_search_call")
                    )
                {
                    let mut event = json!({"type":event_kind,"output_index":block.output,"item_id":block.item["id"],"delta":text});
                    if field == "thinking" {
                        event["summary_index"] = json!(0);
                    } else if field == "text" {
                        event["content_index"] = json!(0);
                    }
                    out.push(self.event(event));
                }
            }
            "content_block_stop" => {
                let index = value["index"]
                    .as_u64()
                    .ok_or_else(|| upstream("Missing Anthropic block index"))?;
                out.extend(self.close(index)?);
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = reason.into();
                }
                if let Some(values) = value["usage"].as_object() {
                    for (key, val) in values {
                        self.token_usage[key] = val.clone();
                    }
                }
            }
            "message_stop" => {
                if !self.started
                    || self.stop_reason.is_empty()
                    || self.blocks.values().any(|v| !v.done)
                {
                    return Err(upstream("Anthropic message ended with unfinished blocks"));
                }
                self.finished = true;
                let mut finished_searches = vec![];
                for block in self
                    .blocks
                    .values_mut()
                    .filter(|block| block.item["type"] == "web_search_call")
                {
                    super::super::anthropic_search::finalize(&mut block.item, &self.stop_reason);
                    finished_searches.push(json!({"type":"response.output_item.done","output_index":block.output,"item":block.item}));
                }
                for event in finished_searches {
                    out.push(self.event(event))
                }
                let incomplete = matches!(
                    self.stop_reason.as_str(),
                    "max_tokens" | "model_context_window_exceeded" | "refusal"
                );
                self.response["status"] = json!(if incomplete {
                    "incomplete"
                } else {
                    "completed"
                });
                self.response["output"] = json!(self
                    .blocks
                    .values()
                    .filter(|v| !v.item.is_null())
                    .map(|v| v.item.clone())
                    .collect::<Vec<_>>());
                self.response["usage"] = usage(&self.token_usage);
                if incomplete {
                    self.response["incomplete_details"] = json!({"reason":if self.stop_reason=="refusal"{"content_filter"}else{"max_output_tokens"}});
                }
                if self.stop_reason == "pause_turn" {
                    self.response["end_turn"] = json!(false)
                }
                if let Some((cache, scope)) = &self.search_cache {
                    cache.record(Some(scope), &self.response);
                }
                out.push(self.event(json!({"type":if incomplete {"response.incomplete"}else{"response.completed"},"response":self.response})));
            }
            _ => return Err(upstream("Unsupported Anthropic stream event")),
        }
        Ok(out)
    }
    fn close(&mut self, index: u64) -> Result<Vec<Value>, ServiceError> {
        let block = self
            .blocks
            .get_mut(&index)
            .ok_or_else(|| upstream("Unknown Anthropic block completion"))?;
        if block.done {
            return Err(upstream("Duplicate Anthropic block completion"));
        }
        if block.item.is_null() {
            block.done = true;
            return Ok(vec![]);
        }
        if block.item["type"] == "web_search_call" {
            if !block.arguments.is_empty() {
                let input: Value = serde_json::from_str(&block.arguments)
                    .map_err(|_| upstream("Anthropic streamed invalid search arguments"))?;
                block.item["action"]["query"] = input["query"].clone();
            }
            block.done = true;
            return Ok(vec![]);
        }
        if block.block["type"] == "tool_use" && !block.arguments.is_empty() {
            block.block["input"] = serde_json::from_str(&block.arguments)
                .map_err(|_| upstream("Anthropic streamed invalid tool arguments"))?;
        }
        let mut item = output_item(&block.block, &self.context)?;
        item["id"] = block.item["id"].clone();
        let output = block.output;
        block.done = true;
        block.item = item.clone();
        let mut events = vec![];
        match item["type"].as_str().unwrap_or("") {
            "message" => {
                events.push(json!({"type":"response.output_text.done","output_index":output,"item_id":item["id"],"content_index":0,"text":item["content"][0]["text"]}));
                events.push(json!({"type":"response.content_part.done","output_index":output,"item_id":item["id"],"content_index":0,"part":item["content"][0]}));
            }
            "reasoning" if item["summary"].as_array().is_some_and(|s| !s.is_empty()) => {
                events.push(json!({"type":"response.reasoning_summary_text.done","output_index":output,"item_id":item["id"],"summary_index":0,"text":item["summary"][0]["text"]}));
                events.push(json!({"type":"response.reasoning_summary_part.done","output_index":output,"item_id":item["id"],"summary_index":0,"part":item["summary"][0]}));
            }
            "custom_tool_call" => {
                let delta = item["input"]
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix(&block.custom_input)
                    .ok_or_else(|| {
                        upstream("Anthropic changed an already streamed custom tool input")
                    })?;
                if !delta.is_empty() {
                    events.push(json!({"type":"response.custom_tool_call_input.delta","output_index":output,"item_id":item["id"],"delta":delta}));
                }
                events.push(json!({"type":"response.custom_tool_call_input.done","output_index":output,"item_id":item["id"],"input":item["input"]}));
            }
            "function_call" => {
                events.push(json!({"type":"response.function_call_arguments.done","output_index":output,"item_id":item["id"],"arguments":item["arguments"]}));
            }
            _ => {}
        }
        events.push(json!({"type":"response.output_item.done","output_index":output,"item":item}));
        Ok(events.into_iter().map(|v| self.event(v)).collect())
    }
    pub(crate) fn finish(&mut self) -> Result<Vec<Value>, ServiceError> {
        if self.finished {
            Ok(vec![])
        } else {
            Err(upstream(
                "Anthropic stream disconnected before message_stop",
            ))
        }
    }
}
