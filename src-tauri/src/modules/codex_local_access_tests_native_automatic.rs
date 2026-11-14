fn native_automatic_fixture() -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "pool-a".into(),
        "fixture@example.invalid".into(),
        "fixture-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        Some("fixture".into()),
        Some("Fixture".into()),
        vec!["client-text".into(), "client-vision".into()],
    );
    account.api_wire_api = Some("chat_completions".into());
    account.egress_proxy_disabled = true;
    account.api_model_mappings = vec![
        crate::models::codex::CodexApiModelMapping {
            client_model: "client-text".into(),
            upstream_model: "upstream-text".into(),
        },
        crate::models::codex::CodexApiModelMapping {
            client_model: "client-vision".into(),
            upstream_model: "upstream-vision".into(),
        },
    ];
    account
        .api_model_vision_support
        .insert("upstream-text".into(), false);
    account
        .api_model_vision_support
        .insert("upstream-vision".into(), true);
    account
}

#[tokio::test]
async fn pool_wakeup_uses_current_active_accounts_in_saved_order_and_cancels_without_sending() {
    use crate::models::codex::{CodexAccountIndex, CodexAccountSummary};
    use crate::modules::codex_wakeup;

    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let mut index = CodexAccountIndex::new();
    for id in ["first", "second", "added"] {
        let mut account = native_test_account();
        account.id = id.into();
        codex_account::save_account(&account).unwrap();
        index.accounts.push(CodexAccountSummary {
            id: account.id, email: account.email, plan_type: account.plan_type,
            subscription_active_until: None, created_at: account.created_at, last_used: account.last_used,
        });
    }
    codex_account::save_account_index(&index).unwrap();
    codex_account::save_account(&native_automatic_fixture()).unwrap();
    let mut collection = native_test_collection();
    collection.account_ids = ["second", "pool-a", "first", "second", "missing"].map(str::to_owned).to_vec();
    let path = local_access_file_path().unwrap();
    fs::write(&path, serde_json::to_vec(&collection).unwrap()).unwrap();
    assert_eq!(codex_wakeup::resolve_pool_wakeup_account_ids().unwrap(), ["second", "first"]);
    let task_state: codex_wakeup::CodexWakeupState = serde_json::from_value(json!({
        "enabled":true,"tasks":[{"id":"pool","name":"Fixture pool","enabled":true,
        "useApiServicePool":true,"accountIds":[],"schedule":{"kind":"startup"},
        "createdAt":1,"updatedAt":1}]
    })).unwrap();
    codex_wakeup::save_state(&task_state).unwrap();

    collection.paused_account_ids.push("second".into());
    collection.account_ids.insert(0, "added".into());
    fs::write(&path, serde_json::to_vec(&collection).unwrap()).unwrap();
    let ids = codex_wakeup::resolve_pool_wakeup_account_ids().unwrap();
    assert_eq!(ids, ["added", "first"]);
    let restored = codex_wakeup::load_state().unwrap();
    assert!(restored.tasks[0].use_api_service_pool);
    assert!(restored.tasks[0].account_ids.is_empty());

    let scope = "fixture-pool-wakeup-cancelled";
    codex_wakeup::cancel_wakeup_scope(scope).unwrap();
    let result = codex_wakeup::run_batch(
        None, ids, None,
        codex_wakeup::CodexWakeupExecutionConfig { model: None, model_display_name: None, model_reasoning_effort: None },
        codex_wakeup::TaskRunContext { trigger_type: "startup".into(), task_id: Some("pool".into()), task_name: Some("Fixture pool".into()) },
        None, Some(scope),
    ).await;
    codex_wakeup::release_wakeup_scope(scope).unwrap();
    let batch = result.unwrap();
    assert_eq!(batch.records.iter().map(|record| record.account_id.as_str()).collect::<Vec<_>>(), ["added", "first"]);
    assert_eq!(batch.success_count, 0);
    assert!(batch.records.iter().all(|record| record.error.as_deref().is_some_and(|error| error.contains("已取消"))));
}

