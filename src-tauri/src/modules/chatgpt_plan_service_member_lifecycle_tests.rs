use super::*;

#[test]
fn authorization_release_keeps_member_scopes_history_and_explicit_disposition() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        for disposition in ["local_gateway", "pause"] {
            let source = lifecycle_source(&format!("release-{disposition}"));
            let grant = fixture.login().await;
            bind_account(&grant.id, &source.id).await.unwrap();
            service::set_service_mode(&source.id, Some(&grant.id), "official", None)
                .await
                .unwrap();
            let projection = codex_account::chatgpt_plan_projection_id(&grant.id).unwrap();
            let key_state = access::create_local_access_api_key(Some("Release scope".into()))
                .await
                .unwrap();
            let key = key_state
                .collection
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.label == "Release scope")
                .unwrap();
            access::update_local_access_api_key(
                key.id.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![source.id.clone()]),
                Some(false),
            )
            .await
            .unwrap();
            access::set_local_access_api_key_account_priority(
                key.id.clone(),
                source.id.clone(),
                true,
            )
            .await
            .unwrap();

            let released = service::release_authorization(&grant.id, disposition)
                .await
                .unwrap();
            assert!(released.remote_revoked);
            assert!(!released.authorization.connected);
            assert!(released.authorization.source_account_id.is_none());
            let collection = released.state.collection.unwrap();
            assert_eq!(collection.account_ids, vec![source.id.clone()]);
            assert_eq!(
                collection.paused_account_ids.contains(&source.id),
                disposition == "pause"
            );
            let scoped = collection
                .api_keys
                .iter()
                .find(|item| item.id == key.id)
                .unwrap();
            assert_eq!(scoped.account_ids, vec![source.id.clone()]);
            assert_eq!(scoped.priority_account_ids, vec![source.id.clone()]);
            assert_eq!(scoped.inherit_account_pool, Some(false));
            assert!(collection.official_account_bindings.is_empty());
            assert_eq!(released.state.service_members.len(), 1);
            assert_eq!(
                released.state.service_members[0].source_account_id,
                source.id
            );
            assert_eq!(
                released.state.service_members[0].paused,
                disposition == "pause"
            );
            assert!(codex_account::load_account(&projection).is_none());
            assert_eq!(
                codex_account::load_account(&source.id)
                    .unwrap()
                    .tokens
                    .access_token,
                source.tokens.access_token
            );
            assert_eq!(fixture.state.lock().unwrap().response_started, 0);
            assert!(storage::read(&grant.id)
                .await
                .unwrap()
                .encrypted_credentials
                .is_none());

            // Resume a valid local placeholder without a grant; this is a
            // switch of an existing member, never a new membership operation.
            let resumed =
                service::set_service_mode_with_intent(&source.id, None, "codex", None, "switch")
                    .await
                    .unwrap();
            assert_eq!(
                resumed.collection.as_ref().unwrap().account_ids,
                vec![source.id.clone()]
            );
            assert!(resumed.collection.unwrap().paused_account_ids.is_empty());
            access::remove_local_access_account(&source.id)
                .await
                .unwrap();
            assert_eq!(
                service::set_service_mode_with_intent(&source.id, None, "codex", None, "switch")
                    .await
                    .unwrap_err(),
                "CHATGPT_PLAN_SERVICE_MEMBER_REMOVED"
            );
            assert!(access::chatgpt_plan_pool_snapshot()
                .await
                .unwrap()
                .1
                .is_empty());
        }
    });
}

#[test]
fn authorization_release_preserves_existing_local_and_unjoined_states() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        for joined in [false, true] {
            let source = lifecycle_source(if joined { "local-member" } else { "nonmember" });
            let grant = fixture.login().await;
            bind_account(&grant.id, &source.id).await.unwrap();
            if joined {
                service::set_service_mode(&source.id, Some(&grant.id), "codex", None)
                    .await
                    .unwrap();
            }
            let before = access::chatgpt_plan_pool_snapshot().await.unwrap().1;
            let result = service::release_authorization(&grant.id, "pause")
                .await
                .unwrap();
            let collection = result.state.collection.unwrap();
            assert_eq!(collection.account_ids, before);
            assert!(collection.paused_account_ids.is_empty());
            assert_eq!(fixture.state.lock().unwrap().response_started, 0);
            if joined {
                access::remove_local_access_account(&source.id)
                    .await
                    .unwrap();
            }
        }
    });
}

