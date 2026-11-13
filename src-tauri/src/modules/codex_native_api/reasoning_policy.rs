use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use crate::models::codex_local_access::CodexReasoningLevel as Level;

use super::{
    contract::{Caller, Route, ServiceError, WireApi},
    mapping::Prepared,
    server::AppState,
};

const MAX_PROOFS: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResponseScope {
    response_id: String,
    caller: String,
    owner: String,
    account: String,
    model: String,
    upstream: String,
    session: Option<String>,
    thread: Option<String>,
}

impl ResponseScope {
    fn new(prepared: &Prepared, response_id: &str) -> Option<Self> {
        if !native_updates(&prepared.route)
            || prepared.compact
            || response_id.is_empty()
            || response_id.len() > 256
        {
            return None;
        }
        Some(Self {
            response_id: response_id.into(),
            caller: prepared.caller.id.clone(),
            owner: prepared.route.owner_id.clone(),
            account: prepared.route.continuity_id().into(),
            model: prepared.route.model.clone(),
            upstream: prepared.route.base_url.clone(),
            session: prepared.request.session_id.clone(),
            thread: prepared.request.thread_id.clone(),
        })
    }
}

#[derive(Default)]
pub(super) struct ProofCache(Mutex<ProofEntries>);

#[derive(Default)]
struct ProofEntries {
    levels: HashMap<ResponseScope, Level>,
    order: VecDeque<ResponseScope>,
}

impl ProofCache {
    pub(super) fn parent(&self, prepared: &Prepared, wire: &Value) -> Option<Level> {
        let id = wire["previous_response_id"].as_str()?;
        let scope = ResponseScope::new(prepared, id)?;
        self.0.lock().ok()?.levels.get(&scope).copied()
    }

    pub(super) fn remember(&self, prepared: &Prepared, event: &Value) {
        let Some(effort) = prepared.reasoning_ceiling_proof else {
            return;
        };
        if !matches!(
            super::events::response_terminal_kind(event),
            Some("response.completed" | "response.done")
        ) {
            return;
        }
        let response = event.get("response").unwrap_or(event);
        if response["status"] != "completed"
            || event.get("error").is_some_and(|error| !error.is_null())
            || response.get("error").is_some_and(|error| !error.is_null())
        {
            return;
        }
        let Some(scope) = response["id"]
            .as_str()
            .and_then(|id| ResponseScope::new(prepared, id))
        else {
            return;
        };
        let Ok(mut entries) = self.0.lock() else {
            return;
        };
        if let Some(existing) = entries.levels.get_mut(&scope) {
            *existing = (*existing).max(effort);
            return;
        }
        if entries.order.len() == MAX_PROOFS {
            if let Some(oldest) = entries.order.pop_front() {
                entries.levels.remove(&oldest);
            }
        }
        entries.order.push_back(scope.clone());
        entries.levels.insert(scope, effort);
    }
}

fn native_updates(route: &Route) -> bool {
    route.chatgpt && !route.chatgpt_plan && !super::grok_errors::applies(route)
}

fn unverifiable() -> ServiceError {
    ServiceError::new(
        403,
        "reasoning_effort_unverifiable",
        "This API key requires an explicit, supported reasoning effort. For inherited Responses configuration, include a reasoning configuration_update.",
    )
}

fn exceeds_ceiling() -> ServiceError {
    ServiceError::new(
        403,
        "reasoning_effort_exceeds_key_limit",
        "The effective reasoning effort exceeds this API key's maximum",
    )
}

// The latest update with an effort takes precedence. A malformed explicit
// effort cannot be interpreted as missing and replaced with a lower default.
fn configuration_effort(body: &Value) -> Option<&Value> {
    body.get("input")?
        .as_array()?
        .iter()
        .rev()
        .filter(|item| item["type"] == "configuration_update")
        .find_map(|item| item.pointer("/reasoning/effort"))
}

fn explicit_effort(body: &Value) -> Option<&Value> {
    body.pointer("/reasoning/effort")
        .or_else(|| body.get("reasoning_effort"))
}

fn parse(value: &Value) -> Result<Level, ServiceError> {
    value
        .as_str()
        .and_then(Level::parse)
        .ok_or_else(unverifiable)
}

