//! Compatibility fixes require an explicit pre-execution rejection, never a guess.
use super::{
    chat,
    contract::{Route, ServiceError},
};
use serde_json::{json, Value};

pub(super) fn ingress(mut body: Value) -> Result<Value, ServiceError> {
    let native = body.get("input").is_some_and(|input| !input.is_null());
    if !native
        && body["messages"]
            .as_array()
            .is_some_and(|messages| !messages.is_empty())
    {
        // Preserve only the native fields the Chat adapter consumes. Do not
        // clone the entire conversation just to restore a few declarations.
        let template = if body.get("prompt").is_some_and(|prompt| !prompt.is_string()) {
            body.as_object_mut().unwrap().remove("prompt")
        } else {
            None
        };
        let stream_options = body.as_object_mut().unwrap().remove("stream_options");
        let native_choice = body
            .get("tool_choice")
            .filter(|choice| {
                matches!(choice["type"].as_str(), Some("function" | "custom"))
                    && choice.get("function").is_none()
                    && choice.get("custom").is_none()
            })
            .cloned();
        let mut native_tools = Vec::new();
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            for (index, tool) in tools.iter_mut().enumerate() {
                if tool["type"] == "function" && tool.get("function").is_none() {
                    native_tools.push((index, tool.clone()));
                    let mut function = tool.as_object().unwrap().clone();
                    function.remove("type");
                    *tool = json!({"type":"function","function":function});
                }
            }
        }
        body = chat::prepare_responses_ingress(body)?.responses;
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            for (index, tool) in native_tools {
                tools[index] = tool;
            }
        }
        if let Some(template) = template {
            body["prompt"] = template;
        }
        if let Some(choice) = native_choice {
            body["tool_choice"] = choice;
        }
        if let Some(mut options) = stream_options {
            if let Some(object) = options.as_object_mut() {
                object.remove("include_usage");
            }
            if !options.as_object().is_some_and(|object| object.is_empty()) {
                body["stream_options"] = options;
            }
        }
        body.as_object_mut().unwrap().remove("previous_response_id");
    }
    if let Some(object) = body.as_object_mut() {
        object.remove("messages");
        object.remove("commands");
        if object.get("prompt").is_some_and(Value::is_string) {
            let prompt = object.remove("prompt").unwrap();
            if object.get("input").is_none_or(Value::is_null) {
                object.insert("input".into(), prompt);
            }
        }
    }
    Ok(body)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Repair {
    Reasoning(super::encrypted_reasoning::Repair),
    Remove(String),
    Status(String),
    NullContent(usize),
    Schema(String),
}
impl Repair {
    fn apply(&self, body: &mut Value) -> bool {
        match self {
            Self::Reasoning(repair) => repair.apply(body),
            Self::Remove(pointer) => {
                let Some((parent, key)) = pointer.rsplit_once('/') else {
                    return false;
                };
                body.pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .is_some_and(|object| object.remove(key).is_some())
            }
            Self::Status(kind) => {
                let mut changed = false;
                for item in body["input"].as_array_mut().into_iter().flatten() {
                    if item["type"].as_str() == Some(kind) {
                        changed |= item
                            .as_object_mut()
                            .is_some_and(|object| object.remove("status").is_some());
                    }
                }
                changed
            }
            Self::NullContent(index) => {
                let Some(item) = body
                    .get_mut("input")
                    .and_then(Value::as_array_mut)
                    .and_then(|input| input.get_mut(*index))
                else {
                    return false;
                };
                if item.get("content") != Some(&Value::Null) {
                    return false;
                }
                if item["type"] == "reasoning" {
                    item.as_object_mut().unwrap().remove("content");
                } else if item["type"] == "message" || item["role"].is_string() {
                    item["content"] = json!("");
                } else {
                    return false;
                }
                true
            }
            Self::Schema(pointer) => {
                let Some(schema) = body.pointer_mut(pointer).and_then(Value::as_object_mut) else {
                    return false;
                };
                if schema.get("type").is_some_and(|v| !v.is_null()) {
                    return false;
                }
                schema.insert("type".into(), json!("object"));
                true
            }
        }
    }
}
fn parameter(error: &Value) -> Option<String> {
    let object = super::diagnostics::error_object(error);
    if let Some(param) = object["param"].as_str().filter(|v| !v.is_empty()) {
        return Some(param.to_ascii_lowercase());
    }
    let message = object["message"].as_str()?.to_ascii_lowercase();
    let regex = regex::Regex::new(r#"(?:unknown|unsupported) parameter\s*[:=]?\s*["']?([a-z_]+(?:\[\d+\])?(?:\.[a-z_]+(?:\[\d+\])?)*)"#).ok()?;
    regex.captures(&message).map(|c| c[1].to_owned())
}
fn pointer(param: &str) -> Option<String> {
    let regex = regex::Regex::new(r"^[a-z_]+(?:\[\d+\])?(?:\.[a-z_]+(?:\[\d+\])?)*$").ok()?;
    regex.is_match(param).then(|| {
        format!(
            "/{}",
            param.replace('[', "/").replace(']', "").replace('.', "/")
        )
    })
}
fn repair(body: &Value, error: &Value) -> Option<Repair> {
    let error = super::diagnostics::error_object(error);
    let code = error["code"].as_str().unwrap_or_default();
    let message = error["message"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let param = parameter(error)?;
    let pointer = pointer(&param)?;
    let explicit = matches!(code, "unknown_parameter" | "unsupported_parameter")
        || message.contains("unknown parameter")
        || message.contains("unsupported parameter");
    if explicit && matches!(param.as_str(), "max_output_tokens" | "truncation") {
        return Some(Repair::Remove(pointer));
    }
    if code == "invalid_function_parameters"
        && message.contains("type")
        && message.contains("none")
        && pointer.starts_with("/tools/")
        && pointer.ends_with("/parameters")
    {
        return Some(Repair::Schema(pointer));
    }
    if param.ends_with("prompt_cache_breakpoint")
        && (code == "invalid_parameter" || message.contains("not supported on this model"))
    {
        return Some(Repair::Remove(pointer));
    }
    let rest = param.strip_prefix("input[")?;
    let (index, field) = rest.split_once("].")?;
    let index = index.parse::<usize>().ok()?;
    let item = body["input"].as_array()?.get(index)?;
    match field {
        "status" if explicit => Some(
            item["type"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map_or(Repair::Remove(pointer), |kind| {
                    Repair::Status(kind.to_owned())
                }),
        ),
        "namespace"
            if explicit
                && matches!(
                    item["type"].as_str(),
                    Some(
                        "function_call"
                            | "function_call_output"
                            | "custom_tool_call"
                            | "custom_tool_call_output"
                    )
                ) =>
        {
            Some(Repair::Remove(pointer))
        }
        "content"
            if matches!(code, "invalid_type" | "invalid_request_error" | "")
                && message.contains("null")
                && message.contains(&param) =>
        {
            Some(Repair::NullContent(index))
        }
        "content"
            if code == "array_above_max_length"
                && message.contains("maximum length 0")
                && message.contains(&param)
                && item["type"] == "reasoning" =>
        {
            Some(Repair::Remove(pointer))
        }
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct Budget {
    attempts: u8,
    reasoning_repaired: bool,
    last_reasoning: bool,
    repairs: Vec<(String, String, Repair)>,
}
impl Budget {
    pub fn reason(&self) -> &'static str {
        if self.last_reasoning { "encrypted_reasoning_repaired" } else { "unsupported_field_repaired" }
    }
    pub fn commit_reasoning(&self, prepared: &mut super::mapping::Prepared) {
        if !self.last_reasoning { return; }
        let Some((_,_,Repair::Reasoning(repair))) = self.repairs.last() else { return; };
        let mut replay = prepared.replay_body().clone();
        repair.apply(&mut replay);
        if let Some(plan) = &mut prepared.rebuild_plan { repair.apply(&mut plan.body); }
        prepared.bridge_body = Some(replay);
        prepared.binding.repair_rejected_reasoning(repair);
        // Keep the wire template unchanged: earlier indexed field repairs must
        // run before this removal on every send, including subsequent repairs.
        let mut outbound = prepared.body.clone();
        self.apply(&prepared.route, &mut outbound);
        prepared.binding.retain_transmitted_input_blobs(&outbound);
    }
    pub fn apply(&self, route: &Route, body: &mut Value) {
        for (account, model, repair) in &self.repairs {
            if account == &route.account_id && model == &route.model {
                repair.apply(body);
            }
        }
    }
    pub fn rejected_for(&mut self, prepared: &super::mapping::Prepared, body: &Value, status: u16, event: &Value) -> bool {
        let allow_reasoning = !prepared.compact && prepared.raw_chat.is_none()
            && prepared.image_route_request.is_none() && !prepared.source_headers.contains_key("conversation_id");
        self.rejected_inner(&prepared.route, body, status, event, allow_reasoning)
    }
    fn rejected_inner(&mut self, route: &Route, body: &Value, status: u16, event: &Value, allow_reasoning: bool) -> bool {
        if status != 400 || self.attempts >= 6 || super::bootstrap::http_execution_observed(event) {
            return false;
        }
        self.last_reasoning = false;
        let reasoning = (allow_reasoning && !self.reasoning_repaired && route.wire_api == super::contract::WireApi::Responses && !route.chatgpt_plan)
            .then(|| super::encrypted_reasoning::Repair::rejected(body, event)).flatten();
        let Some(repair) = reasoning.map(Repair::Reasoning).or_else(||repair(body, event)) else {
            return false;
        };
        let key = (route.account_id.clone(), route.model.clone(), repair);
        if self.repairs.contains(&key) || !key.2.apply(&mut body.clone()) {
            return false;
        }
        self.attempts += 1;
        self.last_reasoning = matches!(&key.2, Repair::Reasoning(_));
        self.reasoning_repaired |= self.last_reasoning;
        self.repairs.push(key);
        true
    }
}
