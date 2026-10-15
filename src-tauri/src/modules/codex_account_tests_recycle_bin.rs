fn recycle_seed_account(tokens: CodexTokens) -> CodexAccount {
    upsert_account(tokens).expect("seed through actual account identity flow")
}

#[test]
fn recycle_bin_round_trip_preserves_metadata_and_rejects_stale_writes() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-roundtrip");
    let mut original = recycle_seed_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "old",
        "private-refresh-secret",
    ));
    original.account_name = Some("My account".into());
    save_account(&original).unwrap();
    super::remove_account(&original.id).unwrap();
    let entries = super::list_recycled_accounts().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].account_name, original.account_name);
    assert!(load_account(&original.id).is_none());
    assert!(list_accounts_checked().unwrap().is_empty());
    let stored = fs::read_to_string(super::codex_recycle_path(&entries[0].id).unwrap()).unwrap();
    assert!(stored.contains("AES-256-GCM"));
    assert!(!stored.contains("private-refresh-secret"));
    let summary = serde_json::to_string(&entries).unwrap();
    assert!(!summary.contains("tokens"));
    assert!(save_account(&original).is_err());
    super::restore_recycled_account(&entries[0].id).unwrap();
    let restored = load_account(&original.id).unwrap();
    assert_eq!(restored.tokens.refresh_token, original.tokens.refresh_token);
    assert_eq!(restored.account_name, original.account_name);
    assert!(restored.token_generation > original.token_generation);
    assert!(save_account(&original).is_err());
    assert!(super::list_recycled_accounts().unwrap().is_empty());
    assert_eq!(list_accounts_checked().unwrap().len(), 1);
}

#[test]
fn recycle_bin_reimport_does_not_overwrite_snapshot_or_live_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-reimport");
    let original = recycle_seed_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "old",
        "old-refresh",
    ));
    super::remove_account(&original.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let newer = upsert_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "new",
        "new-refresh",
    ))
    .unwrap();
    assert!(super::restore_recycled_account(&id).is_err());
    assert_eq!(super::list_recycled_accounts().unwrap().len(), 1);
    super::delete_recycled_account(&id).unwrap();
    assert_eq!(
        load_account(&newer.id).unwrap().tokens.refresh_token,
        newer.tokens.refresh_token
    );
}

#[test]
fn recycle_bin_archive_failure_preserves_live_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-failed-write");
    let account = recycle_seed_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "old",
        "refresh",
    ));
    fs::write(super::codex_recycle_dir().unwrap(), "not a directory").unwrap();
    assert!(super::remove_account(&account.id).is_err());
    assert!(load_account(&account.id).is_some());
    assert_eq!(list_accounts_checked().unwrap().len(), 1);
    assert!(!super::account_is_tombstoned(&account.id));
}

#[test]
fn recycle_bin_permanent_delete_keeps_tombstone_and_removes_backup() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-permanent");
    let account = recycle_seed_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "old",
        "refresh",
    ));
    save_account(&account).unwrap();
    super::remove_account(&account.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    super::delete_recycled_account(&id).unwrap();
    assert!(super::list_recycled_accounts().unwrap().is_empty());
    assert!(super::account_is_tombstoned(&account.id));
    assert!(save_account(&account).is_err());
    assert!(!get_accounts_dir()
        .join(format!("{}.json.bak", account.id))
        .exists());
    super::delete_recycled_account(&id).unwrap();
}

#[test]
fn recycle_bin_batch_delete_preserves_every_account_snapshot() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-batch");
    let a = recycle_seed_account(make_codex_tokens(
        "a@example.com",
        "acc-a",
        "org",
        "a",
        "refresh-a",
    ));
    let b = recycle_seed_account(make_codex_tokens(
        "b@example.com",
        "acc-b",
        "org",
        "b",
        "refresh-b",
    ));
    remove_accounts(&[a.id.clone(), b.id.clone()]).unwrap();
    let entries = super::list_recycled_accounts().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        super::restore_recycled_account(&entry.id).unwrap();
    }
    assert!(load_account(&a.id).is_some());
    assert!(load_account(&b.id).is_some());
}