#[test]
fn authorization_unbind_pauses_member_and_blocks_former_endpoint_without_revoking_grant() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("unbind-retained");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        service::set_service_mode(&source.id, Some(&grant.id), "official", None)
            .await
            .unwrap();
        let before = access_token(&grant.id, false).await.unwrap();
        let detached = service::unbind(&grant.id).await.unwrap();
        assert!(detached.connected && detached.source_account_id.is_none());
        assert_eq!(access_token(&grant.id, false).await.unwrap(), before);
        assert_eq!(
            inference_token(&grant.id, false).await.unwrap_err(),
            "CHATGPT_PLAN_BINDING_REQUIRED"
        );
        let state = access::get_local_access_state().await.unwrap();
        assert_eq!(
            state.collection.as_ref().unwrap().account_ids,
            vec![source.id.clone()]
        );
        assert_eq!(
            state.collection.unwrap().paused_account_ids,
            vec![source.id.clone()]
        );
        assert_eq!(fixture.state.lock().unwrap().revoke_count, 0);
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        bind_account(&grant.id, &source.id).await.unwrap();
        let resumed = service::set_service_mode_with_intent(
            &source.id,
            Some(&grant.id),
            "official",
            None,
            "switch",
        )
        .await
        .unwrap();
        assert!(resumed.collection.unwrap().paused_account_ids.is_empty());
        service::release_authorization(&grant.id, "pause")
            .await
            .unwrap();
    });
}

#[test]
fn authorization_release_broken_source_keeps_old_acl_paused_and_rejects_display_alias_resume() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let mut source = lifecycle_source("reused-source-id");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        service::set_service_mode(&source.id, Some(&grant.id), "official", None)
            .await
            .unwrap();
        let projection = codex_account::chatgpt_plan_projection_id(&grant.id).unwrap();
        source.account_id = Some("different-principal".into());
        codex_account::save_account(&source).unwrap();
        assert_eq!(
            service::release_authorization(&grant.id, "local_gateway")
                .await
                .err()
                .unwrap(),
            "CHATGPT_PLAN_SOURCE_CHANGED"
        );
        let result = service::release_authorization(&grant.id, "pause")
            .await
            .unwrap();
        let collection = result.state.collection.unwrap();
        assert_eq!(collection.account_ids, vec![source.id.clone()]);
        assert_eq!(collection.paused_account_ids, vec![source.id.clone()]);
        assert!(collection.official_account_bindings.is_empty());
        assert_eq!(result.state.service_members[0].source_account_id, source.id);
        assert!(result.state.service_members[0].paused);
        assert_eq!(
            service::set_service_mode_with_intent(&source.id, None, "codex", None, "switch")
                .await
                .unwrap_err(),
            "CHATGPT_PLAN_SOURCE_CHANGED"
        );
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
    });
}

#[test]
fn authorization_switch_cas_does_not_readd_a_concurrently_removed_member() {
    authorization_case(|| async {
        let fixture = Fixture::new().await;
        let source = lifecycle_source("switch-removed");
        let grant = fixture.login().await;
        bind_account(&grant.id, &source.id).await.unwrap();
        service::set_service_mode(&source.id, Some(&grant.id), "official", None)
            .await
            .unwrap();
        service::list_models(&grant.id).await.unwrap();
        let lock = storage::lock(&grant.id).await.unwrap();
        let id = grant.id.clone();
        let source_id = source.id.clone();
        let switch = tokio::spawn(async move {
            service::set_service_mode_with_intent(&source_id, Some(&id), "official", None, "switch")
                .await
        });
        tokio::task::yield_now().await;
        access::remove_local_access_account(&source.id)
        .await
        .unwrap();
        drop(lock);
        assert_eq!(
            switch.await.unwrap().unwrap_err(),
            "CHATGPT_PLAN_SERVICE_MEMBER_REMOVED"
        );
        assert!(access::chatgpt_plan_pool_snapshot()
            .await
            .unwrap()
            .1
            .is_empty());
        assert_eq!(fixture.state.lock().unwrap().response_started, 0);
        service::release_authorization(&grant.id, "pause")
            .await
            .unwrap();
    });
}
