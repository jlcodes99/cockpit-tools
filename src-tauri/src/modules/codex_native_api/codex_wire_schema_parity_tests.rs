use super::*;

#[test]
fn numeric_bound_exact_int64_decimal_and_exponent_limits_match_cpa() {
    for (number, outside) in [
        ("9223372036854775807", false),
        ("9223372036854775808", true),
        ("-9223372036854775808", false),
        ("-9223372036854775809", true),
        ("9223372036854775807.0", false),
        ("9223372036854775808.0", true),
        ("-9223372036854775808.000", false),
        ("-9223372036854775809.000", true),
        ("9.223372036854775807e18", false),
        ("9.223372036854775808e18", true),
        ("-9.223372036854775808E18", false),
        ("-9.223372036854775809E18", true),
        ("92233720368547758070e-1", false),
        ("92233720368547758080e-1", true),
        ("1e30", true),
        ("-1e30", true),
        ("0e999999999999999999999999999999999999999999", false),
        ("1e999999999999999999999999999999999999999999", true),
        ("1e-999999999999999999999999999999999999999999", false),
        ("1e-1000000", false),
        ("0.5", false),
        ("9223372036854775808.5", false),
        ("-9223372036854775809.25", false),
        ("-0.0", false),
        ("1.000e0", false),
    ] {
        assert_eq!(numeric_bound_out_of_int64(number), outside, "{number}");
    }
}

#[test]
fn numeric_bounds_keep_original_tokens_and_remove_only_incompatible_integer_constraints() {
    let raw = r#"{"tools":[{"type":"function","name":"limits","parameters":{"type":"object","properties":{"valid":{"minimum":-9223372036854775808.0,"maximum":9223372036854775807.0},"invalid":{"minimum":-9223372036854775809.0,"maximum":9223372036854775808.0,"exclusiveMinimum":-1e30,"exclusiveMaximum":1e30},"fractional":{"minimum":0.5,"maximum":9223372036854775808.5},"data":{"default":{"maximum":18446744073709551615},"enum":[{"minimum":-1e30}]}}}}]}"#;
    let mut body: Value = serde_json::from_str(raw).unwrap();
    let original = body.clone();
    normalize_tools(&mut body);
    let properties = &body["tools"][0]["parameters"]["properties"];
    assert_eq!(
        properties["valid"],
        original["tools"][0]["parameters"]["properties"]["valid"]
    );
    assert_eq!(properties["invalid"], json!({}));
    assert_eq!(
        properties["fractional"],
        original["tools"][0]["parameters"]["properties"]["fractional"]
    );
    assert_eq!(
        properties["data"],
        original["tools"][0]["parameters"]["properties"]["data"]
    );
}

#[test]
fn unsupported_regex_escapes_follow_cpa_escape_parity_and_preserve_ordinary_patterns() {
    for pattern in [r"\p{L}", r"\P{N}", r"[\0]", r"abc\0tail", r"\\\0"] {
        assert!(unsupported_pattern(pattern), "{pattern}");
    }
    for pattern in [
        r"\\p{L}", r"\\0", r"\d+", r"\u0000", r"\x00", r"[a-z]", r"\",
    ] {
        assert!(!unsupported_pattern(pattern), "{pattern}");
    }
    let mut body = json!({"tools":[{"type":"function","name":"patterns","parameters":{
        "properties":{"text":{"type":"string","pattern":r"\0", "default":{"pattern":r"\0"}}},
        "patternProperties":{r"\0":{"type":"string"},r"\\0":{"pattern":r"\p{L}"}}
    }}]});
    normalize_tools(&mut body);
    let params = &body["tools"][0]["parameters"];
    assert!(params["properties"]["text"].get("pattern").is_none());
    assert_eq!(params["properties"]["text"]["default"]["pattern"], r"\0");
    assert!(params["patternProperties"].get(r"\0").is_none());
    assert_eq!(params["patternProperties"][r"\\0"], json!({}));
}

#[test]
fn schema_cleanup_uses_only_the_upstream_schema_keyword_walk() {
    for key in [
        "properties",
        "$defs",
        "definitions",
        "patternProperties",
        "dependentSchemas",
        "dependencies",
    ] {
        let mut body = json!({"tools":[{"type":"custom","name":"fixture","parameters":{key:{"nested":{"pattern":r"\0","maximum":18446744073709551615u64}}}}]});
        normalize_tools(&mut body);
        assert_eq!(
            body["tools"][0]["parameters"][key]["nested"],
            json!({}),
            "{key}"
        );
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
        for value in [
            json!({"pattern":r"\0","maximum":18446744073709551615u64}),
            json!([{"pattern":r"\0","maximum":18446744073709551615u64}]),
        ] {
            let array = value.is_array();
            let mut body =
                json!({"tools":[{"type":"function","name":"fixture","parameters":{key:value}}]});
            normalize_tools(&mut body);
            assert_eq!(
                body["tools"][0]["parameters"][key],
                if array { json!([{}]) } else { json!({}) },
                "{key}"
            );
        }
    }
}

#[test]
fn constant_union_simplification_requires_eight_unique_pure_semantic_constants() {
    let branches: Vec<Value> = (0..8)
        .map(|value| json!({"const":value,"description":"Allowed"}))
        .collect();
    let mut simple = json!({"oneOf":branches.clone()});
    simplify_union(&mut simple);
    assert_eq!(simple, json!({"enum":[0,1,2,3,4,5,6,7]}));
    for mut property in [
        json!({"oneOf":branches[..7]}),
        json!({"oneOf":branches.clone(),"anyOf":branches.clone()}),
        json!({"oneOf":branches.clone(),"enum":[0,1,2]}),
        json!({"oneOf":[{"const":0},{"const":0.0},{"const":2},{"const":3},{"const":4},{"const":5},{"const":6},{"const":7}]}),
        json!({"oneOf":[{"const":0,"minimum":0},{"const":1},{"const":2},{"const":3},{"const":4},{"const":5},{"const":6},{"const":7}]}),
    ] {
        let original = property.clone();
        simplify_union(&mut property);
        assert_eq!(property, original);
    }
}
