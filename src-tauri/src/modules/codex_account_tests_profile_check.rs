// Codex 账号测试：accounts/check 工作区资料解析、目标工作区匹配与持久化回归。
// 使用脱敏占位样例验证嵌套解析、多工作区匹配、缺失不误填及扁平兼容行为。

#[test]
fn profile_check_extracts_nested_workspace_name_and_structure() {
    let target_workspace_id = "11111111-1111-4111-8111-111111111111";
    let account = CodexAccount::new(
        "codex_profile_nested".to_string(),
        "user@example.invalid".to_string(),
        make_codex_tokens(
            "user@example.invalid",
            target_workspace_id,
            "org-team-alpha",
            "nested",
            "rt-nested",
        ),
    );
    let payload = serde_json::json!({
        "accounts": {
            target_workspace_id: {
                "account": {
                    "account_id": target_workspace_id,
                    "name": "Team Alpha",
                    "structure": "workspace"
                },
                "can_access_with_session": true
            }
        },
        "account_ordering": [target_workspace_id]
    });

    let (name, structure, account_id) =
        super::parse_account_profile_from_check_response(&payload, &account).unwrap();
    assert_eq!(name.as_deref(), Some("Team Alpha"));
    assert_eq!(structure.as_deref(), Some("workspace"));
    assert_eq!(account_id.as_deref(), Some(target_workspace_id));
}

#[test]
fn profile_check_matches_non_first_target_workspace_across_multiple_workspaces() {
    let personal_id = "00000000-0000-4000-8000-000000000000";
    let alpha_id = "11111111-1111-4111-8111-111111111111";
    let beta_id = "22222222-2222-4222-8222-222222222222";
    let mut account = CodexAccount::new(
        "codex_profile_multi".to_string(),
        "user@example.invalid".to_string(),
        make_codex_tokens(
            "user@example.invalid",
            beta_id,
            "org-personal",
            "multi",
            "rt-multi",
        ),
    );
    account.account_id = Some(beta_id.to_string());
    account.organization_id = Some("org-personal".to_string());

    let payload = serde_json::json!({
        "accounts": {
            personal_id: {
                "account": {
                    "account_id": personal_id,
                    "organization_id": "org-personal",
                    "name": "Personal",
                    "structure": "personal",
                    "is_default": true
                },
                "can_access_with_session": true
            },
            alpha_id: {
                "account": {
                    "account_id": alpha_id,
                    "name": "Team Alpha",
                    "structure": "workspace"
                },
                "can_access_with_session": true
            },
            beta_id: {
                "account": {
                    "account_id": beta_id,
                    "name": "Team Beta",
                    "structure": "workspace"
                },
                "can_access_with_session": true
            }
        },
        "account_ordering": [personal_id, alpha_id, beta_id]
    });

    let (name, structure, account_id) =
        super::parse_account_profile_from_check_response(&payload, &account).unwrap();
    assert_eq!(name.as_deref(), Some("Team Beta"));
    assert_eq!(structure.as_deref(), Some("workspace"));
    assert_eq!(account_id.as_deref(), Some(beta_id));
}

#[tokio::test]
async fn profile_check_does_not_fallback_to_other_workspace_when_known_target_missing() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-profile-target-missing");

    let missing_id = "33333333-3333-4333-8333-333333333333";
    let personal_id = "00000000-0000-4000-8000-000000000000";
    let alpha_id = "11111111-1111-4111-8111-111111111111";

    let mut account = super::upsert_account(make_codex_tokens(
        "user@example.invalid",
        missing_id,
        "org-personal",
        "missing",
        "rt-missing",
    ))
    .expect("upsert target missing test account");
    account.plan_type = Some("team".to_string());
    account.account_name = Some("Saved Team".to_string());
    account.account_structure = Some("workspace".to_string());
    save_account(&account).expect("seed account with existing name");

    let payload = serde_json::json!({
        "accounts": {
            personal_id: {
                "account": {
                    "account_id": personal_id,
                    "organization_id": "org-personal",
                    "name": "Personal",
                    "structure": "personal",
                    "is_default": true
                },
                "can_access_with_session": true
            },
            alpha_id: {
                "account": {
                    "account_id": alpha_id,
                    "name": "Team Alpha",
                    "structure": "workspace"
                },
                "can_access_with_session": true
            }
        },
        "account_ordering": [alpha_id, personal_id]
    });

    let profile = super::parse_account_profile_from_check_response(&payload, &account);
    let error = super::finish_account_profile_refresh(account.clone(), profile)
        .await
        .expect_err("another workspace cannot supply this profile");
    assert!(error.contains("未找到目标工作区"));

    let current = load_account(&account.id).expect("reload seeded account");
    assert_eq!(current.account_name.as_deref(), Some("Saved Team"));
    assert_eq!(current.account_structure.as_deref(), Some("workspace"));
    assert_eq!(current.account_id.as_deref(), Some(missing_id));
}

