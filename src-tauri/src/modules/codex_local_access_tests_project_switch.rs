fn project_switch_test_profile(root: &std::path::Path, id: &str, binding: Option<String>) -> PathBuf {
    let profile = root.join(id);
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join(CODEX_PROFILE_CONFIG_FILE), "cli_auth_credentials_store = 'file'\n")
        .unwrap();
    let mut store = crate::modules::codex_instance::load_instance_store().unwrap();
    store.instances.push(crate::models::InstanceProfile {
        id: id.to_owned(),
        name: id.to_owned(),
        user_data_dir: profile.to_string_lossy().into_owned(),
        working_dir: None,
        extra_args: String::new(),
        bind_account_id: binding,
        model_routing: None,
        launch_mode: crate::models::InstanceLaunchMode::App,
        app_speed: crate::models::codex::CodexAppSpeed::Standard,
        created_at: 1,
        last_launched_at: None,
        last_pid: None,
    });
    crate::modules::codex_instance::save_instance_store(&store).unwrap();
    profile
}

#[tokio::test]
async fn same_url_instance_keys_keep_models_separate_through_swap_and_reset() {
    use axum::{
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let env = LocalAccessTestDataGuard::new("same-url-instance-keys");
    super::with_chatgpt_plan_test_runtime(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let upstream = tokio::spawn(async move {
            let app = Router::new().route("/v1/responses", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                let model = body["model"].as_str().unwrap_or_default();
                let token = headers.get("authorization").and_then(|value| value.to_str().ok());
                if !matches!((model, token), ("vendor/model-a", Some("Bearer fixture-a")) | ("vendor/model-b", Some("Bearer fixture-b"))) {
                    return (StatusCode::FORBIDDEN, Json(json!({"error":{"message":"wrong key or model"}})));
                }
                (StatusCode::OK, Json(json!({"id":"fixture","object":"response","status":"completed","model":model,"output":[]})))
            }));
            axum::serve(listener, app).await.unwrap();
        });

        let instance_ids = ["key-isolation-one", "key-isolation-two"];
        let profiles = instance_ids.map(|id| project_switch_test_profile(&env.data_dir, id, None));
        let account_ids = ["relay-key-a", "relay-key-b"];
        let model_ids = ["vendor/model-a", "vendor/model-b"];
        for ((id, key), model) in account_ids.into_iter().zip(["fixture-a", "fixture-b"]).zip(model_ids) {
            let mut account = CodexAccount::new_api_key(
                id.into(), "fixture@example.invalid".into(), key.into(), CodexApiProviderMode::Custom,
                Some(base_url.clone()), None, None, vec![model.into()],
            );
            account.api_wire_api = Some("responses".into());
            account.api_sync_model_catalog_to_codex = true;
            account.egress_proxy_disabled = true;
            codex_account::save_account(&account).unwrap();
        }
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
        let shared_profile = project_switch_test_profile(
            &env.data_dir,
            "shared-api-service",
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.into()),
        );
        let shared_model = "gpt-5.6-sol";
        let shared_models = codex_account::read_quick_config_from_config_toml(&shared_profile)
            .unwrap().experimental_model_catalog_reset_models.into_iter()
            .filter(|model| model.model_id == shared_model).collect();
        codex_account::save_model_catalog_for_base_dir_preserving_context(
            &shared_profile, true, shared_models, Some(shared_model.into()),
        ).unwrap();
        super::refresh_api_service_experimental_model_ids();
        let api_service_catalog = super::api_service_experimental_model_catalog();
        assert!(api_service_catalog
            .as_ref()
            .is_some_and(|models| models.iter().any(|model| model == shared_model)));
        for (round, order) in [[0, 1], [1, 0]].into_iter().enumerate() {
            for (instance, selected) in order.into_iter().enumerate() {
                crate::commands::codex_instance::profile_switch::codex_switch_instance_account(
                    instance_ids[instance].into(), account_ids[selected].into(), None,
                ).await.unwrap();
                super::stop_provider_gateways_for_profile(&profiles[instance]).await;
                super::ensure_provider_gateway_for_dir(&profiles[instance], account_ids[selected]).await.unwrap();
            }
            if round == 1 {
                let instance_id = instance_ids[0];
                let save_catalog = crate::commands::codex_instance::codex_save_instance_model_catalog;
                let load_config = crate::commands::codex_instance::codex_get_instance_quick_config;
                let defaults = load_config(instance_id.into(), None).await.unwrap();
                let mut stale_model = defaults.experimental_model_catalog_reset_models[0].clone();
                stale_model.model_id = "old-relay/kimi-k2.7".into();
                let stale_default = Some(stale_model.model_id.clone());
                let saved = save_catalog(instance_id.into(), true, vec![stale_model], stale_default.clone()).await.unwrap();
                assert_eq!(saved.experimental_model_catalog_default_model_id, stale_default);
                super::refresh_api_service_experimental_model_ids();
                assert_eq!(super::api_service_experimental_model_catalog(), api_service_catalog);
                let recovered = save_catalog(
                    instance_id.into(),
                    true,
                    defaults.experimental_model_catalog_reset_models.clone(),
                    defaults.experimental_model_catalog_reset_default_model_id.clone(),
                ).await.unwrap();
                assert_eq!(
                    recovered.experimental_model_catalog_default_model_id,
                    defaults.experimental_model_catalog_reset_default_model_id,
                );
                assert!(!recovered.experimental_model_catalog_models.iter()
                    .any(|model| model.model_id == "old-relay/kimi-k2.7"));
                let disabled = save_catalog(instance_id.into(), false, Vec::new(), None).await.unwrap();
                assert!(!disabled.experimental_model_catalog_enabled);
                assert_eq!(super::api_service_experimental_model_catalog(), api_service_catalog);
                super::stop_provider_gateways_for_profile(&profiles[0]).await;
                super::ensure_provider_gateway_for_dir(&profiles[0], account_ids[order[0]]).await.unwrap();
                assert!(!load_config(instance_id.into(), None).await.unwrap().experimental_model_catalog_enabled);
            }
            for (instance, selected) in order.into_iter().enumerate() {
                let profile = &profiles[instance];
                let auth: Value = serde_json::from_slice(&fs::read(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap()).unwrap();
                let config = crate::modules::codex_config_format::load_codex_config_doc(&profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap();
                let gateway = config["model_providers"]["codex_local_access"]["base_url"].as_str().unwrap();
                let client_model = config["model"].as_str().unwrap();
                let catalog_path = config["model_catalog_json"].as_str().unwrap();
                let catalog: Value = serde_json::from_slice(&fs::read(profile.join(catalog_path)).unwrap()).unwrap();
                assert!(catalog["models"].as_array().unwrap().iter().any(|model|
                    model["slug"] == client_model && model["display_name"] == model_ids[selected]));
                let bearer = auth["OPENAI_API_KEY"].as_str().unwrap();
                let models: Value = client.get(format!("{gateway}/models")).bearer_auth(bearer)
                    .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
                let ids = models["data"].as_array().unwrap();
                assert!(ids.iter().any(|model| model["id"] == model_ids[selected]));
                assert!(!ids.iter().any(|model| model["id"] == model_ids[1 - selected]));
                let request = client.post(format!("{gateway}/responses")).bearer_auth(bearer)
                    .json(&json!({"model":client_model,"input":"fixture"})).send().await.unwrap();
                let status = request.status();
                assert_eq!(status, StatusCode::OK, "{}", request.text().await.unwrap());

                let quick = crate::commands::codex_instance::codex_get_instance_quick_config(instance_ids[instance].into(), None).await.unwrap();
                assert_eq!(quick.experimental_model_catalog_reset_default_model_id.as_deref(), Some(client_model));
                let reset_models = quick.experimental_model_catalog_reset_models;
                assert!(reset_models.iter().any(|model| model.model_id == client_model
                    && model.display_name == model_ids[selected]), "{reset_models:?}");
                assert!(!reset_models.iter().any(|model| model.display_name == model_ids[1 - selected]));
            }
        }
        for profile in &profiles {
            super::stop_provider_gateways_for_profile(profile).await;
        }
        assert!(!super::gateway_runtime().lock().await.running);
        upstream.abort();
    }).await;
}

