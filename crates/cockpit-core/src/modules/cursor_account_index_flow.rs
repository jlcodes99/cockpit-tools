// Shared by desktop and CLI Cursor. Credential serialization stays platform-local.
use crate::modules::antigravity_account_index_store::{IndexRead, IndexStore};
use crate::modules::{cursor_account_recovery, provider_account_index};

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(
        provider_account_index::storage_path::<CursorAccountIndex>(&get_accounts_index_path()?)?
            .to_string_lossy()
            .into_owned(),
    )
}

fn index_store() -> Result<IndexStore<CursorAccountIndex>, String> {
    provider_account_index::store(&get_accounts_index_path()?)
}

fn load_account_index_checked() -> Result<CursorAccountIndex, String> {
    let store = index_store()?;
    match store.load()? {
        IndexRead::Active(index) if index.detail_recovery_complete => Ok(index),
        IndexRead::Active(index) | IndexRead::Legacy(index) => {
            schedule_cursor_recovery(store)?;
            Ok(index)
        }
        IndexRead::Missing => {
            schedule_cursor_recovery(store)?;
            Ok(CursorAccountIndex::new())
        }
        IndexRead::InvalidLegacy(error) => {
            let directory = get_data_dir()?.join(ACCOUNTS_DIR);
            if cursor_account_recovery::directory_has_entries(&directory)? {
                schedule_cursor_recovery(store)?;
                Ok(CursorAccountIndex::new())
            } else {
                Err(crate::error::file_corrupted_error(
                    ACCOUNTS_INDEX_FILE,
                    &store.legacy_path().to_string_lossy(),
                    &error,
                ))
            }
        }
    }
}

fn save_account_index(index: &CursorAccountIndex) -> Result<(), String> {
    index_store()?.replace(index)
}

fn normalize_account_index(index: &mut CursorAccountIndex) -> Result<Vec<CursorAccount>, String> {
    let mut accounts = Vec::new();
    let mut unreadable = Vec::new();
    let mut seen_ids = HashSet::new();
    for summary in &index.accounts {
        if !seen_ids.insert(summary.id.as_str()) {
            continue;
        }
        match load_account(&summary.id) {
            Some(account) if account.id == summary.id => accounts.push(account),
            _ => unreadable.push(summary.clone()),
        }
    }
    if accounts.is_empty() {
        if !unreadable.is_empty() {
            return Err("Cursor 账号索引存在，但详情暂时无法读取".into());
        }
        if index.detail_recovery_requires_account
            || (!index.detail_recovery_complete
                && cursor_account_recovery::directory_has_entries(
                    &get_data_dir()?.join(ACCOUNTS_DIR),
                )?)
        {
            return Err(cursor_recovery_busy());
        }
        return Ok(accounts);
    }

    fn find(parents: &mut [usize], mut node: usize) -> usize {
        while parents[node] != node {
            parents[node] = parents[parents[node]];
            node = parents[node];
        }
        node
    }

    fn union(parents: &mut [usize], left: usize, right: usize) {
        let left = find(parents, left);
        let right = find(parents, right);
        if left != right {
            parents[right] = left;
        }
    }

    // Auth IDs partition accounts. With no auth ID, matching email connects
    // accounts. A token can connect different known emails only via a record
    // without an email, preserving the original transitive grouping.
    let mut parents: Vec<_> = (0..accounts.len()).collect();
    let mut auth_ids = HashMap::new();
    let mut emails = HashMap::new();
    let mut tokens_without_email = HashMap::new();
    let mut tokens_with_email = Vec::new();
    for (position, account) in accounts.iter().enumerate() {
        if let Some(auth_id) = resolve_account_auth_id(account) {
            if let Some(previous) = auth_ids.insert(auth_id, position) {
                union(&mut parents, previous, position);
            }
            continue;
        }
        let token = normalize_token_identity(Some(account.access_token.as_str()));
        if let Some(email) = normalize_email_identity(Some(account.email.as_str())) {
            if let Some(previous) = emails.insert(email, position) {
                union(&mut parents, previous, position);
            }
            if let Some(token) = token {
                tokens_with_email.push((token, position));
            }
        } else if let Some(token) = token {
            if let Some(previous) = tokens_without_email.insert(token, position) {
                union(&mut parents, previous, position);
            }
        }
    }
    for (token, position) in tokens_with_email {
        if let Some(&previous) = tokens_without_email.get(&token) {
            union(&mut parents, previous, position);
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for position in 0..accounts.len() {
        groups
            .entry(find(&mut parents, position))
            .or_default()
            .push(position);
    }
    let mut normalized = Vec::new();
    let mut merged = Vec::new();
    let mut removed = Vec::new();
    for position in 0..accounts.len() {
        let Some(group) = groups.remove(&find(&mut parents, position)) else {
            continue;
        };
        let primary = choose_primary_account_index(&group, &accounts);
        let mut account = accounts[primary].clone();
        if group.len() > 1 {
            for member in group.into_iter().filter(|&member| member != primary) {
                merge_duplicate_account(&mut account, &accounts[member]);
                removed.push(accounts[member].id.clone());
            }
            merged.push(normalized.len());
        }
        normalized.push(account);
    }

    let mut summaries: Vec<_> = normalized.iter().map(CursorAccount::summary).collect();
    summaries.extend(unreadable);
    if serde_json::to_vec(&summaries).map_err(|error| error.to_string())?
        != serde_json::to_vec(&index.accounts).map_err(|error| error.to_string())?
    {
        for position in merged {
            save_account_file(&normalized[position])?;
        }
        index.accounts = summaries;
        index_store()?.replace_with_removals(index, &removed)?;
        for id in &removed {
            if let Err(error) = delete_account_file(id) {
                logger::log_warn(&format!(
                    "[Cursor Account] 删除重复账号详情失败: id={id}, error={error}"
                ));
            }
        }
    }
    Ok(normalized)
}

pub fn list_accounts() -> Vec<CursorAccount> {
    list_accounts_checked().unwrap_or_else(|error| {
        logger::log_warn(&format!("[Cursor Account] {error}"));
        Vec::new()
    })
}

pub fn list_accounts_checked() -> Result<Vec<CursorAccount>, String> {
    let _lock = CURSOR_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 Cursor 账号锁失败".to_string())?;
    normalize_account_index(&mut load_account_index_checked()?)
}
