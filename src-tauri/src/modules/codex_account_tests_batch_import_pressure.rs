#[test]
fn fast_batch_import_limits_full_checkpoints_and_previews() {
    let mut budget = super::CodexBatchImportUpdateBudget::new(0);
    let mut checkpoints = 0;
    let mut previews = 0;
    for index in 1..=100 {
        let elapsed = std::time::Duration::from_millis(index as u64);
        checkpoints += usize::from(budget.checkpoint_due(index, 100, elapsed));
        previews += usize::from(budget.preview_due(index, 100, elapsed));
    }
    assert_eq!(checkpoints, 3); // Final checkpoint carries the terminal status.
    assert_eq!(previews, 1);
}

#[test]
fn slow_batch_import_keeps_time_based_progress_and_checkpoints() {
    let mut budget = super::CodexBatchImportUpdateBudget::new(50);
    assert!(budget.preview_due(51, 100, std::time::Duration::from_secs(1)));
    assert!(!budget.checkpoint_due(51, 100, std::time::Duration::from_secs(1)));
    assert!(budget.checkpoint_due(52, 100, std::time::Duration::from_secs(2)));
    assert!(!budget.checkpoint_due(53, 100, std::time::Duration::from_millis(2100)));
    assert!(budget.preview_due(100, 100, std::time::Duration::from_millis(2100)));
}

#[test]
fn interrupted_batch_checkpoint_resumes_without_losing_sources_or_finished_items() {
    let session = super::CodexBatchImportSession {
        status: "scanning".into(),
        check_quota: false,
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        scan_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        source_items: (0..100)
            .map(|i| super::CodexBatchImportSourceItem {
                source: "test.json".into(),
                value: serde_json::json!({ "index": i }),
            })
            .collect(),
        next_index: 25,
        total: 100,
        items: (0..25)
            .map(|i| super::CodexBatchImportCachedItem {
                preview: super::CodexBatchImportItem {
                    item_id: format!("test-{}", i),
                    source: "test.json".into(),
                    label: format!("test-{}", i),
                    account_id: None,
                    email: None,
                    account_type: "-".into(),
                    provider: None,
                    quota_status: "skipped".into(),
                    quota_error: None,
                    status: "invalid".into(),
                    error: Some("test".into()),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
                metadata: super::CodexPortableAccountMetadata::default(),
            })
            .collect(),
    };
    let snapshot = super::codex_batch_import_snapshot_from_session(&session);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    let restored =
        super::codex_batch_import_session_from_snapshot(serde_json::from_str(&serialized).unwrap());
    assert_eq!(restored.status, "cancelled");
    assert_eq!(restored.next_index, 25);
    assert_eq!(restored.source_items.len(), 100);
    assert_eq!(restored.items.len(), 25);
    assert_eq!(restored.items[24].preview.item_id, "test-24");
    assert_eq!(restored.source_items[25].value["index"], 25);
}

#[test]
fn very_large_fast_import_keeps_full_checkpoint_count_bounded() {
    let mut budget = super::CodexBatchImportUpdateBudget::new(0);
    let checkpoints = (1..=10_000)
        .filter(|index| budget.checkpoint_due(*index, 10_000, std::time::Duration::from_millis(1)))
        .count();
    assert_eq!(checkpoints, 19);
}

#[test]
fn account_profile_merge_preserves_concurrent_user_and_credential_updates() {
    let original = super::CodexAccount::new(
        "test-profile".into(),
        "test@example.com".into(),
        super::CodexTokens {
            access_token: "test-access".into(),
            id_token: "test-id".into(),
            refresh_token: None,
        },
    );
    let mut current = original.clone();
    current.account_name = Some("User edit".into());
    current.usage_updated_at = Some(123);
    assert!(super::merge_codex_account_profile(
        &mut current,
        &original,
        Some("Remote name".into()),
        Some("personal".into()),
        None
    ));
    assert_eq!(current.account_name.as_deref(), Some("User edit"));
    assert_eq!(current.account_structure.as_deref(), Some("personal"));
    assert_eq!(current.usage_updated_at, Some(123));
    current.tokens.access_token = "new-test-access".into();
    assert!(!super::merge_codex_account_profile(
        &mut current,
        &original,
        Some("Stale name".into()),
        Some("team".into()),
        None
    ));
    assert_eq!(current.account_structure.as_deref(), Some("personal"));
}
