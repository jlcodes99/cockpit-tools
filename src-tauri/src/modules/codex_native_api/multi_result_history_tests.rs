//! Reproduce the old admission failure without treating mock upstream acceptance as protocol proof.
use super::*;

fn multi_result_history(shape: &str) -> Value {
    let call =
        json!({"type":"function_call","name":"read_file","call_id":"same-call","arguments":"{}"});
    let first = json!({"type":"function_call_output","call_id":"same-call","output":"First result\n  keep whitespace  "});
    let second = json!({"type":"function_call_output","call_id":"same-call","output":"Second distinct result"});
    let mut items = vec![
        json!({"role":"user","content":"Original task"}),
        json!({"type":"reasoning","summary":[],"encrypted_content":crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature(82)}),
        call.clone(),
    ];
    match shape {
        "separate_results" => items.extend([first, second]),
        "content_parts" => items.push(
            json!({"type":"function_call_output","call_id":"same-call","output":[
                {"type":"input_text","text":"First result\n  keep whitespace  "},
                {"type":"input_text","text":"Second distinct result"}
            ]}),
        ),
        "parallel_calls" => {
            let mut other_call = call;
            other_call["call_id"] = json!("other-call");
            let mut other_result = second;
            other_result["call_id"] = json!("other-call");
            items.extend([other_call, first, other_result]);
        }
        _ => panic!("unknown synthetic history shape"),
    }
    items.push(json!({"role":"user","content":"Continue the same conversation"}));
    json!({"model":"test","client_metadata":{"session_id":"original-session","thread_id":"original-thread"},"input":items})
}

#[test]
fn multi_result_history_reproduces_the_previous_admission_error_after_reopening_storage() {
    for shape in ["separate_results", "content_parts", "parallel_calls"] {
        let directory = tempfile::tempdir().unwrap();
        let original = multi_result_history(shape);
        let saved = original.clone();
        // mapping.rs previously used this strict entry point for ordinary requests.
        for _ in 0..2 {
            let store = IdentityStore::open(directory.path().join("identity.sqlite")).unwrap();
            let result = store.inspect_for_rebuild("caller", &HeaderMap::new(), &original);
            if shape == "separate_results" {
                let error = result.unwrap_err();
                assert_eq!(error.code, "continuation_history_incomplete");
                assert_eq!(error.message, "Continuation requires complete conversation input, including matching tool calls and results; the original history was preserved");
                let mut plaintext = original.clone();
                plaintext["input"].as_array_mut().unwrap().remove(1);
                assert!(
                    store
                        .inspect_for_rebuild("caller", &HeaderMap::new(), &plaintext)
                        .is_ok(),
                    "the failure also requires unregistered opaque history"
                );
            } else {
                assert!(result.is_ok(), "control history was rejected: {shape}");
            }
            assert_eq!(original, saved);
        }
    }
}

#[tokio::test]
async fn multi_result_history_full_resend_survives_native_http_sse_and_service_restart() {
    for chatgpt in [false, true] {
        for streaming in [false, true] {
            for shape in ["separate_results", "content_parts", "parallel_calls"] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("identity.sqlite");
                let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
                let app = Router::new().route("/v1/responses", post({
                    let seen = seen.clone();
                    move |bytes: Bytes| {
                        let seen = seen.clone();
                        async move {
                            seen.lock().unwrap().push(serde_json::from_slice(&bytes).unwrap());
                            let event = json!({"type":"response.completed","response":{"id":"accepted-multi-result","status":"completed","output":[]}});
                            Response::builder().header("content-type", "text/event-stream")
                                .body(Body::from(format!("data: {event}\n\n"))).unwrap()
                        }
                    }
                }));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let host = Arc::new(HostFixture {
                    url: format!("http://{}/v1", listener.local_addr().unwrap()),
                    chatgpt,
                    selections: Mutex::new(vec![]),
                });
                let upstream =
                    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
                let client = reqwest::Client::builder().no_proxy().build().unwrap();
                let mut original = multi_result_history(shape);
                original["stream"] = json!(streaming);
                let saved = original.clone();
                // Both services open the same database; no reset, migration or history edit.
                for pass in 0..2 {
                    let mut service = crate::modules::codex_native_api::start(
                        "127.0.0.1:0".parse().unwrap(),
                        host.clone(),
                        path.clone(),
                        Options::default(),
                    )
                    .await
                    .unwrap();
                    let response = client
                        .post(format!("http://{}/v1/responses", service.addr))
                        .bearer_auth("test")
                        .json(&original)
                        .send()
                        .await
                        .unwrap();
                    let status = response.status();
                    let payload = response.text().await.unwrap();
                    assert_eq!(
                        status, 200,
                        "{shape}: oauth={chatgpt}, stream={streaming}, pass={pass}: {payload}"
                    );
                    assert!(payload.contains("accepted-multi-result"), "{payload}");
                    assert!(
                        !payload.contains("continuation_history_incomplete"),
                        "{payload}"
                    );
                    {
                        let requests = seen.lock().unwrap();
                        assert_eq!(
                            requests.len(),
                            pass + 1,
                            "must transmit exactly once per resend"
                        );
                        assert_eq!(
                            requests[pass]["input"], saved["input"],
                            "must preserve every result and call_id"
                        );
                    }
                    assert_eq!(original, saved);
                    if pass == 1 && chatgpt && shape == "separate_results" {
                        // Successful forwarding does not prove this history is safe to replay.
                        // OAuth HTTP still needs a complete checkpoint for response-ID-only input.
                        let delta = json!({"model":"test","stream":streaming,
                            "client_metadata":saved["client_metadata"],
                            "previous_response_id":"accepted-multi-result",
                            "input":[{"role":"user","content":"Only the new turn"}]});
                        let response = client
                            .post(format!("http://{}/v1/responses", service.addr))
                            .bearer_auth("test")
                            .json(&delta)
                            .send()
                            .await
                            .unwrap();
                        assert_eq!(response.status(), 409);
                        let payload = response.text().await.unwrap();
                        assert!(
                            payload.contains(
                                "Previous response history is unavailable for HTTP continuation"
                            ),
                            "{payload}"
                        );
                        assert_eq!(seen.lock().unwrap().len(), 2);
                    }
                    service.shutdown(Duration::from_secs(2)).await.unwrap();
                }
                upstream.abort();
            }
        }
    }
}
