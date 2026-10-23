//! Grok's public Responses API accepts JSON-schema function tools. Adapt only
//! OpenAI namespace/custom declarations and client tool search; native function
//! tools, hosted tools and encrypted reasoning stay intact. No hidden history
//! or reasoning cache exists.
//!
//! Contracts: docs.x.ai/developers/tools/function-calling,
//! docs.x.ai/developers/tools/tool-usage-details and
//! docs.x.ai/developers/model-capabilities/text/reasoning (2026-10-13).
use super::contract::ServiceError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[path = "grok_compact.rs"]
pub(super) mod compaction;

#[path = "grok_dispatcher.rs"]
mod dispatcher;
#[path = "grok_encrypted.rs"]
mod encrypted;

pub(super) fn known_signature_provider(raw: &str, gpt: fn(&str) -> bool) -> bool {
    encrypted::known_signature_provider(raw, gpt)
}
#[path = "grok_history.rs"]
mod history;
#[path = "grok_request_compat.rs"]
mod request_compat;
#[path = "grok_response_compat.rs"]
mod response_compat;
#[path = "grok_tool_search.rs"]
mod tool_search;

const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
const MAX_BINDINGS: usize = 4096;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ToolKey {
    namespace: Option<String>,
    name: String,
    custom: bool,
    search: bool,
}
#[derive(Clone, Debug)]
struct ToolBinding {
    key: ToolKey,
    wire_name: String,
}
#[derive(Clone, Debug, Default)]
struct Call {
    binding: Option<ToolBinding>,
    arguments: String,
    emitted: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Context {
    by_key: HashMap<ToolKey, String>,
    by_wire: HashMap<String, ToolBinding>,
    reserved: HashSet<String>,
    calls: HashMap<String, Call>,
    call_aliases: HashMap<String, String>,
    original_tools: Option<Value>,
    original_choice: Option<Value>,
    buffered: usize,
    dispatchers: dispatcher::State,
}
fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_grok_tool", message)
}
fn upstream(message: &str) -> ServiceError {
    ServiceError::new(502, "invalid_grok_tool_arguments", message)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn namespace(value: &Value) -> Option<String> {
    text(value, "namespace")
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Context {
    fn bind(&mut self, key: ToolKey) -> Result<String, ServiceError> {
        if let Some(name) = self.by_key.get(&key) {
            return Ok(name.clone());
        }
        if self.by_key.len() >= MAX_BINDINGS {
            return Err(invalid("Too many Grok tool identities"));
        }
        let identity = if key.search {
            json!([key.namespace, key.name, "client_tool_search"])
        } else {
            json!([key.namespace, key.name, key.custom])
        };
        let encoded =
            serde_json::to_vec(&identity).map_err(|_| invalid("Invalid tool identity"))?;
        let digest = format!("{:x}", Sha256::digest(encoded));
        let base = format!("grok_tool_{}", &digest[..32]);
        let mut wire = base.clone();
        let mut suffix = 0;
        while self.reserved.contains(&wire) {
            suffix += 1;
            wire = format!("{base}_{suffix}");
        }
        self.reserved.insert(wire.clone());
        self.by_key.insert(key.clone(), wire.clone());
        self.by_wire.insert(
            wire.clone(),
            ToolBinding {
                key,
                wire_name: wire.clone(),
            },
        );
        Ok(wire)
    }
    fn resolve(&self, value: &Value) -> Option<ToolBinding> {
        let name = text(value, "name")?;
        if let Some(binding) = self.by_wire.get(name) {
            return Some(binding.clone());
        }
        let namespace = namespace(value);
        // A choice may use type=function for a custom tool exposed via a function
        // schema. Prefer an exact native-function identity when both exist.
        let kinds = if matches!(text(value, "type"), Some("custom" | "custom_tool_call")) {
            [true, false]
        } else {
            [false, true]
        };
        for custom in kinds {
            let key = ToolKey {
                namespace: namespace.clone(),
                name: name.into(),
                custom,
                search: false,
            };
            if let Some(wire) = self.by_key.get(&key) {
                return self.by_wire.get(wire).cloned();
            }
        }
        None
    }
    fn account(&mut self, bytes: usize) -> Result<(), ServiceError> {
        self.buffered = self.buffered.saturating_add(bytes);
        if self.buffered > MAX_STATE_BYTES {
            return Err(upstream("Grok tool arguments exceed the stream limit"));
        }
        Ok(())
    }
}

/// Pure request adaptation. Call this only after selecting a Grok route and
/// before sending any bytes upstream. Context belongs to this one request.
pub fn prepare(body: Value) -> Result<(Value, Context), ServiceError> {
    prepare_inner(body, false)
}

fn prepare_inner(mut body: Value, compact: bool) -> Result<(Value, Context), ServiceError> {
    if !body.is_object() {
        return Err(invalid("Grok request must be an object"));
    }
    let mut context = Context {
        original_tools: body.get("tools").cloned(),
        original_choice: body.get("tool_choice").cloned(),
        ..Default::default()
    };
    collect_names(&body, &mut context.reserved, 0)?;
    tool_search::prepare(&mut body, &mut context)?;
    history::prepare(&mut body)?;
    encrypted::sanitize(&mut body);
    history::compact_prefix(&mut body);
    history::merge_summaries(&mut body);
    request_compat::normalize_search_aliases(&mut body);
    if let Some(tools) = body.get_mut("tools") {
        let original = tools
            .as_array()
            .ok_or_else(|| invalid("Tools must be an array"))?;
        *tools = Value::Array(flatten_tools(original, None, &mut context, 0)?);
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        // Register every call before visiting outputs: tool results may be
        // interleaved, or arrive before another call in a parallel batch.
        let mut converted_custom_ids = HashSet::new();
        for item in input.iter_mut() {
            let custom = text(item, "type") == Some("custom_tool_call");
            if !custom && text(item, "type") != Some("function_call") {
                continue;
            }
            let Some(name) = text(item, "name").map(str::to_owned) else {
                return Err(invalid("Tool call is missing its name"));
            };
            // These prefixes identify xAI-hosted calls, whose encrypted state
            // is already native. A declared client custom tool wins on a name
            // collision; unknown hosted calls must remain byte-for-byte values.
            if custom && hosted_call(item) && context.resolve(item).is_none() {
                continue;
            }
            let ns = namespace(item);
            if custom || ns.is_some() || (name == "web_search" && context.resolve(item).is_some()) {
                let wire = context.bind(ToolKey {
                    namespace: ns,
                    name,
                    custom,
                    search: false,
                })?;
                item["name"] = json!(wire);
                item.as_object_mut().unwrap().remove("namespace");
            }
            if custom {
                let id = text(item, "call_id")
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("Custom tool call requires call_id"))?
                    .to_owned();
                let raw = text(item, "input")
                    .ok_or_else(|| invalid("Custom tool input must be a string"))?
                    .to_owned();
                converted_custom_ids.insert(id);
                item["type"] = json!("function_call");
                item["arguments"] = json!(json!({"input":raw}).to_string());
                item.as_object_mut().unwrap().remove("input");
            }
        }
        for item in input.iter_mut() {
            if text(item, "type") == Some("custom_tool_call_output") {
                let id = text(item, "call_id")
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("Custom tool output requires call_id"))?;
                if hosted_call(item) && !converted_custom_ids.contains(id) {
                    continue;
                }
                // Output-only continuation is also valid with previous_response_id.
                item["type"] = json!("function_call_output");
            }
        }
    }
    if let Some(choice) = body.get_mut("tool_choice") {
        rewrite_choice(choice, &context)?;
    }
    normalize_allowed_tools(&mut body)?;
    request_compat::normalize_choices(&mut body);
    if compact {
        // History lowering needs the original tool declarations. Only then
        // strip them, including declarations promoted from additional_tools.
        compaction::prepare(&mut body);
    } else {
        dispatcher::prepare(&mut body, &mut context)?;
    }
    if !compact
        && body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() > 200)
    {
        return Err(invalid(
            "Grok supports at most 200 active tools; narrow the tool selection",
        ));
    }
    Ok((body, context))
}

