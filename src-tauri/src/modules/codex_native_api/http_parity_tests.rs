use super::*;
use std::io::Write;

const INGRESS_NETWORK_HEADERS: &[&str] = &[
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-forwarded-port",
    "x-forwarded-server",
    "x-real-ip",
    "true-client-ip",
    "cf-connecting-ip",
    "cf-connecting-ipv6",
    "cf-pseudo-ipv4",
    "cf-ipcountry",
    "cf-ray",
    "cf-visitor",
    "cf-warp-tag-id",
    "cf-worker",
    "cf-ew-via",
    "cdn-loop",
];

const PROVIDER_CLIENT_HEADERS: &[(&str, &str)] = &[
    ("user-agent", "codex-test-client/1.0"),
    ("originator", "codex_cli_rs"),
    ("x-openai-actor-authorization", "actor-test-value"),
    ("x-codex-turn-state", "turn-state-value"),
    ("x-codex-installation-id", "installation-test-value"),
    ("x-provider-option", "custom-provider-value"),
    ("cf-aig-authorization", "custom-provider-cf-value"),
    ("x-forwarded-custom-option", "custom-option-value"),
];

#[test]
fn ingress_network_headers_filter_exact_names_without_losing_provider_identity() {
    let mut incoming = HeaderMap::new();
    for name in INGRESS_NETWORK_HEADERS {
        // HeaderMap normalizes mixed casing from incoming proxy headers.
        incoming.insert(
            axum::http::HeaderName::from_bytes(name.to_ascii_uppercase().as_bytes()).unwrap(),
            HeaderValue::from_static("ingress-only"),
        );
    }
    for (name, value) in PROVIDER_CLIENT_HEADERS {
        incoming.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    incoming.insert("session-id", HeaderValue::from_static("session-test"));
    incoming.insert("thread-id", HeaderValue::from_static("thread-test"));
    incoming.insert(
        "authorization",
        HeaderValue::from_static("Bearer local-key"),
    );
    incoming.insert("x-api-key", HeaderValue::from_static("local-key"));
    incoming.insert(
        "chatgpt-account-id",
        HeaderValue::from_static("client-account"),
    );
    let outgoing = super::super::server::outbound_headers(&incoming);
    for name in INGRESS_NETWORK_HEADERS {
        assert!(!outgoing.contains_key(*name), "leaked {name}");
    }
    for (name, value) in PROVIDER_CLIENT_HEADERS {
        assert_eq!(outgoing[*name], *value, "lost {name}");
    }
    assert_eq!(outgoing["session-id"], "session-test");
    assert_eq!(outgoing["thread-id"], "thread-test");
    for name in ["authorization", "x-api-key", "chatgpt-account-id"] {
        assert!(!outgoing.contains_key(name), "leaked {name}");
    }
}

#[tokio::test]
async fn ingress_network_headers_never_reach_responses_or_chat_upstream() {
    for chatgpt in [false, true] {
        let seen = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
        let capture = seen.clone();
        let app = Router::new().route(
            "/v1/responses",
            post(
                move |headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
                    let capture = capture.clone();
                    async move {
                        capture.lock().unwrap().push((headers, body));
                        (
                            [("content-type", "text/event-stream")],
                            format!("{}{}", created(), completed()),
                        )
                    }
                },
            ),
        );
        let (addr, upstream) = mock(app).await;
        let (mut service, host, root) = native(addr, chatgpt, false, Options::default()).await;
        for endpoint in ["responses", "chat/completions"] {
            let chat = endpoint == "chat/completions";
            let request_body = if chat {
                json!({
                    "model":"test-model", "stream":true, "store":false,
                    "messages":[{"role":"user","content":"test"}],
                    "reasoning_effort":"high",
                    "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object","properties":{}}}}],
                    "tool_choice":"auto"
                })
            } else {
                let mut request = body(true);
                request["store"] = json!(false);
                request["tools"] = json!([{"type":"function","name":"lookup","parameters":{"type":"object","properties":{}}}]);
                request["tool_choice"] = json!("auto");
                request
            };
            let mut request = client()
                .post(format!("http://{}/{endpoint}", service.addr))
                .bearer_auth("local-test-key")
                .header("session-id", "client-session")
                .header("thread-id", "client-session")
                .header("x-api-key", "local-test-key")
                .header("chatgpt-account-id", "untrusted-client-account");
            for name in INGRESS_NETWORK_HEADERS {
                request = request.header(*name, "ingress-only");
            }
            for (name, value) in PROVIDER_CLIENT_HEADERS {
                // Continuation tokens need prior registration. The filter unit test
                // checks preservation without bypassing token ownership checks.
                if *name != "x-codex-turn-state" {
                    request = request.header(*name, *value);
                }
            }
            let response = request.json(&request_body).send().await.unwrap();
            assert_eq!(response.status(), 200, "OAuth={chatgpt}, {endpoint}");
            let output = response.text().await.unwrap();
            assert!(
                output.contains(if chat { "[DONE]" } else { "response.completed" }),
                "{output}"
            );
            let (headers, sent) = seen.lock().unwrap().last().unwrap().clone();
            for name in INGRESS_NETWORK_HEADERS {
                assert!(
                    !headers.contains_key(*name),
                    "OAuth={chatgpt}, {endpoint}: leaked {name}"
                );
            }
            for (name, value) in PROVIDER_CLIENT_HEADERS {
                if *name != "x-codex-turn-state" {
                    assert_eq!(
                        headers[*name], *value,
                        "OAuth={chatgpt}, {endpoint}: lost {name}"
                    );
                }
            }
            assert_eq!(headers["authorization"], "Bearer upstream-test-key");
            assert!(!headers.contains_key("x-api-key"));
            assert!(!headers.contains_key("chatgpt-account-id"));
            assert_ne!(headers["session-id"], "client-session");
            assert_eq!(headers["session-id"], headers["thread-id"]);
            assert_eq!(sent["model"], "test-model");
            assert_eq!(sent["stream"], true);
            assert_eq!(sent["store"], false);
            assert_eq!(sent["reasoning"]["effort"], "high");
            assert_eq!(sent["input"][0]["role"], "user");
            assert!(sent["input"][0]["content"].to_string().contains("test"));
            assert_eq!(sent["tools"][0]["name"], "lookup");
            assert_eq!(sent["tools"][0]["parameters"]["type"], "object");
            assert_eq!(sent["tool_choice"], "auto");
            if !chat {
                assert_eq!(sent["unknown"], request_body["unknown"]);
            }
            if chatgpt {
                assert_eq!(sent["service_tier"], "priority");
            }
        }
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(host.selections.lock().unwrap().len(), 2);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        upstream.abort();
        let _ = std::fs::remove_dir_all(root);
    }
}

