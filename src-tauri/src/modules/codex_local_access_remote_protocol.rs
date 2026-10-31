// Public self-service endpoints return only the authenticated key's view.
async fn forward_remote_key_view(
    bridge: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: &NativeManagementRequest,
) -> Option<Result<Value, native_contract::ServiceError>> {
    if bridge.profile_dir.is_none() {
        return None;
    }
    if !matches!(
        request,
        NativeManagementRequest::Connection
            | NativeManagementRequest::Usage { .. }
            | NativeManagementRequest::KeyStats { .. }
            | NativeManagementRequest::KeyRequests { .. }
            | NativeManagementRequest::KeySession { .. }
    ) {
        return None;
    }
    let collection = match bridge.collection().await {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    let gateway = collection
        .api_keys
        .iter()
        .find(|k| k.id == caller.id)?
        .provider_gateway
        .as_ref()?;
    if !crate::modules::codex_remote_service::is_known(&gateway.base_url, &gateway.api_key) {
        return None;
    }
    let result = match request {
        NativeManagementRequest::Connection => crate::modules::codex_remote_service::discover(
            &gateway.base_url,
            &gateway.api_key,
            false,
        )
        .await
        .and_then(|r| r.ok_or("COCKPIT_REMOTE_USAGE_UNAVAILABLE".into()))
        .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        NativeManagementRequest::Usage { range } => {
            crate::modules::codex_remote_service::usage(&gateway.base_url, &gateway.api_key, range)
                .await
        }
        NativeManagementRequest::KeyStats { query } => {
            crate::modules::codex_remote_service::key_data(
                &gateway.base_url,
                &gateway.api_key,
                "stats",
                query,
            )
            .await
        }
        NativeManagementRequest::KeyRequests { query } => {
            crate::modules::codex_remote_service::key_data(
                &gateway.base_url,
                &gateway.api_key,
                "requests",
                query,
            )
            .await
        }
        NativeManagementRequest::KeySession { query } => {
            crate::modules::codex_remote_service::key_data(
                &gateway.base_url,
                &gateway.api_key,
                "session",
                query,
            )
            .await
        }
        _ => return None,
    };
    Some(result.map_err(|error| {
        logger::log_codex_api_warn(&format!("Remote Key view: {error}"));
        if error.contains("UNAVAILABLE") || error.contains("HTTP_501") {
            native_error(
                501,
                "key_view_unsupported",
                "Remote service does not support this Key view",
            )
        } else {
            native_error(
                503,
                "remote_query_failed",
                "Remote Key query failed; retry later",
            )
        }
    }))
}

async fn native_remote_connection(
    bridge: &NativeHostBridge,
    caller: &native_contract::Caller,
) -> Result<Value, native_contract::ServiceError> {
    let collection = bridge.collection().await?;
    let key = collection
        .api_keys
        .iter()
        .find(|key| key.id == caller.id)
        .ok_or_else(|| native_error(401, "invalid_api_key", "Invalid API key"))?;
    let (catalog,accounts)=native_model_catalog_snapshot(bridge,caller,&collection,Some(key)).await?;
    let mut models: Vec<String> = catalog["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect();
    let image_models: Vec<String> = catalog["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["cockpit_image"] == true)
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect();
    let now = now_ms();
    let catalog_collection = collection.clone();
    let catalog_key = key.key.clone();
    let allowed = models.clone();
    let catalog_accounts = accounts.clone();
    let model_catalog = tokio::task::spawn_blocking(move || {
        remote_client_model_catalog(
            &catalog_collection,
            &catalog_key,
            &allowed,
            &catalog_accounts,
        )
    })
    .await
    .map_err(|_| native_error(503, "catalog_unavailable", "Model catalog unavailable"))?
    .map_err(|error| {
        logger::log_codex_api_warn(&format!("Remote catalog: {error}"));
        native_error(503, "catalog_unavailable", "Model catalog unavailable")
    })?;
    let catalog_revision = {
        use sha2::Digest;
        format!(
            "{:x}",
            sha2::Sha256::digest(model_catalog.to_string().as_bytes())
        )
    };
    let priorities: HashMap<&str, i64> = model_catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry["slug"].as_str()?,
                entry["priority"].as_i64().unwrap_or(i64::MAX),
            ))
        })
        .collect();
    models.sort_by_key(|id| priorities.get(id.as_str()).copied().unwrap_or(i64::MAX));
    let state = if !collection.enabled {
        "service_disabled"
    } else if !key.enabled {
        "disabled"
    } else if key.token_limit.is_some_and(|limit| key.token_used >= limit) {
        "token_limit_reached"
    } else if check_key_budget(key, now, false).is_err() {
        "budget_limit_reached"
    } else {
        "active"
    };
    // Creation time is stable across key rotation and reveals no credentials.
    Ok(json!({
        "product":"cockpit-tools","protocolVersion":1,
        "serviceId":format!("cockpit-{}", collection.created_at),"serviceName":"Cockpit Tools",
        "keyLabel":key.label,"status":state,"models":models,
        "modelCatalog":model_catalog,"catalogRevision":catalog_revision,"keyViewsVersion":1,
        "capabilities":{"responses":true,"images":!image_models.is_empty(),"imageEdits":!image_models.is_empty(),
            "hostedImages":collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Enabled && !image_models.is_empty(),
            "usage":true,"websockets":true},
        "imageModels":image_models,
        "tokenLimit":key.token_limit,"tokenUsed":key.token_used,
        "budget":key_budget_snapshot(key, now),"updatedAt":now,
    }))
}