pub(super) fn prepare_compact(body: Value) -> Result<(Value, Context), ServiceError> {
    prepare_inner(body, true)
}

fn hosted_call(item: &Value) -> bool {
    text(item, "call_id").is_some_and(|id| id.starts_with("xs_") || id.starts_with("as_"))
}

fn collect_names(
    value: &Value,
    names: &mut HashSet<String>,
    depth: usize,
) -> Result<(), ServiceError> {
    if depth > 32 {
        return Err(invalid("Tool nesting exceeds its limit"));
    }
    if let Some(items) = value.get("tools").and_then(Value::as_array) {
        for tool in items {
            if let Some(name) = text(tool, "name") {
                names.insert(name.into());
            }
            collect_names(tool, names, depth + 1)?;
        }
    }
    if let Some(items) = value.get("input").and_then(Value::as_array) {
        for item in items {
            if matches!(
                text(item, "type"),
                Some("function_call" | "custom_tool_call")
            ) {
                if let Some(name) = text(item, "name") {
                    names.insert(name.into());
                }
            }
        }
    }
    Ok(())
}
fn flatten_tools(
    tools: &[Value],
    parent: Option<&str>,
    context: &mut Context,
    depth: usize,
) -> Result<Vec<Value>, ServiceError> {
    if depth > 16 {
        return Err(invalid("Tool namespaces exceed their nesting limit"));
    }
    let mut result = vec![];
    let mut search_declared = false;
    for original in tools {
        let kind = text(original, "type").unwrap_or("");
        if kind == "namespace" {
            let name = text(original, "name")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("Namespace requires a name"))?;
            let full = parent
                .map(|parent| format!("{parent}.{name}"))
                .unwrap_or_else(|| name.into());
            let nested = original
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Namespace requires a tools array"))?;
            result.extend(flatten_tools(nested, Some(&full), context, depth + 1)?);
            continue;
        }
        let mut tool = original.clone();
        if tool_search::client_declaration(original) {
            if search_declared {
                continue;
            }
            let mut lowered = tool_search::lower_declaration(original, context)?;
            request_compat::normalize_function(&mut lowered, "tool_search", None)?;
            result.push(lowered);
            search_declared = true;
            continue;
        }
        let ns = namespace(original).or_else(|| parent.map(str::to_owned));
        if kind == "function" {
            request_compat::normalize_function(
                &mut tool,
                text(original, "name").unwrap_or(""),
                ns.as_deref(),
            )?;
        }
        if kind == "web_search" {
            request_compat::normalize_web_search(&mut tool);
        }
        let hosted = matches!(kind, "web_search" | "image_generation" | "x_search");
        if hosted {
            tool.as_object_mut().unwrap().remove("namespace");
        }
        if kind == "custom"
            || (!hosted && (parent.is_some() || namespace(original).is_some()))
            || (kind == "function" && text(original, "name") == Some("web_search"))
        {
            if !matches!(kind, "function" | "custom") {
                return Err(invalid(
                    "Namespaced Grok tools must be function or custom tools",
                ));
            }
            let name = text(original, "name")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("Tool requires a name"))?;
            let wire = context.bind(ToolKey {
                namespace: ns,
                name: name.into(),
                custom: kind == "custom",
                search: false,
            })?;
            tool["name"] = json!(wire);
            tool.as_object_mut().unwrap().remove("namespace");
        }
        if kind == "custom" {
            tool["type"] = json!("function");
            tool["parameters"] = json!({"type":"object","properties":{"input":{"type":"string","description":"Exact, unmodified text passed to the client tool."}},"required":["input"],"additionalProperties":false});
            let mut description = text(original, "description").unwrap_or("").to_owned();
            if let Some(format) = tool.as_object_mut().unwrap().remove("format") {
                description.push_str("\nThe input string must follow this tool format: ");
                description.push_str(&format.to_string());
            }
            if !description.is_empty() {
                tool["description"] = json!(description);
            }
        }
        result.push(tool);
    }
    Ok(result)
}
fn rewrite_choice(choice: &mut Value, context: &Context) -> Result<(), ServiceError> {
    tool_search::rewrite_choice(choice, context);
    if let Some(binding) = context.resolve(choice) {
        if choice.get("type").is_some() {
            choice["type"] = json!("function");
        }
        choice["name"] = json!(binding.wire_name);
        choice.as_object_mut().unwrap().remove("namespace");
    }
    if let Some(function) = choice.get_mut("function") {
        rewrite_choice(function, context)?;
    }
    if let Some(tools) = choice.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            rewrite_choice(tool, context)?;
        }
    }
    Ok(())
}

