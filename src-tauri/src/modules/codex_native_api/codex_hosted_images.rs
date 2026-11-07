//! Native Responses-Lite cannot execute forced hosted image tools (CPA v8.0.58).
use super::contract::ServiceError;
use axum::http::HeaderMap;
use serde_json::Value;

pub(super) fn forced(body: &Value) -> bool {
    let mut tools: Vec<&Value> = body["tools"].as_array().into_iter().flatten().collect();
    for item in body["input"].as_array().into_iter().flatten() {
        if item["type"] == "additional_tools" {
            tools.extend(item["tools"].as_array().into_iter().flatten());
        }
    }
    if !tools.iter().any(|tool| tool["type"] == "image_generation") {
        return false;
    }
    let choice = &body["tool_choice"];
    choice["type"] == "image_generation"
        || choice == "required" && tools.iter().all(|tool| tool["type"] == "image_generation")
        || choice["type"] == "allowed_tools"
            && choice["mode"] == "required"
            && choice["tools"].as_array().is_some_and(|tools| {
                !tools.is_empty() && tools.iter().all(|tool| tool["type"] == "image_generation")
            })
}

pub(super) fn validate(headers: &HeaderMap, body: &Value) -> Result<(), ServiceError> {
    let model = body["model"].as_str().unwrap_or("");
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let catalog_lite = crate::modules::codex_protocol::known_codex_model_template(&model)
        .is_some_and(|entry| entry["use_responses_lite"] == true);
    if (super::codex_wire::responses_lite(headers, body)
        || model.starts_with("gpt-6")
        || catalog_lite)
        && forced(body)
    {
        return Err(ServiceError::new(400, "hosted_image_generation_unsupported",
            "Hosted image_generation cannot be forced on this Codex Responses-Lite route. Use /v1/images/generations or /v1/images/edits instead."));
    }
    Ok(())
}

/// complexity belongs to the standalone Images API, not native Codex hosted tools.
pub(super) fn normalize(body: &mut Value) {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if tool["type"] == "image_generation" {
                if let Some(tool) = tool.as_object_mut() {
                    tool.remove("complexity");
                }
            }
        }
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item["type"] == "additional_tools" {
                normalize(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_only_forced_hosted_lite_images() {
        let headers = HeaderMap::new();
        let mut body = json!({"model":"gpt-6-astra","tools":[{"type":"image_generation","complexity":"high"}],"tool_choice":{"type":"image_generation"}});
        assert_eq!(validate(&headers, &body).unwrap_err().status, 400);
        body["tool_choice"] = json!("auto");
        assert!(validate(&headers, &body).is_ok());
        normalize(&mut body);
        assert!(body["tools"][0].get("complexity").is_none());
        body["model"] = json!("gpt-5.5");
        body["tool_choice"] = json!({"type":"image_generation"});
        assert!(validate(&headers, &body).is_ok());
    }
}
