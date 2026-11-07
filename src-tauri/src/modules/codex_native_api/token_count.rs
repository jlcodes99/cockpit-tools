//! Local compatibility estimate used by the existing Go host, not an upstream tokenizer.
use serde_json::Value;

/// `body` is the preprocessed client protocol body, before Responses conversion.
/// Preserve its byte length for Go's no-text fallback (including whitespace).
pub(super) fn estimate_raw(body: &[u8]) -> u64 {
    serde_json::from_slice::<Value>(body).map_or(1, |value| estimate(&value, body.len()))
}

pub(super) fn estimate(body: &Value, fallback_bytes: usize) -> u64 {
    let chars = text_chars(body);
    ((if chars == 0 { fallback_bytes } else { chars }) as u64 / 4).max(1)
}

/// Count after the caller injects/canonicalizes the model, matching the Go handler's
/// map round-trip. Its fallback discards incoming whitespace and uses Go JSON sizing.
pub(super) fn estimate_preprocessed(body: &Value) -> u64 {
    estimate(body, go_json_bytes(body))
}

fn go_json_bytes(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::String(text) => go_string_bytes(text),
        // encoding/json decodes every untyped JSON number as float64 before the
        // handler marshals its map. Counting only never alters the original value.
        Value::Number(number) => go_float_text(number.as_f64().unwrap_or(0.0)).len(),
        Value::Array(items) => {
            2 + items.len().saturating_sub(1) + items.iter().map(go_json_bytes).sum::<usize>()
        }
        Value::Object(object) => {
            2 + object.len().saturating_sub(1)
                + object
                    .iter()
                    .map(|(key, value)| go_string_bytes(key) + 1 + go_json_bytes(value))
                    .sum::<usize>()
        }
    }
}

fn go_string_bytes(text: &str) -> usize {
    serde_json::to_string(text)
        .expect("String JSON serialization is infallible")
        .len()
        + text
            .chars()
            .map(|ch| match ch {
                '<' | '>' | '&' => 5,
                '\u{2028}' | '\u{2029}' => 3,
                _ => 0,
            })
            .sum::<usize>()
}

fn go_float_text(number: f64) -> String {
    if number == 0.0 {
        return if number.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let text = serde_json::to_string(&number).expect("JSON numbers are finite");
    let negative = text.starts_with('-');
    let text = text.trim_start_matches('-');
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text, 0i32), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(0))
        });
    let integer_len = mantissa
        .split_once('.')
        .map_or(mantissa.len(), |(integer, _)| integer.len()) as i32;
    let mut digits = mantissa.replace('.', "");
    let leading_zeroes = digits.len() - digits.trim_start_matches('0').len();
    digits.drain(..leading_zeroes);
    let decimal = integer_len + exponent - leading_zeroes as i32;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let magnitude = number.abs();
    let sign = if negative { "-" } else { "" };
    if !(1e-6..1e21).contains(&magnitude) {
        let power = decimal - 1;
        let fraction = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return format!(
            "{sign}{}{fraction}e{}{power}",
            &digits[..1],
            if power >= 0 { "+" } else { "" }
        );
    }
    if decimal <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-decimal) as usize))
    } else if decimal as usize >= digits.len() {
        format!(
            "{sign}{digits}{}",
            "0".repeat(decimal as usize - digits.len())
        )
    } else {
        format!(
            "{sign}{}.{}",
            &digits[..decimal as usize],
            &digits[decimal as usize..]
        )
    }
}

fn text_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(text_chars).sum(),
        Value::Object(object) => object
            .iter()
            .filter_map(|(key, value)| {
                if matches!(
                    key.trim().to_ascii_lowercase().as_str(),
                    "text" | "content" | "system" | "prompt"
                ) || value.is_array()
                    || value.is_object()
                {
                    Some(text_chars(value))
                } else {
                    None
                }
            })
            .sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_shared_go_estimator_cases() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/token_count_go.json")).unwrap();
        for case in cases {
            let raw = case["body"].as_str().unwrap();
            let count = if case["preprocessed"].as_bool().unwrap_or(false) {
                let mut body: Value = serde_json::from_str(raw).unwrap();
                let model = case["model"].as_str().unwrap();
                body["model"] = Value::String(model.trim().to_owned());
                estimate_preprocessed(&body)
            } else {
                estimate_raw(raw.as_bytes())
            };
            assert_eq!(count, case["count"].as_u64().unwrap(), "{}", case["name"]);
        }
    }
}
