use super::*;
use serde_json::json;

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-opencode-session", HeaderValue::from_static("session"));
    headers
}

#[test]
fn fallback_uses_a_deterministic_caller_scoped_tuple_not_cache_hints() {
    let original = headers();
    let derive = |caller, session, body: &[u8]| {
        let mut headers = original.clone();
        headers.insert(
            "x-opencode-session",
            HeaderValue::from_str(session).unwrap(),
        );
        parse(body, Some((caller, &headers))).unwrap().1.unwrap()
    };
    let first = derive("caller", "session", b"{}");
    assert_eq!(
        first,
        derive("caller", "session", br#"{"prompt_cache_key":"different"}"#)
    );
    assert_eq!(first["session-id"], first["thread-id"]);
    assert_ne!(first["session-id"], "session");
    let id = uuid::Uuid::parse_str(first["session-id"].to_str().unwrap()).unwrap();
    assert_eq!(id.get_version_num(), 5);
    assert_ne!(
        first["session-id"],
        derive("other", "session", b"{}")["session-id"]
    );
    assert_ne!(
        first["session-id"],
        derive("caller", "other", b"{}")["session-id"]
    );
    assert_ne!(
        derive("ab", "c", b"{}")["session-id"],
        derive("a", "bc", b"{}")["session-id"]
    );
    assert_eq!(
        original.len(),
        1,
        "never mutate original diagnostic headers"
    );
}

#[test]
fn absent_ambiguous_and_invalid_headers_never_establish_fallback_identity() {
    assert!(parse(b"{}", None).unwrap().1.is_none());
    assert!(parse(b"{}", Some(("caller", &HeaderMap::new())))
        .unwrap()
        .1
        .is_none());
    assert!(HeaderValue::from_bytes(b"bad\x7f").is_err());
    for value in ["", " ", "a b", "a,b", "a\tb", &"x".repeat(1025)] {
        let mut headers = headers();
        headers.insert(
            "x-opencode-session",
            HeaderValue::from_bytes(value.as_bytes()).unwrap(),
        );
        assert!(
            parse(b"{}", Some(("caller", &headers)))
                .unwrap()
                .1
                .is_none(),
            "{value:?}"
        );
    }
    let mut repeated = headers();
    repeated.append("x-opencode-session", HeaderValue::from_static("session"));
    assert!(parse(b"{}", Some(("caller", &repeated)))
        .unwrap()
        .1
        .is_none());
    let original = headers();
    for caller in ["", "bad\ncaller", &"x".repeat(4097)] {
        assert!(parse(b"{}", Some((caller, &original))).unwrap().1.is_none());
    }
}

#[test]
fn explicit_identity_presence_including_invalid_values_always_wins() {
    for name in [
        "session-id",
        "session_id",
        "thread-id",
        "thread_id",
        "x-codex-parent-thread-id",
        "x-codex-turn-state",
        "x-codex-turn-metadata",
    ] {
        for value in ["", "explicit"] {
            let mut headers = headers();
            headers.insert(name, HeaderValue::from_static(value));
            let (body, fallback) = parse(b"{}", Some(("caller", &headers))).unwrap();
            assert_eq!(body, json!({}));
            assert!(fallback.is_none(), "{name}");
        }
    }
    let headers = headers();
    for field in ["previous_response_id", "conversation", "conversation_id"] {
        for value in [Value::Null, json!("explicit"), json!(123)] {
            let body = json!({field:value});
            assert!(parse(
                &serde_json::to_vec(&body).unwrap(),
                Some(("caller", &headers))
            )
            .unwrap()
            .1
            .is_none());
        }
    }
    for field in [
        "session_id",
        "thread_id",
        "parent_response_id",
        "parent_thread_id",
        "turn_id",
        "x-codex-parent-thread-id",
        "x-codex-turn-state",
        "x-codex-turn-metadata",
    ] {
        let body = json!({"client_metadata":{field:null}});
        assert!(parse(
            &serde_json::to_vec(&body).unwrap(),
            Some(("caller", &headers))
        )
        .unwrap()
        .1
        .is_none());
    }
}

#[test]
fn conversation_id_header_presence_suppresses_fallback_without_reinterpreting_values() {
    let original = headers();
    assert!(
        parse(b"{}", Some(("caller", &original)))
            .unwrap()
            .1
            .is_some(),
        "absence must retain the ordinary fallback"
    );
    for name in ["conversation_id", "Conversation_ID", "CONVERSATION_ID"] {
        for value in [
            b"explicit-conversation".to_vec(),
            Vec::new(),
            b"invalid\tidentity".to_vec(),
            vec![0xff],
            vec![b'x'; 4097],
        ] {
            let mut headers = original.clone();
            headers.insert(
                axum::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(&value).unwrap(),
            );
            let before = headers.clone();
            for raw in [br#"{"model":"m","input":[]}"#.as_slice(), b"{", b"{} {}"] {
                match (
                    serde_json::from_slice::<Value>(raw),
                    parse(raw, Some(("caller", &headers))),
                ) {
                    (Ok(expected), Ok((actual, fallback))) => {
                        assert_eq!(actual, expected);
                        assert!(fallback.is_none(), "{name}: {value:?}");
                    }
                    (Err(expected), Err(actual)) => {
                        assert_eq!(actual.to_string(), expected.to_string())
                    }
                    (expected, actual) => {
                        panic!("changed native parsing: {expected:?} / {actual:?}")
                    }
                }
            }
            assert_eq!(headers, before, "explicit headers must remain untouched");
        }
    }
}

#[test]
fn duplicate_decoded_keys_suppress_fallback_but_preserve_native_values() {
    let headers = headers();
    for body in [
        r#"{"model":"first","mo\u0064el":"last"}"#,
        r#"{"client_metadata":{"extra":1,"ex\u0074ra":2}}"#,
        r#"{"client_metadata":{"session_id":"explicit"},"client_metadata":{}}"#,
        r#"{"previous_response_id":"explicit","previous_response_id":null}"#,
    ] {
        let (value, fallback) = parse(body.as_bytes(), Some(("caller", &headers))).unwrap();
        assert_eq!(value, serde_json::from_str::<Value>(body).unwrap());
        assert!(fallback.is_none());
    }
    let body = br#"{"client_metadata":{"extension":{"value":1,"value":2}}}"#;
    let (value, fallback) = parse(body, Some(("caller", &headers))).unwrap();
    assert_eq!(value["client_metadata"]["extension"]["value"], 2);
    assert!(fallback.is_some());
}

#[test]
fn parser_preserves_serde_values_errors_and_arbitrary_precision_protocol() {
    let headers = headers();
    for body in [
        "{}",
        "[]",
        "null",
        "true",
        "false",
        "-0",
        "184467440737095516160000",
        "-123456789012345678901234567890",
        "1.234567890123456789e123",
        r#"{"value":184467440737095516160000,"other":-0.0}"#,
        r#"{"$serde_json::private::Number":"123"}"#,
        r#"{"client_metadata":{"$serde_json::private::Number":"123"}}"#,
        r#"{"extra":1,"$serde_json::private::Number":"123"}"#,
        r#"{"$serde_json::private::RawValue":"{}"}"#,
        r#"{"client_metadata":null}"#,
        r#"{"client_metadata":[]}"#,
        r#"{"value":"escaped \u2028 and \u0000"}"#,
        "{",
        "{} {}",
        "[1,]",
        r#"{"client_metadata":{"$serde_json::private::Number":123}}"#,
    ] {
        let expected = serde_json::from_str::<Value>(body);
        let actual = parse(body.as_bytes(), Some(("caller", &headers)));
        match (expected, actual) {
            (Ok(expected), Ok((actual, _))) => assert_eq!(actual, expected, "{body}"),
            (Err(_), Err(_)) => {}
            (expected, actual) => {
                panic!("different native parse result for {body}: {expected:?} / {actual:?}")
            }
        }
    }
    for body in [
        r#"{"$serde_json::private::Number":"123"}"#,
        r#"{"client_metadata":{"$serde_json::private::Number":"123"}}"#,
        r#"{"extra":1,"$serde_json::private::Number":"123"}"#,
    ] {
        assert!(parse(body.as_bytes(), Some(("caller", &headers)))
            .unwrap()
            .1
            .is_none());
    }
    let deep = format!("{{\"value\":{}0{}}}", "[".repeat(129), "]".repeat(129));
    assert!(serde_json::from_str::<Value>(&deep).is_err());
    assert!(parse(deep.as_bytes(), Some(("caller", &headers))).is_err());
}

#[test]
fn only_conversation_http_paths_are_eligible() {
    for path in [
        "/v1/responses",
        "/responses",
        "/v1/chat/completions/v1/responses",
        "/v1/chat/completions",
        "/chat/completions",
        "/v1/completions",
        "/completions",
    ] {
        assert!(conversation_path(path));
    }
    for path in [
        "/v1/responses/compact",
        "/v1/images/generations",
        "/v1/audio/transcriptions",
        "/v1/messages",
        "/v1/alpha/search",
        "/v1/responses/ws",
        "/unknown",
    ] {
        assert!(!conversation_path(path));
    }
}