#[tokio::test]
async fn text_only_responses_pool_handles_both_endpoints_and_service_probe() {
    use axum::{http::StatusCode, response::IntoResponse, routing::post, Json, Router};

    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observed_by_upstream = observed.clone();
    let router = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let observed = observed_by_upstream.clone();
        async move {
            observed.lock().unwrap().push(body.clone());
            if native_contains_image_tool(&body) {
                return (StatusCode::FORBIDDEN, Json(json!({"error":{"message":"Image generation is not enabled for this group","type":"permission_error"}}))).into_response();
            }
            let message = json!({"type":"message","id":"msg_fixture","role":"assistant","content":[{"type":"output_text","text":"pong"}]});
            let events = [
                json!({"type":"response.output_item.done","item":message}),
                json!({"type":"response.completed","response":{"id":"resp_fixture","status":"completed","model":"upstream-text","output":[message],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
            ];
            let payload = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
            ([("content-type", "text/event-stream")], payload).into_response()
        }
    }));
    let upstream_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    let key = collection.api_key.clone();
    let mut account = native_automatic_fixture();
    account.api_base_url = Some(format!("http://{upstream}/v1"));
    account.api_wire_api = Some("responses".into());
    account.api_model_catalog.retain(|model| model == "client-text");
    account.api_model_mappings.retain(|mapping| mapping.client_model == "client-text");
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides.insert(account.id.clone(), account);
    let mut gateway = native_api::start(
        "127.0.0.1:0".parse().unwrap(), Arc::new(host), directory.path().join("identity.sqlite"),
        native_api::Options::default(),
    ).await.unwrap();
    let base_url = format!("http://{}/v1", gateway.addr);
    let client = reqwest::Client::builder()
        .no_proxy().timeout(Duration::from_secs(10)).build().unwrap();

    for (endpoint, body) in [
        ("responses", json!({"model":"client-text","input":"ping"})),
        ("chat/completions", json!({"model":"client-text","messages":[{"role":"user","content":"ping"}]})),
    ] {
        let response = client.post(format!("{base_url}/{endpoint}")).bearer_auth(&key)
            .json(&body).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{endpoint}");
        assert!(response.text().await.unwrap().contains("pong"), "{endpoint}");
    }
    let (_, text) = run_local_access_test_dialog(&base_url, &key, "client-text").await.unwrap();
    assert_eq!(text, "pong");
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream_task.abort();
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|body| body["model"] == "upstream-text" && !native_contains_image_tool(body)));
}

