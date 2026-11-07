// Canonical, non-overlapping OpenAI/Codex usage buckets, matching the SDK v2 contract.
fn native_usage_capture(value: &Value) -> UsageCapture {
    fn field<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a Value> {
        paths.iter().find_map(|path| value.pointer(path))
    }
    fn number(value: Option<&Value>) -> (u64, bool) {
        match value {
            None | Some(Value::Null) => (0, true),
            Some(Value::String(value)) => value
                .parse::<i64>()
                .map(|v| (v.max(0) as u64, v >= 0))
                .unwrap_or((0, false)),
            Some(value) => value
                .as_i64()
                .map(|v| (v.max(0) as u64, v >= 0))
                .unwrap_or((0, false)),
        }
    }
    let input = field(value, &["/prompt_tokens", "/input_tokens"]);
    let output = field(value, &["/completion_tokens", "/output_tokens"]);
    let cache = field(
        value,
        &[
            "/prompt_tokens_details/cached_tokens",
            "/input_tokens_details/cached_tokens",
        ],
    );
    let write = field(
        value,
        &[
            "/input_tokens_details/cache_creation_tokens",
            "/input_tokens_details/cache_write_tokens",
            "/prompt_tokens_details/cache_creation_tokens",
            "/prompt_tokens_details/cache_write_tokens",
        ],
    );
    let reasoning = field(
        value,
        &[
            "/completion_tokens_details/reasoning_tokens",
            "/output_tokens_details/reasoning_tokens",
        ],
    );
    let (input_tokens, input_valid) = number(input);
    let (output_tokens, output_valid) = number(output);
    let (cached_tokens, cache_valid) = number(cache);
    let (write_tokens, write_valid) = number(write);
    let (reasoning_tokens, reasoning_valid) = number(reasoning);
    let (total, total_valid) = number(value.get("total_tokens"));
    let expected = if input_valid && output_valid {
        input_tokens.saturating_add(output_tokens)
    } else {
        0
    };
    let resolved = if total == 0 { expected } else { total };
    let mut breakdown = CodexTokenBreakdown {
        schema_version: 2,
        total_tokens: resolved,
        ..Default::default()
    };
    if input.is_none()
        && output.is_none()
        && cache.is_none()
        && write.is_none()
        && reasoning.is_none()
    {
        breakdown.quality = if total == 0 {
            "complete"
        } else {
            "unclassified"
        }
        .into();
        breakdown.unclassified_tokens = total;
    } else {
        let cache_read = if input.is_some() { cached_tokens } else { 0 };
        let cache_write = if input.is_some() { write_tokens } else { 0 };
        let output_reasoning = if output.is_some() {
            reasoning_tokens
        } else {
            0
        };
        let valid = input_valid
            && output_valid
            && total_valid
            && (input.is_none() || (cache_valid && write_valid))
            && (output.is_none() || reasoning_valid)
            && cache_read.saturating_add(cache_write) <= input_tokens
            && output_reasoning <= output_tokens
            && if input.is_some() && output.is_some() {
                resolved == expected
            } else {
                resolved >= expected
            };
        if valid {
            breakdown.quality = if resolved == expected {
                "complete"
            } else {
                "unclassified"
            }
            .into();
            breakdown.input.total_tokens = input_tokens;
            breakdown.input.uncached_tokens = input_tokens - cache_read - cache_write;
            breakdown.input.cache_read_tokens = cache_read;
            breakdown.input.cache_write_tokens = cache_write;
            breakdown.output.total_tokens = output_tokens;
            breakdown.output.non_reasoning_tokens = output_tokens - output_reasoning;
            breakdown.output.reasoning_tokens = output_reasoning;
            breakdown.unclassified_tokens = resolved - expected;
        } else {
            breakdown.quality = "inconsistent".into();
            breakdown.unclassified_tokens = resolved;
        }
    }
    UsageCapture {
        input_tokens,
        output_tokens,
        total_tokens: resolved,
        cached_tokens,
        reasoning_tokens,
        token_breakdown: Some(breakdown),
    }
}

#[cfg(test)]
mod native_usage_parity_tests {
    use super::*;
    #[test]
    fn canonical_usage_matches_go_sdk_golden() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "codex_native_api/fixtures/codex_usage_go.json"
        ))
        .unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let capture = native_usage_capture(&fixture["input"]);
            assert_eq!(
                serde_json::to_value(capture.token_breakdown.unwrap()).unwrap(),
                fixture["expected"],
                "{}",
                fixture["name"]
            );
            assert_eq!(
                capture.total_tokens,
                fixture["total_tokens"].as_u64().unwrap(),
                "{}",
                fixture["name"]
            );
        }
    }
}
