//! MiniMax Flash wire constraints, applied only after the upstream model is resolved.
use serde_json::{json, Value};

pub(crate) fn normalize_flash_request(body: &mut Value, anthropic: bool) {
    let is_flash = body
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|model| {
            model.eq_ignore_ascii_case("MiniMax-M3.1-Flash")
                || model.eq_ignore_ascii_case("minimax-m3.1-flash-free")
        });
    if !is_flash {
        return;
    }
    let Some(object) = body.as_object_mut() else {
        return;
    };
    object.insert("thinking".into(), json!({"type": "adaptive"}));
    object.remove("service_tier");
    object.remove("reasoning_split");
    if anthropic {
        if !object.get("output_config").is_some_and(Value::is_object) {
            object.insert("output_config".into(), json!({}));
        }
        let config = object
            .get_mut("output_config")
            .unwrap()
            .as_object_mut()
            .unwrap();
        normalize_effort(config, "effort");
    } else {
        normalize_effort(object, "reasoning_effort");
    }
}

fn normalize_effort(object: &mut serde_json::Map<String, Value>, key: &str) {
    if !matches!(
        object.get(key).and_then(Value::as_str),
        Some("low" | "medium" | "high" | "xhigh" | "max")
    ) {
        object.insert(key.into(), json!("max"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_efforts_and_history_are_preserved_for_both_protocols() {
        for anthropic in [false, true] {
            for effort in [
                "low", "medium", "high", "xhigh", "max", "none", "minimal", "",
            ] {
                let mut body = json!({
                    "model": "MiniMax-M3.1-Flash",
                    "thinking": {"type": "disabled"}, "service_tier": "priority",
                    "reasoning_split": false,
                    "messages": [{"role":"assistant", "reasoning_content":"prior thought", "content":"answer"}],
                    "reasoning_effort": effort, "output_config": {"effort": effort, "format": {"type":"json_schema"}}
                });
                let messages = body["messages"].clone();
                normalize_flash_request(&mut body, anthropic);
                let expected = if ["none", "minimal", ""].contains(&effort) {
                    "max"
                } else {
                    effort
                };
                assert_eq!(
                    if anthropic {
                        &body["output_config"]["effort"]
                    } else {
                        &body["reasoning_effort"]
                    },
                    expected
                );
                assert_eq!(body["thinking"], json!({"type":"adaptive"}));
                assert!(body.get("service_tier").is_none());
                assert!(body.get("reasoning_split").is_none());
                assert_eq!(body["messages"], messages);
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
            }
        }
    }

    #[test]
    fn m3_legacy_and_unrelated_models_are_unchanged() {
        for model in ["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.5", "other-model"] {
            let original = json!({"model":model,"thinking":{"type":"disabled"},"reasoning_effort":"none","service_tier":"priority","reasoning_split":false});
            for anthropic in [true, false] {
                let mut body = original.clone();
                normalize_flash_request(&mut body, anthropic);
                assert_eq!(body, original);
            }
        }
    }
}