#[tokio::test]
async fn request_decodes_mixed_case_chained_content_encoding() {
    let seen = Arc::new(Mutex::new(None));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                axum::Json(json!({"id":"compressed-response","status":"completed","output":[]}))
            }
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, _, root) = native(addr, false, false, Options::default()).await;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(body(false).to_string().as_bytes()).unwrap();
    let gzip = gzip.finish().unwrap();
    let encoded = zstd::stream::encode_all(gzip.as_slice(), 1).unwrap();
    let response = client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("content-encoding", " GZip, Identity, ZSTD ")
        .body(encoded)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["input"],
        body(false)["input"]
    );
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn response_headers_match_go_host_sanitizer() {
    let mut headers = HeaderMap::new();
    headers.append(
        "connection",
        HeaderValue::from_static("X-Upstream-Secret, upgrade"),
    );
    headers.append("connection", HeaderValue::from_static("X-Second"));
    for name in [
        "x-upstream-secret",
        "x-second",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "x-litellm-cost",
        "helicone-id",
        "x-portkey-request",
        "cf-aig-id",
        "x-kong-token",
        "x-bt-id",
        "set-cookie",
    ] {
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static("private"),
        );
    }
    headers.insert("x-request-id", HeaderValue::from_static("public-id"));
    headers.insert("retry-after", HeaderValue::from_static("3"));
    let response =
        super::super::server::downstream_response(StatusCode::OK, &headers, Body::empty());
    assert_eq!(response.headers().len(), 2);
    assert_eq!(response.headers()["x-request-id"], "public-id");
    assert_eq!(response.headers()["retry-after"], "3");
}

#[test]
fn overload_does_not_invent_a_retry_after_header() {
    use axum::response::IntoResponse;
    for status in [429, 503] {
        let response = ServiceError::new(status, "upstream_overloaded", "Busy").into_response();
        assert!(!response.headers().contains_key("retry-after"));
        let response = ServiceError::new(status, "upstream_overloaded", "Busy")
            .with_retry_after(Some(Duration::from_secs(7)))
            .into_response();
        assert_eq!(response.headers()["retry-after"], "7");
    }
}