fn inherited_context(body: &Value) -> bool {
    body["type"] == "response.steer"
        || ["previous_response_id", "conversation", "conversation_id"]
            .iter()
            .any(|name| {
                body.get(name).is_some_and(|value| {
                    !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
                })
            })
}

fn wire_effort(
    body: &Value,
    protocol: WireApi,
    native_updates: bool,
    parent: Option<Level>,
) -> Result<Level, ServiceError> {
    if protocol == WireApi::Anthropic {
        return match body.pointer("/thinking/type").and_then(Value::as_str) {
            Some("disabled") => Ok(Level::None),
            Some("adaptive" | "enabled") => {
                if let Some(effort) = body.pointer("/output_config/effort") {
                    parse(effort)
                } else {
                    match body
                        .pointer("/thinking/budget_tokens")
                        .and_then(Value::as_u64)
                    {
                        Some(1..=512) => Ok(Level::Minimal),
                        Some(513..=1024) => Ok(Level::Low),
                        Some(1025..=8192) => Ok(Level::Medium),
                        Some(8193..=24576) => Ok(Level::High),
                        Some(24577..=32768) => Ok(Level::Xhigh),
                        Some(32769..=128000) => Ok(Level::Max),
                        _ => Err(unverifiable()),
                    }
                }
            }
            _ => Err(unverifiable()),
        };
    }
    if protocol == WireApi::Responses && native_updates {
        if let Some(effort) = configuration_effort(body) {
            return parse(effort);
        }
    }
    if inherited_context(body) {
        let parent = parent.ok_or_else(unverifiable)?;
        return Ok(match body.pointer("/reasoning/effort") {
            Some(effort) => parent.max(parse(effort)?),
            None => parent,
        });
    }
    let effort = match protocol {
        WireApi::Chat => body.get("reasoning_effort"),
        WireApi::Responses => body.pointer("/reasoning/effort"),
        WireApi::Anthropic => unreachable!(),
    };
    if let Some(effort) = effort {
        return parse(effort);
    }
    if protocol == WireApi::Chat
        && body["model"]
            .as_str()
            .is_some_and(crate::modules::codex_protocol::is_deepseek_model)
        && body.pointer("/thinking/type").and_then(Value::as_str) == Some("disabled")
    {
        return Ok(Level::None);
    }
    Err(unverifiable())
}

fn check_level(effort: Level, maximum: Level) -> Result<(), ServiceError> {
    if effort > maximum {
        Err(exceeds_ceiling())
    } else {
        Ok(())
    }
}

fn check_requested(maximum: Level, original: &Value) -> Result<(), ServiceError> {
    let suffix = original
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| super::gpt_thinking::parse_model(model).1);
    if let Some(value) = configuration_effort(original) {
        check_level(parse(value)?, maximum)?;
    } else if let Some(suffix) = suffix {
        check_level(Level::parse(&suffix).ok_or_else(unverifiable)?, maximum)?;
    } else if let Some(value) = explicit_effort(original) {
        check_level(parse(value)?, maximum)?;
    }
    Ok(())
}

#[cfg(test)]
fn validate(
    maximum: Level,
    original: &Value,
    wire: &Value,
    protocol: WireApi,
    native_updates: bool,
) -> Result<(), ServiceError> {
    validate_with_parent(maximum, original, wire, protocol, native_updates, None).map(|_| ())
}

fn validate_with_parent(
    maximum: Level,
    original: &Value,
    wire: &Value,
    protocol: WireApi,
    native_updates: bool,
    parent: Option<Level>,
) -> Result<Level, ServiceError> {
    check_requested(maximum, original)?;
    let mut effort = wire_effort(wire, protocol, native_updates, parent)?;
    if protocol == WireApi::Responses && !native_updates {
        if let Some(update) = configuration_effort(wire) {
            effort = effort.max(parse(update)?);
        }
    }
    check_level(effort, maximum)?;
    if protocol != WireApi::Anthropic {
        if let Some(template) = wire["model"]
            .as_str()
            .and_then(crate::modules::codex_protocol::known_codex_model_template)
        {
            if let Some(levels) = template["supported_reasoning_levels"].as_array() {
                let supported = levels.iter().any(|item| {
                    item.as_str()
                        .or_else(|| item["effort"].as_str())
                        .and_then(Level::parse)
                        == Some(effort)
                });
                if !supported {
                    return Err(unverifiable());
                }
            }
        }
    }
    Ok(effort)
}

