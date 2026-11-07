use super::*;

#[test]
fn canonical_members_save_is_atomic_preserves_source_credentials_and_remembers_mode() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("canonical-source");
        let other = lifecycle_source("canonical-other");
        let grant = fixture.login().await;
        let before_binding = access::chatgpt_plan_state_snapshot().await.unwrap();
        bind_account(&grant.id, &source.id).await.unwrap();
        assert_eq!(
            serde_json::to_value(
                access::chatgpt_plan_state_snapshot()
                    .await
                    .unwrap()
                    .collection
            )
            .unwrap(),
            serde_json::to_value(before_binding.collection).unwrap()
        );
        assert!(codex_account::load_account(
            &codex_account::chatgpt_plan_projection_id(&grant.id).unwrap()
        )
        .is_none());
        let before = access::chatgpt_plan_state_snapshot().await.unwrap();
        let invalid = access::save_local_access_accounts_with_official_bindings(
            vec![source.id.clone(), other.id.clone()],
            true,
            None,
            None,
            Some(false),
            Some(8000),
            None,
            Some(HashMap::from([
                (source.id.clone(), grant.id.clone()),
                (other.id.clone(), grant.id.clone()),
            ])),
            None,
        )
        .await;
        assert!(invalid.is_err());
        assert_eq!(
            serde_json::to_value(
                access::chatgpt_plan_state_snapshot()
                    .await
                    .unwrap()
                    .collection
            )
            .unwrap(),
            serde_json::to_value(before.collection).unwrap()
        );
        let saved = access::save_local_access_accounts_with_official_bindings(
            vec![source.id.clone()],
            true,
            Some(vec![]),
            Some(vec![source.id.clone()]),
            Some(false),
            Some(8000),
            Some(HashMap::from([(
                source.id.clone(),
                crate::models::codex_local_access::CodexLocalAccessImageGenerationPolicy::Enabled,
            )])),
            Some(HashMap::from([(source.id.clone(), grant.id.clone())])),
            Some(false),
        )
        .await
        .unwrap()
        .collection
        .unwrap();
        assert_eq!(saved.account_ids, vec![source.id.clone()]);
        assert_eq!(saved.official_account_bindings[&source.id], grant.id);
        assert!(!saved.session_affinity);
        assert!(!saved.session_identity_mapping_enabled);
        assert!(saved
            .custom_routing_rules
            .iter()
            .any(|rule| rule.account_id == source.id && rule.is_preferred));
        assert_eq!(
            serde_json::to_value(codex_account::load_account(&source.id).unwrap().tokens).unwrap(),
            serde_json::to_value(&source.tokens).unwrap()
        );
        let mut overlay = official_runtime_overlay(source.clone(), &grant.id);
        assert_eq!(overlay.id, source.id);
        assert!(overlay.tokens.access_token.is_empty());
        overlay.account_name = Some("must not persist overlay".into());
        assert_eq!(
            codex_account::save_account(&overlay).unwrap_err(),
            "API_SERVICE_RUNTIME_ACCOUNT_READ_ONLY"
        );
        let local = access::save_local_access_accounts_with_official_bindings(
            vec![source.id.clone()],
            true,
            None,
            None,
            None,
            None,
            None,
            Some(HashMap::new()),
            None,
        )
        .await
        .unwrap();
        let local = local.collection.unwrap();
        assert!(local.official_account_bindings.is_empty());
        assert!(!local.session_identity_mapping_enabled,
            "member edits without the optional setting must preserve the saved opt-out");
        assert_eq!(
            get_account(&grant.id)
                .await
                .unwrap()
                .last_service_mode
                .as_deref(),
            Some("codex")
        );
        access::remove_local_access_account(&source.id)
            .await
            .unwrap();
        assert!(get_account(&grant.id).await.unwrap().connected);
        assert_eq!(
            get_account(&grant.id)
                .await
                .unwrap()
                .last_service_mode
                .as_deref(),
            Some("codex")
        );
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}

