// Synthetic accounts and loopback upstreams only; never writes production logs.

#[test]
fn native_route_wrapper_keeps_large_preparation_state_off_the_retry_stack() {
    let host = native_test_routing_host();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let request = native_test_route_request("future-state-budget");
    let future = native_select_route(&host, &caller, request);
    // This wrapper is nested in the HTTP/SSE cooldown path. Before the heap
    // boundary it embedded a 13 KiB preparation future (14 KiB total on arm64).
    // Leave room for platform layout differences while forbidding that state
    // from being embedded again; no larger thread stack is needed.
    assert!(std::mem::size_of_val(&future) < 4096);
}

fn native_test_set_owner_cooldown(host: &NativeHostBridge, delay: Duration) {
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", "gpt-fixture", false),
        NativeCooldown {
            until: Instant::now() + delay,
            quota: false,
            backoff_level: 0,
            reason: "transient_upstream",
        },
    );
}

#[tokio::test]
async fn native_pinned_cooling_is_retryable_with_or_without_other_ready_accounts() {
    use axum::response::IntoResponse;
    for both_cooling in [false, true] {
        let mut host = native_test_routing_host();
        host.collection_override
            .as_mut()
            .unwrap()
            .max_retry_interval_ms = 0;
        native_test_set_owner_cooldown(&host, Duration::from_millis(60_500));
        if both_cooling {
            native_upsert_cooldown(
                &mut host.cooldowns.lock().unwrap(),
                "pool-b",
                "gpt-fixture",
                503,
                None,
                false,
                Instant::now(),
            );
        }
        let mut request = native_test_route_request("original-thread");
        request.preferred_account_id = Some("pool-a".into());
        let error = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
            .await
            .err()
            .unwrap();
        assert_eq!((error.status, error.code), (503, "account_cooling"));
        assert!(error.retry_after.unwrap() > Duration::from_secs(60));
        let response = error.into_response();
        assert_eq!(response.status(), 503);
        // Fractional seconds must round up, so the caller never retries early.
        assert_eq!(response.headers()["retry-after"], "61");
        assert!(host.gates.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn native_pinned_cooling_does_not_hide_unavailable_owner_or_override_disable_switch() {
    let mut host = native_test_routing_host();
    host.collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 0;
    native_test_set_owner_cooldown(&host, Duration::from_secs(60));
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_test_route_request("original-thread");
    request.preferred_account_id = Some("pool-a".into());
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .requires_reauth = true;
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(
        (error.status, error.code),
        (409, "account_binding_unavailable")
    );
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .requires_reauth = false;
    host.collection_override.as_mut().unwrap().disable_cooling = true;
    let selected = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(selected.account_id, "pool-a");
}

struct NativeCoolingContinuationHost {
    bridge: NativeHostBridge,
    short_cooldown: bool,
}

impl native_contract::Host for NativeCoolingContinuationHost {
    fn authorize<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Caller, native_contract::ServiceError>,
    > {
        native_contract::Host::authorize(&self.bridge, key)
    }
    fn route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        native_contract::Host::route(&self.bridge, caller, request)
    }
    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        native_contract::Host::models(&self.bridge, caller)
    }
    fn execution_policy(
        &self,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<native_contract::ExecutionPolicy, native_contract::ServiceError>,
    > {
        Box::pin(async move {
            Ok(native_contract::ExecutionPolicy {
                credential_retries: 0,
                retry_max_delay: Duration::from_millis(200),
                total_timeout: Duration::from_secs(3),
                immediate_sse: true,
                ..Default::default()
            })
        })
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
        max_wait: Duration,
    ) -> futures_util::future::BoxFuture<'a, Result<Option<Duration>, native_contract::ServiceError>>
    {
        native_contract::Host::retry_cooldown_wait(&self.bridge, caller, request, max_wait)
    }
    fn route_failed_with_context<'a>(
        &'a self,
        account: &'a str,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            native_contract::Host::route_failed_with_context(
                &self.bridge,
                account,
                model,
                status,
                retry_after,
                credential_scope,
                message,
            )
            .await;
            if self.short_cooldown {
                // Scale the production 60-second window for the bounded-wait test.
                native_test_set_owner_cooldown(&self.bridge, Duration::from_millis(60));
            }
        })
    }
    fn record(&self, _: native_contract::RequestReport) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

