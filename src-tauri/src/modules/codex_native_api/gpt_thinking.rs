//! Pure OpenAI model suffix compatibility, including API-key custom routes. Never modifies input
//! history, ciphertext, routing grants, or upstream ownership.
use serde_json::{json, Value};

use super::contract::ServiceError;

pub(crate) fn model_name(model: &str) -> &str {
    match model.rfind('(').filter(|_| model.ends_with(')')) {
        Some(open) => &model[..open],
        None => model,
    }
}

pub(crate) fn parse_model(model: &str) -> (String, Option<String>) {
    let base = model_name(model);
    let suffix = (base.len() != model.len()).then(|| &model[base.len() + 1..model.len() - 1]);
    let effort = suffix.and_then(|raw| {
        let lower = raw.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "none" | "auto" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            return Some(lower);
        }
        let budget = raw.parse::<i64>().ok()?;
        Some(
            match budget {
                -1 => "auto",
                0 => "none",
                1..=512 => "minimal",
                513..=1024 => "low",
                1025..=8192 => "medium",
                8193..=24576 => "high",
                24577.. => "xhigh",
                _ => return None,
            }
            .into(),
        )
    });
    (base.to_owned(), effort)
}

fn nearest(level: &str, supported: &[String]) -> Option<String> {
    let order = ["minimal", "low", "medium", "high", "xhigh", "max"];
    let target = order.iter().position(|value| *value == level)?;
    supported
        .iter()
        .filter_map(|value| {
            order
                .iter()
                .position(|level| value.eq_ignore_ascii_case(level))
                .map(|position| (position.abs_diff(target), position, value.clone()))
        })
        .min_by_key(|(distance, position, _)| (*distance, *position))
        .map(|(_, _, value)| value)
}

/// `None` describes an explicit user-defined provider model. For a registered
/// GPT model, pass its actual supported levels; an empty list means no thinking.
pub(crate) fn apply(
    body: &mut Value,
    requested_model: &str,
    chat: bool,
    supported_levels: Option<&[String]>,
) -> Result<(), ServiceError> {
    let (base, effort) = parse_model(requested_model);
    if base == requested_model {
        return Ok(());
    }
    let Some(mut effort) = effort else {
        return Ok(());
    };
    let raw = &requested_model[base.len() + 1..requested_model.len() - 1];
    if let Some(levels) = supported_levels {
        if levels.is_empty() {
            if effort == "none" {
                return Ok(());
            }
            return Err(ServiceError::new(
                400,
                "thinking_not_supported",
                "Thinking is not supported for the selected model",
            ));
        }
        if effort == "none"
            && !levels
                .iter()
                .any(|level| level.eq_ignore_ascii_case("none"))
        {
            effort = levels[0].clone();
        } else if effort == "auto"
            && !levels
                .iter()
                .any(|level| level.eq_ignore_ascii_case("auto"))
        {
            effort = nearest("medium", levels).unwrap_or_else(|| levels[0].clone());
        } else if !levels
            .iter()
            .any(|level| level.eq_ignore_ascii_case(&effort))
        {
            if raw.parse::<i64>().is_ok() {
                effort = nearest(&effort, levels).unwrap_or_else(|| levels[0].clone());
            } else {
                return Err(ServiceError::new(
                    400,
                    "unsupported_thinking_level",
                    "The selected model does not support this thinking level",
                ));
            }
        }
    }
    let object = body.as_object_mut().ok_or_else(|| {
        ServiceError::new(400, "invalid_json", "Request body must be a JSON object")
    })?;
    if chat {
        object.insert("reasoning_effort".into(), Value::String(effort));
    } else {
        let reasoning = object.entry("reasoning").or_insert_with(|| json!({}));
        if !reasoning.is_object() {
            *reasoning = json!({});
        }
        reasoning
            .as_object_mut()
            .unwrap()
            .insert("effort".into(), Value::String(effort));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_compatible_suffix_matches_actual_go_request_helper() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/openai_thinking_go.json")).unwrap();
        for case in cases {
            let mut body = case["input"].clone();
            apply(
                &mut body,
                case["model"].as_str().unwrap(),
                case["chat"].as_bool().unwrap(),
                None,
            )
            .unwrap();
            assert_eq!(body, case["expected"], "{}", case["name"]);
        }
    }

    #[test]
    fn suffix_budget_boundaries_match_go_and_unknown_suffix_keeps_body_policy() {
        for (budget, level) in [
            ("-1", "auto"),
            ("0", "none"),
            ("512", "minimal"),
            ("513", "low"),
            ("1024", "low"),
            ("1025", "medium"),
            ("8192", "medium"),
            ("8193", "high"),
            ("24576", "high"),
            ("24577", "xhigh"),
        ] {
            assert_eq!(
                parse_model(&format!("gpt-fixture({budget})")),
                ("gpt-fixture".into(), Some(level.into()))
            );
        }
        assert_eq!(
            parse_model("gpt-fixture(unknown)"),
            ("gpt-fixture".into(), None)
        );
        assert_eq!(parse_model("gpt-fixture(-2)"), ("gpt-fixture".into(), None));
    }

    #[test]
    fn registered_levels_validate_and_budget_auto_none_clamp_without_touching_history() {
        let levels = vec!["low".into(), "high".into()];
        let original = json!({"model":"wire-model","reasoning":{"summary":"detailed"},"input":[{"encrypted_content":"untouched"}]});
        for (suffix, expected) in [
            ("8192", "low"),
            ("auto", "low"),
            ("none", "low"),
            ("high", "high"),
        ] {
            let mut body = original.clone();
            apply(
                &mut body,
                &format!("gpt-fixture({suffix})"),
                false,
                Some(&levels),
            )
            .unwrap();
            assert_eq!(body["reasoning"]["effort"], expected);
            assert_eq!(body["reasoning"]["summary"], "detailed");
            assert_eq!(body["input"], original["input"]);
            assert_eq!(body["model"], original["model"]);
        }
        assert!(apply(
            &mut original.clone(),
            "gpt-fixture(max)",
            false,
            Some(&levels)
        )
        .is_err());
        let mut chat = original.clone();
        apply(&mut chat, "vendor-model(max)", true, None).unwrap();
        assert_eq!(chat["reasoning_effort"], "max");
        assert_eq!(chat["input"], original["input"]);
    }
}