#[test]
fn recycle_bin_rejects_path_traversal() {
    assert!(super::codex_recycle_path("../outside").is_err());
    assert!(super::validate_recycle_account_id("../outside").is_err());
}

#[test]
fn recycle_bin_failed_restore_keeps_recoverable_snapshot() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-failed-restore");
    let account = recycle_seed_account(make_codex_tokens(
        "recycle@example.com",
        "acc-recycle",
        "org",
        "old",
        "refresh",
    ));
    super::remove_account(&account.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let detail = get_accounts_dir().join(format!("{}.json", account.id));
    fs::create_dir(&detail).unwrap();
    assert!(super::restore_recycled_account(&id).is_err());
    assert!(super::account_is_tombstoned(&account.id));
    assert_eq!(super::list_recycled_accounts().unwrap().len(), 1);
    fs::remove_dir(&detail).unwrap();
    super::restore_recycled_account(&id).unwrap();
    assert_eq!(
        load_account(&account.id).unwrap().tokens.refresh_token,
        account.tokens.refresh_token
    );
}

#[test]
fn recycle_bin_api_key_credentials_are_encrypted_and_restorable() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-api-key");
    let mut account = recycle_seed_account(make_codex_tokens(
        "api@example.com",
        "acc-api",
        "org",
        "old",
        "refresh",
    ));
    account.auth_mode = super::CodexAuthMode::Apikey;
    account.openai_api_key = Some("sk-private-recycle-key".into());
    save_account(&account).unwrap();
    super::remove_account(&account.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let stored = fs::read_to_string(super::codex_recycle_path(&id).unwrap()).unwrap();
    assert!(!stored.contains("sk-private-recycle-key"));
    super::restore_recycled_account(&id).unwrap();
    assert_eq!(
        load_account(&account.id).unwrap().openai_api_key,
        account.openai_api_key
    );
}

#[tokio::test(flavor = "current_thread")]
async fn recycle_bin_export_preserves_oauth_api_key_and_import_compatibility() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-export");
    let oauth = recycle_seed_account(make_codex_tokens(
        "export@example.com",
        "acc-export",
        "org",
        "old",
        "export-refresh",
    ));
    let api = super::import_account_struct(CodexAccount::new_api_key(
        "api-export".into(),
        "api@example.com".into(),
        "sk-export-private-key".into(),
        CodexApiProviderMode::Custom,
        Some("https://example.com/v1".into()),
        Some("export-provider".into()),
        Some("Export Provider".into()),
        vec!["gpt-5".into()],
    ))
    .unwrap();
    let expected = super::export_accounts(&[oauth.id.clone(), api.id.clone()]).unwrap();
    remove_accounts(&[oauth.id.clone(), api.id.clone()]).unwrap();
    let entries = super::list_recycled_accounts().unwrap();
    let ids: Vec<String> = [&oauth.id, &api.id]
        .iter()
        .map(|account_id| {
            entries
                .iter()
                .find(|entry| &entry.account_id == *account_id)
                .unwrap()
                .id
                .clone()
        })
        .collect();
    let exported = super::export_recycled_accounts(&ids).unwrap();
    assert_eq!(exported, expected);
    assert_eq!(super::list_recycled_accounts().unwrap().len(), 2);
    assert!(list_accounts_checked().unwrap().is_empty());
    let imported = import_from_json(&exported).await.unwrap();
    assert_eq!(imported.len(), 2);
    assert_eq!(imported[0].tokens.refresh_token, oauth.tokens.refresh_token);
    assert_eq!(imported[1].openai_api_key, api.openai_api_key);
    assert_eq!(imported[1].api_base_url, api.api_base_url);
}

#[test]
fn recycle_bin_export_uses_snapshot_when_live_account_was_reimported() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = TestEnvGuard::new("codex-recycle-export-reimport");
    let original = recycle_seed_account(make_codex_tokens(
        "export@example.com",
        "acc-export",
        "org",
        "old",
        "old-refresh",
    ));
    super::remove_account(&original.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let newer = recycle_seed_account(make_codex_tokens(
        "export@example.com",
        "acc-export",
        "org",
        "new",
        "new-refresh",
    ));
    let exported: Vec<CodexAccount> =
        serde_json::from_str(&super::export_recycled_accounts(&[id.clone(), id]).unwrap()).unwrap();
    assert_eq!(exported.len(), 1);
    assert_eq!(
        exported[0].tokens.refresh_token,
        original.tokens.refresh_token
    );
    assert_eq!(
        load_account(&newer.id).unwrap().tokens.refresh_token,
        newer.tokens.refresh_token
    );
    assert_eq!(super::list_recycled_accounts().unwrap().len(), 1);
}