#[tokio::test]
async fn project_switch_publishes_all_relay_models_and_forwards_astra_unchanged() {
    use axum::{http::HeaderMap, routing::post, Json, Router};

    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("project-switch-astra");
    super::with_chatgpt_plan_test_runtime(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let upstream = tokio::spawn(async move {
            let app = Router::new().route(
                "/new/v1/responses",
                post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer relay-new");
                    assert_eq!(body["model"], "gpt-6-astra");
                    Json(json!({
                        "id": "astra_fixture", "object": "response", "status": "completed",
                        "model": "gpt-6-astra", "output": []
                    }))
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });

        let models = ["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"];
        for (id, route, catalog) in [
            ("relay-old", "old", &["gpt-5.6-sol"][..]),
            ("relay-new", "new", &models[..]),
        ] {
            let mut account = CodexAccount::new_api_key(
                id.into(),
                "fixture@example.invalid".into(),
                id.into(),
                CodexApiProviderMode::Custom,
                Some(format!("{origin}/{route}/v1")),
                None,
                None,
                catalog.iter().map(|model| model.to_string()).collect(),
            );
            account.api_wire_api = Some("responses".into());
            account.api_sync_model_catalog_to_codex = true;
            account.egress_proxy_disabled = true;
            codex_account::save_account(&account).unwrap();
        }

        let instance_id = "astra-switch";
        let profile = project_switch_test_profile(&env.data_dir, instance_id, None);
        let history = rusqlite::Connection::open(profile.join("state_5.sqlite")).unwrap();
        history.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY, title TEXT, archived INTEGER,
                cwd TEXT, source TEXT, project_id TEXT, has_user_event INTEGER
             );
             INSERT INTO threads VALUES ('active', 'Active title', 0, '/tmp', 'vscode', NULL, 1);
             INSERT INTO threads VALUES ('archived', 'Archived title', 1, '/tmp', 'vscode', NULL, 1);",
        ).unwrap();
        let read_history = || {
            let mut query = history.prepare("SELECT id, title, archived FROM threads ORDER BY id").unwrap();
            query.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            }).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        let original_history = read_history();
        let switch = crate::commands::codex_instance::profile_switch::codex_switch_instance_account;
        switch(instance_id.into(), "relay-old".into(), None).await.unwrap();
        let config_path = profile.join(CODEX_PROFILE_CONFIG_FILE);
        let read_catalog = || {
            let config = crate::modules::codex_config_format::load_codex_config_doc(&config_path).unwrap();
            let path = config["model_catalog_json"].as_str().unwrap();
            let catalog: Value = serde_json::from_slice(&fs::read(profile.join(path)).unwrap()).unwrap();
            catalog["models"].as_array().unwrap().iter()
                .filter(|entry| entry["visibility"] == "list")
                .map(|entry| entry["slug"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(read_catalog(), vec!["gpt-5.6-sol"]);
        switch(instance_id.into(), "relay-new".into(), None).await.unwrap();
        let after_switch = read_catalog();
        assert!(models.iter().all(|model| after_switch.iter().any(|id| id == model)), "{after_switch:?}");

        super::ensure_provider_gateway_for_dir(&profile, "relay-new").await.unwrap();
        assert_eq!(read_catalog(), after_switch);
        let config = crate::modules::codex_config_format::load_codex_config_doc(&config_path).unwrap();
        let gateway = config["model_providers"]["codex_local_access"]["base_url"].as_str().unwrap();
        let auth: Value = serde_json::from_slice(&fs::read(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap()).unwrap();
        let response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5))
            .build().unwrap().post(format!("{gateway}/responses"))
            .bearer_auth(auth["OPENAI_API_KEY"].as_str().unwrap())
            .json(&json!({"model": "gpt-6-astra", "input": "fixture"}))
            .send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        super::stop_provider_gateways_for_profile(&profile).await;
        upstream.abort();
        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        assert!(body.contains("astra_fixture"), "{body}");
        assert_eq!(read_history(), original_history);
    }).await;
}