#[tokio::test]
async fn provider_profile_catalog_sync_keeps_responses_image_input_and_client_tools() {
    use axum::{extract::Json, http::StatusCode, response::IntoResponse, routing::post, Router};

    const UPSTREAM_MODEL: &str = "provider-vision-model";
    const IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let upstream = Router::new().route(
        "/v1/responses",
        post(|Json(body): Json<Value>| async move {
            let image = body.pointer("/input/0/content/1/image_url");
            let tools = body["tools"].as_array().unwrap();
            if body["model"] != UPSTREAM_MODEL
                || image != Some(&json!(IMAGE))
                || !tools
                    .iter()
                    .any(|tool| tool["name"] == "local_computer_fixture")
                || tools.iter().any(|tool| tool["type"] == "image_generation")
            {
                return (StatusCode::BAD_REQUEST, Json(body)).into_response();
            }
            Json(json!({"id":"resp-vision","object":"response","status":"completed","output":[]}))
                .into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    for sync in [false, true] {
        let profile = tempfile::tempdir().unwrap();
        let account_id = format!("fixture-vision-{sync}");
        let mut account = CodexAccount::new_api_key(
            account_id.clone(),
            "fixture@example.invalid".into(),
            "fixture-upstream-key".into(),
            CodexApiProviderMode::Custom,
            Some(format!("http://{addr}/v1")),
            Some("fixture-responses".into()),
            Some("Fixture Responses".into()),
            vec![UPSTREAM_MODEL.into()],
        );
        account.api_wire_api = Some("responses".into());
        account.api_sync_model_catalog_to_codex = sync;
        account.api_instance_access_mode = Some("gateway".into());
        account
            .api_model_vision_support
            .insert(UPSTREAM_MODEL.into(), true);
        account.egress_proxy_disabled = true;
        codex_account::save_account(&account).unwrap();
        codex_account::write_account_bundle_to_dir(profile.path(), &account).unwrap();
        ensure_provider_gateway_for_dir(profile.path(), &account_id)
            .await
            .unwrap();

        let profile_config = fs::read_to_string(profile_config_path(profile.path())).unwrap();
        let config = profile_config.parse::<toml_edit::Document>().unwrap();
        let catalog_path = profile.path().join("cockpit-model-catalog.json");
        let selected_model = {
            let catalog: Value = serde_json::from_slice(&fs::read(catalog_path).unwrap()).unwrap();
            let selected = config["model"].as_str().unwrap();
            assert_ne!(selected, UPSTREAM_MODEL, "exercise the managed model alias");
            let model = catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["slug"] == selected)
                .unwrap();
            assert_eq!(model["input_modalities"], json!(["text", "image"]));
            selected
        };
        let base = codex_account::configured_provider_base_url(profile.path()).unwrap();
        assert_ne!(base, format!("http://{addr}/v1"));
        let key = provider_gateway_profile_api_key(profile.path(), &account_id).unwrap();
        let response = client.post(format!("{}/responses", base.trim_end_matches('/')))
            .bearer_auth(key)
            .header("x-agtools-disable-image-generation", "chat")
            .json(&json!({"model":selected_model,"input":[{"role":"user","content":[
                {"type":"input_text","text":"synthetic image"},{"type":"input_image","image_url":IMAGE}
            ]}],"tools":[
                {"type":"image_generation"},
                {"type":"function","name":"local_computer_fixture","parameters":{"type":"object","properties":{}}}
            ]}))
            .send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        stop_provider_gateways_for_profile(profile.path()).await;
        assert_eq!(status, 200, "sync={sync}: {body}");
        assert!(body.contains("resp-vision"));
    }
    upstream_task.abort();
}


#[tokio::test]
async fn native_api_service_routes_to_a_configured_loopback_responses_upstream() {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };

    let app = Router::new()
        .route(
            "/v1/responses",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                if headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    != Some("Bearer fixture-provider-key")
                    || body["model"] != "fixture-relay-model"
                {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(
                    json!({"id": "resp-loopback", "object": "response", "status": "completed",
                "model": "fixture-relay-model", "output": []}),
                )
                .into_response()
            }),
        )
        .route(
            "/v1/alpha/search",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer fixture-provider-key");
                assert_eq!(body["model"], "fixture-relay-model");
                Json(json!({"output": "fixture search result"}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let mut account = CodexAccount::new_api_key(
        "pool-a".into(),
        "fixture@example.invalid".into(),
        "fixture-provider-key".into(),
        CodexApiProviderMode::Custom,
        Some(format!("http://{upstream_addr}/v1")),
        Some("fixture-loopback".into()),
        Some("Fixture loopback".into()),
        vec!["fixture-relay-model".into()],
    );
    account.api_wire_api = Some("responses".into());
    account.egress_proxy_disabled = true;
    let mut collection = native_test_collection();
    collection.account_ids = vec![account.id.clone()];
    let mut scoped = native_test_key();
    scoped.enabled = true;
    scoped.inherit_account_pool = Some(false);
    scoped.account_ids = vec![account.id.clone()];
    scoped.allowed_models = vec!["fixture-relay-model".into()];
    let scoped_key = scoped.key.clone();
    collection.api_keys = vec![scoped];
    let mut host = NativeHostBridge::for_collection(collection);
    host.profile_dir = Some(directory.path().to_path_buf());
    host.account_overrides.insert(account.id.clone(), account);
    let mut gateway = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(host),
        directory.path().join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await
    .unwrap();
    assert_ne!(gateway.addr.port(), upstream_addr.port());
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let response = client
        .post(format!("http://{}/v1/responses", gateway.addr))
        .bearer_auth(&scoped_key)
        .json(&json!({"model": "fixture-relay-model", "input": "ping"}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("resp-loopback"), "{body}");
    let search_url = format!("http://{}/v1/alpha/search", gateway.addr);
    let search = client
        .post(&search_url)
        .bearer_auth(&scoped_key)
        .json(&json!({"id": "fixture-search", "model": "fixture-relay-model"}))
        .send()
        .await
        .unwrap();
    let status = search.status();
    let body = search.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("fixture search result"), "{body}");

    let denied = client
        .post(&search_url)
        .bearer_auth(&scoped_key)
        .json(&json!({"id": "fixture-search", "model": "different-model"}))
        .send()
        .await
        .unwrap();
    assert!(matches!(
        denied.status(),
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ));
    let missing_model = client
        .post(search_url)
        .bearer_auth(&scoped_key)
        .json(&json!({"id": "fixture-search"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_model.status(), StatusCode::SERVICE_UNAVAILABLE);
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[test]
fn native_automatic_catalog_resolves_declared_client_slots_without_requiring_upstream_in_catalog() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account
        .api_model_mappings
        .push(crate::models::codex::CodexApiModelMapping {
            client_model: "undeclared-client".into(),
            upstream_model: "upstream-text".into(),
        });
    assert_eq!(
        native_account_model(&account, "CLIENT-TEXT").as_deref(),
        Some("upstream-text")
    );
    assert!(native_account_model(&account, "upstream-text").is_none());
    assert!(native_account_model(&account, "undeclared-client").is_none());
    let catalog = native_catalog(&collection, None, &[account]).unwrap();
    let entries = catalog["data"].as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["id"] == "client-text"));
    assert!(!entries
        .iter()
        .any(|entry| entry["id"] == "upstream-text" || entry["id"] == "undeclared-client"));
    assert!(catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["slug"] == "client-text"
            && entry["input_modalities"] == json!(["text", "image"])));
}

#[tokio::test]
async fn opencode_go_vision_reaches_chat_upstream_without_stripping_the_image() {
    use axum::{response::IntoResponse, routing::post, Json, Router};

    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let router = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(body);
            let chunk = json!({"id":"fixture-chat","object":"chat.completion.chunk","model":"deepseek-v4.1-flash",
                "choices":[{"index":0,"delta":{"role":"assistant","content":"fixture response"},"finish_reason":"stop"}]});
            ([("content-type", "text/event-stream")], format!("data: {chunk}\n\ndata: [DONE]\n\n")).into_response()
        }
    }));
    let upstream_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    let key = collection.api_key.clone();
    let mut account = native_automatic_fixture();
    account.api_provider_id = Some("opencode_go".into());
    account.api_base_url = Some(format!("http://{upstream}/v1"));
    account.api_model_catalog = ["deepseek-v4-pro", "deepseek-v4.1-flash"].map(String::from).to_vec();
    account.api_model_mappings.clear();
    account.api_model_vision_support = HashMap::from([("deepseek-v4.1-flash".into(), true)]);
    let mut host = NativeHostBridge::for_collection(collection);
    host.profile_dir = Some(directory.path().to_path_buf());
    host.account_overrides.insert(account.id.clone(), account);
    let mut gateway = native_api::start(
        "127.0.0.1:0".parse().unwrap(), Arc::new(host), directory.path().join("identity.sqlite"),
        native_api::Options::default(),
    ).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    for model in ["deepseek-v4.1-flash", "deepseek-v4-pro"] {
        let response = client.post(format!("http://{}/v1/responses", gateway.addr)).bearer_auth(&key)
            .json(&json!({"model": model, "input":[{"role":"user","content":[
                {"type":"input_text","text":"Describe the test pixel"}, {"type":"input_image","image_url":png}
            ]}]})).send().await.unwrap();
        let status = response.status();
        assert!(status.is_success(), "{model}: {status} {}", response.text().await.unwrap());
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream_task.abort();
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request["model"], "deepseek-v4.1-flash");
        assert!(request["messages"].as_array().unwrap().iter().any(|message|
            message["content"].as_array().is_some_and(|parts| parts.iter().any(|part|
                part["type"] == "image_url" && part["image_url"]["url"] == png
            ))
        ));
    }
}