fn normalize_allowed_tools(body: &mut Value) -> Result<(), ServiceError> {
    let Some(choice) = body.get("tool_choice") else {
        return Ok(());
    };
    if text(choice, "type") != Some("allowed_tools") {
        return Ok(());
    }
    let mode = text(choice, "mode").unwrap_or("auto");
    if !matches!(mode, "auto" | "required") {
        return Err(invalid("Invalid allowed-tools mode"));
    }
    let selected = choice
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Allowed tools must be an array"))?
        .clone();
    let mode = mode.to_owned();
    let tools = body
        .get_mut("tools")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("Allowed-tools selection needs tool declarations"))?;
    let matches = |tool: &Value, allowed: &Value| {
        text(tool, "type") == text(allowed, "type")
            && match text(allowed, "name")
                .or_else(|| allowed.pointer("/function/name").and_then(Value::as_str))
            {
                Some(name) => text(tool, "name") == Some(name),
                None => true,
            }
    };
    if selected
        .iter()
        .any(|allowed| !tools.iter().any(|tool| matches(tool, allowed)))
    {
        return Err(invalid(
            "Allowed-tools selection refers to an undeclared tool",
        ));
    }
    tools.retain(|tool| selected.iter().any(|allowed| matches(tool, allowed)));
    if tools.is_empty() && mode == "required" {
        return Err(invalid("Required allowed-tools selection is empty"));
    }
    body["tool_choice"] = json!(mode);
    Ok(())
}

