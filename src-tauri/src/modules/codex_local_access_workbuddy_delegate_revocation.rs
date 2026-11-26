// Dedicated WorkBuddy grants are immutable and revocation is monotonic. This
// merge runs both inside the disk field transaction and on runtime publication,
// so an unrelated stale settings/usage snapshot cannot resurrect a deleted key.
fn merge_workbuddy_delegate_revocations(
    next: &mut CodexLocalAccessCollection,
    previous_keys: &[CodexLocalAccessApiKey],
    previous_revoked: &[String],
) {
    let mut revoked: std::collections::BTreeSet<String> = next
        .workbuddy_delegate_revoked_key_ids
        .iter()
        .chain(previous_revoked)
        .cloned()
        .collect();
    for previous in previous_keys
        .iter()
        .filter(|key| key.workbuddy_delegate.is_some())
    {
        match next.api_keys.iter_mut().find(|key| key.id == previous.id) {
            Some(key) => {
                // An existing dedicated key cannot be repurposed by a generic
                // editor that omitted its typed execution mode.
                key.workbuddy_delegate = previous.workbuddy_delegate.clone();
                if !previous.enabled || !key.enabled {
                    revoked.insert(key.id.clone());
                }
            }
            None => {
                revoked.insert(previous.id.clone());
            }
        }
    }
    for key in &next.api_keys {
        if key.workbuddy_delegate.is_some() && !key.enabled {
            revoked.insert(key.id.clone());
        }
    }
    for key in &mut next.api_keys {
        if revoked.contains(&key.id) {
            key.enabled = false;
        }
    }
    next.workbuddy_delegate_revoked_key_ids = revoked.into_iter().collect();
}

#[cfg(test)]
mod workbuddy_delegate_revocation_tests {
    use super::*;
    fn fixture() -> CodexLocalAccessCollection {
        let mut collection = new_empty_local_access_collection().unwrap();
        collection.api_keys = vec![serde_json::from_value(json!({
            "id":"owned-native-key","key":"synthetic-key","label":"fixture", "enabled":true,
            "workbuddyDelegate":{"providerAccountId":"linked","modelId":"fixture","publicModel":"workbuddy/fixture","reasoningEffort":"low"}
        })).unwrap()];
        collection
    }
    #[test]
    fn workbuddy_delegate_stale_disk_snapshot_cannot_reenable_revoked_or_deleted_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("collection.json");
        let original = fixture();
        save_collection_to_path(&path, &original).unwrap();
        let mut revoked = original.clone();
        revoked.api_keys[0].enabled = false;
        revoked.api_keys[0].updated_at += 1;
        save_collection_to_path(&path, &revoked).unwrap();
        let mut stale = original.clone();
        stale.debug_logs = true;
        stale.updated_at = i64::MAX; // New collection timestamp does not grant key authority.
        save_collection_to_path(&path, &stale).unwrap();
        let committed: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(!committed.api_keys[0].enabled);
        assert_eq!(
            committed.workbuddy_delegate_revoked_key_ids,
            ["owned-native-key"]
        );
        let mut deleted = committed.clone();
        deleted.api_keys.clear();
        save_collection_to_path(&path, &deleted).unwrap();
        save_collection_to_path(&path, &stale).unwrap();
        let committed: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(!committed.api_keys[0].enabled);
        assert_eq!(
            committed.workbuddy_delegate_revoked_key_ids,
            ["owned-native-key"]
        );
    }
    #[test]
    fn workbuddy_delegate_runtime_merge_preserves_mode_and_revocation_but_not_other_keys() {
        let original = fixture();
        let mut current = original.clone();
        current.api_keys[0].enabled = false;
        merge_workbuddy_delegate_revocations(&mut current, &original.api_keys, &[]);
        let mut stale = original;
        stale.api_keys[0].workbuddy_delegate = None;
        let mut ordinary = build_local_access_api_key(Some("ordinary"));
        ordinary.id = "ordinary".into();
        ordinary.enabled = true;
        stale.api_keys.push(ordinary);
        preserve_runtime_collection_fields(&mut stale, &current);
        assert!(!stale.api_keys[0].enabled);
        assert!(stale.api_keys[0].workbuddy_delegate.is_some());
        assert!(stale.api_keys[1].enabled);
    }
    #[test]
    fn workbuddy_delegate_configuration_cannot_switch_to_sidecar() {
        let collection = fixture();
        assert_eq!(
            native_validate_chatgpt_plan_mode(&collection, CodexLocalAccessGatewayMode::Sidecar)
                .unwrap_err(),
            "WORKBUDDY_DELEGATE_NATIVE_REQUIRED"
        );
        let current = serde_json::to_string(&collection).unwrap();
        assert_eq!(
            native_patch_mode(&current, CodexLocalAccessGatewayMode::Sidecar).unwrap_err(),
            "WORKBUDDY_DELEGATE_NATIVE_REQUIRED"
        );
        assert!(native_validate_chatgpt_plan_mode(
            &collection,
            CodexLocalAccessGatewayMode::Native
        )
        .is_ok());
        let mut revoked = collection;
        revoked.api_keys[0].enabled = false;
        assert!(
            native_validate_chatgpt_plan_mode(&revoked, CodexLocalAccessGatewayMode::Sidecar)
                .is_ok()
        );
        assert!(native_patch_mode(
            &serde_json::to_string(&revoked).unwrap(),
            CodexLocalAccessGatewayMode::Sidecar
        )
        .is_ok());
    }
}