#[test]
fn profile_check_matches_map_key_opaque_token_and_org_without_rewriting_identity() {
    let key_only_id = "44444444-4444-4444-8444-444444444444";
    let mut opaque_account = CodexAccount::new(
        "codex_profile_opaque".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: "at-sanitized-test-token".to_string(),
            refresh_token: None,
        },
    );
    opaque_account.account_id = Some(key_only_id.to_string());

    let key_only_payload = serde_json::json!({
        "accounts": {
            "00000000-0000-4000-8000-000000000000": {
                "account": {
                    "account_id": "00000000-0000-4000-8000-000000000000",
                    "name": "Personal",
                    "structure": "personal"
                }
            },
            key_only_id: {
                "account": {
                    "name": "Team KeyOnly",
                    "structure": "workspace"
                }
            }
        },
        "account_ordering": ["00000000-0000-4000-8000-000000000000", key_only_id]
    });

    let (name, structure, account_id) =
        super::parse_account_profile_from_check_response(&key_only_payload, &opaque_account)
            .unwrap();
    assert_eq!(name.as_deref(), Some("Team KeyOnly"));
    assert_eq!(structure.as_deref(), Some("workspace"));
    assert_eq!(account_id, None);

    let mut org_only_account = CodexAccount::new(
        "codex_profile_org".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: "at-sanitized-org-only".to_string(),
            refresh_token: None,
        },
    );
    org_only_account.organization_id = Some("org-team-beta".to_string());

    let org_payload = serde_json::json!({
        "accounts": {
            "00000000-0000-4000-8000-000000000000": {
                "account": {
                    "account_id": "00000000-0000-4000-8000-000000000000",
                    "name": "Personal",
                    "structure": "personal"
                }
            },
            "org-team-beta": {
                "account": {
                    "account_id": "22222222-2222-4222-8222-222222222222",
                    "name": "Team Beta",
                    "structure": "workspace"
                }
            }
        },
        "account_ordering": ["00000000-0000-4000-8000-000000000000"]
    });

    let (org_name, org_structure, org_account_id) =
        super::parse_account_profile_from_check_response(&org_payload, &org_only_account).unwrap();
    assert_eq!(org_name.as_deref(), Some("Team Beta"));
    assert_eq!(org_structure.as_deref(), Some("workspace"));
    assert_eq!(
        org_account_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );

    org_only_account.organization_id = Some("org-missing".to_string());
    assert!(
        super::parse_account_profile_from_check_response(&org_payload, &org_only_account).is_err()
    );
}

