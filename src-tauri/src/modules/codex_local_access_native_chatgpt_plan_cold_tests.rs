/// The production authorization/selection path with only its final network
/// destination replaced by the caller's loopback mock. No public API is called.
pub(crate) async fn assert_chatgpt_plan_cold_catalog_real_http(projection_id: &str, base: &str) {
    use futures_util::future::BoxFuture;
    struct Host {
        inner: NativeHostBridge,
        base: String,
    }
    impl native_contract::Host for Host {
        fn authorize<'a>(
            &'a self,
            bearer: &'a str,
        ) -> BoxFuture<'a, Result<native_contract::Caller, native_contract::ServiceError>> {
            native_contract::Host::authorize(&self.inner, bearer)
        }
        fn route<'a>(
            &'a self,
            caller: &'a native_contract::Caller,
            request: native_contract::RouteRequest,
        ) -> BoxFuture<'a, Result<native_contract::Route, native_contract::ServiceError>> {
            Box::pin(async move {
                let mut route = native_contract::Host::route(&self.inner, caller, request).await?;
                assert!(route.chatgpt_plan);
                assert_eq!(route.base_url, "https://api.openai.com/v1");
                assert!(route.official_authorization_revision.is_some());
                route.base_url = self.base.clone();
                route.client = reqwest::Client::builder().no_proxy().build().unwrap();
                Ok(route)
            })
        }
        fn models<'a>(
            &'a self,
            caller: &'a native_contract::Caller,
        ) -> BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
            native_contract::Host::models(&self.inner, caller)
        }
        fn record(&self, _: native_contract::RequestReport) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }
    let mut projection = codex_account::load_account(projection_id).unwrap();
    projection.api_model_catalog.clear();
    let bindings = chatgpt_plan_selected_bindings().await.unwrap();
    let mut inner = mixed_oauth_host(projection.clone(), vec![projection_id.into()]);
    inner.collection_override.as_mut().unwrap().official_account_bindings = bindings.clone();
    let host = Arc::new(Host {
        inner,
        base: base.into(),
    });
    let caller = native_contract::Host::authorize(&host.inner, "synthetic-mixed-key")
        .await
        .unwrap();
    let visible = native_contract::Host::models(&host.inner, &caller)
        .await
        .unwrap();
    assert!(visible["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["id"] == "gpt-6.1-sol"));
    let path = tempfile::tempdir().unwrap();
    let mut service = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.path().join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let endpoint = format!("http://{}/v1/responses", service.addr);
    let response = client
        .post(&endpoint)
        .bearer_auth("synthetic-mixed-key")
        .json(&json!({"model":"gpt-6.1-sol","input":"cold catalog fixture","stream":true}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("response.completed"));
    let unknown = client
        .post(&endpoint)
        .bearer_auth("synthetic-mixed-key")
        .json(&json!({"model":"not-an-authorized-model","input":"blocked","stream":true}))
        .send()
        .await
        .unwrap();
    assert!(!unknown.status().is_success());
    let mut excluded = mixed_oauth_host(projection, vec!["ordinary-key".into()]);
    excluded.collection_override.as_mut().unwrap().official_account_bindings = bindings;
    let caller = native_contract::Host::authorize(&excluded, "synthetic-mixed-key")
        .await
        .unwrap();
    assert!(
        native_contract::Host::route(&excluded, &caller, mixed_request("gpt-6.1-sol"))
            .await
            .is_err()
    );
    service.shutdown(Duration::from_secs(3)).await.unwrap();
}