async fn native_test_cooling_continuation_roundtrip(sse_overload: bool, short_cooldown: bool) {
    use axum::{body::Body, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
    let calls = Arc::new(Mutex::new(
        Vec::<(String, axum::http::HeaderMap, Value)>::new(),
    ));
    let app = Router::new().route("/{account}/responses", post({
        let calls = calls.clone();
        move |axum::extract::Path(account): axum::extract::Path<String>, headers: axum::http::HeaderMap, Json(body): Json<Value>| {
            let calls = calls.clone();
            async move {
                let count = {
                    let mut calls = calls.lock().unwrap();
                    calls.push((account.clone(), headers, body));
                    calls.len()
                };
                if count == 2 {
                    let error = json!({"type":"error","error":{"code":"server_is_overloaded","type":"service_unavailable_error","message":"Synthetic overload"}});
                    if sse_overload {
                        return axum::response::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(Body::from(format!("data: {error}\n\n"))).unwrap();
                    }
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(error)).into_response();
                }
                Json(json!({"id":format!("resp-fixture-{count}"),"status":"completed","output":[{"type":"reasoning","id":"reason-fixture","encrypted_content":"synthetic-owned-cipher"}]})).into_response()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut bridge = native_test_routing_host();
    bridge
        .collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 200;
    for (id, account) in &mut bridge.account_overrides {
        account.api_base_url = Some(format!("http://{addr}/{id}"));
    }
    let host = Arc::new(NativeCoolingContinuationHost {
        bridge,
        short_cooldown,
    });
    let root = tempfile::tempdir().unwrap();
    let mut service = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/responses", service.addr);
    let send = |body: Value| {
        client
            .post(&url)
            .bearer_auth("fixture-primary-key")
            .header("session-id", "client-session")
            .header("thread-id", "client-thread")
            .json(&body)
            .send()
    };
    let initial = send(json!({"model":"gpt-fixture","input":[],"stream":false}))
        .await
        .unwrap();
    assert_eq!(initial.status(), 200);
    let _: Value = initial.json().await.unwrap();
    // Exercise the protected continuation without any replay checkpoint. Hot
    // cache eviction alone now leaves the durable history available for rebuild.
    // Preserve ownership so the cooldown must still pin retries to the owner.
    let connection = rusqlite::Connection::open(root.path().join("identity.sqlite")).unwrap();
    let evicted = connection
        .execute("DELETE FROM native_rebuild_checkpoints_v1", [])
        .unwrap();
    assert_eq!(evicted, 1);
    let removed = connection
        .execute("DELETE FROM native_rebuild_history_v1", [])
        .unwrap();
    assert_eq!(removed, 1);
    drop(connection);
    let body = json!({"model":"gpt-fixture","stream":false,"previous_response_id":"resp-fixture-1","input":[{"type":"reasoning","encrypted_content":"synthetic-owned-cipher"}]});
    let retry = send(body.clone()).await.unwrap();
    assert_eq!(retry.status(), if short_cooldown { 200 } else { 503 });
    let _: Value = retry.json().await.unwrap();
    if !short_cooldown {
        let blocked = send(body.clone()).await.unwrap();
        assert_eq!(blocked.status(), 503);
        assert!(
            blocked.headers()["retry-after"]
                .to_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                >= 59
        );
        let error: Value = blocked.json().await.unwrap();
        assert_eq!(error["error"]["code"], "account_cooling");
        let mut stream_body = body.clone();
        stream_body["stream"] = json!(true);
        let blocked_sse = send(stream_body).await.unwrap();
        assert_eq!(blocked_sse.status(), 200);
        let text = blocked_sse.text().await.unwrap();
        let event: Value = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["type"] == "error")
            .unwrap();
        assert_eq!(event["status"], 503);
        assert_eq!(event["error"]["code"], "account_cooling");
        assert!(event["retry_after"].as_f64().unwrap() > 59.0);
        {
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
            let mut upgrade = format!("ws://{}/v1/responses", service.addr)
                .into_client_request()
                .unwrap();
            upgrade.headers_mut().insert(
                "authorization",
                "Bearer fixture-primary-key".parse().unwrap(),
            );
            upgrade
                .headers_mut()
                .insert("session-id", "client-session".parse().unwrap());
            upgrade
                .headers_mut()
                .insert("thread-id", "client-thread".parse().unwrap());
            let (mut socket, _) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
            let mut create = body.clone();
            create["type"] = json!("response.create");
            socket
                .send(Message::Text(create.to_string().into()))
                .await
                .unwrap();
            let error = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Message::Text(frame) = socket.next().await.unwrap().unwrap() {
                        let value: Value = serde_json::from_str(frame.as_str()).unwrap();
                        if value["type"] == "error" {
                            break value;
                        }
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(error["status"], 503);
            assert_eq!(error["error"]["code"], "account_cooling");
            assert!(error["retry_after"].as_f64().unwrap() > 59.0);
        }
        assert_eq!(calls.lock().unwrap().len(), 2);
        // Next client retry waits for the remaining short window, then uses A.
        native_test_set_owner_cooldown(&host.bridge, Duration::from_millis(60));
        let resumed = send(body).await.unwrap();
        assert_eq!(resumed.status(), 200);
        let _: Value = resumed.json().await.unwrap();
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|(account, _, _)| account == "pool-a"));
    for (_, headers, body) in calls.iter().skip(1) {
        assert_eq!(headers.get("session-id"), calls[0].1.get("session-id"));
        assert_eq!(headers.get("thread-id"), calls[0].1.get("thread-id"));
        assert_eq!(body["previous_response_id"], "resp-fixture-1");
        assert_eq!(
            body["input"][0]["encrypted_content"],
            "synthetic-owned-cipher"
        );
    }
    assert!(host
        .bridge
        .gates
        .lock()
        .unwrap()
        .values()
        .all(|gate| gate.available_permits() == 1));
    drop(calls);
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn native_owned_http_and_sse_overload_resumes_original_account_after_cooling() {
    for sse in [false, true] {
        native_test_cooling_continuation_roundtrip(sse, false).await;
    }
}

#[tokio::test]
async fn native_owned_http_and_sse_overload_automatically_retries_within_wait_budget() {
    for sse in [false, true] {
        native_test_cooling_continuation_roundtrip(sse, true).await;
    }
}
