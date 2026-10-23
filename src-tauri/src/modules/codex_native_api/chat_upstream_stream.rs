use super::{responses_usage, status_for, upstream_error, ServiceError, ToolContext};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Default)]
struct Tool {
    output: usize,
    id: String,
    name: String,
    args: String,
    custom: bool,
    wrapped: bool,
    wire_args: String,
    added: bool,
}
pub struct ResponsesStreamAdapter {
    response: Value,
    sequence: u64,
    started: bool,
    finished: bool,
    reason: Option<String>,
    output: Vec<Value>,
    text: Option<usize>,
    refusal: Option<usize>,
    reasoning: Option<usize>,
    tools: BTreeMap<usize, Tool>,
    context: ToolContext,
}
impl ResponsesStreamAdapter {
    pub fn new(model: String) -> Self {
        Self::with_context(model, ToolContext::default())
    }
    pub(crate) fn with_context(model: String, context: ToolContext) -> Self {
        Self {
            response: json!({"id":format!("resp_{}",uuid::Uuid::now_v7()),"object":"response","created_at":chrono::Utc::now().timestamp(),"model":model,"status":"in_progress","error":null,"incomplete_details":null,"output":[],"usage":null}),
            sequence: 0,
            started: false,
            finished: false,
            reason: None,
            output: Vec::new(),
            text: None,
            refusal: None,
            reasoning: None,
            tools: BTreeMap::new(),
            context,
        }
    }
    fn event(&mut self, mut value: Value) -> Value {
        value["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        value
    }
    fn add_text(
        &mut self,
        text: &str,
        kind: &str,
        events: &mut Vec<Value>,
    ) -> Result<(), ServiceError> {
        if text.is_empty() {
            return Ok(());
        }
        let existing = match kind {
            "refusal" => self.refusal,
            "reasoning" => self.reasoning,
            _ => self.text,
        };
        let index = if let Some(i) = existing {
            i
        } else {
            let i = self.output.len();
            let id = format!(
                "{}_{}",
                if kind == "reasoning" { "rs" } else { "msg" },
                uuid::Uuid::now_v7()
            );
            let item = if kind == "reasoning" {
                json!({"id":id,"type":"reasoning","summary":[]})
            } else {
                json!({"id":id,"type":"message","status":"in_progress","role":"assistant","content":[]})
            };
            self.output.push(item.clone());
            events.push(
                self.event(
                    json!({"type":"response.output_item.added","output_index":i,"item":item}),
                ),
            );
            let part = if kind == "refusal" {
                json!({"type":"refusal","refusal":""})
            } else if kind == "reasoning" {
                json!({"type":"summary_text","text":""})
            } else {
                json!({"type":"output_text","text":"","annotations":[],"logprobs":[]})
            };
            if kind == "reasoning" {
                self.reasoning = Some(i);
                self.output[i]["summary"] = json!([part.clone()]);
                events.push(self.event(json!({"type":"response.reasoning_summary_part.added","output_index":i,"item_id":id,"summary_index":0,"part":part})));
            } else {
                if kind == "refusal" {
                    self.refusal = Some(i);
                } else {
                    self.text = Some(i);
                }
                self.output[i]["content"] = json!([part.clone()]);
                events.push(self.event(json!({"type":"response.content_part.added","output_index":i,"item_id":id,"content_index":0,"part":part})));
            }
            i
        };
        let (list, key, event_type) = match kind {
            "refusal" => ("content", "refusal", "response.refusal.delta"),
            "reasoning" => ("summary", "text", "response.reasoning_summary_text.delta"),
            _ => ("content", "text", "response.output_text.delta"),
        };
        let current = self.output[index][list][0][key].as_str().unwrap_or("");
        self.output[index][list][0][key] = json!(format!("{current}{text}"));
        let mut event = json!({"type":event_type,"output_index":index,"item_id":self.output[index]["id"],"delta":text});
        event[if kind == "reasoning" {
            "summary_index"
        } else {
            "content_index"
        }] = json!(0);
        events.push(self.event(event));
        Ok(())
    }
    fn add_tool(&mut self, call: &Value, events: &mut Vec<Value>) -> Result<(), ServiceError> {
        let index = call["index"].as_u64().unwrap_or(0) as usize;
        if !self.tools.contains_key(&index) {
            let output = self.output.len();
            self.output.push(Value::Null);
            self.tools.insert(
                index,
                Tool {
                    output,
                    ..Default::default()
                },
            );
        }
        let custom = call["type"] == "custom" || call.get("custom").is_some();
        let key = if custom { "custom" } else { "function" };
        let name = call[key]["name"].as_str().unwrap_or("");
        let args = call[key][if custom { "input" } else { "arguments" }]
            .as_str()
            .unwrap_or("");
        let tool = self.tools.get_mut(&index).unwrap();
        tool.custom |= custom;
        if let Some(id) = call["id"].as_str().filter(|s| !s.is_empty()) {
            if tool.id.is_empty() {
                tool.id = id.to_string();
            } else if tool.id != id {
                return Err(upstream_error("A streamed tool call changed its ID"));
            }
        }
        tool.name.push_str(name);
        let previous_args = tool.args.len();
        tool.wire_args.push_str(args);
        tool.wrapped = !custom && self.context.custom(&tool.name);
        tool.custom |= tool.wrapped;
        let decoded = if self.context.pending_name(&tool.name) {
            // Arguments may precede the identifying name. Keep their wire bytes
            // buffered until the request declaration decides function vs custom.
            tool.args.clone()
        } else if tool.wrapped {
            super::tools::unwrap_input(&tool.wire_args, true)
        } else {
            tool.wire_args.clone()
        };
        if !decoded.starts_with(&tool.args) {
            return Err(upstream_error(
                "Custom tool input disagrees with its streamed prefix",
            ));
        }
        tool.args = decoded;
        let args = &tool.args[previous_args..];
        let mut raw = Vec::new();
        if !tool.added
            && !tool.id.is_empty()
            && !tool.name.is_empty()
            && !self.context.pending_name(&tool.name)
        {
            let mut item = json!({"id":format!("fc_{}",uuid::Uuid::now_v7()),"type":if tool.custom{"custom_tool_call"}else{"function_call"},"status":"in_progress","call_id":tool.id,"name":tool.name});
            self.context.restore_item(&mut item);
            item[if tool.custom { "input" } else { "arguments" }] = json!("");
            self.output[tool.output] = item.clone();
            tool.added = true;
            raw.push(
                json!({"type":"response.output_item.added","output_index":tool.output,"item":item}),
            );
            if !tool.args.is_empty() {
                raw.push(json!({"type":if tool.custom{"response.custom_tool_call_input.delta"}else{"response.function_call_arguments.delta"},"output_index":tool.output,"item_id":self.output[tool.output]["id"],"delta":tool.args}));
            }
        } else if tool.added && !args.is_empty() {
            raw.push(json!({"type":if tool.custom{"response.custom_tool_call_input.delta"}else{"response.function_call_arguments.delta"},"output_index":tool.output,"item_id":self.output[tool.output]["id"],"delta":args}));
        }
        if tool.added {
            self.output[tool.output]["name"] = json!(self.context.identity(&tool.name).0);
            self.output[tool.output][if tool.custom { "input" } else { "arguments" }] =
                json!(tool.args);
        }
        for event in raw {
            events.push(self.event(event));
        }
        Ok(())
    }
    pub fn push(&mut self, value: &Value) -> Result<Vec<Value>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if let Some(error) = super::super::upstream_errors::chat_failure(value, 502) {
            return Err(error);
        }
        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            if let Some(id) = value["id"].as_str() {
                self.response["id"] = json!(format!("resp_{id}"));
            }
            if let Some(model) = value["model"].as_str() {
                self.response["model"] = json!(model);
            }
            if let Some(created) = value.get("created") {
                self.response["created_at"] = created.clone();
            }
            events.push(
                self.event(json!({"type":"response.created","response":self.response.clone()})),
            );
            events.push(
                self.event(json!({"type":"response.in_progress","response":self.response.clone()})),
            );
        }
        if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
            self.response["usage"] = responses_usage(usage);
        }
        if let Some(tier) = value.get("service_tier") {
            self.response["service_tier"] = tier.clone();
        }
        let choices = value["choices"]
            .as_array()
            .ok_or_else(|| upstream_error("Chat stream frame has no choices array"))?;
        if choices.len() > 1
            || choices
                .iter()
                .any(|c| c["index"].as_u64().unwrap_or(0) != 0)
        {
            return Err(upstream_error(
                "Multiple Chat choices cannot form one Responses output",
            ));
        }
        for choice in choices {
            let delta = &choice["delta"];
            for (key, kind) in [
                ("content", "text"),
                ("refusal", "refusal"),
                ("reasoning_content", "reasoning"),
            ] {
                let value = if key == "reasoning_content"
                    && delta[key].as_str().is_none_or(|s| s.is_empty())
                {
                    delta.get("reasoning").unwrap_or(&delta[key])
                } else {
                    &delta[key]
                };
                if let Some(text) = value.as_str() {
                    self.add_text(text, kind, &mut events)?;
                } else if delta.get(key).is_some_and(|v| !v.is_null()) {
                    return Err(upstream_error("Chat stream text delta must be a string"));
                }
            }
            if let Some(calls) = delta["tool_calls"].as_array() {
                for call in calls {
                    self.add_tool(call, &mut events)?;
                }
            }
            if let Some(call) = delta.get("function_call") {
                let generated = self
                    .tools
                    .get(&0)
                    .map(|t| t.id.clone())
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::now_v7()));
                self.add_tool(
                    &json!({"index":0,"id":generated,"type":"function","function":call}),
                    &mut events,
                )?;
            }
            if let Some(logprobs) = choice
                .pointer("/logprobs/content")
                .and_then(Value::as_array)
            {
                if let Some(index) = self.text {
                    let parts = self.output[index]["content"][0]["logprobs"]
                        .as_array_mut()
                        .unwrap();
                    parts.extend(logprobs.iter().cloned());
                }
                if let Some(event) = events
                    .iter_mut()
                    .rev()
                    .find(|e| e["type"] == "response.output_text.delta")
                {
                    event["logprobs"] = json!(logprobs);
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                if !matches!(
                    reason,
                    "stop" | "length" | "tool_calls" | "content_filter" | "function_call"
                ) {
                    return Err(upstream_error(
                        "Chat stream returned an unknown finish_reason",
                    ));
                }
                self.reason = Some(reason.to_string());
            }
        }
        Ok(events)
    }
    fn complete(&mut self, done_marker: bool) -> Result<Vec<Value>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if !self.started || (!done_marker && self.reason.is_none()) {
            return Err(upstream_error(
                "Chat stream ended before finish_reason or [DONE]",
            ));
        }
        if self.tools.values().any(|t| !t.added) {
            return Err(upstream_error(
                "Chat stream ended before tool call ID and name",
            ));
        }
        let (status, details) = status_for(self.reason.as_deref().unwrap_or("stop"));
        let incomplete = status == "incomplete";
        // Validate every tool before emitting any done event. A finish_reason or
        // [DONE] cannot turn a truncated function argument buffer into a callable
        // tool. Custom input is freeform and must never be JSON-validated.
        for tool in self.tools.values() {
            if incomplete {
                continue;
            }
            if tool.wrapped
                && serde_json::from_str::<Value>(&tool.wire_args).is_err()
                && tool.wire_args.trim_start().starts_with('{')
            {
                return Err(upstream_error(
                    "Chat stream ended with incomplete custom tool JSON wrapper",
                ));
            }
            if tool.args.is_empty() {
                if self.reason.is_none() {
                    return Err(upstream_error(
                        "Chat stream ended before tool arguments or finish_reason",
                    ));
                }
            } else if !tool.custom
                && serde_json::from_str::<serde::de::IgnoredAny>(&tool.args).is_err()
            {
                return Err(upstream_error(
                    "Chat stream ended with incomplete or invalid JSON function arguments",
                ));
            }
        }
        let mut events = Vec::new();
        for index in 0..self.output.len() {
            let mut item = self.output[index].clone();
            match item["type"].as_str().unwrap_or("") {
                "message" => {
                    item["status"] = json!(status);
                    for (part_index, part) in item["content"].as_array().unwrap().iter().enumerate()
                    {
                        let refusal = part["type"] == "refusal";
                        let mut event = json!({"type":if refusal{"response.refusal.done"}else{"response.output_text.done"},"output_index":index,"item_id":item["id"],"content_index":part_index});
                        event[if refusal { "refusal" } else { "text" }] =
                            part[if refusal { "refusal" } else { "text" }].clone();
                        events.push(self.event(event));
                        events.push(self.event(json!({"type":"response.content_part.done","output_index":index,"item_id":item["id"],"content_index":part_index,"part":part})));
                    }
                }
                "reasoning" => {
                    for (part_index, part) in item["summary"].as_array().unwrap().iter().enumerate()
                    {
                        events.push(self.event(json!({"type":"response.reasoning_summary_text.done","output_index":index,"item_id":item["id"],"summary_index":part_index,"text":part["text"]})));
                        events.push(self.event(json!({"type":"response.reasoning_summary_part.done","output_index":index,"item_id":item["id"],"summary_index":part_index,"part":part})));
                    }
                }
                "function_call" | "custom_tool_call" => {
                    item["status"] = json!(status);
                    let custom = item["type"] == "custom_tool_call";
                    if !custom && !incomplete && item["arguments"] == "" {
                        // Explicit normal completion permits a no-argument
                        // function. Never synthesize this for truncated calls.
                        item["arguments"] = json!("{}");
                    }
                    let mut event = json!({"type":if custom{"response.custom_tool_call_input.done"}else{"response.function_call_arguments.done"},"output_index":index,"item_id":item["id"]});
                    event[if custom { "input" } else { "arguments" }] =
                        item[if custom { "input" } else { "arguments" }].clone();
                    events.push(self.event(event));
                }
                _ => {}
            }
            self.output[index] = item.clone();
            events.push(self.event(
                json!({"type":"response.output_item.done","output_index":index,"item":item}),
            ));
        }
        self.response["status"] = json!(status);
        self.response["incomplete_details"] = details;
        self.response["output"] = json!(self.output);
        self.finished = true;
        events.push(self.event(json!({"type":if status=="incomplete"{"response.incomplete"}else{"response.completed"},"response":self.response.clone()})));
        Ok(events)
    }
    pub fn finish(&mut self) -> Result<Vec<Value>, ServiceError> {
        self.complete(false)
    }
    pub fn finish_done(&mut self) -> Result<Vec<Value>, ServiceError> {
        self.complete(true)
    }
}

#[cfg(test)]
#[path = "chat_upstream_stream_tests.rs"]
mod tests;
