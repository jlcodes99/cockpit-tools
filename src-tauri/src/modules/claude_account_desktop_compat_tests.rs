use super::*;

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("claude-compat-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn desktop_plan_scopes_memberships_and_preserves_display_names() {
    let mut profile = json!({"lastActiveOrg": "free-org", "endpoints": {"account": {"memberships": [
        {"organization": {"uuid": "enterprise-org", "organization_type": "claude_enterprise", "capabilities": ["raven"]}},
        {"organization": {"uuid": "free-org", "paidAccountTier": "free", "capabilities": ["chat"]}}
    ]}}});
    assert_eq!(
        desktop_plan_from_profile(&profile).0.as_deref(),
        Some("Free")
    );
    let slim = slim_web_profile_for_storage(&profile);
    assert_eq!(desktop_plan_from_profile(&slim).0.as_deref(), Some("Free"));
    profile["lastActiveOrg"] = json!("enterprise-org");
    assert_eq!(
        desktop_plan_from_profile(&profile).0.as_deref(),
        Some("Enterprise")
    );
    profile["endpoints"]["account"]["memberships"][0]["organization"]["plan_display_name"] =
        json!("Enterprise Custom");
    assert_eq!(
        desktop_plan_from_profile(&profile).0.as_deref(),
        Some("Enterprise Custom")
    );
    profile.as_object_mut().unwrap().remove("lastActiveOrg");
    assert_eq!(desktop_plan_from_profile(&profile).0, None);
    assert_eq!(desktop_plan_from_profile(&json!({"endpoints": {}})).0, None);
}

#[test]
fn desktop_plan_keeps_official_bootstrap_tier_after_slimming() {
    let profile = json!({"endpoints": {"bootstrapAppStart": {"paidAccountTier": "free"}}});
    assert_eq!(
        desktop_plan_from_profile(&slim_web_profile_for_storage(&profile))
            .0
            .as_deref(),
        Some("Free")
    );
}

#[test]
fn desktop_cookie_versions_require_both_session_and_organization() {
    for name in ["sessionKey", "sessionKeyV2", "sessionKeyV3"] {
        let export: ClaudeDesktopAuthCookieExport = serde_json::from_value(json!({"cookies": [
            {"name": name, "value": "test-session", "domain": ".claude.ai"},
            {"name": "lastActiveOrg", "value": "org", "domain": ".claude.ai"}
        ]}))
        .unwrap();
        assert!(ensure_desktop_auth_export_logged_in(&export).is_ok());
        assert!(desktop_profile_metadata_from_export(&export, "test").has_session_key);
        let dir = TestDir::new();
        let db = dir.0.join("Cookies");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE cookies(name TEXT,host_key TEXT,value TEXT,encrypted_value BLOB,expires_utc INTEGER);").unwrap();
        conn.execute(
            "INSERT INTO cookies VALUES(?1,'.claude.ai','test',X'',0)",
            [name],
        )
        .unwrap();
        assert!(!cookies_db_has_required_desktop_session(&db).unwrap());
        conn.execute_batch("INSERT INTO cookies VALUES('lastActiveOrg','.claude.ai','org',X'',0);")
            .unwrap();
        assert!(cookies_db_has_required_desktop_session(&db).unwrap());
        assert!(
            desktop_profile_metadata_from_cookies_db(&dir.0, "test")
                .unwrap()
                .has_session_key
        );
    }
}

#[test]
fn desktop_cookie_two_session_versions_do_not_replace_organization() {
    let dir = TestDir::new();
    let db = dir.0.join("Cookies");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE cookies(name TEXT,host_key TEXT,value TEXT,encrypted_value BLOB); INSERT INTO cookies VALUES('sessionKey','.claude.ai','test',X''),('sessionKeyV3','.claude.ai','test',X'');").unwrap();
    assert!(!cookies_db_has_required_desktop_session(&db).unwrap());
}

#[test]
fn desktop_restore_replaces_auth_keys_and_removes_absent_cookie_paths() {
    let dir = TestDir::new();
    let source = dir.0.join("source");
    let target = dir.0.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    write_config_file(&source.join("config.json"), &json!({"oauth:tokenCacheV2": "new", "coworkTrustedDeviceToken:new-id": "trusted", "coworkTrustedDeviceToken:other-id": "discard"})).unwrap();
    write_config_file(&target.join("config.json"), &json!({"oauth:tokenCache": "old", "oauth:tokenCacheV2": "old-v2", "lastKnownAccountUuid": "old-id", "coworkTrustedDeviceToken:old-id": "old", "theme": "dark"})).unwrap();
    fs::write(target.join("Cookies"), b"old-session").unwrap();
    fs::write(target.join("Cookies-wal"), b"stale-wal").unwrap();
    fs::write(target.join("unrelated"), b"keep").unwrap();
    restore_desktop_profile_transaction(
        &source,
        &target,
        Some("new-id"),
        apply_desktop_profile_snapshot,
    )
    .unwrap();
    let config = read_config_file(&target.join("config.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        config,
        json!({"oauth:tokenCacheV2": "new", "lastKnownAccountUuid": "new-id", "coworkTrustedDeviceToken:new-id": "trusted", "theme": "dark"})
    );
    assert!(!target.join("Cookies").exists());
    assert!(!target.join("Cookies-wal").exists());
    assert_eq!(fs::read(target.join("unrelated")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
}

#[test]
fn desktop_restore_rolls_back_partial_writes_and_new_files() {
    let dir = TestDir::new();
    let source = dir.0.join("source");
    let target = dir.0.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(source.join("Cookies"), b"new").unwrap();
    fs::write(target.join("Cookies"), b"old").unwrap();
    let original = json!({"oauth:tokenCacheV2": "old", "theme": "dark"});
    write_config_file(&target.join("config.json"), &original).unwrap();
    let result =
        restore_desktop_profile_transaction(&source, &target, Some("new-id"), |staged, target| {
            apply_desktop_profile_snapshot(staged, target)?;
            fs::write(target.join("Preferences"), b"partial").unwrap();
            Err("simulated disk failure".into())
        });
    assert!(result.is_err());
    assert_eq!(fs::read(target.join("Cookies")).unwrap(), b"old");
    assert_eq!(
        read_config_file(&target.join("config.json"))
            .unwrap()
            .unwrap(),
        original
    );
    assert!(!target.join("Preferences").exists());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
}

#[test]
fn desktop_restore_recovers_interrupted_transaction_idempotently() {
    let dir = TestDir::new();
    let target = dir.0.join("target");
    let transaction = target.join(".cockpit-auth-test");
    fs::create_dir_all(transaction.join("rollback")).unwrap();
    fs::write(transaction.join("rollback/Cookies"), b"original").unwrap();
    fs::write(target.join("Cookies"), b"partial").unwrap();
    fs::write(target.join("Preferences"), b"new-partial").unwrap();
    write_config_file(
        &transaction.join("journal.json"),
        &json!({"originals": ["Cookies"], "applying": true, "committed": false}),
    )
    .unwrap();
    recover_desktop_profile_transaction(&target, &transaction).unwrap();
    assert_eq!(fs::read(target.join("Cookies")).unwrap(), b"original");
    assert!(!target.join("Preferences").exists());
    assert!(!transaction.exists());
}

#[test]
fn desktop_restore_rejects_concurrent_switch_without_waiting() {
    let _lock = CLAUDE_PROFILE_RESTORE_LOCK.lock().unwrap();
    assert!(
        restore_desktop_profile_snapshot(Path::new("unused"), Path::new("unused"), None)
            .unwrap_err()
            .contains("CLAUDE_PROFILE_RESTORE_BUSY")
    );
}
