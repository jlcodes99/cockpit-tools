// Display-only helpers for the injected usage dialog. Call on a blocking worker;
// these helpers never prepare routes, refresh credentials or migrate account files.
const INJECTION_DISPLAY_ACCOUNT_FILE_LIMIT: u64 = 4 * 1024 * 1024;
const INJECTION_DISPLAY_KEY_FILE_LIMIT: u64 = 4096;

/// Authenticate the exact profile bearer without requiring a request-log database.
/// A legacy primary key inherits the global pool only when no client keys exist.
pub(crate) fn injection_gpt_api_key_account_count_for_bearer(
    collection: &CodexLocalAccessCollection,
    bearer: &str,
    data_dir: &Path,
) -> Option<i64> {
    if bearer.trim().is_empty() {
        return None;
    }
    if let Some(key) = collection.api_keys.iter().find(|key| key.key == bearer) {
        if !key.enabled {
            return None;
        }
        return injection_display_count_in_dir(
            data_dir,
            effective_api_key_account_ids(collection, key),
        );
    }
    if collection.api_keys.is_empty() && collection.api_key == bearer {
        return injection_display_count_in_dir(data_dir, collection.account_ids.clone());
    }
    None
}

/// Count GPT/Codex-capable API Key accounts for an already authenticated key ID.
/// The caller supplies the same data directory used to read the service/logs.
/// Unknown metadata stays unknown; it must not become an apparently empty pool.
pub(crate) fn injection_gpt_api_key_account_count(
    collection: &CodexLocalAccessCollection,
    key_id: &str,
    data_dir: &Path,
) -> Option<i64> {
    let ids = injection_display_account_ids(collection, key_id)?;
    injection_display_count_in_dir(data_dir, ids)
}

fn injection_display_account_ids(
    collection: &CodexLocalAccessCollection,
    key_id: &str,
) -> Option<Vec<String>> {
    if key_id.is_empty() {
        return None;
    }
    let key = collection
        .api_keys
        .iter()
        .find(|key| key.enabled && !key.key.trim().is_empty() && key.id == key_id)?;
    // In particular, inherit_account_pool=false + [] stays empty. Never fall
    // back to the collection's primary key or pool after a failed key lookup.
    Some(effective_api_key_account_ids(collection, key))
}

fn injection_display_count_in_dir(data_dir: &Path, ids: Vec<String>) -> Option<i64> {
    let mut seen = HashSet::new();
    let mut count = 0i64;
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let account = injection_display_read_account(data_dir, &id)?;
        if account.is_api_key_auth() && account_provides_gpt_models(&account) {
            count += 1;
        }
    }
    Some(count)
}

fn injection_display_read_account(data_dir: &Path, id: &str) -> Option<CodexAccount> {
    if id.is_empty()
        || id.len() > 256
        || matches!(id, "." | "..")
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return None;
    }
    // A stale collection must not count an account whose detail remains after
    // deletion. Presence/error is unknown here; this reader never repairs it.
    let tombstone = data_dir
        .join("codex_account_tombstones")
        .join(format!("{id}.json"));
    match fs::symlink_metadata(tombstone) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return None,
    }
    let path = data_dir.join("codex_accounts").join(format!("{id}.json"));
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > INJECTION_DISPLAY_ACCOUNT_FILE_LIMIT {
        return None;
    }
    let key_path = data_dir.join("secure-account-storage.key");
    match fs::symlink_metadata(&key_path) {
        Ok(metadata)
            if !metadata.is_file() || metadata.len() > INJECTION_DISPLAY_KEY_FILE_LIMIT =>
        {
            return None;
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return None,
        _ => {}
    }
    let value = crate::modules::secure_account_storage::read_account_file_readonly::<Value>(
        &path, &key_path,
    )
    .ok()?;
    // The normal loader may infer or migrate legacy authentication metadata.
    // A read-only counter must not turn an absent auth mode into OAuth/zero.
    value.get("auth_mode")?.as_str()?;
    let account: CodexAccount = serde_json::from_value(value).ok()?;
    (account.id == id).then_some(account)
}

#[cfg(test)]
mod injection_display_tests {
    use super::*;

    fn collection(ids: &[&str], inherit: bool) -> CodexLocalAccessCollection {
        serde_json::from_value(json!({
            "enabled": true,
            "port": 58554,
            "apiKey": "primary-key",
            "apiKeys": [{
                "id": "client-key",
                "label": "Fixture client",
                "key": "fixture-bearer",
                "enabled": true,
                "inheritAccountPool": inherit,
                "accountIds": if inherit { Vec::<&str>::new() } else { ids.to_vec() },
            }],
            "accountIds": ids,
            "createdAt": 1,
            "updatedAt": 1,
        }))
        .unwrap()
    }

