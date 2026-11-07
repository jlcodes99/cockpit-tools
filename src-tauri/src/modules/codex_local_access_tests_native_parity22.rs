// Go-captured policy fixtures plus real native HTTP edges; no live accounts or upstreams.

fn parity22_policy_fixture() -> Value {
    serde_json::from_str(include_str!(
        "codex_native_api/fixtures/parity22_policy_go.json"
    ))
    .unwrap()
}

fn parity22_provider(
    model: &str,
) -> crate::models::codex_local_access::CodexLocalAccessProviderGateway {
    serde_json::from_value(json!({
        "baseUrl":"http://127.0.0.1:9/v1", "apiKey":"fixture-provider-key",
        "upstreamModel":model, "upstreamModels":[model], "wireApi":"responses"
    }))
    .unwrap()
}

fn parity22_api_account(model: &str) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "fixture-account".into(),
        "fixture@example.invalid".into(),
        "fixture-provider-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("http://127.0.0.1:9/v1".into()),
        None,
        None,
        vec![model.into()],
    );
    account.egress_proxy_disabled = true;
    account
}

#[test]
fn native_client_catalog_does_not_grant_chatgpt_credentials_provider_models() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    struct CatalogRestore(Option<Vec<String>>);
    impl Drop for CatalogRestore {
        fn drop(&mut self) {
            *API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.0.take();
        }
    }
    let _catalog = {
        let mut catalog = API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CatalogRestore(catalog.replace(vec!["gemini-4-argon".into(), "gpt-6.1-sol".into()]))
    };
    let collection = native_test_collection();
    let oauth = native_test_account();
    let provider = parity22_api_account("gemini-4-argon");
    let provider_plan = native_model_route_plan(&collection, None, "gemini-4-argon").unwrap();
    assert!(native_plan_account_model(&provider_plan, &oauth).is_none());
    assert_eq!(
        native_plan_account_model(&provider_plan, &provider).as_deref(),
        Some("gemini-4-argon")
    );
    let mut official = oauth.clone();
    official.api_provider_id = Some("cockpit_chatgpt_plan".into());
    assert!(native_plan_account_model(&provider_plan, &official).is_none());
    let native_plan = native_model_route_plan(&collection, None, "gpt-6.1-sol").unwrap();
    for account in [&oauth, &official] {
        assert_eq!(
            native_plan_account_model(&native_plan, account).as_deref(),
            Some("gpt-6.1-sol")
        );
    }
    // A fetched account catalog remains authoritative, unlike the client catalog.
    official.api_model_catalog = vec!["account-specific-model".into()];
    let granted = native_model_route_plan(&collection, None, "account-specific-model").unwrap();
    assert_eq!(
        native_plan_account_model(&granted, &official).as_deref(),
        Some("account-specific-model")
    );
}

#[test]
fn parity22_model_rules_match_go_and_keep_all_permission_layers() {
    for case in parity22_policy_fixture()["rules"].as_array().unwrap() {
        assert_eq!(
            native_pattern_matches(
                case["pattern"].as_str().unwrap(),
                case["model"].as_str().unwrap()
            ),
            case["matches"].as_bool().unwrap(),
            "{case}"
        );
    }
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    key.allowed_models = vec![" GPT-6.1-* ".into()];
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "gpt-6.1-sol").unwrap(),
        "gpt-6.1-sol"
    );
    key.excluded_models = vec!["GPT-6.1-SOL".into()];
    assert!(native_model_for_key(&collection, Some(&key), "gpt-6.1-sol").is_err());
    key.excluded_models.clear();
    collection.excluded_models = vec!["GPT-*".into()];
    assert!(native_model_for_key(&collection, Some(&key), "gpt-6.1-sol").is_err());
    collection.excluded_models.clear();
    collection.account_ids = vec!["fixture-account".into()];
    let plan = native_model_route_plan(&collection, Some(&key), "gpt-6.1-sol").unwrap();
    collection.account_model_rules = serde_json::from_value(
        json!([{"accountId":"fixture-account","excludedModels":["GPT-6.1-*"]}]),
    )
    .unwrap();
    assert!(!native_model_visible(
        &collection,
        Some(&key),
        &plan,
        &native_test_account()
    ));
    key.model_prefix = Some("team".into());
    assert!(native_model_for_key(&collection, Some(&key), "other/gpt-6.1-sol").is_err());
}

