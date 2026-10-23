//! Request-local client tool discovery over ordinary public Responses functions.
//! Native function/custom/namespace tools and their saved arguments stay native.
use super::{client_tool_discovery as discovery, contract::ServiceError};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_ARGUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CALLS: usize = 4096;

#[derive(Clone, Default, Debug)]
pub(super) struct Context {
    wire_name: Option<String>,
    original_tools: Option<Value>,
    original_choice: Option<Value>,
    calls: HashMap<String, String>,
    aliases: HashMap<String, String>,
    buffered: usize,
    delivered: Vec<Value>,
    delivered_bytes: usize,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_client_tool_discovery", message).with_error_details(
        json!({"code":"invalid_client_tool_discovery","message":message,"param":"tools"}),
    )
}

fn upstream(message: &str) -> ServiceError {
    ServiceError::new(502, "invalid_client_tool_search_arguments", message)
}

/// The public Responses default is hosted search. Only an explicit local
/// execution declaration authorizes this gateway's client-function adapter.
pub(super) fn client_declaration(tool: &Value) -> bool {
    tool["type"] == "tool_search" && tool["execution"] == "client"
}

pub(super) fn has_client_declaration(body: &Value) -> bool {
    if body
        .get("tools")
        .is_some_and(|tools| tools.as_array().is_none_or(Vec::is_empty))
    {
        return false;
    }
    body.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(client_declaration))
        || body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|input| {
                input
                    .iter()
                    .filter(|item| item["type"] == "additional_tools")
                    .any(|item| {
                        item.get("tools")
                            .and_then(Value::as_array)
                            .is_some_and(|tools| tools.iter().any(client_declaration))
                    })
            })
}

fn names(value: &Value, used: &mut HashSet<String>, depth: usize) -> Result<(), ServiceError> {
    if depth > 16 {
        return Err(invalid("Tool namespaces exceed their nesting limit"));
    }
    if let Some(tools) = value.get("tools").and_then(Value::as_array) {
        for tool in tools {
            if let Some(name) = text(tool, "name") {
                used.insert(name.into());
            }
            names(tool, used, depth + 1)?;
        }
    }
    if let Some(items) = value.get("input").and_then(Value::as_array) {
        for item in items {
            if let Some(name) = text(item, "name") {
                used.insert(name.into());
            }
        }
    }
    Ok(())
}

pub(super) fn prepare(body: &mut Value) -> Result<Context, ServiceError> {
    let mut context = Context {
        original_tools: body.get("tools").cloned(),
        original_choice: body.get("tool_choice").cloned(),
        ..Default::default()
    };
    let promoted = discovery::promote(body)?;
    // A discovered declaration is untrusted input too. Never promote a hosted
    // capability merely because it arrived in a tool result instead of tools.
    super::chatgpt_plan_wire::validate_tools_at(body.get("tools"), "tools", 0)?;
    if promoted.active() {
        let mut used = HashSet::new();
        names(body, &mut used, 0)?;
        let base = "cockpit_client_tool_search";
        let mut wire = base.to_string();
        let mut suffix = 0;
        while used.contains(&wire) {
            suffix += 1;
            wire = format!("{base}_{suffix}");
        }
        context.wire_name = Some(wire.clone());
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            let mut lowered = Vec::new();
            let mut search_declared = false;
            for tool in tools.iter() {
                if client_declaration(tool) {
                    if !search_declared {
                        lowered.push(discovery::lower_declaration(tool, &wire)?);
                        search_declared = true;
                    }
                } else {
                    lowered.push(tool.clone());
                }
            }
            *tools = lowered;
        }
        if let Some(choice) = body.get_mut("tool_choice") {
            rewrite_choice(choice, &wire);
        }
        discovery::lower_history(body, &wire, &promoted)?;
    } else {
        discovery::lower_history(body, "", &promoted)?;
    }
    if let Some(input) = body.get("input").and_then(Value::as_array) {
        for (index, item) in input.iter().enumerate() {
            if matches!(
                item["type"].as_str(),
                Some("tool_search_call" | "tool_search_output")
            ) {
                // An unmarked historical search with no declared client search
                // cannot safely be reinterpreted as locally executed discovery.
                return Err(super::chatgpt_plan_wire::unsupported_tool(
                    item,
                    &format!("input[{index}]"),
                ));
            }
        }
    }
    group_qualified_tools(body)?;
    Ok(context)
}

