#[tokio::test]
async fn native_paused_member_is_not_selected_by_catalog_explicit_scope_or_continuation() {
    let mut host = native_test_routing_host();
    host.collection_override
        .as_mut()
        .unwrap()
        .paused_account_ids = vec!["pool-a".into()];
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let request = native_test_route_request("pause-route");
    let route = native_select_route(&host, &caller, request.clone())
        .await
        .unwrap();
    assert_eq!(route.account_id, "pool-b");
    drop(route);
    let mut pinned = request.clone();
    pinned.preferred_account_id = Some("pool-a".into());
    assert_eq!(
        native_select_route(&host, &caller, pinned.clone())
            .await
            .err()
            .unwrap()
            .code,
        "service_member_paused"
    );
    for operation in ["responses.websocket", "responses.compact", "files"] {
        pinned.auxiliary = Some(operation.into());
        assert_eq!(
            native_select_route(&host, &caller, pinned.clone())
                .await
                .err()
                .unwrap()
                .code,
            "service_member_paused"
        );
    }
    let mut key = native_test_key();
    key.account_ids = vec!["pool-a".into()];
    key.inherit_account_pool = Some(false);
    host.collection_override.as_mut().unwrap().api_keys = vec![key.clone()];
    let scoped = native_test_caller(&key.id);
    let model = native_contract::Host::models(&host, &scoped).await.unwrap();
    assert!(model["data"].as_array().unwrap().is_empty());
    assert!(native_select_route(&host, &scoped, request).await.is_err());
    assert_eq!(
        host.collection_override.as_ref().unwrap().account_ids,
        vec!["pool-a", "pool-b"]
    );
    assert_eq!(
        host.collection_override.as_ref().unwrap().api_keys[0].account_ids,
        vec!["pool-a"]
    );
}
