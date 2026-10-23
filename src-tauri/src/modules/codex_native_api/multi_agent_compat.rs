//! Codex multi-agent compatibility, ported from CPA v8.0.32 (911f49c).
//! Policy is supplied by the host: model compatibility, optimization and orphan
//! delegation are independent gates. No function in this module decrypts data.
use axum::http::HeaderMap;
use serde_json::{json, Value};
use std::collections::HashMap;

#[path = "multi_agent_compat_integer.rs"]
mod integer;
#[path = "grok_integer_schema.rs"]
pub(crate) mod integer_fields;
#[path = "multi_agent_compat_models.rs"]
mod models;
#[path = "multi_agent_compat_plaintext.rs"]
mod plaintext;
pub(super) use integer::normalize_request as normalize_integer_tools;
pub(super) use models::format_models;
pub use models::ModelProfile;

const COLLABORATION: &str = "collaboration";
const OPTIMIZED: &str = "collaboration-optimize";

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub optimize: bool,
    pub is_compat: bool,
    pub orphan_delegation: bool,
    /// Equivalent to CPA's Responses-boundary tools-prepared request context.
    pub tools_prepared: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub optimized_namespace: bool,
    plaintext_enabled: bool,
    plaintext: plaintext::Policy,
}

impl Context {
    pub fn is_active(&self) -> bool {
        self.optimized_namespace || (self.plaintext_enabled && self.plaintext.can_mark())
    }
    /// Restore before protocol translation, plaintext declaration or recording
    /// client-visible history. Arguments and tool result payloads are opaque.
    pub fn restore(&self, value: &mut Value) {
        if self.optimized_namespace {
            restore_value(value);
        }
        if self.plaintext_enabled {
            self.plaintext.mark(value, false);
        }
    }

    /// Refresh after the final wire adapter has moved or replaced declarations.
    /// No input text, argument content or credentials are retained.
    pub fn capture_final(&mut self, body: &Value) {
        self.plaintext = plaintext::Policy::for_request(body, self.optimized_namespace);
    }

    /// Only use after the caller proves a continuation on the same authenticated
    /// upstream socket. Top-level replacements and malformed additions revoke
    /// inherited evidence according to the Go sidecar's policy.
    pub fn inherit_plaintext(&mut self, previous: &Self) {
        if self.plaintext_enabled && previous.plaintext_enabled {
            self.plaintext.inherit(&previous.plaintext);
        }
    }

    /// Identity observation occurs before namespace restoration on some paths.
    pub fn mark_wire_calls(&self, value: &mut Value) {
        if self.plaintext_enabled {
            self.plaintext.mark(value, true);
        }
    }
}

pub(super) fn is_codex_client(user_agent: &str) -> bool {
    let user_agent = user_agent.trim();
    user_agent.starts_with("Codex Desktop/")
        || user_agent.starts_with("codex-tui/")
        || user_agent == "codex_cli_rs"
        || user_agent.starts_with("codex_cli_rs/")
        || user_agent.starts_with("codex_exec/")
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim()
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn message_tool(tool: &Value) -> bool {
    field(tool, "type").trim() == "function"
        && matches!(
            field(tool, "name").trim(),
            "spawn_agent" | "send_message" | "followup_task"
        )
}

/// Executor order matches OptimizeCodexMultiAgentV2RequestForAuth: orphan
/// delegation, optional optimization, then model-specific input compatibility.
/// `headers` must be the original caller headers, before upstream UA rewriting.
pub(super) fn prepare(
    headers: &HeaderMap,
    body: &mut Value,
    options: Options,
    profiles: &[ModelProfile],
) -> Context {
    rewrite_orphans(headers, body, options.orphan_delegation);
    let enabled = options.optimize && is_codex_client(header(headers, "user-agent"));
    let mut context = Context {
        plaintext_enabled: enabled,
        ..Default::default()
    };
    if enabled {
        rewrite_agent_content(body);
        let conflict = has_namespace_conflict(body);
        let markdown = if options.tools_prepared || conflict {
            String::new()
        } else {
            format_models(profiles)
        };
        visit_tool_roots(body, &mut |tools| prepare_tools(tools, &markdown));
        if !conflict {
            visit_tool_roots(body, &mut |tools| {
                context.optimized_namespace |= optimize_namespaces(tools);
            });
        }
    }
    if options.is_compat {
        rewrite_input(body, true, true);
    }
    context.capture_final(body);
    context
}

/// CPA's pre-translation path for non-Codex target protocols. The caller only
/// invokes this for Responses input translated to a non-Responses target.
pub(super) fn prepare_translated_input(headers: &HeaderMap, body: &mut Value, options: Options) {
    rewrite_orphans(headers, body, options.orphan_delegation);
    let enabled =
        options.is_compat || (options.optimize && is_codex_client(header(headers, "user-agent")));
    if enabled {
        rewrite_input(body, true, options.is_compat);
    }
}

/// Ordinary provider gateways used this projection before the Rust migration,
/// including native Responses targets. Keep it distinct from Codex's private
/// agent_message envelope and from the public ChatGPT plan adapter.
pub(super) fn prepare_portable_gateway_input(
    headers: &HeaderMap,
    body: &mut Value,
    options: Options,
) -> Result<(), super::contract::ServiceError> {
    if !options.is_compat
        && !(options.optimize && is_codex_client(header(headers, "user-agent")))
    {
        return Ok(());
    }
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in items {
            if field(item, "type").trim() == "agent_message" {
                super::agent_communication::portable_message(item).map_err(|message| {
                    super::contract::ServiceError::new(400, "invalid_protocol_history", message)
                })?;
            }
        }
    }
    Ok(())
}

