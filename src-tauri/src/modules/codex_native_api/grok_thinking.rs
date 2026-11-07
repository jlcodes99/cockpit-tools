//! Model-suffix thinking for the explicitly selected native xAI route.
//! The host must keep both full and base model names in its ACL checks. This
//! helper does not authorize a model, choose an account, or inspect ciphertext.
use super::contract::ServiceError;
use serde_json::{Map, Value};

#[derive(Clone, Copy)]
enum Intent<'a> {
    None,
    Auto,
    Level(&'a str),
    Budget(i64),
}

fn suffix(model: &str) -> (&str, Option<&str>) {
    let model = model.trim();
    let Some(open) = model.rfind('(').filter(|_| model.ends_with(')')) else {
        return (model, None);
    };
    (&model[..open], Some(&model[open + 1..model.len() - 1]))
}

fn intent(raw: &str) -> Option<Intent<'_>> {
    if raw.eq_ignore_ascii_case("none") {
        Some(Intent::None)
    } else if raw.eq_ignore_ascii_case("auto") || raw == "-1" {
        Some(Intent::Auto)
    } else if ["minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .any(|level| raw.eq_ignore_ascii_case(level))
    {
        Some(Intent::Level(raw))
    } else {
        raw.parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .map(|value| {
                if value == 0 {
                    Intent::None
                } else {
                    Intent::Budget(value)
                }
            })
    }
}

// These are the actual native xAI model capabilities, rather than the GPT
// metadata that Codex uses to display a third-party model. Keep this limited
// table aligned with the supported xAI catalog; unknown models retain the Go
// executor's user-defined-model behavior and are validated by the provider.
fn levels(model: &str) -> Option<&'static [&'static str]> {
    match model.rsplit('/').next().unwrap_or(model) {
        "grok-4.3" | "grok-build-0.1" => Some(&["none", "low", "medium", "high"]),
        "grok-4.6" | "grok-4.7" | "grok-4.7-latest" | "grok-4.7-build-fast" => {
            Some(&["low", "medium", "high", "xhigh"])
        }
        "grok-3-mini" | "grok-3-mini-fast" | "grok-4.5" | "grok-4.20-multi-agent-0309" => {
            Some(&["low", "medium", "high"])
        }
        "grok-4.20-0309-non-reasoning" | "grok-4.20-0309-reasoning" | "grok-composer-2.5-fast" => {
            Some(&[])
        }
        _ => None,
    }
}

fn budget_level(value: i64) -> &'static str {
    match value {
        1..=512 => "minimal",
        513..=1024 => "low",
        1025..=8192 => "medium",
        8193..=24576 => "high",
        _ => "xhigh",
    }
}

fn normalize(intent: Intent<'_>, model: &str) -> Option<String> {
    let supported = levels(model);
    if supported.is_some_and(|levels| levels.is_empty()) {
        return None;
    }
    let effort = match intent {
        Intent::None => supported.map_or("none", |levels| levels[0]).to_owned(),
        Intent::Auto => if supported.is_some() {
            "medium"
        } else {
            "auto"
        }
        .to_owned(),
        Intent::Budget(value) => budget_level(value).to_owned(),
        Intent::Level(level) => level.to_ascii_lowercase(),
    };
    let Some(supported) = supported else {
        return Some(effort);
    };
    if supported.contains(&effort.as_str()) {
        return Some(effort);
    }
    // Go's cross-protocol level validation clamps to the nearest supported
    // level, with lower levels winning ties. Numeric suffixes use the same
    // clamp after the 512/1024/8192/24576 budget thresholds.
    let order = ["minimal", "low", "medium", "high", "xhigh", "max"];
    let target = order.iter().position(|level| *level == effort)?;
    supported
        .iter()
        .filter_map(|level| {
            order
                .iter()
                .position(|candidate| candidate == level)
                .map(|index| (index.abs_diff(target), index, *level))
        })
        .min_by_key(|(distance, index, _)| (*distance, *index))
        .map(|(_, _, level)| level.to_owned())
}

/// Parse the last terminal `(value)` exactly as the Go executor does. Unknown,
/// empty, negative, or overflowing values strip the suffix without creating a
/// thinking override. Call only for an explicitly authorized native xAI route.
pub(crate) fn parse_model(model: &str) -> Result<(String, Option<String>), ServiceError> {
    let (base, raw) = suffix(model);
    if raw.is_some() && base.trim().is_empty() {
        return Err(ServiceError::new(
            400,
            "model_required",
            "A model is required before a thinking suffix",
        ));
    }
    Ok((
        base.to_owned(),
        raw.and_then(intent)
            .and_then(|intent| normalize(intent, base)),
    ))
}