pub(super) async fn check_dispatch(
    state: &AppState,
    caller: &Caller,
    route: &Route,
    original: &Value,
    wire: &Value,
    protocol: WireApi,
) -> Result<(), ServiceError> {
    check_dispatch_with_parent(state, caller, route, original, wire, protocol, None)
        .await
        .map(|_| ())
}

pub(super) async fn check_dispatch_with_parent(
    state: &AppState,
    caller: &Caller,
    route: &Route,
    original: &Value,
    wire: &Value,
    protocol: WireApi,
    parent: Option<Level>,
) -> Result<Option<Level>, ServiceError> {
    let maximum = super::execution::within(state, state.host.reasoning_ceiling(caller)).await?;
    if let Some(maximum) = maximum {
        return validate_with_parent(
            maximum,
            original,
            wire,
            protocol,
            native_updates(route),
            parent,
        )
        .map(Some);
    }
    Ok(None)
}

pub(super) async fn check_external_images(
    state: &AppState,
    caller: &Caller,
    original: &Value,
) -> Result<(), ServiceError> {
    if let Some(maximum) =
        super::execution::within(state, state.host.reasoning_ceiling(caller)).await?
    {
        check_requested(maximum, original)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reasoning_policy_checks_both_client_intent_and_final_wire() {
        let low = json!({"reasoning":{"effort":"low"}});
        let high = json!({"reasoning":{"effort":"high"}});
        for (original, wire) in [(&low, &high), (&high, &low)] {
            assert_eq!(
                validate(Level::Medium, original, wire, WireApi::Responses, false)
                    .unwrap_err()
                    .code,
                "reasoning_effort_exceeds_key_limit"
            );
        }
        for wire in [
            json!({}),
            json!({"reasoning":{"effort":"auto"}}),
            json!({"thinking":{"type":"disabled"}}),
            json!({"reasoning":{"effort":"low"},"previous_response_id":"prior"}),
        ] {
            assert_eq!(
                validate(Level::Medium, &low, &wire, WireApi::Responses, false)
                    .unwrap_err()
                    .code,
                "reasoning_effort_unverifiable"
            );
        }
        validate(Level::Medium, &low, &low, WireApi::Responses, false).unwrap();
        validate(
            Level::Medium,
            &low,
            &json!({"reasoning_effort":"medium"}),
            WireApi::Chat,
            false,
        )
        .unwrap();
        let anthropic = json!({"thinking":{"type":"enabled","budget_tokens":1024}});
        validate(Level::Medium, &low, &anthropic, WireApi::Anthropic, false).unwrap();
        let raised = json!({"thinking":{"type":"adaptive"},"output_config":{"effort":"high"}});
        assert_eq!(
            validate(Level::Medium, &low, &raised, WireApi::Anthropic, false)
                .unwrap_err()
                .code,
            "reasoning_effort_exceeds_key_limit"
        );
    }

    #[test]
    fn reasoning_policy_uses_authoritative_updates_for_native_continuations() {
        let wire = json!({
            "reasoning":{"effort":"high"},
            "previous_response_id":"prior",
            "input":[
                {"type":"configuration_update","reasoning":{"effort":"xhigh"}},
                {"type":"configuration_update","reasoning":{"effort":"medium"}}
            ]
        });
        validate(Level::Medium, &wire, &wire, WireApi::Responses, true).unwrap();
        assert_eq!(
            validate(Level::Medium, &wire, &wire, WireApi::Responses, false)
                .unwrap_err()
                .code,
            "reasoning_effort_unverifiable"
        );
        let mut malformed = wire.clone();
        malformed["input"][1]["reasoning"]["effort"] = Value::Null;
        assert_eq!(
            validate(
                Level::Medium,
                &malformed,
                &malformed,
                WireApi::Responses,
                true
            )
            .unwrap_err()
            .code,
            "reasoning_effort_unverifiable"
        );
        let possible_override = json!({
            "reasoning":{"effort":"low"},
            "input":[{"type":"configuration_update","reasoning":{"effort":"medium"}}]
        });
        assert_eq!(
            validate_with_parent(
                Level::Medium,
                &possible_override,
                &possible_override,
                WireApi::Responses,
                false,
                None,
            )
            .unwrap(),
            Level::Medium
        );
    }
}