/// SIWC is a public Responses boundary. Its adapter can explicitly request
/// standard message input independently of the optional CPA optimization gate.
pub(super) fn portable_input(body: &mut Value) {
    rewrite_input(body, true, true);
}

/// Request-boundary preparation leaves namespaces intact. Reusing the returned
/// flag as Options.tools_prepared avoids replacing the selected model catalog a
/// second time when the eventual executor applies its account policy.
pub(super) fn prepare_boundary_tools(
    headers: &HeaderMap,
    body: &mut Value,
    enabled: bool,
    profiles: &[ModelProfile],
) -> bool {
    if !enabled || !is_codex_client(header(headers, "user-agent")) {
        return false;
    }
    let markdown = if has_namespace_conflict(body) {
        String::new()
    } else {
        format_models(profiles)
    };
    visit_tool_roots(body, &mut |tools| prepare_tools(tools, &markdown));
    true
}

/// Full Responses-boundary stage, before identity inspection or translation.
/// Orphan compatibility uses its own flag/header even if optimization is off.
pub(super) fn prepare_boundary(
    headers: &HeaderMap,
    body: &mut Value,
    options: Options,
    profiles: &[ModelProfile],
) -> bool {
    rewrite_orphans(headers, body, options.orphan_delegation);
    prepare_boundary_tools(headers, body, options.optimize, profiles)
}

pub(super) fn prepare_ingress(
    headers: &HeaderMap,
    body: &mut Value,
    options: Options,
    profiles: &[ModelProfile],
) -> bool {
    prepare_boundary(headers, body, options, profiles)
}

fn visit_tool_roots(body: &mut Value, visitor: &mut impl FnMut(&mut Vec<Value>)) {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        visitor(tools);
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if field(item, "type").trim() == "additional_tools" {
                if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                    visitor(tools);
                }
            }
        }
    }
}

fn prepare_tools(tools: &mut [Value], model_list: &str) {
    for tool in tools {
        if message_tool(tool) {
            if let Some(message) = tool
                .pointer_mut("/parameters/properties/message")
                .and_then(Value::as_object_mut)
            {
                message.remove("encrypted");
            }
            if field(tool, "name").trim() == "spawn_agent" && !model_list.is_empty() {
                if let Some(description) = tool.get("description").and_then(Value::as_str) {
                    let description = models::replace_model_sections(description, model_list);
                    tool["description"] = Value::String(description);
                }
            }
        }
        if field(tool, "type").trim() == "namespace" {
            if let Some(nested) = tool.get_mut("tools").and_then(Value::as_array_mut) {
                prepare_tools(nested, model_list);
            }
        }
    }
}

fn tools_conflict(tools: &Value) -> bool {
    tools.as_array().is_some_and(|tools| {
        tools.iter().any(|tool| {
            let name = field(tool, "name").trim();
            name == OPTIMIZED
                || name.starts_with("collaboration-optimize__")
                || name.starts_with("collaboration-optimize.")
                || (field(tool, "type").trim() == "namespace"
                    && tool.get("tools").is_some_and(tools_conflict))
        })
    })
}

pub(super) fn has_namespace_conflict(body: &Value) -> bool {
    body.get("tools").is_some_and(tools_conflict)
        || body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|input| {
                input.iter().any(|item| {
                    field(item, "type").trim() == "additional_tools"
                        && item.get("tools").is_some_and(tools_conflict)
                })
            })
}

