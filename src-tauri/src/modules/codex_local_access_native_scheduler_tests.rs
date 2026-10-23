mod native_scheduler_tests {
    use super::*;

    fn collection() -> CodexLocalAccessCollection {
        serde_json::from_value(json!({"enabled":true,"port":1455,"apiKey":"fixture","accountIds":["a","b"],"createdAt":1,"updatedAt":1})).unwrap()
    }
    fn account(id: &str) -> CodexAccount {
        CodexAccount::new_api_key(
            id.into(),
            "fixture@example.invalid".into(),
            "fixture-key".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://example.invalid/v1".into()),
            None,
            None,
            vec![],
        )
    }
    fn caller(id: &str) -> native_contract::Caller {
        native_contract::Caller {
            id: id.into(),
            label: "fixture".into(),
        }
    }

    #[test]
    fn rotation_is_scoped_by_caller_model_and_authorized_pool() {
        let host = NativeHostBridge::default();
        let collection = collection();
        let a = native_routing_namespace(
            &caller("key-a"),
            "gpt-fixture",
            &collection.account_ids,
            None,
        );
        let b = native_routing_namespace(
            &caller("key-b"),
            "gpt-fixture",
            &collection.account_ids,
            None,
        );
        let other_model = native_routing_namespace(
            &caller("key-a"),
            "vendor/gpt-fixture",
            &collection.account_ids,
            None,
        );
        for namespace in [&a, &b, &other_model] {
            let mut accounts = vec![account("a"), account("b")];
            native_sort_accounts_scoped(&host, &mut accounts, &collection, None, namespace);
            assert_eq!(accounts[0].id, "a");
            native_commit_routing_cursor(&host, namespace, "a");
        }
        let mut accounts = vec![account("a"), account("b")];
        native_sort_accounts_scoped(&host, &mut accounts, &collection, None, &a);
        assert_eq!(accounts[0].id, "b");
        assert_ne!(
            a,
            native_routing_namespace(&caller("key-a"), "gpt-fixture", &["a".into()], None)
        );
    }

    #[test]
    fn single_account_uses_existing_rank_and_rotates_only_equal_candidates() {
        let host = NativeHostBridge::default();
        let mut collection = collection();
        collection.routing_strategy = CodexLocalAccessRoutingStrategy::SingleAccount;
        let mut a = account("a");
        let mut b = account("b");
        a.plan_type = Some("plus".into());
        b.plan_type = Some("pro".into());
        let mut accounts = vec![a.clone(), b.clone()];
        native_sort_accounts_scoped(&host, &mut accounts, &collection, None, "scope");
        assert_eq!(accounts[0].id, "b");
        native_commit_routing_cursor(&host, "scope", "b");
        native_sort_accounts_scoped(&host, &mut accounts, &collection, None, "scope");
        assert_eq!(accounts[0].id, "b");
        a.plan_type = b.plan_type.clone();
        let mut accounts = vec![a, b];
        native_sort_accounts_scoped(&host, &mut accounts, &collection, None, "scope");
        assert_eq!(accounts[0].id, "a");
    }

    #[tokio::test]
    async fn soft_cache_has_ttl_eviction_and_settings_epoch_without_rejecting_new_history() {
        let host = NativeHostBridge::default();
        native_contract::Host::set_session_affinity_enabled(&host, true);
        let epoch = host.binding_epoch.load(Ordering::Acquire);
        let mut bindings = host.bindings.lock().await;
        native_prune_soft_bindings(&host, &mut bindings, epoch, true, 60_000);
        native_publish_soft_bindings(
            &host,
            &mut bindings,
            (0..=NATIVE_SOFT_BINDING_LIMIT).map(|index| format!("session-{index:08}")),
            "a",
            now_ms(),
        );
        assert_eq!(bindings.len(), NATIVE_SOFT_BINDING_LIMIT);
        assert!(bindings.contains_key(&format!("session-{:08}", NATIVE_SOFT_BINDING_LIMIT)));
        native_publish_soft_bindings(
            &host,
            &mut bindings,
            ["expired".into()],
            "a",
            now_ms() - 61_000,
        );
        native_prune_soft_bindings(&host, &mut bindings, epoch, true, 60_000);
        assert!(!bindings.contains_key("expired"));
        native_contract::Host::set_session_affinity_enabled(&host, false);
        native_contract::Host::set_session_affinity_enabled(&host, true);
        let changed = host.binding_epoch.load(Ordering::Acquire);
        assert_ne!(epoch, changed);
        native_prune_soft_bindings(&host, &mut bindings, changed, true, 60_000);
        assert!(bindings.is_empty());
        assert_ne!(
            native_soft_binding_key("caller-a", "model-a", "session"),
            native_soft_binding_key("caller-b", "model-a", "session")
        );
        assert_ne!(
            native_soft_binding_key("caller-a", "model-a", "session"),
            native_soft_binding_key("caller-a", "model-b", "session")
        );
    }

    #[test]
    fn model_suffix_keeps_base_acl_denials_and_search_empty_model_no_fictional_fallback() {
        let mut collection = collection();
        assert_eq!(
            native_model_for_key(&collection, None, "gpt-fixture(high)").unwrap(),
            "gpt-fixture"
        );
        collection.excluded_models = vec!["gpt-fixture".into()];
        assert!(native_model_for_key(&collection, None, "gpt-fixture(high)").is_err());
        assert_eq!(native_model_for_key(&collection, None, "").unwrap(), "");
        let plan = native_model_route_plan(&collection, None, "").unwrap();
        assert_eq!(
            native_operation_model(&collection, &plan, &account("a"), Some("search"), false),
            Some(String::new())
        );
        assert!(!native_auxiliary_eligible(&account("a"), Some("search")));
    }
}