fn decode_input(arguments: &str) -> Result<String, ServiceError> {
    let value: Value = serde_json::from_str(arguments)
        .map_err(|_| upstream("Grok returned invalid custom tool arguments"))?;
    text(&value, "input")
        .map(str::to_owned)
        .ok_or_else(|| upstream("Grok custom tool arguments must contain an input string"))
}
fn restore_item(
    context: &Context,
    item: &mut Value,
    allow_partial: bool,
) -> Result<Option<ToolBinding>, ServiceError> {
    if text(item, "type") != Some("function_call") {
        return Ok(None);
    }
    let Some(binding) = context.resolve(item) else {
        return Ok(None);
    };
    if binding.key.search {
        tool_search::restore_item(item, allow_partial)?;
        return Ok(Some(binding));
    }
    item["name"] = json!(binding.key.name);
    match &binding.key.namespace {
        Some(ns) => item["namespace"] = json!(ns),
        None => {
            item.as_object_mut().unwrap().remove("namespace");
        }
    }
    // Grok's ordinary arguments are plaintext. Preserve explicit encrypted
    // state, but mark restored collaboration calls for the official client.
    if !binding.key.custom && !allow_partial {
        if binding.key.namespace.as_deref() == Some("collaboration") {
            history::normalize_arguments(item);
        }
        super::agent_communication::declare_plaintext_message(item);
    }
    if binding.key.custom {
        let arguments = text(item, "arguments").unwrap_or("");
        let input = if arguments.is_empty() && allow_partial {
            String::new()
        } else {
            decode_input(arguments)?
        };
        item["type"] = json!("custom_tool_call");
        item["input"] = json!(input);
        item.as_object_mut().unwrap().remove("arguments");
    }
    Ok(Some(binding))
}
fn call_key(event: &Value) -> Option<String> {
    text(event, "item_id")
        .map(|id| format!("id:{id}"))
        .or_else(|| {
            event
                .get("output_index")
                .and_then(Value::as_u64)
                .map(|index| format!("index:{index}"))
        })
}
fn remember_call(
    context: &mut Context,
    event: &Value,
    binding: ToolBinding,
) -> Result<(), ServiceError> {
    let mut keys = vec![];
    if let Some(id) = event.pointer("/item/id").and_then(Value::as_str) {
        keys.push(format!("id:{id}"));
        if binding.key.search {
            keys.push(format!("id:{}", tool_search::wire_item_id(id)));
        }
    }
    if let Some(id) = event.pointer("/item/call_id").and_then(Value::as_str) {
        keys.push(format!("call:{id}"));
    }
    if let Some(index) = event.get("output_index").and_then(Value::as_u64) {
        keys.push(format!("index:{index}"));
    }
    let Some(canonical) = keys.first().cloned() else {
        return Ok(());
    };
    if context.calls.len() + 1 > MAX_BINDINGS
        || context.call_aliases.len() + keys.len() > MAX_BINDINGS * 2
    {
        return Err(upstream("Too many streamed Grok tool calls"));
    }
    for key in keys {
        context.call_aliases.insert(key, canonical.clone());
    }
    context.calls.entry(canonical).or_default().binding = Some(binding);
    Ok(())
}

fn restore_response(context: &Context, response: &mut Value) -> Result<(), ServiceError> {
    if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) {
        for item in output {
            restore_item(context, item, false)?;
        }
    }
    if response.get("tools").is_some() {
        if let Some(tools) = &context.original_tools {
            response["tools"] = tools.clone();
        }
    }
    if response.get("tool_choice").is_some() {
        if let Some(choice) = &context.original_choice {
            response["tool_choice"] = choice.clone();
        }
    }
    Ok(())
}