#[tokio::test]
async fn parity22_model_catalog_websocket_flags_match_go_configuration() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    struct CatalogRestore(Option<Vec<String>>);
    impl Drop for CatalogRestore {
        fn drop(&mut self) {
            *API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.0.take();
        }
    }
    let _catalog = {
        let mut catalog = API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CatalogRestore(catalog.replace(vec!["gpt-6.1-sol".into()]))
    };
    for case in parity22_policy_fixture()["catalogs"].as_array().unwrap() {
        let mut collection = native_test_collection();
        collection.account_ids = vec!["fixture-account".into()];
        collection.responses_websockets_enabled = case["enabled"].as_bool().unwrap();
        let mut key = native_test_key();
        let account = if case["provider"] == true {
            key.provider_gateway = Some(parity22_provider("gpt-6.1-sol"));
            parity22_api_account("gpt-6.1-sol")
        } else {
            native_test_account()
        };
        if case["namespaced"] == true {
            key.model_routing = Some(
                serde_json::from_value(
                    json!({"defaultRoute":"oauth","failurePolicy":"strict","routes":[]}),
                )
                .unwrap(),
            );
        }
        let catalog = native_catalog(&collection, Some(&key), &[account.clone()]).unwrap();
        let model = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol")
            .unwrap();
        assert_eq!(model["prefer_websockets"], case["prefer"], "{case}");
        collection.api_keys = vec![key];
        let mut bridge = NativeHostBridge::for_collection(collection);
        bridge.account_overrides.insert(account.id.clone(), account);
        let host = Arc::new(Parity22CountHost {
            bridge,
            generation_calls: AtomicU64::new(0),
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
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("http://{}/v1/models", service.addr))
            .bearer_auth("fixture-secret")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let catalog: Value = response.json().await.unwrap();
        let model = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol")
            .unwrap();
        assert_eq!(model["prefer_websockets"], case["prefer"], "HTTP {case}");
        assert_eq!(host.generation_calls.load(Ordering::SeqCst), 0);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
    }
}

struct Parity22CountHost {
    bridge: NativeHostBridge,
    generation_calls: AtomicU64,
}

impl native_contract::Host for Parity22CountHost {
    fn authorize<'a>(
        &'a self,
        bearer: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Caller, native_contract::ServiceError>,
    > {
        native_contract::Host::authorize(&self.bridge, bearer)
    }
    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        native_contract::Host::models(&self.bridge, caller)
    }
    fn token_count_model<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        model: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<String, native_contract::ServiceError>> {
        native_contract::Host::token_count_model(&self.bridge, caller, model)
    }
    fn route<'a>(
        &'a self,
        _: &'a native_contract::Caller,
        _: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        self.generation_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(native_error(
                500,
                "unexpected_generation",
                "Token count must remain local",
            ))
        })
    }
    fn record(&self, _: native_contract::RequestReport) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn parity22_token_estimate_http_matches_go_alias_preprocessing_without_generation() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    for case in parity22_policy_fixture()["counts"].as_array().unwrap() {
        let root = tempfile::tempdir().unwrap();
        let mut collection = native_test_collection();
        collection.account_ids = vec!["fixture-account".into()];
        let alias = case["alias"].as_str().unwrap();
        let upstream = case["upstream"].as_str().unwrap();
        collection.model_aliases =
            serde_json::from_value(json!([{"sourceModel":upstream,"alias":alias}])).unwrap();
        let mut key = native_test_key();
        if case["mode"] == "provider" {
            key.provider_gateway = Some(parity22_provider(upstream));
        }
        if case["mode"] == "explicit_oauth_alias" {
            key.model_routing = Some(
                serde_json::from_value(
                    json!({"defaultRoute":"oauth","failurePolicy":"strict","routes":[]}),
                )
                .unwrap(),
            );
        }
        collection.api_keys = vec![key];
        let mut bridge = NativeHostBridge::for_collection(collection);
        if matches!(case["mode"].as_str(), Some("plain_profile" | "provider")) {
            bridge.profile_dir = Some(root.path().join("profile"));
        }
        bridge.account_overrides.insert(
            "fixture-account".into(),
            if case["mode"] == "explicit_oauth_alias" {
                native_test_account()
            } else {
                parity22_api_account(upstream)
            },
        );
        let host = Arc::new(Parity22CountHost {
            bridge,
            generation_calls: AtomicU64::new(0),
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
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client
            .post(format!("http://{}/v1/messages/count_tokens", service.addr))
            .bearer_auth("fixture-secret")
            .json(&case["input"])
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{case}: {body}");
        assert_eq!(body["input_tokens"], case["count"], "{case}");
        let caller = native_test_caller("fixture-key-id");
        let canonical = native_contract::Host::token_count_model(host.as_ref(), &caller, alias)
            .await
            .unwrap();
        assert_eq!(
            canonical,
            case["canonical"]["model"].as_str().unwrap(),
            "{case}"
        );
        let denied = client
            .post(format!("http://{}/v1/messages/count_tokens", service.addr))
            .bearer_auth("fixture-secret")
            .json(&json!({"model":"unlisted","messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403);
        assert_eq!(host.generation_calls.load(Ordering::SeqCst), 0);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
    }
}