#[tokio::test]
async fn project_switch_oauth_to_relay_does_not_reuse_the_previous_api_service_route() {
    use axum::{http::HeaderMap, routing::post, Json, Router};

    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("project-switch-oauth-relay");
    super::with_chatgpt_plan_test_runtime(|| async {
        let old_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let old_port = old_listener.local_addr().unwrap().port();
        let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_base = format!("http://{}/relay/v1", relay_listener.local_addr().unwrap());
        let relay = tokio::spawn(async move {
            let app = Router::new().route(
                "/relay/v1/responses",
                post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer selected-relay-key");
                    assert_eq!(body["model"], "gpt-5.3-codex-spark");
                    Json(json!({"id":"selected_relay","object":"response","status":"completed","output":[]}))
                }),
            );
            axum::serve(relay_listener, app).await.unwrap();
        });

        let token = make_test_jwt(json!({
            "sub":"fixture-user",
            "email":"oauth@example.invalid",
            "exp":4_102_444_800i64,
            "https://api.openai.com/auth":{"user_id":"fixture-user","chatgpt_account_id":"fixture-workspace"}
        }));
        let oauth = CodexAccount::new(
            "oauth-fixture".into(),
            "oauth@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: token.clone(),
                id_token: token.clone(),
                refresh_token: Some("fixture-refresh".into()),
            },
        );
        codex_account::save_account(&oauth).unwrap();
        let mut selected = CodexAccount::new_api_key(
            "selected-relay".into(), "relay@example.invalid".into(), "selected-relay-key".into(),
            CodexApiProviderMode::Custom, Some(relay_base.clone()), None, None,
            vec!["gpt-5.3-codex-spark".into()],
        );
        selected.api_wire_api = Some("responses".into());
        selected.api_sync_model_catalog_to_codex = true;
        selected.egress_proxy_disabled = true;
        codex_account::save_account(&selected).unwrap();

        let instance_id = "oauth-to-relay";
        let profile = project_switch_test_profile(
            &env.data_dir, instance_id,
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.into()),
        );
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = old_port;
        super::write_local_access_profile_takeover(&profile, &collection, None, true).await.unwrap();
        let config_path = profile.join(CODEX_PROFILE_CONFIG_FILE);
        let read_config = || crate::modules::codex_config_format::load_codex_config_doc(&config_path).unwrap();
        let old_url = read_config()["model_providers"]["codex_local_access"]["base_url"].as_str().unwrap().to_owned();
        assert_eq!(url::Url::parse(&old_url).unwrap().port(), Some(old_port));

        let switch = crate::commands::codex_instance::profile_switch::codex_switch_instance_account;
        switch(instance_id.into(), oauth.id.clone(), None).await.unwrap();
        let auth_path = profile.join(CODEX_PROFILE_AUTH_FILE);
        let read_auth = || serde_json::from_slice::<Value>(&fs::read(&auth_path).unwrap()).unwrap();
        assert_eq!(read_auth()["tokens"]["access_token"], token);
        assert_ne!(read_config().get("model_provider").and_then(toml_edit::Item::as_str), Some("codex_local_access"));

        switch(instance_id.into(), selected.id.clone(), None).await.unwrap();
        assert!(!super::provider_gateway_runtime_store().lock().await.contains_key(
            &super::provider_gateway_runtime_key(&profile, &selected.id),
        ));
        super::ensure_provider_gateway_for_dir(&profile, &selected.id).await.unwrap();
        let config = read_config();
        let gateway = config["model_providers"]["codex_local_access"]["base_url"].as_str().unwrap();
        assert_ne!(url::Url::parse(gateway).unwrap().port(), Some(old_port));
        assert_eq!(codex_account::load_account(&selected.id).unwrap().api_base_url.as_deref(), Some(relay_base.as_str()));

        let response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5))
            .build().unwrap().post(format!("{gateway}/responses"))
            .bearer_auth(read_auth()["OPENAI_API_KEY"].as_str().unwrap())
            .json(&json!({"model":"gpt-5.3-codex-spark","input":"fixture"}))
            .send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        super::stop_provider_gateways_for_profile(&profile).await;
        relay.abort();
        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        assert!(body.contains("selected_relay"), "{body}");
    }).await;
}