#[tokio::test]
async fn legacy_key_without_allowlist_tracks_catalog_updates_but_fixed_key_does_not() {
    use axum::{response::IntoResponse, routing::post, Json, Router};

    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let upstream = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(body["model"].as_str().unwrap().to_owned());
            let event = json!({"type":"response.completed","response":{"id":"fixture-response","status":"completed","model":body["model"],"output":[]}});
            ([("content-type", "text/event-stream")], format!("data: {event}\n\n")).into_response()
        }
    }));
    let upstream_task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    collection.api_keys = serde_json::from_value(json!([
        {"id":"old-default","label":"Old default","key":"fixture-default"},
        {"id":"old-fixed","label":"Old fixed","key":"fixture-fixed","allowedModels":["gpt-6-astra"]}
    ])).unwrap();
    let mut account = native_automatic_fixture();
    account.api_base_url = Some(base);
    account.api_wire_api = Some("responses".into());
    account.api_model_catalog = ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"].map(String::from).to_vec();
    account.api_model_mappings.clear();
    let mut host = NativeHostBridge::for_collection(collection);
    host.profile_dir = Some(directory.path().to_path_buf());
    host.account_overrides.insert(account.id.clone(), account);
    let mut gateway = native_api::start(
        "127.0.0.1:0".parse().unwrap(), Arc::new(host), directory.path().join("identity.sqlite"),
        native_api::Options::default(),
    ).await.unwrap();
    let base = format!("http://{}/v1", gateway.addr);
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
    for (key, permitted) in [("fixture-default", true), ("fixture-fixed", false)] {
        let models: Value = client.get(format!("{base}/models")).bearer_auth(key)
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        for model in ["gpt-6-sol", "gpt-6-luna"] {
            assert_eq!(models["data"].as_array().unwrap().iter().any(|entry| entry["id"] == model), permitted);
            let response = client.post(format!("{base}/responses")).bearer_auth(key)
                .json(&json!({"model":model,"input":"fixture"})).send().await.unwrap();
            assert_eq!(response.status().is_success(), permitted, "{key}: {model}");
            if !permitted {
                assert!(response.status().is_client_error());
            }
        }
    }
    assert_eq!(*observed.lock().unwrap(), ["gpt-6-sol", "gpt-6-luna"]);
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream_task.abort();
}