fn invalid_effort() -> ServiceError {
    ServiceError::new(
        400,
        "invalid_reasoning_effort",
        "Reasoning effort is not supported by the selected Grok model",
    )
}

fn body_intent(body: &Value, chat: bool) -> Result<Option<Intent<'_>>, ServiceError> {
    let effort = if chat {
        body.get("reasoning_effort")
    } else {
        body.get("reasoning")
            .and_then(|reasoning| reasoning.get("effort"))
    };
    let Some(effort) = effort.filter(|effort| !effort.is_null()) else {
        return Ok(None);
    };
    let raw = effort.as_str().ok_or_else(invalid_effort)?;
    if raw.is_empty() {
        return Ok(None);
    }
    // The native Responses/Chat bodies carry a discrete effort, not a numeric
    // budget. Only a model suffix uses the Go budget threshold conversion.
    Ok(Some(if raw.eq_ignore_ascii_case("none") {
        Intent::None
    } else if raw.eq_ignore_ascii_case("auto") {
        Intent::Auto
    } else {
        Intent::Level(raw)
    }))
}

fn strip_generation_controls(body: &mut Value) -> Result<(), ServiceError> {
    let object = body.as_object_mut().ok_or_else(|| {
        ServiceError::new(400, "invalid_request", "Grok request must be an object")
    })?;
    if object
        .get("reasoning")
        .is_some_and(|reasoning| !reasoning.is_object() && !reasoning.is_null())
    {
        // Never replace an invalid/opaque reasoning value with an empty object.
        return Err(ServiceError::new(
            400,
            "invalid_request",
            "Grok reasoning must be an object",
        ));
    }
    if object
        .get("reasoning_effort")
        .is_some_and(|value| !value.is_string() && !value.is_null())
    {
        return Err(invalid_effort());
    }
    if let Some(reasoning) = object.get("reasoning").and_then(Value::as_object) {
        if reasoning
            .get("effort")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            return Err(invalid_effort());
        }
        for key in ["budget", "budget_tokens"] {
            if reasoning
                .get(key)
                .is_some_and(|value| !value.is_i64() && !value.is_null())
            {
                return Err(ServiceError::new(
                    400,
                    "invalid_reasoning_budget",
                    "Grok reasoning budget must be an integer",
                ));
            }
        }
    }
    object.remove("reasoning_effort");
    if let Some(reasoning) = object.get_mut("reasoning").and_then(Value::as_object_mut) {
        // Go removes the whole reasoning object for non-thinking models. Limit
        // cleanup to generation controls so opaque values and other fields stay
        // available to owner admission and retain their original byte values.
        for key in ["effort", "budget", "budget_tokens"] {
            reasoning.remove(key);
        }
    }
    Ok(())
}

/// Normalize generation effort for the selected native xAI model. A valid model
/// suffix takes priority; without a suffix the existing body effort is validated
/// and clamped using that model's capabilities. Keep all other reasoning fields,
/// history items, encrypted values and tool arguments. Mapping owns model rewrite.
pub(super) fn apply(body: &mut Value, source_model: &str, chat: bool) -> Result<(), ServiceError> {
    let (base, raw) = suffix(source_model);
    let _ = parse_model(source_model)?;
    let selected_model = body.get("model").and_then(Value::as_str).unwrap_or(base);
    let selected_model = suffix(selected_model).0.to_owned();
    if levels(&selected_model).is_some_and(|levels| levels.is_empty()) {
        return strip_generation_controls(body);
    }
    let configuration = match raw {
        // An unknown suffix suppresses the body override in Go ApplyThinking.
        Some(raw) => intent(raw),
        None => body_intent(body, chat)?,
    };
    let Some(configuration) = configuration else {
        return Ok(());
    };
    let effort = normalize(configuration, &selected_model).ok_or_else(invalid_effort)?;
    let object = body.as_object_mut().ok_or_else(|| {
        ServiceError::new(400, "invalid_request", "Grok request must be an object")
    })?;
    if chat {
        object.insert("reasoning_effort".into(), Value::String(effort));
    } else {
        let reasoning = object
            .entry("reasoning")
            .or_insert_with(|| Value::Object(Map::new()));
        if reasoning.is_null() {
            *reasoning = Value::Object(Map::new());
        }
        reasoning
            .as_object_mut()
            .ok_or_else(|| {
                ServiceError::new(400, "invalid_request", "Grok reasoning must be an object")
            })?
            .insert("effort".into(), Value::String(effort));
    }
    Ok(())
}

#[cfg(test)]
#[path = "grok_thinking_tests.rs"]
mod tests;