#[tokio::test]
async fn profile_check_reports_missing_team_name_and_persists_extracted_name() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-profile-persist-check");

    let target_id = "11111111-1111-4111-8111-111111111111";
    let mut account = super::upsert_account(make_codex_tokens(
        "user@example.invalid",
        target_id,
        "org-team-alpha",
        "persist",
        "rt-persist",
    ))
    .expect("upsert persist test account");
    account.plan_type = Some("team".to_string());
    account.account_name = Some("Existing Name".to_string());
    save_account(&account).expect("seed existing named account");

    let nameless_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "account": {
                    "account_id": target_id,
                    "name": serde_json::Value::Null,
                    "structure": "workspace"
                }
            }
        }
    });
    let profile = super::parse_account_profile_from_check_response(&nameless_payload, &account);
    let error = super::finish_account_profile_refresh(account.clone(), profile)
        .await
        .expect_err("nameless Team profile must reach the refresh error path");
    assert!(error.contains("缺少 Team/Business 工作区名称"));
    let reloaded = load_account(&account.id).expect("load after failed refresh");
    assert_eq!(reloaded.account_name.as_deref(), Some("Existing Name"));
    assert_eq!(reloaded.tokens.access_token, account.tokens.access_token);
    assert_eq!(reloaded.token_generation, account.token_generation);

    let mut personal_payload = nameless_payload;
    personal_payload["accounts"][target_id]["account"]["structure"] = serde_json::json!("personal");
    let personal_profile =
        super::parse_account_profile_from_check_response(&personal_payload, &account).unwrap();
    assert_eq!(
        personal_profile,
        (None, Some("personal".into()), Some(target_id.into()))
    );

    let mut blank_account = reloaded;
    blank_account.account_name = None;
    blank_account.account_structure = None;
    save_account(&blank_account).expect("clear account name for hydration test");

    let named_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "account": {
                    "account_id": target_id,
                    "name": "Team Alpha",
                    "structure": "workspace"
                }
            }
        }
    });
    let profile = super::parse_account_profile_from_check_response(&named_payload, &blank_account);
    let account_id = blank_account.id.clone();
    super::finish_account_profile_refresh(blank_account, profile)
        .await
        .expect("hydrate profile through the existing save path");
    let persisted = load_account(&account_id).expect("load hydrated account");
    assert_eq!(persisted.account_name.as_deref(), Some("Team Alpha"));
    assert_eq!(persisted.account_structure.as_deref(), Some("workspace"));
}

#[test]
fn profile_check_keeps_flat_and_unknown_target_compatibility() {
    let target_id = "55555555-5555-4555-8555-555555555555";
    let mut account = CodexAccount::new(
        "codex_profile_flat".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: "at-flat".to_string(),
            refresh_token: None,
        },
    );
    account.account_id = Some(target_id.to_string());

    let flat_object_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "id": target_id,
                "organization_name": "Flat Workspace",
                "account_structure": "team"
            }
        }
    });
    assert_eq!(
        super::parse_account_profile_from_check_response(&flat_object_payload, &account).unwrap(),
        (
            Some("Flat Workspace".to_string()),
            Some("team".to_string()),
            Some(target_id.to_string()),
        )
    );

    let flat_array_payload = serde_json::json!([
        {
            "account_id": "00000000-0000-4000-8000-000000000000",
            "name": "Personal",
            "structure": "personal"
        },
        {
            "account_id": target_id,
            "workspace_name": "Array Workspace",
            "type": "workspace"
        }
    ]);
    assert_eq!(
        super::parse_account_profile_from_check_response(&flat_array_payload, &account).unwrap(),
        (
            Some("Array Workspace".to_string()),
            Some("workspace".to_string()),
            Some(target_id.to_string()),
        )
    );

    let unknown_target_account = CodexAccount::new(
        "codex_profile_unknown".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: String::new(),
            refresh_token: None,
        },
    );
    let default_payload = serde_json::json!({
        "accounts": {
            "11111111-1111-4111-8111-111111111111": {
                "account": {
                    "account_id": "11111111-1111-4111-8111-111111111111",
                    "name": "Team Alpha",
                    "structure": "workspace"
                }
            },
            "00000000-0000-4000-8000-000000000000": {
                "account": {
                    "account_id": "00000000-0000-4000-8000-000000000000",
                    "name": "Default Workspace",
                    "structure": "personal",
                    "is_default": true
                }
            }
        }
    });
    assert_eq!(
        super::parse_account_profile_from_check_response(&default_payload, &unknown_target_account)
            .unwrap(),
        (
            Some("Default Workspace".to_string()),
            Some("personal".to_string()),
            Some("00000000-0000-4000-8000-000000000000".to_string()),
        )
    );

    let nested_array_payload = serde_json::json!({
        "accounts": [
            {
                "account": {
                    "account_id": target_id,
                    "name": "Nested Array Workspace",
                    "structure": "workspace"
                }
            }
        ]
    });
    assert_eq!(
        super::parse_account_profile_from_check_response(&nested_array_payload, &account).unwrap(),
        (
            Some("Nested Array Workspace".to_string()),
            Some("workspace".to_string()),
            Some(target_id.to_string()),
        )
    );
}