/// Restore only tools declared by this request. Hosted X Search custom calls
/// and all unknown events/fields retain their original representation.
/// A custom JSON wrapper is buffered until complete; then one raw-input delta
/// and its done event are emitted. Identity/ownership observation must happen
/// after this hook so future calls refer to the original caller-visible tools.
pub fn restore_event(context: &mut Context, event: Value) -> Result<Vec<Value>, ServiceError> {
    let events = dispatcher::restore(context, event)?;
    let mut restored = Vec::new();
    for event in events {
        restored.extend(restore_event_inner(context, event)?);
    }
    Ok(restored)
}

fn restore_event_inner(
    context: &mut Context,
    mut event: Value,
) -> Result<Vec<Value>, ServiceError> {
    super::grok_errors::normalize_event(&mut event);
    let reasoning_text_done = response_compat::normalize(&mut event);
    let kind = text(&event, "type").unwrap_or("").to_owned();
    if kind == "response.output_item.done" {
        tool_search::complete_item(context, &mut event)?;
    }
    if matches!(
        kind.as_str(),
        "response.output_item.added" | "response.output_item.done"
    ) {
        let binding = if let Some(item) = event.get_mut("item") {
            restore_item(context, item, kind == "response.output_item.added")?
        } else {
            None
        };
        if let Some(binding) = binding {
            remember_call(context, &event, binding)?;
        }
    }
    if matches!(
        kind.as_str(),
        "response.function_call_arguments.delta" | "response.function_call_arguments.done"
    ) {
        if kind.ends_with(".done") {
            let binding = context.resolve(&event).or_else(|| {
                tool_search::stream_key(context, &event)
                    .and_then(|key| context.calls.get(&key))
                    .and_then(|call| call.binding.clone())
            });
            if binding.is_some_and(|binding| {
                !binding.key.custom && binding.key.namespace.as_deref() == Some("collaboration")
            }) {
                history::normalize_arguments(&mut event);
            }
        }
        if let Some(binding) = context
            .resolve(&event)
            .filter(|binding| !binding.key.custom && !binding.key.search)
        {
            event["name"] = json!(binding.key.name);
            if let Some(namespace) = binding.key.namespace {
                event["namespace"] = json!(namespace);
            } else {
                event.as_object_mut().unwrap().remove("namespace");
            }
        }
        if let Some(key) = tool_search::stream_key(context, &event) {
            if tool_search::arguments_event(context, &key, &event)? {
                return Ok(vec![]);
            }
            let custom = context
                .calls
                .get(&key)
                .and_then(|call| call.binding.as_ref())
                .is_some_and(|binding| binding.key.custom);
            if custom {
                if kind.ends_with(".delta") {
                    let delta = text(&event, "delta")
                        .ok_or_else(|| upstream("Grok tool argument delta must be text"))?;
                    context.account(delta.len())?;
                    context
                        .calls
                        .get_mut(&key)
                        .unwrap()
                        .arguments
                        .push_str(delta);
                    return Ok(vec![]);
                }
                let call = context.calls.get_mut(&key).unwrap();
                let raw = text(&event, "arguments").unwrap_or(&call.arguments);
                let input = decode_input(raw)?;
                let mut delta = event.clone();
                delta["type"] = json!("response.custom_tool_call_input.delta");
                delta["delta"] = json!(input);
                let object = delta
                    .as_object_mut()
                    .ok_or_else(|| upstream("Grok tool event must be an object"))?;
                object.remove("arguments");
                object.remove("sequence_number");
                event["type"] = json!("response.custom_tool_call_input.done");
                event["input"] = json!(input);
                event.as_object_mut().unwrap().remove("arguments");
                if call.emitted {
                    return Ok(vec![event]);
                }
                call.emitted = true;
                return Ok(vec![delta, event]);
            }
        }
    }
    if let Some(response) = event.get_mut("response") {
        restore_response(context, response)?;
    }
    if event.get("output").is_some() {
        restore_response(context, &mut event)?;
    }
    Ok(match reasoning_text_done {
        Some(text_done) => vec![text_done, event],
        None => vec![event],
    })
}

#[cfg(test)]
#[path = "grok_wire_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn test_cipher() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD_NO_PAD.encode((0u8..96).collect::<Vec<_>>())
}
