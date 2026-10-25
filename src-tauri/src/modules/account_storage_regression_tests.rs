use super::*;

struct StorageFixture {
    dir: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl StorageFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", dir.path());
        Self { dir, previous }
    }

    fn pending(&self) -> Account {
        create_pending_oauth_account(
            "fixture@example.com".into(),
            AccountNoteUpdate {
                note: Some("keep this note".into()),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn path(&self, account: &Account) -> PathBuf {
        self.dir
            .path()
            .join("accounts")
            .join(format!("{}.json", account.id))
    }

    fn authorize(&self) -> Result<Account, String> {
        upsert_account(
            "fixture@example.com".into(),
            Some("Fixture User".into()),
            TokenData::new(
                "new-access".into(),
                "new-refresh".into(),
                3600,
                None,
                None,
                None,
            ),
        )
    }
}

impl Drop for StorageFixture {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", previous),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
    }
}

#[test]
fn oauth_accepts_bom_encrypted_pending_account_without_restoring_older_notes() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let mut pending = fixture.pending();
    pending.notes = Some("newest note".into());
    save_account(&pending).unwrap();
    let path = fixture.path(&pending);
    let encrypted = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("\u{feff}{encrypted}")).unwrap();
    let authorized = fixture
        .authorize()
        .expect("BOM must not block completed OAuth");
    assert_eq!(authorized.id, pending.id);
    assert_eq!(authorized.notes.as_deref(), Some("newest note"));
    assert_eq!(authorized.token.refresh_token, "new-refresh");
    assert!(!authorized.pending_oauth);
    assert_eq!(load_account_index().unwrap().accounts.len(), 1);
}

#[test]
fn oauth_accepts_bom_legacy_plaintext_pending_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    fs::write(
        &path,
        format!("\u{feff}{}", serde_json::to_string(&pending).unwrap()),
    )
    .unwrap();
    let authorized = fixture
        .authorize()
        .expect("legacy BOM must not block OAuth");
    assert_eq!(authorized.id, pending.id);
    assert_eq!(authorized.notes, pending.notes);
    assert!(!authorized.pending_oauth);
    assert!(load_account(&pending.id).is_ok());
}

#[test]
fn oauth_recovers_encrypted_backup_in_the_same_attempt() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    let encrypted = fs::read_to_string(&path).unwrap();
    fs::write(path.with_extension("json.bak"), &encrypted).unwrap();
    fs::write(&path, "").unwrap();
    assert_eq!(
        load_account(&pending.id)
            .expect("recover on first read")
            .notes,
        pending.notes
    );
    let authorized = fixture
        .authorize()
        .expect("valid encrypted backup must recover before OAuth upsert");
    assert_eq!(authorized.id, pending.id);
    assert_eq!(authorized.notes, pending.notes);
    assert_eq!(
        load_account(&pending.id).unwrap().token.refresh_token,
        "new-refresh"
    );
}

#[test]
fn encrypted_backup_survives_repeated_recovery_of_json_with_corrupt_ciphertext() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    let backup_path = path.with_extension("json.bak");
    let valid_snapshot = fs::read_to_string(&path).unwrap();
    fs::write(&backup_path, &valid_snapshot).unwrap();
    let mut corrupted: serde_json::Value = serde_json::from_str(&valid_snapshot).unwrap();
    corrupted["ciphertext"] = serde_json::json!(general_purpose::STANDARD.encode([0u8; 32]));
    let corrupted = serde_json::to_string(&corrupted).unwrap();

    for _ in 0..2 {
        fs::write(&path, &corrupted).unwrap();
        let restored = load_account(&pending.id).expect("validated backup must remain reusable");
        assert_eq!(restored.id, pending.id);
        assert_eq!(restored.notes, pending.notes);
        assert_eq!(fs::read_to_string(&path).unwrap(), valid_snapshot);
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), valid_snapshot);
    }
}

#[test]
fn missing_encryption_key_is_reported_without_creating_a_replacement() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    let bytes = fs::read(&path).unwrap();
    let key = fixture.dir.path().join("secure-account-storage.key");
    fs::remove_file(&key).unwrap();
    let error = fixture.authorize().unwrap_err();
    assert!(error.contains("读取账号详情加密密钥失败"), "{error}");
    assert!(
        !key.exists(),
        "a read must not create a different encryption key"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn corrupt_account_and_invalid_backup_are_not_overwritten() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    fs::write(&path, "not-json").unwrap();
    let backup = path.with_extension("json.bak");
    fs::write(&backup, "also-not-json").unwrap();
    let error = fixture.authorize().unwrap_err();
    assert!(error.contains("解析账号数据失败"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "not-json");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "also-not-json");
}

#[test]
fn another_accounts_encrypted_backup_is_not_restored() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    let path = fixture.path(&pending);
    let mut other = pending.clone();
    other.id = "other-account".into();
    let backup = serialize_account_for_storage(&other).unwrap();
    fs::write(path.with_extension("json.bak"), &backup).unwrap();
    fs::write(&path, "broken").unwrap();
    assert!(fixture.authorize().is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
    assert_eq!(
        fs::read_to_string(path.with_extension("json.bak")).unwrap(),
        backup
    );
}

#[test]
fn recovery_of_an_old_snapshot_does_not_replace_a_concurrent_save() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let mut pending = fixture.pending();
    let path = fixture.path(&pending);
    let old_backup = fs::read_to_string(&path).unwrap();
    pending.notes = Some("concurrently saved note".into());
    save_account(&pending).unwrap();
    fs::write(path.with_extension("json.bak"), &old_backup).unwrap();
    let current = fs::read(&path).unwrap();
    let loaded = deserialize_account_from_storage(&path, "previously read broken bytes").unwrap();
    assert_eq!(loaded.notes, pending.notes);
    assert_eq!(fs::read(&path).unwrap(), current);
    assert_eq!(
        fs::read_to_string(path.with_extension("json.bak")).unwrap(),
        old_backup
    );
}

#[test]
fn oauth_still_accepts_legacy_token_only_encryption_with_a_bom() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fixture = StorageFixture::new();
    let pending = fixture.pending();
    fs::write(
        fixture.dir.path().join(LEGACY_TOKEN_KEY_FILE),
        general_purpose::STANDARD.encode([7u8; 32]),
    )
    .unwrap();
    let nonce = [3u8; 12];
    let encrypted = legacy_token_cipher()
        .unwrap()
        .encrypt(
            Nonce::from_slice(&nonce),
            serde_json::to_vec(&pending.token).unwrap().as_slice(),
        )
        .unwrap();
    let envelope = EncryptedTokenEnvelope {
        version: LEGACY_TOKEN_VERSION,
        algorithm: "AES-256-GCM".into(),
        key_id: "legacy-fixture".into(),
        nonce: general_purpose::STANDARD.encode(nonce),
        ciphertext: general_purpose::STANDARD.encode(encrypted),
        encrypted_at: chrono::Utc::now().timestamp(),
    };
    let mut value = serde_json::to_value(&pending).unwrap();
    value.as_object_mut().unwrap().remove("token");
    value["token_encrypted"] = serde_json::to_value(envelope).unwrap();
    fs::write(fixture.path(&pending), format!("\u{feff}{value}")).unwrap();
    let authorized = fixture.authorize().unwrap();
    assert_eq!(authorized.id, pending.id);
    assert_eq!(authorized.notes, pending.notes);
    assert_eq!(authorized.token.refresh_token, "new-refresh");
}