fn optimize_namespaces(tools: &mut [Value]) -> bool {
    let mut optimized = false;
    for tool in tools {
        if field(tool, "type").trim() != "namespace" {
            continue;
        }
        let collaboration = field(tool, "name").trim() == COLLABORATION;
        if let Some(children) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            let has_spawn = children.iter().any(|child| {
                field(child, "type").trim() == "function"
                    && field(child, "name").trim() == "spawn_agent"
            });
            optimized |= optimize_namespaces(children);
            if collaboration && has_spawn {
                tool["name"] = json!(OPTIMIZED);
                optimized = true;
            }
        }
    }
    optimized
}

fn rewrite_agent_content(body: &mut Value) {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for item in input {
        if field(item, "type").trim() != "agent_message" {
            continue;
        }
        let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            if field(part, "type").trim() == "encrypted_content" {
                if let Some(text) = part.get("encrypted_content").and_then(Value::as_str) {
                    let text = text.to_owned();
                    part["type"] = json!("input_text");
                    part["text"] = Value::String(text);
                    part.as_object_mut().unwrap().remove("encrypted_content");
                }
            }
        }
    }
}

pub(super) fn rewrite_input(body: &mut Value, enabled: bool, compat: bool) {
    if enabled {
        rewrite_agent_content(body);
    }
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for item in input {
        if enabled && field(item, "type").trim() == "agent_message" {
            item["type"] = json!("message");
            item["role"] = json!("user");
        }
        if compat {
            if let Some(item) = item.as_object_mut() {
                for key in [
                    "author",
                    "recipient",
                    "internal_chat_message_metadata_passthrough",
                ] {
                    item.remove(key);
                }
            }
        }
    }
}

fn rewrite_orphans(headers: &HeaderMap, body: &mut Value, enabled: bool) {
    if !enabled || !header(headers, "x-openai-subagent").eq_ignore_ascii_case("collab_spawn") {
        return;
    }
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    // CPA counts all calls in the request, including calls appearing later.
    // Preserve exact call IDs: trimming only decides whether an ID is empty.
    let mut available: HashMap<String, usize> = HashMap::new();
    for item in input.iter() {
        let id = field(item, "call_id");
        if field(item, "type") == "function_call" && !id.trim().is_empty() {
            *available.entry(id.to_owned()).or_default() += 1;
        }
    }
    for item in input {
        if field(item, "type") != "function_call_output" {
            continue;
        }
        if let Some(count) = available.get_mut(field(item, "call_id")) {
            if *count > 0 {
                *count -= 1;
                continue;
            }
        }
        if field(item, "namespace") != "codex_app" {
            continue;
        }
        let label = match field(item, "name") {
            "create_thread" => "codex_app__create_thread",
            "send_message_to_thread" => "codex_app__send_message_to_thread",
            _ => continue,
        };
        let output = item
            .get("output")
            .map(|output| {
                output
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| output.to_string())
            })
            .unwrap_or_default();
        *item = json!({"type":"message","role":"user","content":[{
            "type":"input_text","text":format!("Tool output from {label}:\n{output}")
        }]});
    }
}

fn restore_value(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(restore_value),
        Value::Object(object) => {
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            let call = matches!(kind.as_str(), "function_call" | "custom_tool_call");
            if call && object.get("namespace").and_then(Value::as_str) == Some(OPTIMIZED) {
                object.insert("namespace".into(), json!(COLLABORATION));
            }
            if let Some(name) = object
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                if kind == "namespace" && name == OPTIMIZED {
                    object.insert("name".into(), json!(COLLABORATION));
                } else if call {
                    if let Some(name) = name
                        .strip_prefix("collaboration-optimize.")
                        .filter(|name| !name.is_empty())
                    {
                        object.insert("namespace".into(), json!(COLLABORATION));
                        object.insert("name".into(), json!(name));
                    } else if let Some(name) = name.strip_prefix("collaboration-optimize__") {
                        object.insert("name".into(), json!(format!("collaboration__{name}")));
                    }
                }
            }
            for (key, child) in object {
                if key == "arguments"
                    || key == "input"
                    || (key == "output"
                        && matches!(
                            kind.as_str(),
                            "function_call_output" | "custom_tool_call_output"
                        ))
                {
                    continue;
                }
                restore_value(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "multi_agent_compat_tests.rs"]
mod tests;
