use super::*;

fn retired_config() -> CodexLocalAccessMultiAgentConfig {
    CodexLocalAccessMultiAgentConfig {
        optimize: false,
        orphan_delegation: false,
        compat_models: vec![
            crate::models::codex_local_access::CodexLocalAccessCompatModel {
                account_id: "missing".into(),
                model: "*".into(),
            },
        ],
    }
}
fn config() -> CodexLocalAccessMultiAgentConfig {
    Default::default()
}
fn collection() -> CodexLocalAccessCollection {
    let mut value = new_empty_local_access_collection().unwrap();
    value.account_ids = vec!["a".into()];
    value
}
#[test]
fn legacy_multi_agent_flags_and_retired_model_lists_cannot_disable_support() {
    for value in [
        json!({}),
        json!({
            "optimize":false,"orphanDelegation":false,
            "compatModels":[{"accountId":"missing","model":"*"}]
        }),
    ] {
        let legacy: CodexLocalAccessMultiAgentConfig = serde_json::from_value(value).unwrap();
        assert_eq!(legacy, config());
        assert!(serde_json::to_value(&legacy)
            .unwrap()
            .get("compatModels")
            .is_none());
    }
    assert_eq!(
        normalize_multi_agent(retired_config(), &[]).unwrap(),
        config()
    );
}
#[test]
fn multi_agent_field_transaction_canonicalizes_retired_inputs_and_preserves_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.json");
    let original = collection();
    fs::write(&path, serde_json::to_string(&original).unwrap()).unwrap();
    let published = persist_multi_agent_at_path(&path, retired_config(), None).unwrap();
    assert_eq!(published.multi_agent, config());
    save_collection_to_path(&path, &original).unwrap();
    let stored: CodexLocalAccessCollection =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored.multi_agent, config());
    assert_eq!(stored.account_ids, original.account_ids);
    assert_eq!(stored.api_key, original.api_key);
    fs::write(&path, "broken").unwrap();
    assert!(persist_multi_agent_at_path(&path, Default::default(), None).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "broken");
}
#[test]
fn multi_agent_account_maintenance_discards_retired_policy_and_is_idempotent() {
    let mut value = collection();
    value.multi_agent = retired_config();
    assert!(prune_multi_agent_references(&mut value));
    assert_eq!(value.multi_agent, config());
    assert!(!prune_multi_agent_references(&mut value));
    assert!(replace_collection_account_references(&mut value, "a", "b"));
    assert_eq!(value.account_ids, vec!["b"]);
    assert_eq!(value.multi_agent, config());
}

struct TestData {
    _directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    previous_server: Option<std::ffi::OsString>,
}
impl TestData {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        std::env::set_var("CHATGPT_PLAN_TEST_SERVER", "http://127.0.0.1:9");
        Self {
            _directory: directory,
            previous,
            previous_server,
        }
    }
}
impl Drop for TestData {
    fn drop(&mut self) {
        for (name, value) in [
            ("COCKPIT_TOOLS_TEST_DATA_DIR", &self.previous),
            ("CHATGPT_PLAN_TEST_SERVER", &self.previous_server),
        ] {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}

#[tokio::test]
async fn multi_agent_command_publishes_and_reloads_without_starting_service() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    with_chatgpt_plan_test_runtime(|| async {
        let _data = TestData::new();
        let path = local_access_file_path().unwrap();
        let original = collection();
        save_collection_to_path(&path, &original).unwrap();
        {
            let mut runtime = gateway_runtime().lock().await;
            runtime.collection = Some(original.clone());
            runtime.loaded = true;
        }
        let state =
            crate::commands::codex_multi_agent::codex_local_access_update_multi_agent(config())
                .await
                .unwrap();
        assert_eq!(state.collection.unwrap().multi_agent, config());
        assert!(!state.running);
        {
            let mut runtime = gateway_runtime().lock().await;
            sync_runtime_collection(&mut runtime, original);
            assert_eq!(runtime.collection.as_ref().unwrap().multi_agent, config());
            runtime.collection = None;
            runtime.loaded = false;
        }
        ensure_runtime_loaded_without_start().await.unwrap();
        let state = snapshot_state_without_gateway_reload().await.unwrap();
        assert_eq!(state.collection.unwrap().multi_agent, config());
        assert!(!state.running);
        let canonical = crate::commands::codex_multi_agent::codex_local_access_update_multi_agent(
            retired_config(),
        )
        .await
        .unwrap();
        assert_eq!(canonical.collection.unwrap().multi_agent, config());
        let stored: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(stored.multi_agent, config());
    })
    .await;
}
