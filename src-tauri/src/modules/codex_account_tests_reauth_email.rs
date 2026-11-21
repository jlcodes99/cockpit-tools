#[test]
fn reauth_updates_changed_email_for_the_same_user_and_workspace() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-reauth-email-change");
    let mut existing = upsert_account(make_codex_tokens(
        "old@example.invalid", "acc-target", "org-target", "same-user", "rt-old",
    ))
    .unwrap();
    existing.account_note = Some("keep this note".into());
    save_account(&existing).unwrap();

    for (account_id, org_id, user) in [
        ("acc-target", "org-target", "different-user"),
        ("other-workspace", "org-target", "same-user"),
        ("acc-target", "other-org", "same-user"),
    ] {
        let error = upsert_account_for_reauth(
            make_codex_tokens("new@example.invalid", account_id, org_id, user, "rt-rejected"),
            &existing.id,
        )
        .unwrap_err();
        assert!(error.starts_with("CODEX_OAUTH_RESTART_REQUIRED:"));
    }
    let unchanged = load_account(&existing.id).unwrap();
    assert_eq!(unchanged.email, "old@example.invalid");
    assert_eq!(unchanged.tokens.refresh_token.as_deref(), Some("rt-old"));

    let result = upsert_account_for_reauth(
        make_codex_tokens("new@example.invalid", "acc-target", "org-target", "same-user", "rt-new"),
        &existing.id,
    )
    .unwrap();
    assert_eq!(result.id, existing.id);
    assert_eq!(result.email, "new@example.invalid");
    let saved = load_account(&existing.id).unwrap();
    assert_eq!(saved.email, "new@example.invalid");
    assert_eq!(saved.account_note.as_deref(), Some("keep this note"));
    assert_eq!(saved.tokens.refresh_token.as_deref(), Some("rt-new"));
    let index = load_account_index();
    assert_eq!(index.accounts.len(), 1);
    assert_eq!(index.accounts[0].email, saved.email);
}

#[test]
fn reauth_email_change_keeps_a_conflicting_existing_record() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-reauth-email-collision");
    let original = upsert_account(make_codex_tokens(
        "old@example.invalid", "acc-shared", "org-shared", "original", "rt-original",
    ))
    .unwrap();
    let existing = upsert_account(make_codex_tokens(
        "new@example.invalid", "acc-shared", "org-shared", "other-user", "rt-other",
    ))
    .unwrap();
    assert_ne!(original.id, existing.id);
    assert!(upsert_account_for_reauth(
        make_codex_tokens("new@example.invalid", "acc-shared", "org-shared", "original", "rt-rejected"),
        &original.id,
    )
    .is_err());
    assert_eq!(load_account(&original.id).unwrap().email, "old@example.invalid");
    let existing = load_account(&existing.id).unwrap();
    assert_eq!(existing.email, "new@example.invalid");
    assert_eq!(existing.tokens.refresh_token.as_deref(), Some("rt-other"));
}
