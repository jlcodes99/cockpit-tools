use super::*;

async fn request_sse(fixture: &Fixture) -> (u16, String) {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("local")
        .json(&json!({"model":"model","input":"fixture","stream":true}))
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

#[tokio::test]
async fn cpa_error_policy_refusal_preserves_message_without_penalizing_accounts() {
    for (error, code, message) in [
        (
            json!({"error":{},"body":{"error":{"code":"bio_policy","message":"Synthetic policy refusal"}},
            "sequence_number":7}),
            "bio_policy",
            "Synthetic policy refusal",
        ),
        (
            json!({"error":"Invalid prompt: we've limited access to this content for safety reasons.",
            "sequence_number":7}),
            "content_policy_violation",
            "Invalid prompt: we've limited access to this content for safety reasons.",
        ),
        (
            json!({"type":"response.failed","response":{"status":"failed",
                "error":{"code":"bio_policy","message":"  "}},"sequence_number":7}),
            "bio_policy",
            "Upstream rejected the request",
        ),
    ] {
        for websocket in [false, true] {
            let fixture =
                Fixture::start(Behavior::FirstError(error.clone()), policy(), false).await;
            let (status, error) = if websocket {
                let frame = fixture.request(request(), 1).await.remove(0);
                (frame["status"].as_u64().unwrap_or_default() as u16, frame)
            } else {
                let (status, body) = request_sse(&fixture).await;
                assert_eq!(status, 400, "{body}");
                (status, serde_json::from_str(&body).unwrap())
            };
            assert_eq!(status, 400, "{error}");
            assert_eq!(error["error"]["code"], code);
            assert_eq!(error["error"]["message"], message);
            if error["type"] == "response.failed" {
                assert_eq!(error["response"]["error"]["message"], message);
            }
            assert_eq!(error["sequence_number"], 7);
            assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn cpa_error_named_metadata_does_not_consume_quota_or_end_the_response() {
    let metadata = json!({"type":"response.created","response":{"id":"completed"},
        "body":{"error":{"code":"usage_limit_reached","message":"Synthetic embedded data"}}});
    for websocket in [false, true] {
        let fixture = Fixture::start(
            Behavior::MetadataThenSuccess(metadata.clone()),
            policy(),
            false,
        )
        .await;
        if websocket {
            let events = fixture.request(request(), 2).await;
            assert_eq!(events[0]["type"], "response.created");
            assert_eq!(events[1]["type"], "response.completed");
        } else {
            let (status, body) = request_sse(&fixture).await;
            assert_eq!(status, 200, "{body}");
            assert!(
                body.contains("event: response.created") && body.contains("response.completed"),
                "{body}"
            );
        }
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        let reports = fixture.host.reports.lock().unwrap();
        assert!(reports.last().unwrap().success);
        assert_eq!(reports.last().unwrap().status, 200);
        drop(reports);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn cpa_error_named_sse_failure_uses_its_wrapped_error_details() {
    let error = json!({"type":"invalid_request_error","body":{"error":{
        "code":"content_policy_violation","message":"Synthetic nested policy refusal"
    }}});
    let fixture = Fixture::start(Behavior::FirstError(error), policy(), false).await;
    let (status, body) = request_sse(&fixture).await;
    assert_eq!(status, 400, "{body}");
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "content_policy_violation");
    assert_eq!(
        response["error"]["message"],
        "Synthetic nested policy refusal"
    );
    assert!(fixture.host.failures.lock().unwrap().is_empty());
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn cpa_error_model_capacity_and_quota_keep_distinct_retry_hints_and_scope() {
    for (status, code, retry_after, credential) in [
        (503, "server_is_overloaded", 17, false),
        (429, "usage_limit_reached", 37, true),
    ] {
        for websocket in [false, true] {
            let error = json!({"status":200,"body":{"error":{"status_code":status.to_string(),
                "code":code,"message":"Synthetic upstream failure",
                "headers":{"Retry-After":[retry_after.to_string()]}}}});
            let gateway_policy = ExecutionPolicy {
                status_retries: 0,
                ..policy()
            };
            let fixture = Fixture::start(Behavior::FirstError(error), gateway_policy, false).await;
            if websocket {
                let result = fixture.request(request(), 1).await;
                assert_eq!(result[0]["response"]["id"], "completed-b", "{result:?}");
            } else {
                let (status, body) = request_sse(&fixture).await;
                assert_eq!(status, 200, "{body}");
                assert!(body.contains("http-backup"), "{body}");
            }
            assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(
                *fixture.host.failures.lock().unwrap(),
                vec![("a".into(), status, Some(Duration::from_secs(retry_after)))],
            );
            assert_eq!(
                *fixture.host.failure_scopes.lock().unwrap(),
                vec![credential]
            );
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn cpa_error_wrapped_capacity_after_websocket_handshake_can_switch() {
    let fixture = Fixture::start(
        Behavior::MetadataThenError(
            json!({"type":"response.created","response":{"id":"opening","output":[]}}),
            json!({"body":{"error":{"status_code":503,"code":"server_is_overloaded",
                "message":"Synthetic overload","headers":{"Retry-After":["17"]}}}}),
        ),
        ExecutionPolicy {
            status_retries: 0,
            ..policy()
        },
        false,
    )
    .await;
    let events = fixture.request(request(), 1).await;
    assert_eq!(events[0]["response"]["id"], "completed-b", "{events:?}");
    assert_eq!(*fixture.host.failure_scopes.lock().unwrap(), vec![false]);
    fixture.finish().await;
}

#[tokio::test]
async fn cpa_error_wrapped_quota_reaches_websocket_client_when_alternates_fail() {
    let error = json!({"type":"error","status":200,"body":{"error":{"status_code":429,
        "code":"usage_limit_reached","message":"Synthetic quota exhaustion"},
        "headers":{"Retry-After":["37"]}}});
    let fixture = Fixture::start_with_second(
        Behavior::FirstError(error.clone()),
        Behavior::FirstError(error),
        ExecutionPolicy {
            status_retries: 0,
            ..policy()
        },
        false,
    )
    .await;
    let events = fixture.request(request(), 1).await;
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    let event = &events[0];
    assert_eq!(event["type"], "error");
    assert_eq!(event["status"], 429, "{event}");
    assert_eq!(event["error"]["code"], "usage_limit_reached");
    assert_eq!(event["error"]["message"], "Synthetic quota exhaustion");
    assert_eq!(event["retry_after"].as_f64(), Some(37.0));
    fixture.finish().await;
}

#[tokio::test]
async fn cpa_error_generic_rate_limit_preserves_status_and_remains_distinct_from_quota() {
    for error in [
        json!({"error":{"type":"rate_limit_error","message":"Fixture rate limit"},
            "headers":{"retry-after":"11"}}),
        json!({"body":{"status":429,"error":{"message":"Fixture rate limit"}},
            "headers":{"retry-after":"11"}}),
    ] {
        for websocket in [false, true] {
            let fixture = Fixture::start(
                Behavior::FirstError(error.clone()),
                ExecutionPolicy {
                    status_retries: 0,
                    ..policy()
                },
                false,
            )
            .await;
            if websocket {
                let events = fixture.request(request(), 1).await;
                assert_eq!(events[0]["response"]["id"], "completed-b", "{events:?}");
            } else {
                let (status, body) = request_sse(&fixture).await;
                assert_eq!(status, 200, "{body}");
                assert!(body.contains("http-backup"), "{body}");
            }
            assert_eq!(
                *fixture.host.failures.lock().unwrap(),
                vec![("a".into(), 429, Some(Duration::from_secs(11)))]
            );
            assert_eq!(*fixture.host.failure_scopes.lock().unwrap(), vec![false]);
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn cpa_error_plain_non_policy_message_reaches_websocket_client() {
    let fixture = Fixture::start(
        Behavior::FirstError(json!({"type":"error","status":400,
            "body":{"error":"Synthetic invalid signature"}})),
        policy(),
        false,
    )
    .await;
    let events = fixture.request(request(), 1).await;
    assert_eq!(events[0]["status"], 400);
    assert_eq!(events[0]["error"]["message"], "Synthetic invalid signature");
    assert_eq!(events[0]["body"]["error"], "Synthetic invalid signature");
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[test]
fn cpa_error_sse_named_response_blocks_fallback_on_a_conflicting_generic_type() {
    use super::super::super::events::{response_terminal_kind, SseDecoder};

    let event = json!({"type":"server_error","response":{"id":"opening"},
        "body":{"error":{"code":"usage_limit_reached","message":"Synthetic metadata"}}});
    let mut decoder = SseDecoder::new(8192);
    let frames = decoder
        .push_frames(format!("event: response.created\ndata: {event}\n\n").as_bytes())
        .unwrap();
    let observed = frames[0].value.as_ref().unwrap();
    assert_eq!(observed["type"], "response.created", "{observed}");
    assert_eq!(response_terminal_kind(observed), None);
}
