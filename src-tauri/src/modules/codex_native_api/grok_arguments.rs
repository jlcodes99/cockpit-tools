//! Lossless normalization of integer-valued JSON argument numbers. In
//! particular, never round large IDs through f64 or rewrite string contents.
use serde_json::Value;

pub(super) fn normalize(raw: &str) -> String {
    if serde_json::from_str::<Value>(raw).is_err() {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut output = String::with_capacity(raw.len());
    let mut start = 0;
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        if quoted {
            if bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if bytes[index] == b'"' {
                quoted = false;
            }
        } else if bytes[index] == b'"' {
            quoted = true;
        } else if bytes[index] == b'-' || bytes[index].is_ascii_digit() {
            let first = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_digit() || b"-.eE+".contains(&bytes[index]))
            {
                index += 1;
            }
            if let Some(number) = integral(&raw[first..index]) {
                output.push_str(&raw[start..first]);
                output.push_str(&number);
                start = index;
            }
            continue;
        }
        index += 1;
    }
    output.push_str(&raw[start..]);
    output
}

fn integral(token: &str) -> Option<String> {
    if !token.contains(['.', 'e', 'E']) {
        return None;
    }
    let (negative, unsigned) = token
        .strip_prefix('-')
        .map_or((false, token), |v| (true, v));
    let (mantissa, exponent) = unsigned
        .split_once(['e', 'E'])
        .map_or(Some((unsigned, 0i32)), |(value, exponent)| {
            exponent.parse::<i32>().ok().map(|e| (value, e))
        })?;
    let fractional = mantissa.split_once('.').map_or(0, |(_, rest)| rest.len());
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let scale = i64::from(exponent) - i64::try_from(fractional).ok()?;
    if scale < 0 {
        let remove = usize::try_from(-scale).ok()?;
        if remove > digits.len() || !digits[digits.len() - remove..].bytes().all(|v| v == b'0') {
            return None;
        }
        digits.truncate(digits.len() - remove);
    } else {
        digits = digits.trim_start_matches('0').to_owned();
        if digits.len().saturating_add(usize::try_from(scale).ok()?) > 19 {
            return None;
        }
        digits.extend(std::iter::repeat_n('0', scale as usize));
    }
    let digits = digits.trim_start_matches('0');
    let integer = if digits.is_empty() {
        "0".to_owned()
    } else if negative {
        format!("-{digits}")
    } else {
        digits.to_owned()
    };
    integer.parse::<i64>().ok().map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_integral_arguments_preserve_precision_strings_and_fractional_values() {
        let raw = r#"{ "timeout_ms":180000.0,"exponent":1.8e5,"id":9007199254740993.0,"max":9223372036854775807.0,"min":-9223372036854775808.0,"fraction":1.25,"text":"18.0 \\\"123\\\" 中文","huge":12345678901234567890123456789,"nested":[-0.0,1e300] }"#;
        let normalized = normalize(raw);
        assert!(normalized.contains(r#""timeout_ms":180000"#));
        assert!(normalized.contains(r#""exponent":180000"#));
        assert!(normalized.contains(r#""id":9007199254740993"#));
        assert!(normalized.contains(r#""max":9223372036854775807"#));
        assert!(normalized.contains(r#""min":-9223372036854775808"#));
        assert!(normalized.contains(r#""fraction":1.25"#));
        assert!(normalized.contains(r#""text":"18.0 \\\"123\\\" 中文"#));
        assert!(normalized.contains("12345678901234567890123456789"));
        assert!(normalized.contains("[0,1e300]"));
        assert_eq!(normalize(&normalized), normalized);
        assert_eq!(normalize(r#"{"partial":18.0"#), r#"{"partial":18.0"#);
    }
}
