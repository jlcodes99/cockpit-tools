use super::*;

fn credentials(access: &str, refresh: &str, expires_at: Option<i64>) -> Value {
    let mut value = json!({"claudeAiOauth": {"accessToken": access, "refreshToken": refresh}});
    if let Some(expiry) = expires_at {
        value["claudeAiOauth"]["expiresAt"] = json!(expiry);
    }
    value
}

#[test]
fn cli_sync_rejects_stale_cleared_and_incomplete_instance_tokens() {
    let saved = credentials("new", "new-refresh", Some(2000));
    for candidate in [
        credentials("old", "old-refresh", Some(1000)),
        credentials("", "", Some(3000)),
        credentials("old", "", Some(3000)),
        credentials("different", "different-refresh", Some(2000)),
        credentials("new", "new-refresh", Some(1000)),
        credentials("different", "different-refresh", None),
    ] {
        assert!(!should_sync_cli_oauth_credentials(Some(&saved), &candidate));
    }
}

#[test]
fn cli_sync_accepts_newer_cli_rotation_and_identical_credentials() {
    let saved = credentials("old", "old-refresh", Some(1000));
    let rotated = credentials("new", "new-refresh", Some(2000));
    assert!(should_sync_cli_oauth_credentials(Some(&saved), &rotated));
    assert!(should_sync_cli_oauth_credentials(Some(&rotated), &rotated));
    assert!(should_sync_cli_oauth_credentials(None, &rotated));
    let unknown_expiry = credentials("same", "same-refresh", None);
    assert!(should_sync_cli_oauth_credentials(
        Some(&unknown_expiry),
        &unknown_expiry
    ));
    assert!(!should_sync_cli_oauth_credentials(
        Some(&unknown_expiry),
        &credentials("different", "different-refresh", None)
    ));
}

#[test]
fn cli_sync_updates_only_oauth_fields_and_is_idempotent() {
    let saved = credentials("new", "new-refresh", Some(2000));
    let mut instance = credentials("old", "old-refresh", Some(1000));
    instance["otherCredential"] = json!({"token": "keep"});
    let merged = merge_newer_account_oauth_credentials(&saved, &instance).unwrap();
    assert_eq!(merged["claudeAiOauth"], saved["claudeAiOauth"]);
    assert_eq!(merged["otherCredential"], instance["otherCredential"]);
    assert!(merge_newer_account_oauth_credentials(&saved, &merged).is_none());
}

#[test]
fn cli_sync_does_not_replace_newer_ambiguous_or_logged_out_instances() {
    let saved = credentials("saved", "saved-refresh", Some(2000));
    for instance in [
        credentials("newer", "newer-refresh", Some(3000)),
        credentials("different", "different-refresh", Some(2000)),
        credentials("", "", Some(0)),
        json!({}),
        Value::Null,
    ] {
        assert!(merge_newer_account_oauth_credentials(&saved, &instance).is_none());
    }
    assert!(
        merge_newer_account_oauth_credentials(&credentials("saved", "", Some(3000)), &saved)
            .is_none()
    );
}

#[test]
fn cli_sync_requires_matching_account_and_organization() {
    let config = json!({"oauthAccount": {
        "accountUuid": "account-a", "emailAddress": "a@example.com", "organizationUuid": "org-a"
    }});
    let saved = derive_account_from_snapshots(
        credentials("saved", "saved-refresh", Some(2000)),
        config.clone(),
        None,
    )
    .unwrap();
    assert!(cli_config_matches_account(&saved, &config));
    let mut changed = config.clone();
    changed["oauthAccount"]["accountUuid"] = json!("account-b");
    assert!(!cli_config_matches_account(&saved, &changed));
    let mut changed = config;
    changed["oauthAccount"]["organizationUuid"] = json!("org-b");
    assert!(!cli_config_matches_account(&saved, &changed));
    assert!(!cli_config_matches_account(&saved, &json!({})));
}

