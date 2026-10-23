use super::*;

#[tokio::test]
async fn websocket_unknown_error_without_status_is_forwarded_without_retry() {
    let event =
        json!({"type":"error","error":{"code":"mystery","message":"Unknown upstream failure"}});
    let fixture = Fixture::start(Behavior::FirstError(event.clone()), policy(), false).await;
    let events = fixture.request(request(), 1).await;
    assert_eq!(events[0], event);
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.host.selections.lock().unwrap().len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_numeric_string_error_status_still_retries_before_output() {
    for event in [
        json!({"type":"error","status_code":"503","error":{"code":"server_error","message":"Provider failed"}}),
        json!({"type":"error","status":200,"error":{"status_code":"503","code":"server_error","message":"Provider failed"}}),
    ] {
        let fixture = Fixture::start(Behavior::FirstError(event), policy(), false).await;
        let events = fixture.request(request(), 1).await;
        assert_eq!(events[0]["type"], "response.completed");
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.host.failures.lock().unwrap()[0].1, 503);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_request_fault_first_event_keeps_original_error_without_account_penalty() {
    for (status, code) in [
        (403, "cyber_policy"),
        (401, "invalid_request_error"),
        (503, "invalid_request_error"),
    ] {
        let event = json!({"type":"error","status":status,"error":{"code":code,"type":"invalid_request_error","message":"Rejected request"}});
        let fixture = Fixture::start(Behavior::FirstError(event.clone()), policy(), false).await;
        let events = fixture.request(request(), 1).await;
        assert_eq!(events[0], event);
        assert_eq!(fixture.host.selections.lock().unwrap().len(), 1);
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.host.reports.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let reports = fixture.host.reports.lock().unwrap();
        assert!(
            reports
                .last()
                .unwrap()
                .diagnostics
                .as_ref()
                .unwrap()
                .upstream_request_scoped
        );
        assert!(!reports.last().unwrap().credential_scope);
        drop(reports);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_handshake_body_classification_precedes_refresh_and_cooling() {
    for (status, body, request_scoped) in [
        (
            403,
            json!({"error":{"code":"cyber_policy","type":"invalid_request_error","message":"Rejected request"}}),
            true,
        ),
        (
            401,
            json!({"error":{"code":"invalid_request_error","message":"Malformed request"}}),
            true,
        ),
        (
            401,
            json!({"error":{"code":"invalid_request_error","type":"authentication_error","message":"Expired key"}}),
            false,
        ),
        (
            402,
            json!({"error":{"code":"invalid_request_error","message":"Payment required"}}),
            false,
        ),
        (
            429,
            json!({"error":{"code":"invalid_request_error","message":"Quota exhausted"}}),
            false,
        ),
        (
            400,
            json!({"error":{"code":"model_not_found","type":"invalid_request_error","message":"Model missing"}}),
            false,
        ),
    ] {
        let fixture = Fixture::start(Behavior::HandshakeBody(status, body), policy(), false).await;
        if status == 401 {
            fixture.host.refresh_fails.store(true, Ordering::SeqCst);
        }
        let events = fixture.request(request(), 1).await;
        if request_scoped {
            assert_eq!(events[0]["status"], status);
            assert_eq!(fixture.host.selections.lock().unwrap().len(), 1);
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(
                events[0]["type"], "response.completed",
                "status={status}, events={events:?}"
            );
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
            assert!(!fixture.host.failures.lock().unwrap().is_empty());
        }
        fixture.finish().await;
    }
}
