//! Schema-only compatibility. Defaults, enum data and user text are never traversed as schemas.
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn normalize_tools(body: &mut Value) {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            normalize_tool(tool);
        }
    }
}

fn normalize_tool(tool: &mut Value) {
    match tool["type"].as_str() {
        Some("namespace") => {
            if let Some(tools) = tool.get_mut("tools").and_then(Value::as_array_mut) {
                for tool in tools {
                    normalize_tool(tool);
                }
            }
        }
        Some("function" | "custom") => {
            if let Some(params) = tool.get_mut("parameters").filter(|value| value.is_object()) {
                strip_patterns(params);
                if let Some(properties) =
                    params.get_mut("properties").and_then(Value::as_object_mut)
                {
                    for property in properties.values_mut() {
                        simplify_union(property);
                    }
                }
            }
        }
        _ => {}
    }
}

fn unsupported_pattern(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            if matches!(bytes.get(cursor + 1), Some(b'p' | b'P'))
                && bytes.get(cursor + 2) == Some(&b'{')
            {
                return true;
            }
            if bytes.get(cursor + 1) == Some(&b'0') {
                return true;
            }
            cursor += 1;
        }
        cursor += 1;
    }
    false
}

fn strip_patterns(schema: &mut Value) {
    if let Some(items) = schema.as_array_mut() {
        for item in items {
            strip_patterns(item);
        }
        return;
    }
    let Some(schema) = schema.as_object_mut() else {
        return;
    };
    if schema
        .get("pattern")
        .and_then(Value::as_str)
        .is_some_and(unsupported_pattern)
    {
        schema.remove("pattern");
    }
    for key in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if schema
            .get(key)
            .and_then(Value::as_number)
            .is_some_and(|number| numeric_bound_out_of_int64(&number.to_string()))
        {
            schema.remove(key);
        }
    }
    for key in [
        "properties",
        "$defs",
        "definitions",
        "patternProperties",
        "dependentSchemas",
        "dependencies",
    ] {
        if let Some(map) = schema.get_mut(key).and_then(Value::as_object_mut) {
            if key == "patternProperties" {
                map.retain(|pattern, _| !unsupported_pattern(pattern));
            }
            for child in map.values_mut() {
                strip_patterns(child);
            }
        }
    }
    for key in [
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
        if let Some(child) = schema.get_mut(key) {
            strip_patterns(child);
        }
    }
}

/// CPA drops only integer-valued bounds outside int64, including integral
/// decimal/exponent spelling. Avoid f64 conversion, exponent-sized allocation
/// and integer overflow while checking hostile-but-valid numeric schemas.
fn numeric_bound_out_of_int64(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return false;
    }
    let negative = raw.starts_with('-');
    let raw_unsigned = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    let (mantissa, exponent) = raw_unsigned
        .split_once(['e', 'E'])
        .unwrap_or((raw_unsigned, "0"));
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() && fraction.is_empty() {
        return false;
    }
    if !integer
        .bytes()
        .chain(fraction.bytes())
        .all(|value| value.is_ascii_digit())
    {
        return false;
    }
    let combined = format!("{integer}{fraction}");
    let significant = combined.trim_start_matches('0');
    if significant.is_empty() {
        return false;
    }
    let exponent = match exponent.parse::<i128>() {
        Ok(value) => value,
        Err(error) if matches!(error.kind(), std::num::IntErrorKind::PosOverflow) => return true,
        Err(_) => return false,
    };
    // These guards also bound the following arithmetic for extreme exponents.
    if exponent < -(raw.len() as i128) {
        return false;
    }
    if exponent > raw.len() as i128 + 20 {
        return true;
    }
    let digits = significant.trim_end_matches('0');
    let power = exponent - fraction.len() as i128 + (significant.len() - digits.len()) as i128;
    if power < 0 {
        return false;
    }
    let length = digits.len() as i128 + power;
    if length > 19 {
        return true;
    }
    if length < 19 {
        return false;
    }
    let normalized = format!("{digits}{}", "0".repeat(power as usize));
    normalized.as_str()
        > if negative {
            "9223372036854775808"
        } else {
            "9223372036854775807"
        }
}

#[cfg(test)]
#[path = "codex_wire_schema_parity_tests.rs"]
mod parity_tests;

fn simplify_union(property: &mut Value) {
    let Some(object) = property.as_object_mut() else {
        return;
    };
    let union_name = match (object.contains_key("oneOf"), object.contains_key("anyOf")) {
        (true, false) => "oneOf",
        (false, true) => "anyOf",
        _ => return,
    };
    let Some(branches) = object
        .get(union_name)
        .and_then(Value::as_array)
        .filter(|branches| branches.len() >= 8)
    else {
        return;
    };
    let mut keys = HashSet::new();
    let mut values = Vec::with_capacity(branches.len());
    for branch in branches {
        let Some(branch) = branch.as_object() else {
            return;
        };
        if branch
            .keys()
            .any(|key| !matches!(key.as_str(), "const" | "title" | "description"))
        {
            return;
        }
        let Some(value) = branch.get("const") else {
            return;
        };
        let Some(key) = scalar_key(value) else {
            return;
        };
        if !keys.insert(key) {
            return;
        }
        values.push(value.clone());
    }
    if let Some(existing) = object.get("enum").and_then(Value::as_array) {
        let Some(existing_keys) = existing.iter().map(scalar_key).collect::<Option<Vec<_>>>()
        else {
            return;
        };
        if existing_keys.len() != keys.len()
            || existing_keys.into_iter().collect::<HashSet<_>>() != keys
        {
            return;
        }
    } else {
        object.insert("enum".into(), json!(values));
    }
    object.remove(union_name);
}

fn scalar_key(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".into()),
        Value::Bool(value) => Some(format!("b:{value}")),
        Value::String(value) => Some(format!("s:{value}")),
        Value::Number(value) => Some(format!("n:{}", decimal_key(&value.to_string()))),
        _ => None,
    }
}

/// Exact decimal equivalence without converting integer constants to floating point.
fn decimal_key(number: &str) -> String {
    let (mantissa, exponent) = number
        .split_once(['e', 'E'])
        .map_or((number, 0), |(m, e)| (m, e.parse::<i64>().unwrap_or(0)));
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len() as i64);
    let mut digits = mantissa.replace('.', "").trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return "0".into();
    }
    let mut power = exponent - fractional;
    while digits.ends_with('0') {
        digits.pop();
        power += 1;
    }
    format!("{}{digits}e{power}", if negative { "-" } else { "" })
}
