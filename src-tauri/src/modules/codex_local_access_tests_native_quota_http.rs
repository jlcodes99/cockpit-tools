// Real native selection with synthetic accounts and loopback upstreams only.
// No production logs, credentials, quota mutation or account-health writes.

struct NativeQuotaHttpHost {
    bridge: TokioMutex<NativeHostBridge>,
    upstream: String,
    selections: Mutex<Vec<native_contract::RouteRequest>>,
    reports: Mutex<Vec<native_contract::RequestReport>>,
    entered: tokio::sync::Notify,
}

impl native_contract::Host for NativeQuotaHttpHost {
    fn authorize<'a>(
        &'a self,
        bearer: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Caller, native_contract::ServiceError>,
    > {
        Box::pin(async move {
            let bridge = self.bridge.lock().await;
            native_contract::Host::authorize(&*bridge, bearer).await
        })
    }

    fn route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            self.entered.notify_one();
            let bridge = self.bridge.lock().await;
            let mut route = native_select_route(&bridge, caller, request).await?;
            // Only transport destination/client change. Preserve real selected
            // account, owner, installation identity, headers and OAuth policy.
            route.base_url = format!("{}/{}", self.upstream, route.account_id);
            route.client = reqwest::Client::builder().no_proxy().build().unwrap();
            Ok(route)
        })
    }

    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        Box::pin(async move {
            let bridge = self.bridge.lock().await;
            native_contract::Host::models(&*bridge, caller).await
        })
    }

    fn execution_policy(
        &self,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<native_contract::ExecutionPolicy, native_contract::ServiceError>,
    > {
        Box::pin(async {
            Ok(native_contract::ExecutionPolicy {
                account_switch_rebuild: true,
                total_timeout: Duration::from_secs(8),
                open_timeout: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(5),
                retry_max_delay: Duration::from_secs(3),
                status_retries: 0,
                send_retries: 0,
                bootstrap_retries: 0,
                ..Default::default()
            })
        })
    }

    fn record(
        &self,
        report: native_contract::RequestReport,
    ) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

fn native_quota_http_synthetic_cipher() -> String {
    use base64::Engine;
    // Same fixture shape as codex_cpa_wire::synthetic_gpt_signature: a
    // Fernet-shaped transport value only, NOT valid cryptography or ownership.
    let mut bytes = vec![91; 73];
    bytes[0] = 0x80;
    bytes[1..9].fill(0);
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

#[tokio::test]
async fn native_quota_http_multi_result_history_waits_for_original_owner_without_rewriting() {
    use axum::{body::Body, routing::post, Json, Router};
    use base64::Engine;

    let calls = Arc::new(Mutex::new(
        Vec::<(String, i64, axum::http::HeaderMap, Value)>::new(),
    ));
    let first_output = json!([
        {"type":"reasoning","id":"rs_quota_fixture","summary":[],"encrypted_content":native_quota_http_synthetic_cipher()},
        {"type":"custom_tool_call","id":"ctc_quota_fixture","call_id":"call_quota_fixture","name":"fixture_tool","input":"original input","status":"completed"}
    ]);
    let app = Router::new().route("/{account}/responses", post({
        let calls = calls.clone();
        let first_output = first_output.clone();
        move |axum::extract::Path(account): axum::extract::Path<String>, headers: axum::http::HeaderMap, Json(body): Json<Value>| {
            let calls = calls.clone();
            let first_output = first_output.clone();
            async move {
                let count = {
                    let mut calls = calls.lock().unwrap();
                    calls.push((account, now_ms(), headers, body));
                    calls.len()
                };
                let output = if count == 1 { first_output } else {
                    json!([{"type":"message","id":"msg_quota_resumed","role":"assistant","content":[{"type":"output_text","text":"resumed"}]}])
                };
                let id = format!("quota-http-{count}");
                let mut events = String::new();
                for (index, item) in output.as_array().unwrap().iter().enumerate() {
                    events.push_str(&format!("data: {}\n\n", json!({"type":"response.output_item.done","output_index":index,"item":item})));
                }
                events.push_str(&format!("data: {}\n\n", json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output}})));
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(events)).unwrap()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let mut bridge = native_test_routing_host();
    bridge.profile_dir = Some(root.path().to_path_buf());
    let collection = bridge.collection_override.as_mut().unwrap();
    collection.session_affinity = false;
    collection.max_retry_interval_ms = 3_000;
    collection.bound_oauth_quota_reserve = None;
    let model = api_service_routable_codex_model_ids()
        .into_iter()
        .find(|model| !native_is_reserve_model(model))
        .unwrap();
    let mut owner = native_test_account_with_quota(0, 70);
    owner.id = "pool-a".into();
    owner.plan_type = Some("plus".into());
    owner.egress_proxy_disabled = true;
    owner.tokens.refresh_token = None;
    owner.tokens.access_token = format!(
        "e30.{}.fixture",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"exp":chrono::Utc::now().timestamp()+86400}).to_string())
    );
    // An already elapsed old window permits the initial response without ever
    // changing the 0% value. A new future window is installed once below.
    owner.quota.as_mut().unwrap().hourly_reset_time = Some(chrono::Utc::now().timestamp() - 1);
    bridge.account_overrides.insert(owner.id.clone(), owner);
    bridge
        .account_overrides
        .get_mut("pool-b")
        .unwrap()
        .api_model_catalog = vec![model.clone()];
    let host = Arc::new(NativeQuotaHttpHost {
        bridge: TokioMutex::new(bridge),
        upstream,
        selections: Mutex::new(Vec::new()),
        reports: Mutex::new(Vec::new()),
        entered: tokio::sync::Notify::new(),
    });
    let mut service = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let url = format!("http://{}/v1/responses", service.addr);
    let send = |body: Value| {
        client
            .post(&url)
            .bearer_auth("fixture-primary-key")
            .header("session-id", "quota-http-session")
            .header("thread-id", "quota-http-thread")
            .json(&body)
            .send()
    };
    let user = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"original task"}]});
    let first = send(json!({"model":model,"stream":false,"input":[user.clone()]}))
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let first: Value = first.json().await.unwrap();
    assert_eq!(first["output"], first_output);
    assert_eq!(calls.lock().unwrap()[0].0, "pool-a");
    // Drain the notification from the completed initial selection.
    host.entered.notified().await;
    let mut input = vec![user];
    input.extend(first["output"].as_array().unwrap().iter().cloned());
    let call_id = first["output"][1]["call_id"].as_str().unwrap();
    input.push(json!({"type":"custom_tool_call_output","call_id":call_id,"output":"first result\n  preserve spaces  "}));
    input.push(json!({"type":"custom_tool_call_output","call_id":call_id,"output":[{"type":"input_text","text":"second distinct result"}]}));
    input.push(
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}),
    );
    let continuation = json!({"model":model,"stream":false,"input":input});
    // There is deliberately no previous_response_id: the full history is
    // forwarded on its accepted owner, not expanded or rebuilt across owners.
    let reset_at = chrono::Utc::now().timestamp() + 2;
    let quota_before = {
        let mut bridge = host.bridge.lock().await;
        let quota = bridge
            .account_overrides
            .get_mut("pool-a")
            .unwrap()
            .quota
            .as_mut()
            .unwrap();
        quota.hourly_reset_time = Some(reset_at);
        assert_eq!(quota.hourly_percentage, 0);
        serde_json::to_value(&*quota).unwrap()
    };
    let pending = send(continuation.clone());
    tokio::pin!(pending);
    tokio::select! {
        response = &mut pending => panic!("continuation terminated before waiting for quota: {:?}", response.map(|r| r.status())),
        _ = host.entered.notified() => {}
    }
    assert!(now_ms() < reset_at * 1000);
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "neither owner nor backup may receive a request before reset"
    );
    let response = pending.await.unwrap();
    let status = response.status();
    let result: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["id"], "quota-http-2");
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls.iter().all(|(account, _, _, _)| account == "pool-a"),
            "backup B must never receive this history"
        );
        assert!(
            calls[1].1 >= reset_at * 1000,
            "no upstream dispatch before the actual reset"
        );
        assert_eq!(
            calls[1].3["input"], continuation["input"],
            "every result, ID, whitespace and cipher must survive unchanged"
        );
        for header in [
            "session-id",
            "thread-id",
            "x-codex-installation-id",
            "authorization",
        ] {
            assert_eq!(
                calls[0].2.get(header),
                calls[1].2.get(header),
                "same owner must preserve {header}"
            );
        }
    }
    {
        let selected = host.selections.lock().unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[1].preferred_account_id.as_deref(), Some("pool-a"));
        assert!(selected[1].excluded_account_ids.is_empty());
    }
    {
        let bridge = host.bridge.lock().await;
        assert_eq!(
            serde_json::to_value(bridge.account_overrides["pool-a"].quota.as_ref().unwrap())
                .unwrap(),
            quota_before,
            "recovery must depend on elapsed time, not a rewritten quota snapshot"
        );
        assert!(bridge.cooldowns.lock().unwrap().is_empty());
        assert!(bridge
            .gates
            .lock()
            .unwrap()
            .values()
            .all(|gate| gate.available_permits() == 1));
    }
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports
        .iter()
        .all(|report| report.account_id == "pool-a" && report.status == 200));
    drop(reports);
    task.abort();
    let _ = task.await;
}
