// Serialization must retain workspace identity even without an id_token.
#[test]
fn auth_projection_identity_falls_back_for_missing_or_blank_metadata() {
    for stored in [None, Some(String::new()), Some("  ".to_string())] {
        let mut tokens = make_codex_tokens(
            "projection@example.com",
            "acc-access",
            "org-access",
            "projection",
            "synthetic-refresh",
        );
        tokens.id_token.clear();
        let mut account = build_test_oauth_account(tokens);
        account.account_id = stored;
        let value = build_auth_file_value(&account).expect("serialize OAuth auth");
        assert_eq!(value["tokens"]["account_id"].as_str(), Some("acc-access"));
        assert_eq!(value["tokens"]["id_token"].as_str(), Some(""));
    }
}

#[test]
fn auth_projection_identity_preserves_explicit_stored_account_id() {
    let tokens = make_codex_tokens(
        "projection@example.com",
        "acc-access",
        "org-access",
        "projection",
        "synthetic-refresh",
    );
    let mut account = build_test_oauth_account(tokens);
    account.account_id = Some("acc-stored".into());
    let value = build_auth_file_value(&account).expect("serialize OAuth auth");
    assert_eq!(value["tokens"]["account_id"].as_str(), Some("acc-stored"));
}

#[test]
fn auth_projection_identity_does_not_invent_identity_for_opaque_tokens() {
    let mut tokens = make_codex_tokens(
        "projection@example.com",
        "acc-access",
        "org-access",
        "projection",
        "synthetic-refresh",
    );
    tokens.id_token.clear();
    tokens.access_token = "synthetic-opaque-access".into();
    let mut account = build_test_oauth_account(tokens);
    account.account_id = None;
    let value = build_auth_file_value(&account).expect("serialize opaque OAuth auth");
    assert!(value["tokens"]["account_id"].is_null());
}

#[test]
fn auth_projection_identity_keeps_personal_access_token_shape() {
    let mut tokens = make_codex_tokens(
        "projection@example.com",
        "acc-access",
        "org-access",
        "projection",
        "synthetic-refresh",
    );
    tokens.id_token.clear();
    tokens.refresh_token = None;
    let mut account = build_test_oauth_account(tokens);
    account.account_id = None;
    let value = build_auth_file_value(&account).expect("serialize personal access token");
    assert!(value.get("tokens").is_none());
    assert_eq!(
        value["personal_access_token"].as_str(),
        Some(account.tokens.access_token.as_str())
    );
}
