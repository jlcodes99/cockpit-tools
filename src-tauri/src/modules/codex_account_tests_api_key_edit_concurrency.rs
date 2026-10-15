// Exercise the production credential editor and real account files/index under concurrent edits.
use super::{
    build_api_key_account_id, commit_api_key_credential_index, load_account_index_checked,
};
use std::collections::HashMap;

fn seed_api_key_edit_accounts(count: usize) -> Vec<CodexAccount> {
    let mut index = CodexAccountIndex::new();
    let accounts = (0..count)
        .map(|n| {
            let key = format!("sk-concurrency-before-{n}");
            let account = CodexAccount::new_api_key(
                build_api_key_account_id(&key),
                format!("key-{n}@example.invalid"),
                key,
                CodexApiProviderMode::Custom,
                Some("https://example.invalid/v1".into()),
                Some("fixture".into()),
                Some("Fixture".into()),
                Vec::new(),
            );
            save_account(&account).unwrap();
            index.accounts.push(CodexAccountSummary {
                id: account.id.clone(),
                email: account.email.clone(),
                plan_type: account.plan_type.clone(),
                subscription_active_until: account.subscription_active_until.clone(),
                created_at: account.created_at,
                last_used: account.last_used,
            });
            account
        })
        .collect::<Vec<_>>();
    save_account_index(&index).unwrap();
    accounts
}

#[test]
fn api_key_edit_concurrent_commits_merge_latest_index_and_current_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = TestEnvGuard::new("codex-key-edit-index-concurrency");
    let accounts = seed_api_key_edit_accounts(3);
    let mut index = load_account_index();
    index.current_account_id = Some(accounts[0].id.clone());
    save_account_index(&index).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let edited = std::thread::scope(|scope| {
        let handles = accounts[..2]
            .iter()
            .enumerate()
            .map(|(n, original)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    // Both edits have read the same early index before reference migration.
                    let early_index = load_account_index_checked().unwrap();
                    assert!(early_index
                        .accounts
                        .iter()
                        .any(|item| item.id == original.id));
                    let mut edited = original.clone();
                    edited.id = build_api_key_account_id(&format!("sk-concurrency-after-{n}"));
                    edited.openai_api_key = Some(format!("sk-concurrency-after-{n}"));
                    save_account(&edited).unwrap();
                    barrier.wait();
                    commit_api_key_credential_index(&original.id, &edited).unwrap();
                    edited
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let index = load_account_index_checked().unwrap();
    assert_eq!(index.accounts.len(), 3);
    assert_eq!(
        index.current_account_id.as_deref(),
        Some(edited[0].id.as_str())
    );
    for (original, edited) in accounts.iter().zip(&edited) {
        assert!(index.accounts.iter().any(|item| item.id == edited.id));
        assert!(load_account(&edited.id).is_some());
        assert!(load_account(&original.id).is_none());
    }
    assert!(index.accounts.iter().any(|item| item.id == accounts[2].id));
    assert!(load_account(&accounts[2].id).is_some());
}

#[test]
fn api_key_edit_parallel_credentials_preserve_all_rotated_accounts() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = TestEnvGuard::new("codex-key-edit-production-concurrency");
    let accounts = seed_api_key_edit_accounts(8);
    let barrier = std::sync::Barrier::new(accounts.len());
    let edited = std::thread::scope(|scope| {
        let handles = accounts
            .iter()
            .enumerate()
            .map(|(n, original)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    update_api_key_credentials(
                        &original.id,
                        format!("sk-concurrency-after-{n}"),
                        Some("https://example.invalid/v1".into()),
                        Some(CodexApiProviderMode::Custom),
                        Some("fixture".into()),
                        Some("Fixture".into()),
                        Vec::new(),
                        Some(false),
                        Some("responses".into()),
                        false,
                        false,
                        HashMap::new(),
                        None,
                        None,
                        None,
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let index = load_account_index_checked().unwrap();
    assert_eq!(index.accounts.len(), accounts.len());
    for (n, (original, edited)) in accounts.iter().zip(&edited).enumerate() {
        assert!(index.accounts.iter().any(|item| item.id == edited.id));
        assert!(load_account(&original.id).is_none());
        assert_eq!(
            load_account(&edited.id).unwrap().openai_api_key.as_deref(),
            Some(format!("sk-concurrency-after-{n}").as_str())
        );
    }
}

#[test]
fn api_key_edit_commit_accepts_new_detail_discovered_by_index_repair() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = TestEnvGuard::new("codex-key-edit-repair-before-commit");
    let accounts = seed_api_key_edit_accounts(2);
    let original = &accounts[0];
    let mut index = load_account_index();
    index.current_account_id = Some(original.id.clone());
    save_account_index(&index).unwrap();
    let mut edited = original.clone();
    edited.id = build_api_key_account_id("sk-discovered-before-commit");
    edited.openai_api_key = Some("sk-discovered-before-commit".into());
    save_account(&edited).unwrap();
    // A maintenance read can legitimately pick up the new detail during reference migration.
    let repaired = load_account_index_checked().unwrap();
    assert!(repaired
        .accounts
        .iter()
        .any(|summary| summary.id == edited.id));
    assert!(repaired
        .accounts
        .iter()
        .any(|summary| summary.id == original.id));
    let expected = repaired
        .accounts
        .iter()
        .filter(|summary| summary.id != edited.id)
        .map(|summary| {
            if summary.id == original.id {
                edited.id.clone()
            } else {
                summary.id.clone()
            }
        })
        .collect::<Vec<_>>();
    commit_api_key_credential_index(&original.id, &edited).unwrap();
    let index = load_account_index_checked().unwrap();
    assert_eq!(
        index
            .accounts
            .iter()
            .map(|summary| summary.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        index.current_account_id.as_deref(),
        Some(edited.id.as_str())
    );
    assert!(load_account(&original.id).is_none());
    assert_eq!(
        load_account(&edited.id).unwrap().openai_api_key,
        edited.openai_api_key
    );
}

#[test]
fn api_key_edit_shared_destination_cannot_overwrite_another_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = TestEnvGuard::new("codex-key-edit-shared-destination");
    let accounts = seed_api_key_edit_accounts(2);
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles = accounts
            .iter()
            .map(|original| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    update_api_key_credentials(
                        &original.id,
                        "sk-shared-destination".into(),
                        Some("https://example.invalid/v1".into()),
                        Some(CodexApiProviderMode::Custom),
                        Some("fixture".into()),
                        Some("Fixture".into()),
                        Vec::new(),
                        Some(false),
                        Some("responses".into()),
                        false,
                        false,
                        HashMap::new(),
                        None,
                        None,
                        None,
                    )
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let index = load_account_index_checked().unwrap();
    assert_eq!(index.accounts.len(), 2);
    for summary in &index.accounts {
        assert!(load_account(&summary.id).is_some());
    }
    let winning = results
        .iter()
        .find_map(|result| result.as_ref().ok())
        .unwrap();
    assert_eq!(load_account(&winning.id).unwrap().email, winning.email);
    let losing = results.iter().position(Result::is_err).unwrap();
    assert!(index
        .accounts
        .iter()
        .any(|item| item.id == accounts[losing].id));
}