    fn api_account(id: &str, models: &[&str]) -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            id.into(),
            format!("{id}@example.test"),
            format!("sk-fixture-{id}"),
            CodexApiProviderMode::Custom,
            Some("https://example.test/v1".into()),
            None,
            None,
            models.iter().map(|model| model.to_string()).collect(),
        );
        account.api_wire_api = Some("responses".into());
        account
    }

    fn save(dir: &Path, account: &CodexAccount) -> PathBuf {
        let parent = dir.join("codex_accounts");
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!("{}.json", account.id));
        fs::write(&path, serde_json::to_vec(account).unwrap()).unwrap();
        path
    }

    fn count(dir: &Path, collection: &CodexLocalAccessCollection, key_id: &str) -> Option<i64> {
        injection_gpt_api_key_account_count(collection, key_id, dir)
    }

    #[test]
    fn injection_display_counts_real_gpt_api_accounts_once_without_rewriting_files() {
        let dir = tempfile::tempdir().unwrap();
        let gpt = api_account("gpt", &["vendor/gpt-6.1-sol"]);
        let responses = api_account("responses", &[]);
        let mut chat_empty = api_account("chat-empty", &[]);
        chat_empty.api_wire_api = Some("chat_completions".into());
        let mut oauth = api_account("oauth", &["gpt-6.1-sol"]);
        oauth.auth_mode = CodexAuthMode::OAuth;
        oauth.openai_api_key = None;
        let path = save(dir.path(), &gpt);
        let before = fs::read(&path).unwrap();
        for account in [&responses, &chat_empty, &oauth] {
            save(dir.path(), account);
        }
        let collection = collection(&["gpt", "gpt", "responses", "chat-empty", "oauth"], true);
        assert_eq!(count(dir.path(), &collection, "client-key"), Some(2));
        assert_eq!(fs::read(path).unwrap(), before);
        assert!(!dir.path().join("secure-account-storage.key").exists());
    }

    #[test]
    fn injection_display_bearer_count_honors_scope_and_legacy_primary_without_running_service() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &api_account("allowed", &["gpt-6.1-sol"]));
        let mut collection = collection(&["allowed"], false);
        collection.enabled = false;
        let count = |collection: &CodexLocalAccessCollection, bearer| {
            injection_gpt_api_key_account_count_for_bearer(collection, bearer, dir.path())
        };
        assert_eq!(count(&collection, "fixture-bearer"), Some(1));
        collection.api_keys[0].account_ids.clear();
        assert_eq!(count(&collection, "fixture-bearer"), Some(0));
        collection.api_keys[0].enabled = false;
        assert_eq!(count(&collection, "fixture-bearer"), None);
        assert_eq!(count(&collection, "primary-key"), None);
        collection.api_keys.clear();
        assert_eq!(count(&collection, "primary-key"), Some(1));
        assert_eq!(count(&collection, "unknown"), None);
        assert!(!dir.path().join("codex_local_access_logs.sqlite").exists());
    }

    #[test]
    fn injection_display_excludes_deepseek_gpt_shells_and_grok_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let mut deepseek = api_account("deepseek", &["gpt-6.1-sol"]);
        deepseek.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
            client_model: "gpt-6.1-sol".into(),
            upstream_model: "deepseek-flash".into(),
        }];
        let grok = api_account("grok", &["grok-4"]);
        let mut grok_identity = api_account("grok-identity", &["gpt-6.1-sol"]);
        grok_identity.upstream_grok_account_id = Some("upstream-grok".into());
        for account in [&deepseek, &grok, &grok_identity] {
            save(dir.path(), account);
        }
        let collection = collection(&["deepseek", "grok", "grok-identity"], false);
        assert_eq!(count(dir.path(), &collection, "client-key"), Some(0));
    }

    #[test]
    fn injection_display_uses_enabled_key_scope_and_keeps_explicit_empty_scope() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &api_account("allowed", &["gpt-6.1-sol"]));
        let mut collection = collection(&["allowed"], false);
        collection.account_ids = vec!["missing-global-account".into()];
        assert_eq!(count(dir.path(), &collection, "client-key"), Some(1));
        collection.api_keys[0].account_ids.clear();
        assert_eq!(count(dir.path(), &collection, "client-key"), Some(0));
        assert_eq!(count(dir.path(), &collection, "primary-key"), None);
        assert_eq!(count(dir.path(), &collection, "unknown"), None);
        collection.api_key = "fixture-bearer".into();
        collection.api_keys[0].enabled = false;
        assert_eq!(count(dir.path(), &collection, "client-key"), None);
        assert_eq!(count(dir.path(), &collection, ""), None);
    }

    #[test]
    fn injection_display_returns_unknown_when_required_account_metadata_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &api_account("known", &["gpt-6.1-sol"]));
        let collection = collection(&["known", "missing"], true);
        assert_eq!(count(dir.path(), &collection, "client-key"), None);
        fs::write(dir.path().join("codex_accounts/missing.json"), "{}").unwrap();
        assert_eq!(count(dir.path(), &collection, "client-key"), None);
        let mut wrong_identity = api_account("missing", &["gpt-6.1-sol"]);
        let path = save(dir.path(), &wrong_identity);
        let mut missing_mode = serde_json::to_value(&wrong_identity).unwrap();
        missing_mode.as_object_mut().unwrap().remove("auth_mode");
        fs::write(&path, serde_json::to_vec(&missing_mode).unwrap()).unwrap();
        assert_eq!(count(dir.path(), &collection, "client-key"), None);
        wrong_identity.id = "another-account".into();
        fs::write(path, serde_json::to_vec(&wrong_identity).unwrap()).unwrap();
        assert_eq!(count(dir.path(), &collection, "client-key"), None);
    }

    #[test]
    fn injection_display_rejects_unsafe_oversized_and_deleted_account_files() {
        let dir = tempfile::tempdir().unwrap();
        let account = api_account("known", &["gpt-6.1-sol"]);
        let path = save(dir.path(), &account);
        assert!(injection_display_read_account(dir.path(), "../known").is_none());
        assert!(injection_display_read_account(dir.path(), ".").is_none());
        assert!(injection_display_read_account(dir.path(), "").is_none());
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(INJECTION_DISPLAY_ACCOUNT_FILE_LIMIT + 1)
            .unwrap();
        assert!(injection_display_read_account(dir.path(), "known").is_none());
        save(dir.path(), &account);
        let tombstones = dir.path().join("codex_account_tombstones");
        fs::create_dir(&tombstones).unwrap();
        fs::write(tombstones.join("known.json"), "{}").unwrap();
        assert!(injection_display_read_account(dir.path(), "known").is_none());
    }
}