/// A discovered function may carry namespace as an identity field. Public
/// declaration schemas express that identity through a namespace container.
/// Re-container only those declarations; argument/format schemas stay intact.
fn group_qualified_tools(body: &mut Value) -> Result<(), ServiceError> {
    fn emit(
        tool: Value,
        parent: Option<&str>,
        template: Option<&Value>,
        grouped: &mut Vec<Value>,
    ) -> Result<(), ServiceError> {
        if tool["type"] == "namespace" {
            let short = text(&tool, "name").ok_or_else(|| invalid("Namespace requires a name"))?;
            let name = parent
                .map(|parent| format!("{parent}.{short}"))
                .unwrap_or_else(|| short.to_string());
            let children = tool["tools"]
                .as_array()
                .ok_or_else(|| invalid("Namespace requires a tools array"))?;
            for child in children {
                emit(child.clone(), Some(&name), Some(&tool), grouped)?;
            }
            return Ok(());
        }
        let mut tool = tool;
        let namespace = text(&tool, "namespace")
            .filter(|name| !name.is_empty())
            .or(parent)
            .map(str::to_owned);
        if matches!(text(&tool, "type"), Some("function" | "custom")) {
            if let Some(namespace) = namespace {
                tool.as_object_mut().unwrap().remove("namespace");
                let position = grouped.iter().position(|existing| {
                    existing["type"] == "namespace" && existing["name"] == namespace
                });
                let index = if let Some(index) = position {
                    index
                } else {
                    let mut group = template.cloned().unwrap_or_else(
                        || json!({"type":"namespace","description":"Client tools"}),
                    );
                    group["name"] = json!(namespace);
                    group["tools"] = json!([]);
                    grouped.push(group);
                    grouped.len() - 1
                };
                let children = grouped[index]["tools"].as_array_mut().unwrap();
                if children.iter().any(|existing| {
                    existing["type"] == tool["type"] && existing["name"] == tool["name"]
                }) {
                    return Err(invalid("Duplicate client tool declaration in namespace"));
                }
                children.push(tool);
                return Ok(());
            }
        }
        grouped.push(tool);
        Ok(())
    }
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let mut grouped = Vec::new();
    for tool in std::mem::take(tools) {
        emit(tool, None, None, &mut grouped)?;
    }
    *tools = grouped;
    Ok(())
}

fn rewrite_choice(choice: &mut Value, wire: &str) {
    if text(choice, "type") == Some("tool_search") && text(choice, "execution") != Some("server") {
        choice["type"] = json!("function");
        choice["name"] = json!(wire);
        if let Some(object) = choice.as_object_mut() {
            object.remove("execution");
        }
    }
    if let Some(tools) = choice.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            rewrite_choice(tool, wire);
        }
    }
}

impl Context {
    fn search(&self, item: &Value) -> bool {
        item["type"] == "function_call"
            && self
                .wire_name
                .as_deref()
                .is_some_and(|wire| text(item, "name") == Some(wire))
            && text(item, "namespace").is_none_or(str::is_empty)
    }

    fn remember(&mut self, event: &Value, item: &Value) -> Result<Option<String>, ServiceError> {
        if !self.search(item) {
            return Ok(None);
        }
        let mut keys = Vec::new();
        if let Some(id) = text(item, "id") {
            keys.push(format!("id:{id}"));
            keys.push(format!("id:{}", discovery::client_item_id(id)));
        }
        if let Some(id) = text(item, "call_id") {
            keys.push(format!("call:{id}"));
        }
        if let Some(index) = event["output_index"].as_u64() {
            keys.push(format!("index:{index}"));
        }
        let Some(key) = keys.first().cloned() else {
            return Err(upstream(
                "Client tool search response is missing its call identity",
            ));
        };
        if !self.calls.contains_key(&key) && self.calls.len() >= MAX_CALLS {
            return Err(upstream("Too many client tool search calls"));
        }
        self.calls.entry(key.clone()).or_default();
        for alias in keys {
            self.aliases.insert(alias, key.clone());
        }
        Ok(Some(key))
    }

    fn event_key(&self, event: &Value) -> Option<String> {
        let keys = [
            text(event, "item_id").map(|id| format!("id:{id}")),
            text(event, "call_id").map(|id| format!("call:{id}")),
            event["output_index"]
                .as_u64()
                .map(|index| format!("index:{index}")),
        ];
        keys.into_iter()
            .flatten()
            .find_map(|key| self.aliases.get(&key).cloned())
    }

    fn restore_item(
        &self,
        item: &mut Value,
        partial: bool,
        cached: Option<&str>,
    ) -> Result<(), ServiceError> {
        if !self.search(item) {
            super::agent_communication::declare_plaintext_message(item);
            return Ok(());
        }
        let remembered = text(item, "id")
            .map(|id| format!("id:{id}"))
            .or_else(|| text(item, "call_id").map(|id| format!("call:{id}")))
            .and_then(|key| self.aliases.get(&key))
            .and_then(|key| self.calls.get(key));
        let raw = text(item, "arguments")
            .filter(|raw| !raw.is_empty())
            .or(cached)
            .or_else(|| remembered.map(String::as_str))
            .unwrap_or("");
        let arguments = discovery::decode_arguments(raw, partial)?;
        item["type"] = json!("tool_search_call");
        item["execution"] = json!("client");
        item["arguments"] = arguments;
        if let Some(id) = text(item, "id").map(str::to_owned) {
            item["id"] = json!(discovery::client_item_id(&id));
        }
        let object = item.as_object_mut().unwrap();
        object.remove("name");
        object.remove("namespace");
        Ok(())
    }

