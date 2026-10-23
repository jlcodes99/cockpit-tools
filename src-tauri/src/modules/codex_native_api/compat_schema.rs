//! Match the Go Claude bridge's strict-schema downgrade without walking instance data.
use serde_json::Value;

pub(super) fn misses_required(schema: &Value) -> bool {
    if let Some(items) = schema.as_array() {
        return items.iter().any(misses_required);
    }
    let Some(object) = schema.as_object() else {
        return false;
    };
    if let Some(properties) = schema["properties"].as_object() {
        if properties.keys().any(|name| {
            !schema["required"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(name)))
        }) {
            return true;
        }
    }
    for keyword in [
        "properties",
        "$defs",
        "definitions",
        "patternProperties",
        "dependentSchemas",
        "dependencies",
    ] {
        if object
            .get(keyword)
            .and_then(Value::as_object)
            .is_some_and(|children| children.values().any(misses_required))
        {
            return true;
        }
    }
    for keyword in [
        "items",
        "prefixItems",
        "contains",
        "additionalProperties",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
        "additionalItems",
        "contentSchema",
        "anyOf",
        "oneOf",
        "allOf",
        "not",
        "if",
        "then",
        "else",
    ] {
        if object.get(keyword).is_some_and(misses_required) {
            return true;
        }
    }
    false
}