#[test]
fn canonical_legacy_migration_requires_verified_source_and_preserves_newer_selection() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("legacy-canonical-source");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        let projection = codex_account::save_chatgpt_plan_projection(
            &grant.id,
            &source.email,
            "legacy",
            vec!["test-model".into()],
        )
        .unwrap();
        access::save_local_access_accounts(
            vec![projection.id.clone()],
            true,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let state = access::create_local_access_api_key(Some("Legacy scoped".into()))
            .await
            .unwrap();
        let key = state
            .collection
            .unwrap()
            .api_keys
            .into_iter()
            .find(|key| key.label == "Legacy scoped")
            .unwrap();
        access::update_local_access_api_key(
            key.id.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(vec![projection.id.clone()]),
            Some(false),
        )
        .await
        .unwrap();
        access::set_local_access_api_key_account_priority(
            key.id.clone(),
            projection.id.clone(),
            true,
        )
        .await
        .unwrap();
        access::migrate_legacy_official_member(&grant.id, &source.id)
            .await
            .unwrap();
        let migrated = access::chatgpt_plan_state_snapshot()
            .await
            .unwrap()
            .collection
            .unwrap();
        assert_eq!(migrated.account_ids, vec![source.id.clone()]);
        assert_eq!(migrated.official_account_bindings[&source.id], grant.id);
        let scope = migrated
            .api_keys
            .iter()
            .find(|entry| entry.id == key.id)
            .unwrap();
        assert_eq!(scope.account_ids, vec![source.id.clone()]);
        assert_eq!(scope.priority_account_ids, vec![source.id.clone()]);
        let revision = migrated.chatgpt_plan_service_revision;
        access::migrate_legacy_official_member(&grant.id, &source.id)
            .await
            .unwrap();
        assert_eq!(
            access::chatgpt_plan_state_snapshot()
                .await
                .unwrap()
                .collection
                .unwrap()
                .chatgpt_plan_service_revision,
            revision
        );
        service::set_service_mode_with_intent(&source.id, Some(&grant.id), "codex", None, "switch")
            .await
            .unwrap();
        access::append_local_access_accounts(vec![projection.id.clone()])
            .await
            .unwrap();
        access::migrate_legacy_official_member(&grant.id, &source.id)
            .await
            .unwrap();
        let conflict = access::chatgpt_plan_state_snapshot()
            .await
            .unwrap()
            .collection
            .unwrap();
        assert!(conflict.official_account_bindings.is_empty());
        assert!(
            conflict.account_ids.contains(&source.id)
                && conflict.account_ids.contains(&projection.id)
        );
        assert!(conflict.paused_account_ids.contains(&projection.id));
        assert!(!conflict.paused_account_ids.contains(&source.id));
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}

#[test]
fn canonical_token_source_fence_does_not_modify_grant_health() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("token-source-fence");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        let before = storage::read(&grant.id).await.unwrap();
        assert!(
            inference_snapshot_for_source(&grant.id, "unrelated-source", true)
                .await
                .is_err()
        );
        let after = storage::read(&grant.id).await.unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.summary.check_error, before.summary.check_error);
        assert!(!after.summary.needs_reauth);
        assert_eq!(fixture.state.lock().unwrap().refresh_count, 0);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}

#[test]
fn canonical_unbound_legacy_member_is_retained_paused_without_credential_transfer() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let grant = fixture.login().await;
        let projection = codex_account::save_chatgpt_plan_projection(
            &grant.id,
            grant.email.as_deref().unwrap(),
            "legacy-unbound",
            vec!["test-model".into()],
        )
        .unwrap();
        access::save_local_access_accounts(
            vec![projection.id.clone()],
            true,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            inference_snapshot(&grant.id, false).await.unwrap_err(),
            "CHATGPT_PLAN_BINDING_REQUIRED"
        );
        service::list_accounts().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let state = access::chatgpt_plan_state_snapshot().await.unwrap();
                let collection = state.collection.unwrap();
                assert_eq!(collection.account_ids, vec![projection.id.clone()]);
                assert!(collection.official_account_bindings.is_empty());
                if collection.paused_account_ids.contains(&projection.id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(get_account(&grant.id).await.unwrap().connected);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}

#[test]
fn canonical_bulk_save_does_not_readd_a_member_removed_while_credentials_wait() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("bulk-concurrent-removal");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        service::set_service_mode(&source.id, Some(&grant.id), "codex", None)
            .await
            .unwrap();
        expire(&grant.id).await;
        let release = Arc::new(tokio::sync::Notify::new());
        fixture.state.lock().unwrap().refresh_release = Some(release.clone());
        let source_id = source.id.clone();
        let registration = grant.id.clone();
        let pending = tokio::spawn(async move {
            access::save_local_access_accounts_with_official_bindings(
                vec![source_id.clone()],
                true,
                None,
                None,
                None,
                None,
                None,
                Some(HashMap::from([(source_id, registration)])),
                None,
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while fixture.state.lock().unwrap().refresh_started == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        access::remove_local_access_account(&source.id)
            .await
            .unwrap();
        release.notify_one();
        assert_eq!(
            pending.await.unwrap().err().unwrap(),
            "CHATGPT_PLAN_SERVICE_CHANGED"
        );
        let state = access::chatgpt_plan_state_snapshot().await.unwrap();
        assert!(state.collection.as_ref().unwrap().account_ids.is_empty());
        assert!(state
            .collection
            .unwrap()
            .official_account_bindings
            .is_empty());
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}