#[test]
fn cli_sync_targets_only_bound_cli_instances_and_deduplicates_paths() {
    let store: crate::models::InstanceStore = serde_json::from_value(json!({
        "instances": [
            {"id": "cli-a", "name": "A", "userDataDir": "/a", "extraArgs": "",
             "bindAccountId": "a", "launchMode": "cli", "createdAt": 0},
            {"id": "cli-duplicate", "name": "Duplicate", "userDataDir": "/a", "extraArgs": "",
             "bindAccountId": "a", "launchMode": "cli", "createdAt": 0},
            {"id": "cli-b", "name": "B", "userDataDir": "/b", "extraArgs": "",
             "bindAccountId": "b", "launchMode": "cli", "createdAt": 0},
            {"id": "desktop-a", "name": "Desktop", "userDataDir": "/desktop", "extraArgs": "",
             "bindAccountId": "a", "launchMode": "app", "createdAt": 0}
        ],
        "defaultSettings": {"bindAccountId": "a", "launchMode": "cli"}
    }))
    .unwrap();
    let dirs = bound_cli_config_dirs(&store, "a", Some(PathBuf::from("/default")));
    assert_eq!(
        dirs,
        BTreeSet::from([PathBuf::from("/a"), PathBuf::from("/default")])
    );
}

#[test]
fn cli_sync_queue_coalesces_requests_and_allows_retry_after_completion() {
    let mut queue = CliCredentialSyncQueue::default();
    assert!(queue.enqueue("a"));
    assert!(!queue.enqueue("a"));
    assert!(!queue.enqueue("a"));
    assert!(queue.enqueue("b"));
    assert!(queue.finish_pass("a"));
    assert!(!queue.finish_pass("a"));
    assert!(!queue.finish_pass("b"));
    assert!(queue.accounts.is_empty());
    assert!(queue.enqueue("a"));
}

#[test]
fn oauth_result_merges_user_edits_and_rejects_reauthorization_or_deleted_accounts() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let dir = std::env::temp_dir().join(format!("claude-oauth-cas-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
    let config =
        json!({"oauthAccount": {"accountUuid": "cas-account", "emailAddress": "cas@example.com"}});
    let original = derive_account_from_snapshots(
        credentials("original", "refresh-1", Some(1000)),
        config,
        None,
    )
    .unwrap();
    let original = save_account_and_index(original).unwrap();
    update_account_tags(&original.id, vec!["keep-new-tag".to_string()]).unwrap();
    update_account_note(&original.id, Some("keep-new-note")).unwrap();
    let rotated = credentials("rotated", "refresh-2", Some(2000));
    let saved = update_oauth_account_if_current(
        &original.id,
        &original.claude_credentials_raw,
        |current| {
            current.claude_credentials_raw = Some(rotated.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(saved.tags, Some(vec!["keep-new-tag".to_string()]));
    assert_eq!(saved.account_note.as_deref(), Some("keep-new-note"));
    assert_eq!(saved.claude_credentials_raw, Some(rotated.clone()));

    // An older refresh or stale CLI snapshot completes after rotation.
    let result =
        update_oauth_account_if_current(&original.id, &original.claude_credentials_raw, |_| {
            panic!("stale OAuth result must not be applied")
        })
        .unwrap();
    assert_eq!(result.claude_credentials_raw, Some(rotated.clone()));

    // Usage requested with the old login cannot overwrite a new login/status.
    let mut reauthorized = saved.clone();
    reauthorized.claude_credentials_raw = Some(credentials("reauth", "refresh-3", Some(3000)));
    reauthorized.status = Some("new-login".to_string());
    let reauthorized = save_account_and_index(reauthorized).unwrap();
    let mut old_usage = saved;
    old_usage.claude_usage_raw = Some(json!({"stale": true}));
    let result = save_oauth_quota_result(&old_usage).unwrap();
    assert_eq!(
        result.claude_credentials_raw,
        reauthorized.claude_credentials_raw
    );
    assert_eq!(result.status.as_deref(), Some("new-login"));
    assert!(result.claude_usage_raw.is_none());

    remove_account(&original.id).unwrap();
    assert!(save_oauth_quota_result(&old_usage).is_err());
    assert!(!account_file_path(&original.id).unwrap().exists());
    assert!(!load_index()
        .unwrap()
        .accounts
        .iter()
        .any(|account| account.id == original.id));
    if let Some(value) = previous {
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value);
    } else {
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn quota_refresh_guard_deduplicates_and_releases_on_cancellation() {
    let id = format!("refresh-guard-{}", uuid::Uuid::new_v4());
    let first = ClaudeQuotaRefreshGuard::acquire(&id).unwrap();
    assert!(ClaudeQuotaRefreshGuard::acquire(&id).is_none());
    assert!(ClaudeQuotaRefreshGuard::acquire("another-account").is_some());
    drop(first);
    assert!(ClaudeQuotaRefreshGuard::acquire(&id).is_some());
}