async fn native_remote_usage(
    caller: &native_contract::Caller,
    range: &str,
) -> Result<Value, native_contract::ServiceError> {
    let mut runtime = gateway_runtime().lock().await;
    ensure_stats_windows_current(&mut runtime.stats, now_ms());
    Ok(remote_usage_from_stats(&runtime.stats, &caller.id, range))
}

fn remote_usage_from_stats(stats: &CodexLocalAccessStats, key_id: &str, range: &str) -> Value {
    let rows = match range {
        "day" => &stats.daily.api_keys,
        "month" => &stats.monthly.api_keys,
        "all" => &stats.api_keys,
        _ => &stats.weekly.api_keys,
    };
    let usage = rows
        .iter()
        .find(|row| row.api_key_id == key_id)
        .map(|row| row.usage.clone())
        .unwrap_or_default();
    json!({"product":"cockpit-tools","protocolVersion":1,"range":range,"usage":usage,"updatedAt":stats.updated_at})
}

#[cfg(test)]
mod remote_protocol_tests {
    use super::*;
    use native_contract::Host;

    #[tokio::test]
    async fn remote_instance_forwards_only_public_key_views_with_the_upstream_key() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = axum::Router::new().route(
            "/v1/cockpit/session",
            axum::routing::get({
                let captured = captured.clone();
                move |headers: axum::http::HeaderMap,
                      axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| {
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().push((headers, query));
                        axum::Json(json!({"ready":true,"session":{"model":"public-model"}}))
                    }
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let descriptor=serde_json::from_value(json!({"product":"cockpit-tools","protocolVersion":1,
            "serviceId":"forward-fixture","serviceName":"Cockpit Tools","keyLabel":"B","status":"active",
            "models":["public-model"],"imageModels":[],"keyViewsVersion":1,"tokenLimit":null,"tokenUsed":0,
            "budget":{"policy":{"period":"lifetime"},"usedMicrousd":0},"updatedAt":1,
            "capabilities":{"responses":true,"images":false,"imageEdits":false,"hostedImages":false,"usage":true,"websockets":true}})).unwrap();
        crate::modules::codex_remote_service::remember_test_connection(
            &base, "key-at-A", descriptor,
        );
        let collection=serde_json::from_value(json!({"enabled":true,"port":0,"apiKey":"key-at-B","accountIds":[],"createdAt":42,"updatedAt":42,
            "apiKeys":[{"id":"B-instance","label":"B","key":"key-at-B","providerGateway":{"baseUrl":base,"apiKey":"key-at-A","upstreamModel":"public-model"}}]})).unwrap();
        let mut host = NativeHostBridge::for_collection(collection);
        let dir = tempfile::tempdir().unwrap();
        host.profile_dir = Some(dir.path().to_owned());
        let caller = host.authorize_management("key-at-B").await.unwrap();
        let query = crate::modules::codex_remote_service::KeyQuery {
            thread_id: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
            instance_id: Some("instance-b".into()),
            installation_id: Some("install-b".into()),
            ..Default::default()
        };
        let mut service = native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(host),
            dir.path().join("identity.sqlite"),
            native_api::Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let local_base = format!("http://{}/v1", service.addr);
        let connection = client
            .get(format!("{local_base}/cockpit/connection"))
            .bearer_auth("key-at-B")
            .send()
            .await
            .unwrap();
        assert_eq!(connection.status().as_u16(), 200);
        let connection: Value = connection.json().await.unwrap();
        assert_eq!(connection["serviceId"], "forward-fixture");
        assert_eq!(connection["keyViewsVersion"], 1);
        assert!(!connection.to_string().contains("key-at-A"));
        let result = crate::modules::codex_remote_service::key_data(
            &local_base,
            "key-at-B",
            "session",
            &query,
        )
        .await
        .unwrap();
        assert_eq!(
            client
                .get(format!("{local_base}/cockpit/diagnostics/events"))
                .bearer_auth("key-at-B")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            403
        );
        assert_eq!(result["session"]["model"], "public-model");
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0["authorization"], "Bearer key-at-A");
        assert_eq!(sent[0].1["instanceId"], "instance-b");
        assert_eq!(sent[0].1["installationId"], "install-b");
        assert!(!sent[0].1.contains_key("apiKeyId"));
        drop(sent);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn exhausted_key_can_read_its_status_but_cannot_call_models_or_management() {
        let mut collection: CodexLocalAccessCollection = serde_json::from_value(json!({
            "enabled":true,"port":0,"apiKey":"legacy","accountIds":[],"createdAt":42,"updatedAt":42,
            "apiKeys":[{"id":"remote-one","label":"B","key":"fixture-one","tokenLimit":10,"tokenUsed":10},
                       {"id":"remote-two","label":"C","key":"fixture-two"}]
        })).unwrap();
        collection.image_generation_account_ids.clear();
        let host = Arc::new(NativeHostBridge::for_collection(collection));
        assert_eq!(
            host.authorize("fixture-one").await.unwrap_err().code,
            "token_limit_reached"
        );
        let caller = host.authorize_management("fixture-one").await.unwrap();
        let connection = native_remote_connection(&host, &caller).await.unwrap();
        assert_eq!(connection["status"], "token_limit_reached");
        assert_eq!(connection["tokenUsed"], 10);
        let serialized = connection.to_string();
        for secret in ["fixture-one", "fixture-two", "remote-two"] {
            assert!(!serialized.contains(secret));
        }
        let root = tempfile::tempdir().unwrap();
        let mut service = native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            host,
            root.path().join("identity.sqlite"),
            native_api::Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("http://{}", service.addr);
        let info = client
            .get(format!("{base}/v1/cockpit/connection?apiKeyId=remote-two"))
            .bearer_auth("fixture-one")
            .send()
            .await
            .unwrap();
        assert_eq!(info.status().as_u16(), 200);
        assert_eq!(info.json::<Value>().await.unwrap()["keyLabel"], "B");
        assert_eq!(
            client
                .get(format!("{base}/v1/models"))
                .bearer_auth("fixture-one")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            429
        );
        assert_eq!(
            client
                .get(format!("{base}/v1/cockpit/diagnostics/events"))
                .bearer_auth("fixture-two")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            403
        );
        assert_eq!(
            client
                .get(format!("{base}/v1/cockpit/connection"))
                .bearer_auth("wrong")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            401
        );
        service.shutdown(Duration::from_secs(2)).await.unwrap();
    }

    #[test]
    fn remote_usage_never_includes_another_keys_stats() {
        let mut stats = CodexLocalAccessStats::default();
        stats.api_keys = vec![
            CodexLocalAccessApiKeyStats {
                api_key_id: "B".into(),
                usage: CodexLocalAccessUsageStats {
                    total_tokens: 12,
                    ..Default::default()
                },
                ..Default::default()
            },
            CodexLocalAccessApiKeyStats {
                api_key_id: "C".into(),
                usage: CodexLocalAccessUsageStats {
                    total_tokens: 9000,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        assert_eq!(
            remote_usage_from_stats(&stats, "B", "all")["usage"]["totalTokens"],
            12
        );
        assert_eq!(
            remote_usage_from_stats(&stats, "missing", "all")["usage"]["totalTokens"],
            0
        );
    }

    #[tokio::test]
    async fn remote_images_keep_the_server_namespace_through_an_instance_gateway() {
        let base = format!("http://192.0.2.10:1455/{}/v1", uuid::Uuid::new_v4());
        let account = CodexAccount::new_api_key(
            "remote-image".into(),
            "fixture@invalid.test".into(),
            "remote-key".into(),
            CodexApiProviderMode::Custom,
            Some(base.clone()),
            None,
            None,
            vec!["team/gpt-5.5".into()],
        );
        let remote = serde_json::from_value(json!({
            "product":"cockpit-tools","protocolVersion":1,"serviceId":"fixture","serviceName":"Cockpit Tools",
            "keyLabel":"B","status":"active","models":["team/gpt-5.5","team/gpt-image-2.5"],"imageModels":["team/gpt-image-2.5"],
            "capabilities":{"responses":true,"images":true,"imageEdits":true,"hostedImages":true,"usage":true,"websockets":true},
            "tokenLimit":null,"tokenUsed":0,"budget":{"policy":{"period":"lifetime"},"usedMicrousd":0},"updatedAt":1
        })).unwrap();
        crate::modules::codex_remote_service::remember_test_connection(&base, "remote-key", remote);
        let collection = serde_json::from_value(json!({
            "enabled":true,"port":0,"apiKey":"local-key","accountIds":[],"createdAt":42,"updatedAt":42,
            "imageGenerationModel":"team/gpt-image-2.5","imageGenerationAccountIds":["remote-image"],
            "imageGenerationAccountModels":{"remote-image":"team/gpt-image-2.5"},
            "imageGenerationAccountPolicies":{"remote-image":"enabled"},
            "apiKeys":[{"id":"local-instance-key","label":"local","key":"local-key","accountIds":["remote-image"],"inheritAccountPool":false}]
        })).unwrap();
        let mut host = NativeHostBridge::for_collection(collection);
        host.account_overrides.insert(account.id.clone(), account);
        let caller = native_contract::Caller {
            id: "local-instance-key".into(),
            label: "local".into(),
        };
        let request = native_contract::ImageRouteRequest {
            model: Some("gpt-image-2.5".into()),
            responses_model: None,
            edit: false,
            target_account_id: None,
            force_refresh: false,
            preferred_account_id: None,
            excluded_account_ids: vec![],
        };
        let selected = native_image_route(&host, &caller, request).await.unwrap();
        assert_eq!(selected.image_model, "team/gpt-image-2.5");
        assert_eq!(selected.route.base_url, base);
        assert!(selected.main_model.is_none());
        let connection = native_remote_connection(&host, &caller).await.unwrap();
        assert_eq!(connection["capabilities"]["images"], true);
        assert!(connection["imageModels"]
            .as_array()
            .unwrap()
            .contains(&json!("team/gpt-image-2.5")));
    }
}
