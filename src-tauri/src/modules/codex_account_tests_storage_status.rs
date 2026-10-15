#[test]
fn account_storage_status_is_read_only_and_does_not_decode_details() {
    use super::CodexAccountStorageStatus::{Missing, Present};
    let dir = make_temp_dir("codex-storage-status-read-only");
    let absent_root = dir.join("absent");
    assert_eq!(
        super::account_storage_status_in_dir(&absent_root, "account-1").unwrap(),
        Missing
    );
    assert!(
        !absent_root.exists(),
        "inspection must not create directories"
    );
    fs::create_dir_all(dir.join("codex_accounts")).unwrap();
    let detail = dir.join("codex_accounts/account-1.json");
    fs::write(&detail, "not decrypted or valid JSON").unwrap();
    assert_eq!(
        super::account_storage_status_in_dir(&dir, "account-1").unwrap(),
        Present
    );
    assert_eq!(
        fs::read_to_string(detail).unwrap(),
        "not decrypted or valid JSON"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn account_storage_status_respects_deleted_tombstone_before_stale_detail() {
    use super::CodexAccountStorageStatus::{Deleted, Missing, Present};
    let dir = make_temp_dir("codex-storage-status-deleted");
    let tombstone_dir = dir.join(super::CODEX_ACCOUNT_TOMBSTONES_DIR);
    fs::create_dir_all(&tombstone_dir).unwrap();
    let tombstone = tombstone_dir.join("account-1.json");
    fs::write(&tombstone, r#"{"deleted":false,"generation":1}"#).unwrap();
    assert_eq!(
        super::account_storage_status_in_dir(&dir, "account-1").unwrap(),
        Missing
    );
    fs::create_dir_all(dir.join("codex_accounts")).unwrap();
    fs::write(dir.join("codex_accounts/account-1.json"), "stale auth").unwrap();
    assert_eq!(
        super::account_storage_status_in_dir(&dir, "account-1").unwrap(),
        Present
    );
    fs::write(&tombstone, r#"{"deleted":true,"generation":2}"#).unwrap();
    assert_eq!(
        super::account_storage_status_in_dir(&dir, "account-1").unwrap(),
        Deleted
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn account_storage_status_rejects_unknown_revocation_and_non_file_details() {
    let dir = make_temp_dir("codex-storage-status-errors");
    let tombstone_dir = dir.join(super::CODEX_ACCOUNT_TOMBSTONES_DIR);
    fs::create_dir_all(&tombstone_dir).unwrap();
    let tombstone = tombstone_dir.join("account-1.json");
    for content in [
        "{broken",
        r#"{"generation":1}"#,
        r#"{"deleted":"false","generation":1}"#,
    ] {
        fs::write(&tombstone, content).unwrap();
        assert!(super::account_storage_status_in_dir(&dir, "account-1").is_err());
    }
    fs::remove_file(&tombstone).unwrap();
    fs::create_dir(&tombstone).unwrap();
    assert!(super::account_storage_status_in_dir(&dir, "account-1").is_err());
    fs::remove_dir(&tombstone).unwrap();
    fs::create_dir_all(dir.join("codex_accounts/account-1.json")).unwrap();
    assert!(super::account_storage_status_in_dir(&dir, "account-1").is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn account_storage_status_rejects_invalid_ids_before_reading_storage() {
    let dir = make_temp_dir("codex-storage-status-invalid-id");
    for id in [
        "",
        "../outside",
        "/absolute",
        "a/b",
        "a\\b",
        "a.json",
        " a ",
        "a\0b",
    ] {
        assert!(
            super::account_storage_status_in_dir(&dir, id).is_err(),
            "accepted {id:?}"
        );
        assert!(
            super::account_storage_status(id).is_err(),
            "accepted {id:?}"
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn account_storage_status_uses_isolated_account_store_override() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = TestEnvGuard::new("codex-storage-status-override");
    let data_dir = env.home_dir.join(".antigravity_cockpit");
    fs::create_dir_all(data_dir.join("codex_accounts")).unwrap();
    fs::write(data_dir.join("codex_accounts/account-1.json"), "opaque").unwrap();
    assert_eq!(
        super::account_storage_status("account-1").unwrap(),
        super::CodexAccountStorageStatus::Present
    );
}
