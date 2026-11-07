#[test]
fn approved_issue_timeout_values_round_trip_to_sidecar_without_changing_service_settings() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let env = LocalAccessTestDataGuard::new("approved-issue-timeouts");
    let mut collection = test_local_access_collection(Vec::new());
    // Preparing configuration is enough; never launch a process or contact an upstream.
    collection.enabled = false;
    assert_eq!(collection.timeouts.sidecar_stream_open_timeout_ms, 60_000);
    collection.timeouts.sidecar_stream_open_timeout_ms = 1_000;
    collection.timeouts.sidecar_image_stream_open_timeout_ms = 600_000;
    super::normalize_timeouts(&mut collection.timeouts);
    super::save_collection_to_disk(&collection).unwrap();
    let before = fs::read(super::local_access_file_path().unwrap()).unwrap();
    let loaded = super::load_collection_from_disk().unwrap().unwrap();
    assert_eq!(loaded.timeouts.sidecar_stream_open_timeout_ms, 1_000);
    assert_eq!(loaded.timeouts.sidecar_image_stream_open_timeout_ms, 600_000);
    assert_eq!(loaded.timeouts.sidecar_stream_open_max_attempts, 1);
    assert_eq!(loaded.port, collection.port);
    assert_eq!(loaded.api_key, collection.api_key);
    assert_eq!(serde_json::to_value(&loaded.api_keys).unwrap(), serde_json::to_value(&collection.api_keys).unwrap());
    assert_eq!(loaded.account_ids, collection.account_ids);
    let sidecar_dir = env.data_dir.join("synthetic-sidecar");
    super::prepare_sidecar_launch_config_in_dir_sync(
        &loaded, sidecar_dir.clone(), HashMap::new(), None, HashMap::new(), true, None,
    ).unwrap();
    let config: Value = serde_json::from_str(&fs::read_to_string(super::sidecar_config_path(&sidecar_dir)).unwrap()).unwrap();
    assert_eq!(config["streaming"]["stream-open-timeout-ms"], 1_000);
    assert_eq!(config["streaming"]["image-stream-open-timeout-ms"], 600_000);
    assert_eq!(config["streaming"]["stream-open-max-attempts"], 1);
    assert_eq!(fs::read(super::local_access_file_path().unwrap()).unwrap(), before);
}