#[test]
fn native_automatic_vision_routes_use_actual_upstream_model_and_fail_closed_on_ambiguity() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    let plan = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, false).as_deref(),
        Some("upstream-text")
    );
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-vision")
    );
    account.api_model_catalog.push("other-vision".into());
    account
        .api_model_vision_support
        .insert("other-vision".into(), true);
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-text")
    );
    account.api_vision_routing_model = Some("upstream-vision".into());
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-vision")
    );
}

#[tokio::test]
async fn native_automatic_vision_selection_applies_target_acl_and_cooldown_before_credentials() {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    collection.max_retry_interval_ms = 0;
    let mut key = native_test_key();
    key.excluded_models = vec!["upstream-vision".into()];
    collection.api_keys = vec![key.clone()];
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides
        .insert("pool-a".into(), native_automatic_fixture());
    let caller = native_test_caller(&key.id);
    let mut request = native_test_route_request("vision-policy");
    request.model = "client-text".into();
    request.image_input = true;
    assert_eq!(
        native_select_route(&host, &caller, request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "no_available_account"
    );
    assert!(host.bindings.lock().await.is_empty());
    host.collection_override.as_mut().unwrap().api_keys[0]
        .excluded_models
        .clear();
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "pool-a",
        "upstream-vision",
        429,
        Some(Duration::from_secs(90)),
        false,
        Instant::now(),
    );
    assert!(native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap()
        .retry_after
        .is_some());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    request.image_input = false;
    let route = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(route.model, "upstream-text");
}