#[tokio::test]
async fn project_switch_writes_selected_profile_and_defers_chat_listener_until_launch() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("pure-instance-project-switch");
    super::with_chatgpt_plan_test_runtime(|| async {
        let profile = project_switch_test_profile(&env.data_dir, "target-switch-fixture", None);
        let state_path = profile.join(".codex-global-state.json");
        fs::write(&state_path, json!({
            "electron-account-renderer-state-v1":{"owner":"prior","inactive":{}},
            "electron-persisted-atom-state":{"last_completed_onboarding":"complete"}
        }).to_string()).unwrap();
        let mut account = CodexAccount::new_api_key(
            "pure-switch-fixture".into(), "fixture@example.test".into(), "switch-fixture-key".into(),
            CodexApiProviderMode::Custom, Some("https://api.openai.com/v1".into()),
            None, None, vec!["gpt-test".into()],
        );
        codex_account::save_account(&account).unwrap();
        let switch = crate::commands::codex_instance::profile_switch::codex_switch_instance_account;
        assert!(!switch("target-switch-fixture".into(), account.id.clone(), None).await.unwrap());
        let auth: Value = serde_json::from_str(
            &fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap(),
        ).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "switch-fixture-key");
        let state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert!(state.get("cockpit-local-project-account-state-v1").is_some());

        account.api_wire_api = Some("chat_completions".into());
        account.api_base_url = Some("https://relay.example.test/v1".into());
        codex_account::save_account(&account).unwrap();
        fs::write(&state_path, "{\"electron-account-renderer-state-v1\":{}}").unwrap();
        assert!(switch("target-switch-fixture".into(), account.id.clone(), None).await.unwrap());
        let store = crate::modules::codex_instance::load_instance_store().unwrap();
        let binding = crate::modules::codex_instance::provider_gateway_bind_account_id(&account.id);
        assert_eq!(store.instances[0].bind_account_id, binding);
        assert!(store.default_settings.bind_account_id.is_none());
        let config = fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap();
        assert!(config.contains("codex_local_access"));
        let key = super::provider_gateway_runtime_key(&profile, &account.id);
        assert!(!super::provider_gateway_runtime_store().lock().await.contains_key(&key));
        assert!(!super::gateway_runtime().lock().await.running);
    }).await;
}

