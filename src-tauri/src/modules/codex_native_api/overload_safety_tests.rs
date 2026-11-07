//! Explicit execution evidence must win over status retry, failover and capacity hints.
use super::*;

#[tokio::test]
async fn http_503_with_executed_tool_never_replays() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        http_status: 503, delayed: false,
        error: json!({"type":"response.failed","error":{"code":"server_is_overloaded"},
            "response":{"output":[{"type":"function_call","name":"write_file","call_id":"already-executed","arguments":"{}"}]}}),
        ..Default::default()
    }).await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap();
    let _ = response.bytes().await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(
        host.inner.calls[0].load(Ordering::SeqCst),
        1,
        "a tool-bearing rejection must never reissue the request"
    );
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
}

fn executed_rejections() -> Vec<Value> {
    let tool = json!({"type":"function_call","name":"write_file","call_id":"already-executed","arguments":"{}"});
    vec![
        json!({"error":{"code":"queue_full"},"response":{"output":[tool.clone()]}}),
        json!({"error":{"code":"server_overloaded"},"output":[tool.clone()]}),
        json!({"error":{"code":"flex_unavailable"},"tool_calls":[tool.clone()]}),
        json!({"error":{"code":"server_is_overloaded"},"usage":{"output_tokens":1}}),
        json!({"error":{"code":"server_is_overloaded"},"response":{"usage":{"output_tokens_details":{"reasoning_tokens":1}}}}),
        json!({"error":{"code":"server_is_overloaded"},"metadata":{"arguments":"partial tool arguments"}}),
        json!({"body":{"error":{"code":"server_is_overloaded"},"response":{"output":[tool]}}}),
    ]
}

#[tokio::test]
async fn execution_evidence_blocks_status_retries_and_account_failover() {
    for error in executed_rejections() {
        for allow_alternates in [false, true] {
            let (host, mut handle, task, _temp) = fixture(Scenario {
                http_status: 503,
                error: error.clone(),
                status_retries: 2,
                allow_alternates,
                delayed: false,
                ..Default::default()
            })
            .await;
            let response = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("fixture")
                .json(&input(false))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), 503);
            let _ = response.bytes().await.unwrap();
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
            let requests = host.inner.requests.lock().unwrap();
            assert_eq!(
                requests.len(),
                1,
                "execution evidence must prevent every replay: {error}"
            );
            assert_eq!(requests[0].0, 0);
            assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
            assert_eq!(host.inner.reports.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn available_alternate_wins_before_capacity_fallback_for_http_and_sse() {
    for http_status in [200, 503] {
        for delayed in [false, true] {
            for streaming in [false, true] {
                let (host, mut handle, task, _temp) = fixture(Scenario {
                    http_status,
                    delayed,
                    allow_alternates: true,
                    failures: usize::MAX,
                    ..Default::default()
                })
                .await;
                let response = client()
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("fixture")
                    .json(&input(streaming))
                    .send()
                    .await
                    .unwrap();
                let text = response.text().await.unwrap();
                assert!(text.contains("completed"), "{text}");
                handle.shutdown(Duration::from_secs(2)).await.unwrap();
                task.abort();
                assert_eq!(
                    host.inner
                        .requests
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|request| request.0)
                        .collect::<Vec<_>>(),
                    vec![0, 1]
                );
                assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
                let reports = host.inner.reports.lock().unwrap();
                assert!(reports[0].success);
                assert!(reports[0]
                    .diagnostics
                    .as_ref()
                    .unwrap()
                    .attempts
                    .iter()
                    .all(|attempt| attempt.overload_retry.is_none()));
            }
        }
    }
}

#[tokio::test]
async fn configured_status_budget_is_retained_before_normal_failover() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        http_status: 503,
        status_retries: 2,
        allow_alternates: true,
        failures: usize::MAX,
        delayed: false,
        ..Default::default()
    })
    .await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains("completed"), "{text}");
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(
        host.inner
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.0)
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 1]
    );
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn response_configuration_and_tool_declarations_do_not_block_failover() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        http_status: 503, allow_alternates: true, delayed: false,
        error: json!({"object":"response","status":"failed","output":[],"usage":{"output_tokens":0},
            "error":{"code":"queue_full"},"reasoning":{"effort":"high","summary":"auto"},
            "text":{"format":{"type":"text"}},"tools":[{"type":"function","name":"write_file","parameters":{"type":"object","properties":{"content":{"type":"string"}}}}]}),
        ..Default::default()
    }).await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains("completed"), "{text}");
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(
        host.inner
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.0)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[tokio::test]
async fn sse_execution_and_unknown_tool_state_disable_every_replay() {
    for event in [
        json!({"type":"response.output_item.added","item":{"type":"function_call","name":"write_file","call_id":"started","arguments":"{}"}}),
        json!({"type":"response.function_call_arguments.delta","delta":"partial arguments"}),
        json!({"type":"response.server_tool.started","tool_call_id":"unknown-execution-state"}),
        json!({"type":"response.failed","response":{"error":{"code":"server_overloaded"},"usage":{"output_tokens":1},"output":[]}}),
    ] {
        for allow_alternates in [false, true] {
            for streaming in [false, true] {
                let (host, mut handle, task, _temp) = fixture(Scenario {
                    prefix: vec![created(), event.clone()],
                    allow_alternates,
                    status_retries: 2,
                    ..Default::default()
                })
                .await;
                let _ = client()
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("fixture")
                    .json(&input(streaming))
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap();
                handle.shutdown(Duration::from_secs(2)).await.unwrap();
                task.abort();
                assert_eq!(host.inner.requests.lock().unwrap().len(), 1, "{event}");
                assert_eq!(host.refreshes.load(Ordering::SeqCst), 0);
            }
        }
    }
}

#[tokio::test]
async fn tool_started_on_capacity_retry_cannot_trigger_a_third_send() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        failures: usize::MAX,
        retry_prefix: Some(vec![
            created(),
            json!({"type":"response.function_call_arguments.delta","delta":"already-started-tool"}),
        ]),
        ..Default::default()
    })
    .await;
    let _ = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(host.inner.requests.lock().unwrap().len(), 2);
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn same_account_fallback_does_not_pin_a_later_available_alternate() {
    let (host, mut handle, task, _temp) = fixture(Scenario {
        http_status: 503,
        failures: usize::MAX,
        alternates_after_refresh: true,
        delayed: false,
        ..Default::default()
    })
    .await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&input(true))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains("completed"), "{text}");
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    assert_eq!(
        host.inner
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.0)
            .collect::<Vec<_>>(),
        vec![0, 0, 1]
    );
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
}
