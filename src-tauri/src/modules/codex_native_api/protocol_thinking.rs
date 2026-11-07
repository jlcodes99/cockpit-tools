//! Go SDK summary visibility and numeric thinking compatibility.
use serde_json::{json, Value};

pub(super) fn budget(value: i64) -> Option<&'static str> {
    Some(match value {
        -1 => "auto",
        0 => "none",
        1..=512 => "minimal",
        513..=1024 => "low",
        1025..=8192 => "medium",
        8193..=24576 => "high",
        24577.. => "xhigh",
        _ => return None,
    })
}

fn boolean(body: &Value, paths: &[&str]) -> Option<Option<String>> {
    paths.iter().find_map(|path| {
        body.pointer(path)
            .and_then(Value::as_bool)
            .map(|enabled| enabled.then(|| "auto".into()))
    })
}

fn summary(body: &Value, paths: &[&str]) -> Option<Option<String>> {
    paths.iter().find_map(|path| match body.pointer(path) {
        Some(Value::Null) => Some(None),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(None),
            "auto" | "concise" | "detailed" => Some(Some(value.trim().to_ascii_lowercase())),
            _ => None,
        },
        _ => None,
    })
}

pub(super) fn apply(body: &Value, target: &mut Value, source: &str) {
    if target.pointer("/reasoning/effort").is_none() {
        if !target["reasoning"].is_object() {
            target["reasoning"] = json!({});
        }
        target["reasoning"]["effort"] = json!("medium");
    }
    let intent = match source {
        "chat" => boolean(
            body,
            &[
                "/extra_body/google/thinking_config/include_thoughts",
                "/extra_body/google/thinking_config/includeThoughts",
                "/extra_body/google/thinkingConfig/include_thoughts",
                "/extra_body/google/thinkingConfig/includeThoughts",
                "/extra_body/extra_body/google/thinking_config/include_thoughts",
                "/extra_body/extra_body/google/thinking_config/includeThoughts",
                "/google/thinking_config/include_thoughts",
                "/google/thinking_config/includeThoughts",
                "/thinking/includeThoughts",
                "/thinking/include_thoughts",
                "/reasoning/includeThoughts",
                "/reasoning/include_thoughts",
                "/generationConfig/thinkingConfig/includeThoughts",
                "/generationConfig/thinkingConfig/include_thoughts",
                "/generation_config/thinking_config/include_thoughts",
                "/generation_config/thinking_config/includeThoughts",
            ],
        )
        .or_else(|| summary(body, &["/reasoning/summary", "/reasoning/generate_summary"]))
        .or_else(|| {
            body.pointer("/reasoning/exclude")
                .and_then(Value::as_bool)
                .map(|exclude| (!exclude).then(|| "auto".into()))
        })
        .or_else(|| boolean(body, &["/include_reasoning", "/reasoning/enabled"]))
        .or_else(|| {
            body["reasoning_effort"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|effort| (!effort.trim().eq_ignore_ascii_case("none")).then(|| "auto".into()))
        }),
        "claude" => {
            let active = match body.pointer("/thinking/type").and_then(Value::as_str) {
                Some(kind) if kind.trim().eq_ignore_ascii_case("adaptive") => true,
                Some(kind) if kind.trim().eq_ignore_ascii_case("enabled") => body
                    .pointer("/thinking/budget_tokens")
                    .and_then(Value::as_i64)
                    .is_none_or(|budget| budget == -1 || budget > 0),
                _ => false,
            };
            if active {
                body.pointer("/thinking/display")
                    .and_then(Value::as_str)
                    .and_then(
                        |display| match display.trim().to_ascii_lowercase().as_str() {
                            "summarized" => Some(Some("auto".into())),
                            "omitted" => Some(None),
                            _ => None,
                        },
                    )
            } else {
                None
            }
        }
        "gemini" => boolean(
            body,
            &[
                "/generationConfig/thinkingConfig/includeThoughts",
                "/generationConfig/thinkingConfig/include_thoughts",
                "/generation_config/thinking_config/include_thoughts",
                "/generation_config/thinking_config/includeThoughts",
            ],
        ),
        _ => summary(body, &["/reasoning/summary", "/reasoning/generate_summary"]),
    };
    let Some(intent) = intent else { return };
    if let Some(ref detail) = intent {
        if !target["reasoning"].is_object() {
            target["reasoning"] = json!({});
        }
        target["reasoning"]["summary"] = json!(detail);
    }
    if let Some(reasoning) = target.get_mut("reasoning").and_then(Value::as_object_mut) {
        if source == "chat" {
            for alias in ["exclude", "enabled", "includeThoughts", "include_thoughts"] {
                reasoning.remove(alias);
            }
        }
        if intent.is_none() {
            reasoning.remove("summary");
        }
        reasoning.remove("generate_summary");
        if reasoning.is_empty() {
            target.as_object_mut().unwrap().remove("reasoning");
        }
    }
}