#[test]
fn profile_check_prefers_stored_account_id_matching_request_header_over_token_claim() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-profile-target-id-precedence");

    let stored_id = "66666666-6666-4666-8666-666666666666";
    let token_claim_id = "77777777-7777-4777-8777-777777777777";
    let tokens = make_codex_tokens(
        "user@example.invalid",
        token_claim_id,
        "org-shared",
        "precedence",
        "rt-precedence",
    );
    let mut account = CodexAccount::new(
        "codex_profile_precedence".to_string(),
        "user@example.invalid".to_string(),
        tokens.clone(),
    );
    account.plan_type = Some("team".to_string());
    account.account_id = Some(stored_id.to_string());
    account.organization_id = Some("org-shared".to_string());
    save_account(&account).expect("seed account with distinct stored account_id");

    let headers =
        super::build_account_check_headers(&account).expect("build account check headers");
    assert_eq!(
        headers
            .get("ChatGPT-Account-Id")
            .and_then(|value| value.to_str().ok()),
        Some(stored_id)
    );

    let both_payload = serde_json::json!({
        "accounts": {
            token_claim_id: {
                "account": {
                    "account_id": token_claim_id,
                    "organization_id": "org-shared",
                    "name": "Team TokenClaim",
                    "structure": "workspace"
                }
            },
            stored_id: {
                "account": {
                    "account_id": stored_id,
                    "organization_id": "org-shared",
                    "name": "Team Selected",
                    "structure": "workspace"
                }
            }
        },
        "account_ordering": [token_claim_id, stored_id]
    });

    let (name, structure, remote_id) =
        super::parse_account_profile_from_check_response(&both_payload, &account).unwrap();
    assert_eq!(name.as_deref(), Some("Team Selected"));
    assert_eq!(structure.as_deref(), Some("workspace"));
    assert_eq!(remote_id.as_deref(), Some(stored_id));

    let mut current = load_account(&account.id).expect("reload seeded account");
    assert!(super::merge_codex_account_profile(
        &mut current,
        &account,
        name,
        structure,
        remote_id
    ));
    super::save_account_with_tombstone_guard(&current).expect("persist matched workspace profile");
    let persisted = load_account(&account.id).expect("load persisted account");
    assert_eq!(persisted.account_name.as_deref(), Some("Team Selected"));
    assert_eq!(persisted.account_structure.as_deref(), Some("workspace"));
    assert_eq!(persisted.account_id.as_deref(), Some(stored_id));

    let only_token_claim_payload = serde_json::json!({
        "accounts": {
            token_claim_id: {
                "account": {
                    "account_id": token_claim_id,
                    "organization_id": "org-shared",
                    "name": "Team TokenClaim",
                    "structure": "workspace"
                }
            }
        },
        "account_ordering": [token_claim_id]
    });
    assert!(
        super::parse_account_profile_from_check_response(&only_token_claim_payload, &account)
            .is_err()
    );

    let mut fallback_account = CodexAccount::new(
        "codex_profile_token_fallback".to_string(),
        "user@example.invalid".to_string(),
        tokens,
    );
    fallback_account.account_id = Some("   ".to_string());
    let fallback_headers =
        super::build_account_check_headers(&fallback_account).expect("build fallback headers");
    assert_eq!(
        fallback_headers
            .get("ChatGPT-Account-Id")
            .and_then(|value| value.to_str().ok()),
        Some(token_claim_id)
    );
    assert_eq!(
        super::parse_account_profile_from_check_response(&both_payload, &fallback_account).unwrap(),
        (
            Some("Team TokenClaim".to_string()),
            Some("workspace".to_string()),
            Some(token_claim_id.to_string()),
        )
    );
}

