use serde_json::Value;

pub(super) fn metadata_enabled(body: &Value) -> bool {
    body.pointer("/client_metadata/ws_request_header_x_openai_internal_codex_responses_lite")
        .is_some_and(|value| {
            value == true
                || value
                    .as_str()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
        })
}

fn supported(tool: &Value) -> bool {
    match tool["type"].as_str().unwrap_or("").trim() {
        "function" | "custom" | "namespace" => true,
        "tool_search" => tool["execution"]
            .as_str()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("client")),
        _ => false,
    }
}

fn filter_array(object: &mut Value, key: &str) {
    if let Some(items) = object.get_mut(key).and_then(Value::as_array_mut) {
        items.retain(supported);
        if items.is_empty() {
            object.as_object_mut().unwrap().remove(key);
        }
    }
}

fn filter_choice(choice: &mut Value) -> bool {
    if let Some(choice) = choice.as_str() {
        return matches!(
            choice.trim().to_ascii_lowercase().as_str(),
            "auto" | "none" | "required"
        );
    }
    if !choice.is_object() {
        return false;
    }
    match choice["type"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "function" | "custom" | "namespace" => true,
        "tool_search" => supported(choice),
        "allowed_tools" => {
            filter_array(choice, "tools");
            filter_array(choice, "allowed_tools");
            if let Some(nested) = choice
                .get_mut("allowed_tools")
                .filter(|value| value.is_object())
            {
                filter_array(nested, "tools");
            }
            choice
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
                || choice
                    .get("allowed_tools")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
                || choice
                    .pointer("/allowed_tools/tools")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
        }
        _ => false,
    }
}

pub(super) fn filter(object: &mut Value) {
    if let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) {
        for item in input.iter_mut().filter(|item| additional(item)) {
            filter(item);
        }
        input.retain(|item| {
            !additional(item)
                || item
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|tools| !tools.is_empty())
        });
    }
    if let Some(response) = object.get_mut("response").filter(|value| value.is_object()) {
        filter(response);
    }
    filter_array(object, "tools");
    if let Some(choice) = object.get_mut("tool_choice") {
        if !filter_choice(choice) {
            object.as_object_mut().unwrap().remove("tool_choice");
        }
    }
}

fn additional(item: &Value) -> bool {
    item["type"]
        .as_str()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("additional_tools"))
}

fn image(tool: &Value) -> bool {
    let eq = |value: &Value, expected: &str| {
        value
            .as_str()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
    };
    if eq(tool, "image_generation")
        || eq(tool, "image_gen.imagegen")
        || eq(&tool["type"], "image_generation")
        || eq(&tool["type"], "tool") && eq(&tool["name"], "image_generation")
        || eq(&tool["name"], "image_gen.imagegen")
        || tool
            .pointer("/function/name")
            .is_some_and(|name| eq(name, "image_gen.imagegen"))
    {
        return true;
    }
    (eq(&tool["name"], "image_gen") || eq(&tool["namespace"], "image_gen"))
        && tool["tools"].as_array().is_some_and(|tools| {
            tools.iter().any(|tool| {
                eq(&tool["name"], "imagegen")
                    || tool
                        .pointer("/function/name")
                        .is_some_and(|name| eq(name, "imagegen"))
            })
        })
}

pub(super) fn uses_image(object: &Value) -> bool {
    object["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(image))
        || object.get("tool_choice").is_some_and(image)
        || object["input"]
            .as_array()
            .is_some_and(|input| input.iter().filter(|item| additional(item)).any(uses_image))
        || object
            .get("response")
            .filter(|value| value.is_object())
            .is_some_and(uses_image)
}