    fn record_delivered(&mut self, item: &Value) -> Result<Value, ServiceError> {
        let call_id = text(item, "call_id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| upstream("Client tool search response is missing its call identity"))?;
        let identity = json!({"call_id":call_id,"id":item.get("id"),"execution":item["execution"],"arguments":item["arguments"]});
        if let Some(previous) = self.delivered.iter().find(|previous| {
            previous["call_id"] == call_id
                || (!identity["id"].is_null() && previous["id"] == identity["id"])
        }) {
            if previous != &identity {
                return Err(upstream("Conflicting client tool search result"));
            }
        } else {
            self.delivered_bytes = self
                .delivered_bytes
                .saturating_add(identity.to_string().len());
            if self.delivered.len() >= MAX_CALLS || self.delivered_bytes > MAX_ARGUMENT_BYTES {
                return Err(upstream(
                    "Client tool search results exceed the stream limit",
                ));
            }
            self.delivered.push(identity.clone());
        }
        Ok(identity)
    }

    fn restore_response(
        &mut self,
        response: &mut Value,
        terminal: bool,
    ) -> Result<(), ServiceError> {
        // OutputCollector reconstructs missing/empty terminal snapshots from
        // complete done items in this attempt. Let it retain its existing
        // identity-evidence checks; only a populated snapshot can contradict
        // an already delivered search call here.
        let omitted_output = response
            .get("output")
            .is_none_or(|output| output.is_null() || output.as_array().is_some_and(Vec::is_empty));
        let mut completed = Vec::new();
        if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
            for item in items {
                let search = self.search(item);
                self.restore_item(item, false, None)?;
                if search && terminal {
                    completed.push(self.record_delivered(item)?);
                }
            }
        }
        if terminal
            && !omitted_output
            && self
                .delivered
                .iter()
                .any(|delivered| !completed.contains(delivered))
        {
            return Err(upstream(&format!(
                "Completed response omitted a delivered client tool search result (output_items={}, restored_searches={}, delivered_searches={})",
                response.get("output").and_then(Value::as_array).map_or(0, Vec::len),
                completed.len(),
                self.delivered.len(),
            )));
        }
        if response.get("tools").is_some() {
            if let Some(tools) = &self.original_tools {
                response["tools"] = tools.clone();
            } else if self.wire_name.is_some() {
                response.as_object_mut().unwrap().remove("tools");
            }
        }
        if response.get("tool_choice").is_some() {
            if let Some(choice) = &self.original_choice {
                response["tool_choice"] = choice.clone();
            }
        }
        Ok(())
    }
}

/// Run before identity/checkpoint observation and before all client protocol
/// adapters so persisted history keeps the caller's original tool identities.
pub(super) fn restore_event(
    context: &mut Context,
    mut event: Value,
) -> Result<Vec<Value>, ServiceError> {
    let kind = text(&event, "type").unwrap_or("").to_string();
    if matches!(
        kind.as_str(),
        "response.output_item.added" | "response.output_item.done"
    ) {
        let item = event.get("item").cloned().unwrap_or(Value::Null);
        let key = context.remember(&event, &item)?;
        let cached = key
            .as_ref()
            .and_then(|key| context.calls.get(key))
            .map(String::as_str);
        if let Some(item) = event.get_mut("item") {
            context.restore_item(item, kind.ends_with(".added"), cached)?;
            if key.is_some() && kind.ends_with(".done") {
                context.record_delivered(item)?;
            }
        }
    }
    if matches!(
        kind.as_str(),
        "response.function_call_arguments.delta" | "response.function_call_arguments.done"
    ) {
        if let Some(key) = context.event_key(&event) {
            if kind.ends_with(".delta") {
                let delta = text(&event, "delta")
                    .ok_or_else(|| upstream("Client tool search argument delta must be text"))?;
                context.buffered = context.buffered.saturating_add(delta.len());
                if context.buffered > MAX_ARGUMENT_BYTES {
                    return Err(upstream(
                        "Client tool search arguments exceed the stream limit",
                    ));
                }
                context.calls.get_mut(&key).unwrap().push_str(delta);
            } else if let Some(arguments) = text(&event, "arguments").filter(|raw| !raw.is_empty())
            {
                context.buffered = context
                    .buffered
                    .saturating_add(arguments.len().saturating_sub(context.calls[&key].len()));
                if context.buffered > MAX_ARGUMENT_BYTES {
                    return Err(upstream(
                        "Client tool search arguments exceed the stream limit",
                    ));
                }
                discovery::decode_arguments(arguments, false)?;
                context.calls.insert(key, arguments.into());
            }
            // The caller dispatches search from the restored completed item.
            // Private function argument events must never escape the gateway.
            return Ok(Vec::new());
        }
    }
    if let Some(response) = event.get_mut("response") {
        context.restore_response(response, kind == "response.completed")?;
    }
    if event.get("output").is_some() {
        context.restore_response(&mut event, kind.is_empty())?;
    }
    Ok(vec![event])
}

#[cfg(test)]
#[path = "chatgpt_plan_tools_tests.rs"]
mod tests;