#[test]
fn recycle_bin_export_failures_preserve_snapshot_and_destination() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = TestEnvGuard::new("codex-recycle-export-failure");
    let original = recycle_seed_account(make_codex_tokens(
        "export@example.com",
        "acc-export",
        "org",
        "old",
        "old-refresh",
    ));
    super::remove_account(&original.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let snapshot = super::codex_recycle_path(&id).unwrap();
    let original_bytes = fs::read(&snapshot).unwrap();
    let destination = env.home_dir.join("export.json");
    fs::write(&destination, "previous export").unwrap();
    for invalid in ["../outside".to_string(), uuid::Uuid::new_v4().to_string()] {
        assert!(super::export_recycled_accounts_to_file(
            &[id.clone(), invalid],
            destination.to_str().unwrap(),
        )
        .is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "previous export");
        assert_eq!(fs::read(&snapshot).unwrap(), original_bytes);
    }
    assert!(super::export_recycled_accounts(&[]).is_err());
    assert!(
        super::export_recycled_accounts_to_file(&[id.clone()], snapshot.to_str().unwrap(),)
            .is_err()
    );
    assert!(
        super::export_recycled_accounts_to_file(&[id.clone()], env.home_dir.to_str().unwrap(),)
            .is_err()
    );
    let mut entry: serde_json::Value = serde_json::from_slice(&original_bytes).unwrap();
    entry["summary"]["account_id"] = serde_json::json!("different-account");
    fs::write(&snapshot, serde_json::to_vec(&entry).unwrap()).unwrap();
    assert!(super::export_recycled_accounts(&[id.clone()])
        .unwrap_err()
        .contains("identity mismatch"));
    entry["summary"]["account_id"] = serde_json::json!(original.id);
    entry["encrypted_account"] = serde_json::json!("corrupt envelope");
    fs::write(&snapshot, serde_json::to_vec(&entry).unwrap()).unwrap();
    let corrupt_bytes = fs::read(&snapshot).unwrap();
    assert!(super::export_recycled_accounts(&[id.clone()]).is_err());
    assert_eq!(fs::read(&snapshot).unwrap(), corrupt_bytes);
    fs::write(&snapshot, &original_bytes).unwrap();
    let key = crate::modules::account::get_data_dir()
        .unwrap()
        .join("secure-account-storage.key");
    fs::remove_file(&key).unwrap();
    assert!(super::export_recycled_accounts(&[id]).is_err());
    assert!(!key.exists());
    assert_eq!(fs::read(&snapshot).unwrap(), original_bytes);
    assert!(load_account(&original.id).is_none());
}

#[test]
fn recycle_bin_export_writes_private_file_without_removing_snapshot() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = TestEnvGuard::new("codex-recycle-export-file");
    let original = recycle_seed_account(make_codex_tokens(
        "export@example.com",
        "acc-export",
        "org",
        "old",
        "old-refresh",
    ));
    super::remove_account(&original.id).unwrap();
    let id = super::list_recycled_accounts().unwrap()[0].id.clone();
    let path = env.home_dir.join("export.json");
    super::export_recycled_accounts_to_file(&[id], path.to_str().unwrap()).unwrap();
    let exported: Vec<CodexAccount> =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        exported[0].tokens.refresh_token,
        original.tokens.refresh_token
    );
    assert_eq!(super::list_recycled_accounts().unwrap().len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let alias = env.home_dir.join("storage-alias");
        std::os::unix::fs::symlink(crate::modules::account::get_data_dir().unwrap(), &alias)
            .unwrap();
        assert!(super::export_recycled_accounts_to_file(
            &[super::list_recycled_accounts().unwrap()[0].id.clone()],
            alias.join("export.json").to_str().unwrap(),
        )
        .is_err());
    }
}
