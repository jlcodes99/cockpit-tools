use super::*;

struct DataDirGuard {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}
impl DataDirGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("antigravity-stable-id-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &path);
        Self { path, previous }
    }
}
impl Drop for DataDirGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn token(value: &str) -> TokenData {
    TokenData::new(
        value.into(),
        value.into(),
        3600,
        Some("legacy@example.com".into()),
        None,
        None,
    )
}
fn seed(id: &str) -> Account {
    let mut account = Account::new(id.into(), "legacy@example.com".into(), token("old"));
    account.notes = Some("keep notes".into());
    save_account(&account).unwrap();
    let mut index = load_account_index().unwrap();
    if !index.accounts.iter().any(|entry| entry.id == id) {
        index.accounts.push(AccountSummary {
            id: id.into(),
            email: account.email.clone(),
            name: None,
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    index.current_account_id = Some(id.into());
    save_account_index(&index).unwrap();
    account
}

#[test]
fn reimport_and_pending_edits_preserve_legacy_files_metadata_and_references() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let guard = DataDirGuard::new();
    let old = seed("legacy-uuid");
    // A second historical same-email account may still be referenced elsewhere.
    let sibling = seed("other-legacy-uuid");
    let sibling_path = get_accounts_dir()
        .unwrap()
        .join(format!("{}.json", sibling.id));
    let sibling_bytes = fs::read(&sibling_path).unwrap();
    let mut index = load_account_index().unwrap();
    index.current_account_id = Some(old.id.clone());
    save_account_index(&index).unwrap();
    let references = ["account_groups.json", "instances.json", "wakeup_tasks.json"];
    for file in references {
        fs::write(guard.path.join(file), r#"{"accountId":"legacy-uuid"}"#).unwrap();
    }
    let updated = upsert_account(" Legacy@Example.com ".into(), None, token("new")).unwrap();
    assert_eq!(updated.id, old.id);
    assert_eq!(updated.created_at, old.created_at);
    assert_eq!(updated.notes, old.notes);
    assert_eq!(load_account(&old.id).unwrap().token.refresh_token, "new");
    assert_eq!(fs::read(&sibling_path).unwrap(), sibling_bytes);
    assert_eq!(load_account_index().unwrap().accounts.len(), 2);
    assert_eq!(
        load_account_index().unwrap().current_account_id.as_deref(),
        Some(old.id.as_str())
    );
    let pending =
        create_pending_oauth_account("legacy@example.com".into(), AccountNoteUpdate::default())
            .unwrap();
    assert_eq!(pending.id, old.id);
    assert!(!pending.pending_oauth);
    for file in references {
        assert_eq!(
            fs::read_to_string(guard.path.join(file)).unwrap(),
            r#"{"accountId":"legacy-uuid"}"#
        );
    }
    assert!(!get_accounts_dir()
        .unwrap()
        .join(format!(
            "{}.json",
            build_account_storage_id("legacy@example.com")
        ))
        .exists());
}

#[test]
fn unreadable_existing_account_is_not_replaced_by_import_or_pending_placeholder() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = DataDirGuard::new();
    let old = seed("legacy-uuid");
    let path = get_accounts_dir().unwrap().join(format!("{}.json", old.id));
    fs::write(&path, "corrupt account").unwrap();
    assert!(upsert_account(old.email.clone(), None, token("new")).is_err());
    assert!(create_pending_oauth_account(old.email, AccountNoteUpdate::default()).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "corrupt account");
    assert_eq!(load_account_index().unwrap().accounts[0].id, old.id);
}

#[test]
fn new_accounts_use_canonical_id_and_reimport_does_not_duplicate() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = DataDirGuard::new();
    let first =
        create_pending_oauth_account(" New@Example.com ".into(), AccountNoteUpdate::default())
            .unwrap();
    assert!(first.pending_oauth);
    assert_eq!(first.id, build_account_storage_id("new@example.com"));
    let updated = upsert_account("new@example.com".into(), None, token("new")).unwrap();
    assert_eq!(updated.id, first.id);
    assert!(!updated.pending_oauth);
    assert_eq!(load_account_index().unwrap().accounts.len(), 1);
}