#[test]
fn native_provider_vision_omission_changes_only_content_and_preserves_opaque_and_tool_data() {
    let collection = native_test_collection();
    let account = native_automatic_fixture();
    for (body, text_type, pointer) in [
        (
            json!({"model":"upstream-text","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}],"reasoning":{"encrypted_content":"cipher"},"tools":[{"type":"function","parameters":{"type":"input_image"}}]}),
            "input_text",
            "/input/0/content/0",
        ),
        (
            json!({"model":"upstream-text","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"fixture"}}]}],"reasoning":{"encrypted_content":"cipher"},"tools":[{"type":"function","parameters":{"type":"input_image"}}]}),
            "text",
            "/messages/0/content/0",
        ),
    ] {
        let result = native_prepare_provider_vision_body(&collection, None, &account, body.clone());
        assert_eq!(result.pointer(pointer).unwrap()["type"], text_type);
        assert!(result.pointer(pointer).unwrap().get("image_url").is_none());
        assert_eq!(result["reasoning"], body["reasoning"]);
        assert_eq!(result["tools"], body["tools"]);
    }
    let body = json!({"model":"upstream-vision","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, None, &account, body.clone()),
        body
    );
}

#[test]
fn native_gateway_vision_uses_current_account_capabilities_without_overriding_explicit_model_values(
) {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    let mut key = native_test_key();
    key.provider_gateway = Some(serde_json::from_value(json!({"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"upstream-text","upstreamModels":["upstream-text","upstream-vision"]})).unwrap());
    let plan = native_model_route_plan(&collection, Some(&key), "upstream-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &plan, &account, "upstream-text"),
        "upstream-vision"
    );
    assert!(native_catalog_vision(
        &collection,
        &plan,
        &account,
        "upstream-text"
    ));
    let images = json!({"model":"upstream-vision","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, Some(&key), &account, images.clone()),
        images
    );
    key.provider_gateway
        .as_mut()
        .unwrap()
        .model_capabilities
        .insert(
            "upstream-vision".into(),
            CodexLocalAccessProviderGatewayModelCapability {
                supports_vision: false,
            },
        );
    let plan = native_model_route_plan(&collection, Some(&key), "upstream-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &plan, &account, "upstream-text"),
        "upstream-text"
    );
    assert!(!native_catalog_vision(
        &collection,
        &plan,
        &account,
        "upstream-text"
    ));
    let result =
        native_prepare_provider_vision_body(&collection, Some(&key), &account, images.clone());
    assert_eq!(result["input"][0]["content"][0]["type"], "input_text");
    account.api_model_vision_support.clear();
    account.api_supports_vision = true;
    let automatic = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &automatic, &account, "upstream-text"),
        "upstream-text"
    );
    assert!(native_catalog_vision(
        &collection,
        &automatic,
        &account,
        "upstream-text"
    ));
    let images = json!({"model":"upstream-text","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, None, &account, images.clone()),
        images
    );
}

#[test]
fn native_automatic_catalog_hides_retired_gpt_names_but_keeps_request_routing() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account.api_model_catalog = vec!["gpt-4.1".into(), "client-text".into()];
    let catalog = native_catalog(&collection, None, &[account.clone()]).unwrap();
    assert!(!catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "gpt-4.1"));
    assert!(native_account_model(&account, "gpt-4.1").is_some());
}

#[tokio::test]
async fn native_automatic_third_party_names_skip_oauth_pool_and_preserve_native_fallback_models() {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let mut collection = native_test_collection();
    collection.session_affinity = false;
    let account = native_automatic_fixture();
    let mut oauth = native_test_account();
    oauth.id = "pool-b".into();
    let provider_plan = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert!(native_plan_account_model(&provider_plan, &oauth).is_none());
    assert_eq!(
        native_plan_account_model(&provider_plan, &account).as_deref(),
        Some("upstream-text")
    );
    let native_model = api_service_routable_codex_model_ids()
        .into_iter()
        .next()
        .unwrap();
    let native_plan = native_model_route_plan(&collection, None, &native_model).unwrap();
    assert_eq!(
        native_plan_account_model(&native_plan, &oauth).as_deref(),
        Some(native_model.as_str())
    );
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides.insert(account.id.clone(), account);
    host.account_overrides.insert(oauth.id.clone(), oauth);
    let mut request = native_test_route_request("mixed-provider-only");
    request.model = "client-text".into();
    let route = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await
        .unwrap();
    assert_eq!(route.account_id, "pool-a");
    assert_eq!(route.model, "upstream-text");
}

#[test]
fn native_automatic_empty_responses_pool_does_not_claim_undeclared_provider_models() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account.api_wire_api = Some("responses".into());
    account.api_model_catalog.clear();
    account.api_model_mappings.clear();
    let plan = native_model_route_plan(&collection, None, "other-provider-only").unwrap();
    assert!(native_plan_account_model(&plan, &account).is_none());
    let mut key = native_test_key();
    key.provider_gateway = Some(serde_json::from_value(json!({"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"other-provider-only"})).unwrap());
    let explicit = native_model_route_plan(&collection, Some(&key), "other-provider-only").unwrap();
    assert_eq!(
        native_plan_account_model(&explicit, &account).as_deref(),
        Some("other-provider-only")
    );
}

#[test]
fn native_official_quota_reset_reopens_elapsed_windows_but_keeps_other_windows_and_429() {
    let collection = native_test_collection();
    let now = chrono::Utc::now().timestamp();
    let mut account = native_test_account_with_quota(0, 70);
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now - 1);
    assert!(native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now + 60);
    assert!(!native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now - 1);
    account.quota.as_mut().unwrap().weekly_percentage = 0;
    account.quota.as_mut().unwrap().weekly_reset_time = Some(now + 60);
    assert!(!native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().weekly_reset_time = Some(now - 1);
    assert!(native_quota_allows(&collection, &account));
    let mut states = HashMap::new();
    native_upsert_cooldown(
        &mut states,
        &account.id,
        "gpt-fixture",
        429,
        Some(Duration::from_secs(90)),
        true,
        Instant::now(),
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-fixture", false, Instant::now()).is_some()
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-fixture", true, Instant::now()).is_none()
    );
    let mut reserved = collection;
    reserved.bound_oauth_account_id = Some(account.id.clone());
    reserved.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 10,
        weekly_percent: 10,
        strict: false, safety_margin_percent: 3,
});
    assert!(!native_quota_allows(&reserved, &account));
}

#[test]
fn native_xai_suffix_is_route_scoped_and_checks_full_and_base_model_acl() {
    let collection = native_test_collection();
    let mut key = native_test_key();
    key.model_routing = Some(serde_json::from_value(json!({"defaultRoute":"oauth","failurePolicy":"strict","routes":[{"id":"xai","namespace":"grok","providerAccountId":"pool-a","nativeProvider":"xai","providerGateway":{"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"grok-4.3","upstreamModels":["grok-4.3"]}}]})).unwrap());
    let plan = native_model_route_plan(&collection, Some(&key), "grok/grok-4.3(8192)").unwrap();
    let account = native_automatic_fixture();
    assert_eq!(
        native_plan_account_model(&plan, &account).as_deref(),
        Some("grok-4.3")
    );
    assert!(!native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.excluded_models = vec!["grok-4.3".into()];
    assert!(native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.excluded_models.clear();
    key.allowed_models = vec!["grok/grok-4.3(8192)".into()];
    assert!(native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.allowed_models.push("grok/grok-4.3".into());
    assert!(!native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.model_routing.as_mut().unwrap().routes[0].native_provider = None;
    assert!(native_model_route_plan(&collection, Some(&key), "grok/grok-4.3(8192)").is_err());
}
