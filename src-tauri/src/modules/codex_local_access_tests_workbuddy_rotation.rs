// These fixtures execute the public commands against isolated storage only.
// No WorkBuddy account is imported and no native model process is launched.
fn workbuddy_rotation_managed_key() -> CodexLocalAccessApiKey {
    let mut key = super::build_local_access_api_key(Some("Synthetic managed key"));
    let config = crate::models::codex_local_access::CodexLocalAccessWorkbuddyDelegate {
        provider_account_id: "synthetic-workbuddy-link".into(),
        public_model: "workbuddy/glm-5.3-flash".into(),
        model_id: "glm-5.3-flash".into(),
        reasoning_effort: "low".into(),
    };
    key.inherit_account_pool = Some(false);
    key.account_ids = vec![config.provider_account_id.clone()];
    key.allowed_models = vec![config.public_model.clone()];
    key.workbuddy_delegate = Some(config);
    key
}

async fn workbuddy_rotation_install_fixture(
    keys: Vec<CodexLocalAccessApiKey>,
) -> CodexLocalAccessCollection {
    let mut collection = super::new_empty_local_access_collection().unwrap();
    // Commands remain real, but an ordinary successful rotation must not start
    // a listener, perform source discovery or touch an installed model client.
    collection.enabled = false;
    collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
    collection.api_key = keys[0].key.clone();
    collection.api_keys = keys;
    super::normalize_collection_api_keys(&mut collection);
    // A disabled managed key comes from an existing grant, not from a first
    // write of a fabricated already-disabled collection. Seed that history so
    // the real disk merge establishes its revocation tombstone.
    let mut initially_enabled = collection.clone();
    for key in &mut initially_enabled.api_keys {
        if key.workbuddy_delegate.is_some() {
            key.enabled = true;
        }
    }
    super::normalize_collection_api_keys(&mut initially_enabled);
    super::save_collection_to_disk(&initially_enabled).unwrap();
    super::save_collection_to_disk(&collection).unwrap();
    let collection = super::load_collection_from_disk().unwrap().unwrap();
    let mut runtime = super::gateway_runtime().lock().await;
    runtime.loaded = true;
    runtime.collection = Some(collection.clone());
    collection
}

async fn workbuddy_rotation_rejects_managed(
    legacy: bool,
    enabled: bool,
    disabled_ordinary_first: bool,
) {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("workbuddy-managed-rotation");
    super::with_chatgpt_plan_test_runtime(|| async {
        let mut managed = workbuddy_rotation_managed_key();
        managed.enabled = enabled;
        let mut keys = Vec::new();
        if disabled_ordinary_first {
            let mut ordinary = super::build_local_access_api_key(Some("Disabled ordinary key"));
            ordinary.enabled = false;
            keys.push(ordinary);
        }
        keys.push(managed.clone());
        workbuddy_rotation_install_fixture(keys).await;
        let root = super::workbuddy_delegate_profile_root().unwrap();
        let profile =
            super::workbuddy_delegate_write_profile(&root, &managed, "synthetic-source", 12345)
                .unwrap();
        let dir = PathBuf::from(&profile.profile_dir);
        let marker = dir.join(super::WORKBUDDY_DELEGATE_PROFILE_MARKER);
        let marker_before = fs::read(&marker).unwrap();
        let collection_path = super::local_access_file_path().unwrap();
        let disk_before = fs::read(&collection_path).unwrap();
        fs::write(dir.join("user-session.txt"), "preserve user session").unwrap();

        let result = if legacy {
            super::rotate_local_access_api_key().await
        } else {
            super::rotate_local_access_named_api_key(managed.id.clone()).await
        };
        assert_eq!(
            result.err().as_deref(),
            Some("WORKBUDDY_DELEGATE_KEY_ROTATION_UNSUPPORTED")
        );
        assert_eq!(fs::read(&collection_path).unwrap(), disk_before);
        assert_eq!(fs::read(&marker).unwrap(), marker_before);
        let runtime = super::gateway_runtime().lock().await;
        let retained = runtime
            .collection
            .as_ref()
            .unwrap()
            .api_keys
            .iter()
            .find(|key| key.id == managed.id)
            .unwrap();
        assert_eq!(retained.key, managed.key);
        assert_eq!(retained.enabled, enabled);
        drop(runtime);

        // A rejected reset must not strand the profile's revocation ownership.
        super::restore_workbuddy_delegate_profile(profile.id)
            .await
            .unwrap();
        let saved = super::load_collection_from_disk().unwrap().unwrap();
        assert!(
            !saved
                .api_keys
                .iter()
                .find(|key| key.id == managed.id)
                .unwrap()
                .enabled
        );
        assert!(saved
            .workbuddy_delegate_revoked_key_ids
            .contains(&managed.id));
        assert!(!marker.exists());
        assert_eq!(
            fs::read_to_string(dir.join("user-session.txt")).unwrap(),
            "preserve user session"
        );
    })
    .await;
}

#[tokio::test]
async fn workbuddy_delegate_named_rotation_preserves_managed_key_and_revocation() {
    workbuddy_rotation_rejects_managed(false, true, false).await;
}

#[tokio::test]
async fn workbuddy_delegate_primary_rotation_checks_selected_enabled_key() {
    workbuddy_rotation_rejects_managed(true, true, true).await;
}

#[tokio::test]
async fn workbuddy_delegate_named_rotation_rejects_disabled_managed_key() {
    workbuddy_rotation_rejects_managed(false, false, false).await;
}

#[tokio::test]
async fn workbuddy_delegate_primary_rotation_checks_disabled_fallback_key() {
    workbuddy_rotation_rejects_managed(true, false, false).await;
}

async fn workbuddy_rotation_preserves_ordinary_rotation(legacy: bool) {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("workbuddy-ordinary-rotation");
    super::with_chatgpt_plan_test_runtime(|| async {
        let ordinary = super::build_local_access_api_key(Some("Ordinary key"));
        let managed = workbuddy_rotation_managed_key();
        workbuddy_rotation_install_fixture(vec![ordinary.clone(), managed.clone()]).await;
        let result = if legacy {
            super::rotate_local_access_api_key().await
        } else {
            super::rotate_local_access_named_api_key(ordinary.id.clone()).await
        };
        result.expect("ordinary key rotation remains supported");
        let saved = super::load_collection_from_disk().unwrap().unwrap();
        let rotated = saved
            .api_keys
            .iter()
            .find(|key| key.id == ordinary.id)
            .unwrap();
        assert_ne!(rotated.key, ordinary.key);
        assert_eq!(saved.api_key, rotated.key);
        assert_eq!(rotated.enabled, ordinary.enabled);
        let unchanged = saved
            .api_keys
            .iter()
            .find(|key| key.id == managed.id)
            .unwrap();
        assert_eq!(unchanged.key, managed.key);
        assert_eq!(unchanged.workbuddy_delegate, managed.workbuddy_delegate);
    })
    .await;
}

#[tokio::test]
async fn workbuddy_delegate_named_rotation_keeps_ordinary_keys_working() {
    workbuddy_rotation_preserves_ordinary_rotation(false).await;
}

#[tokio::test]
async fn workbuddy_delegate_primary_rotation_keeps_ordinary_keys_working() {
    workbuddy_rotation_preserves_ordinary_rotation(true).await;
}