#[test]
fn profile_check_prioritizes_explicit_account_id_over_generic_id_on_conflict() {
    let target_id = "11111111-1111-4111-8111-111111111111";
    let other_id = "22222222-2222-4222-8222-222222222222";
    let mut account = CodexAccount::new(
        "codex_profile_id_conflict".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: "at-id-conflict".to_string(),
            refresh_token: None,
        },
    );
    account.account_id = Some(target_id.to_string());

    // Negative: accounts[target].account has generic id == target but explicit account_id == other.
    // Must NOT falsely match "Other".
    let conflicting_target_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "account": {
                    "id": target_id,
                    "account_id": other_id,
                    "name": "Other",
                    "structure": "workspace"
                }
            }
        }
    });
    assert!(super::parse_account_profile_from_check_response(
        &conflicting_target_payload,
        &account
    )
    .is_err());

    // Positive: accounts[target].account has generic id == other but explicit account_id == target.
    // Must prefer explicit account_id and extract the target workspace name.
    let explicit_account_id_wins_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "account": {
                    "id": other_id,
                    "account_id": target_id,
                    "name": "Target Workspace",
                    "structure": "workspace"
                }
            }
        }
    });
    assert_eq!(
        super::parse_account_profile_from_check_response(
            &explicit_account_id_wins_payload,
            &account
        )
        .unwrap(),
        (
            Some("Target Workspace".to_string()),
            Some("workspace".to_string()),
            Some(target_id.to_string()),
        )
    );

    // Also when stored under an alias key while a conflicting target-keyed entry points elsewhere.
    let aliased_explicit_payload = serde_json::json!({
        "accounts": {
            target_id: {
                "account": {
                    "id": target_id,
                    "account_id": other_id,
                    "name": "Other",
                    "structure": "workspace"
                }
            },
            "slot-secondary": {
                "account": {
                    "id": "row-secondary",
                    "account_id": target_id,
                    "name": "Target From Explicit AccountId",
                    "structure": "workspace"
                }
            }
        }
    });
    assert_eq!(
        super::parse_account_profile_from_check_response(&aliased_explicit_payload, &account)
            .unwrap(),
        (
            Some("Target From Explicit AccountId".to_string()),
            Some("workspace".to_string()),
            Some(target_id.to_string()),
        )
    );
}

#[test]
fn profile_check_matches_legacy_flat_org_entry_under_alias_key_without_false_reject() {
    let target_ws_id = "11111111-1111-4111-8111-111111111111";
    let mut account = CodexAccount::new(
        "codex_profile_flat_alias_org".to_string(),
        "user@example.invalid".to_string(),
        CodexTokens {
            id_token: String::new(),
            access_token: "at-flat-alias-org".to_string(),
            refresh_token: None,
        },
    );
    account.account_id = Some(target_ws_id.to_string());
    account.organization_id = Some("org-team-alpha".to_string());

    let legacy_alias_payload = serde_json::json!({
        "accounts": {
            "00000000-0000-4000-8000-000000000000": {
                "organization_id": "org-personal",
                "name": "Personal",
                "structure": "personal"
            },
            "primary": {
                "organization_id": "org-team-alpha",
                "name": "Legacy Alias Team",
                "structure": "team"
            }
        },
        "account_ordering": ["00000000-0000-4000-8000-000000000000", "primary"]
    });

    assert_eq!(
        super::parse_account_profile_from_check_response(&legacy_alias_payload, &account).unwrap(),
        (
            Some("Legacy Alias Team".to_string()),
            Some("team".to_string()),
            None,
        )
    );

    // Ensure UUID-keyed record for another workspace with matching org still stays rejected
    // when its map key is confirmably a different workspace ID.
    let mut mismatched_ws_account = account.clone();
    mismatched_ws_account.organization_id = Some("org-personal".to_string());
    assert!(super::parse_account_profile_from_check_response(
        &legacy_alias_payload,
        &mismatched_ws_account
    )
    .is_err());
}