#[tokio::test]
async fn project_switch_responses_gateway_repairs_receipts_for_legacy_provider_without_catalog() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("responses-instance-project-switch");
    super::with_chatgpt_plan_test_runtime(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let upstream = tokio::spawn(async move {
            let app = axum::Router::new().route("/v1/responses", axum::routing::post(
                |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer responses-fixture-key");
                    assert_eq!(body["model"], "vendor/model-without-catalog");
                    let input = body["input"].as_array().unwrap();
                    if input.iter().any(|item| item["type"] == "function_call_output"
                        && item["call_id"].as_str().is_none())
                    {
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({
                            "error":{"message":"function_call_output requires call_id on HTTP requests"}
                        })));
                    }
                    assert_eq!(input[1]["type"], "message");
                    assert!(input[1]["content"][0]["text"].as_str().unwrap().contains("page receipt"));
                    (axum::http::StatusCode::OK, axum::Json(json!({
                        "id":"resp_fixture","object":"response","status":"completed","output":[]
                    })))
                },
            ));
            axum::serve(listener, app).await.unwrap();
        });
        let instance_id = "responses-switch-fixture";
        let profile = project_switch_test_profile(&env.data_dir, instance_id, None);
        fs::write(
            profile.join(CODEX_PROFILE_CONFIG_FILE),
            "cli_auth_credentials_store = 'file'\nmodel = 'vendor/model-without-catalog'\nmodel_catalog_json = 'custom-models.json'\n",
        ).unwrap();
        fs::write(profile.join("custom-models.json"), r#"{"models":[{"slug":"vendor/model-without-catalog"}]}"#).unwrap();
        let mut account = CodexAccount::new_api_key(
            "responses-fixture".into(), "responses@example.test".into(), "responses-fixture-key".into(),
            CodexApiProviderMode::Custom, Some(base_url.clone()), None, None, Vec::new(),
        );
        codex_account::save_account(&account).unwrap();
        crate::commands::codex_instance::profile_switch::codex_switch_instance_account(
            instance_id.into(), account.id.clone(), None,
        ).await.unwrap();
        let key = super::provider_gateway_runtime_key(&profile, &account.id);
        assert!(!super::provider_gateway_runtime_store().lock().await.contains_key(&key));

        let direct_config = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            &fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        ).unwrap();
        assert_eq!(codex_account::configured_provider_base_url(&profile).as_deref(), Some(base_url.as_str()));
        assert_eq!(direct_config["model_catalog_json"].as_str(), Some("custom-models.json"));

        account.api_instance_access_mode = Some("gateway".into());
        codex_account::save_account(&account).unwrap();
        super::ensure_provider_gateway_for_dir(&profile, &account.id).await.unwrap();
        let auth: Value = serde_json::from_str(
            &fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap(),
        ).unwrap();
        let config = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            &fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        ).unwrap();
        assert_eq!(config["model"].as_str(), Some("vendor/model-without-catalog"));
        let base_url = config["model_providers"]["codex_local_access"]["base_url"].as_str().unwrap();
        let result = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5))
            .build().unwrap().post(format!("{base_url}/responses"))
            .bearer_auth(auth["OPENAI_API_KEY"].as_str().unwrap())
            .json(&json!({"model":"vendor/model-without-catalog","input":[
                {"role":"user","content":"Continue the conversation"},
                {"type":"function_call_output","namespace":"agent","name":"codex_apps_open_page",
                 "output":"page receipt"}
            ]})).send().await.unwrap();
        let status = result.status();
        let body = result.text().await.unwrap();
        super::stop_provider_gateways_for_profile(&profile).await;
        upstream.abort();
        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        assert!(body.contains("resp_fixture"));
        assert!(!super::gateway_runtime().lock().await.running);
    }).await;
}