#[test]
fn account_profile_reads_nested_workspace_for_the_target_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-nested-workspace-profile");
    let mut account = CodexAccount::new(
        "workspace-profile-test".into(),
        "workspace@example.test".into(),
        make_codex_tokens(
            "workspace@example.test",
            "workspace-team",
            "org-workspace-team",
            "workspace-profile",
            "fixture",
        ),
    );
    account.plan_type = Some("business".into());
    let payload = serde_json::json!({
        "accounts": {
            "workspace-personal": {"account": {"name": "Personal", "structure": "personal"}},
            "workspace-team": {"account": {
                "id": "profile-record-id",
                "account_id": "workspace-team",
                "name": "Team Alpha",
                "structure": "workspace"
            }}
        },
        "account_ordering": ["workspace-personal", "workspace-team"]
    });

    let (name, structure, remote_id) =
        super::parse_account_profile_from_check_response(&payload, &account).unwrap();
    save_account(&account).unwrap();
    super::persist_codex_account_profile(&account, name, structure, remote_id).unwrap();
    let saved = load_account(&account.id).unwrap();
    assert_eq!(saved.account_name.as_deref(), Some("Team Alpha"));
    assert_eq!(saved.account_structure.as_deref(), Some("workspace"));
    assert_eq!(saved.account_id.as_deref(), Some("workspace-team"));
    assert_eq!(saved.tokens.access_token, account.tokens.access_token);

    let legacy = serde_json::json!({"accounts": [
        {"account_id": "workspace-team", "name": "Team Alpha", "structure": "workspace"}
    ]});
    assert_eq!(
        super::parse_account_profile_from_check_response(&legacy, &account)
            .unwrap().0.as_deref(),
        Some("Team Alpha")
    );
}

#[test]
fn official_account_check_accepts_target_account_key() {
    let account = CodexAccount::new(
        "codex_account_check".to_string(),
        "check@example.com".to_string(),
        make_codex_tokens(
            "check@example.com",
            "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
            "org-check",
            "check",
            "rt-check",
        ),
    );
    let payload = serde_json::json!({
        "accounts": {
            "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3": {
                "account": {
                    "account_id": "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
                    "account_residency_region": "no_constraint"
                },
                "can_access_with_session": true
            }
        },
        "account_ordering": ["3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3"]
    });

    super::validate_account_check_payload(&payload, &account)
        .expect("target account should pass official account check validation");
}

#[test]
fn official_account_check_rejects_another_account() {
    let account = CodexAccount::new(
        "codex_account_check_mismatch".to_string(),
        "check-mismatch@example.com".to_string(),
        make_codex_tokens(
            "check-mismatch@example.com",
            "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
            "org-check",
            "check-mismatch",
            "rt-check-mismatch",
        ),
    );
    let payload = serde_json::json!({
        "accounts": {
            "6a7dc3f2-ea90-4456-9426-a46bd8b3e6f9": {
                "name": "Another"
            }
        },
        "account_ordering": ["6a7dc3f2-ea90-4456-9426-a46bd8b3e6f9"]
    });

    let error = super::validate_account_check_payload(&payload, &account)
        .expect_err("another account must not pass target account validation");
    assert!(error.message.contains("与目标账号不一致"));
}

#[test]
fn official_account_check_rejects_session_without_account_access() {
    let account = CodexAccount::new(
        "codex_account_check_denied".to_string(),
        "check-denied@example.com".to_string(),
        make_codex_tokens(
            "check-denied@example.com",
            "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3",
            "org-check",
            "check-denied",
            "rt-check-denied",
        ),
    );
    let payload = serde_json::json!({
        "accounts": {
            "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3": {
                "account": {
                    "account_id": "3a7dc3f2-ea90-4456-9426-a46bd8b3e6f3"
                },
                "can_access_with_session": false
            }
        }
    });

    let error = super::validate_account_check_payload(&payload, &account)
        .expect_err("session without account access must be rejected");
    assert!(error.message.contains("不允许当前登录态访问目标账号"));
}