#[tokio::test]
async fn project_switch_api_disable_restores_oauth_archive_along_with_credentials() {
    use sha2::{Digest, Sha256};
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("disable-project-switch");
    let profile = project_switch_test_profile(
        &env.data_dir, "disable-switch-fixture",
        Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.into()),
    );
    let token = make_test_jwt(json!({"exp":4_102_444_800i64,"https://api.openai.com/auth":{
        "user_id":"user-a","chatgpt_account_id":"workspace-a"
    }}));
    let original_auth = json!({"auth_mode":"chatgpt","tokens":{
        "access_token":token,"id_token":token,"refresh_token":"fixture-refresh","account_id":"workspace-a"
    }});
    let auth_path = profile.join(CODEX_PROFILE_AUTH_FILE);
    fs::write(&auth_path, original_auth.to_string()).unwrap();
    let collection = test_local_access_collection(Vec::new());
    super::save_profile_takeover_backup(&profile, &collection.api_key).unwrap();
    super::write_local_access_profile_takeover(&profile, &collection, None, true).await.unwrap();
    let owner = format!("{:x}", Sha256::digest(br#"["user-a","workspace-a"]"#));
    let state_path = profile.join(".codex-global-state.json");
    fs::write(&state_path, json!({
        "cockpit-local-project-account-state-v1":{"owner":null,"inactive":{
            (owner.as_str()):{"atoms":{"private":"oauth"},"globals":{"pinned-project-ids":["original"]}}
        }}
    }).to_string()).unwrap();

    assert!(!super::restore_takeover_profiles_after_disable_with_projects(
        &collection, || Ok(()), true,
    ).unwrap());
    let auth: Value = serde_json::from_str(&fs::read_to_string(auth_path).unwrap()).unwrap();
    assert_eq!(auth["tokens"]["access_token"], original_auth["tokens"]["access_token"]);
    let state: Value = serde_json::from_str(&fs::read_to_string(state_path).unwrap()).unwrap();
    assert!(state.get("cockpit-local-project-account-state-v1").is_none());
    assert_eq!(state["electron-account-renderer-state-v1"]["owner"], owner);
    assert_eq!(state["pinned-project-ids"], json!(["original"]));
}
