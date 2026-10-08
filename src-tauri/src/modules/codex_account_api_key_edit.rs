// API Key edits hold only per-ID locks while migrating profiles and gateway references.
// Weak entries avoid retaining locks for accounts that have already been removed.
fn api_key_credential_edit_locks(
    old_id: &str,
    new_id: &str,
) -> Result<Vec<Arc<Mutex<()>>>, String> {
    static LOCKS: std::sync::LazyLock<Mutex<HashMap<String, std::sync::Weak<Mutex<()>>>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut locks = LOCKS
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    let mut ids = vec![old_id.to_string(), new_id.to_string()];
    // Sharing either the source or destination ID serializes edits; ordering prevents deadlocks.
    ids.sort();
    ids.dedup();
    Ok(ids
        .into_iter()
        .map(|id| {
            let lock = locks
                .get(&id)
                .and_then(std::sync::Weak::upgrade)
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            locks.insert(id, Arc::downgrade(&lock));
            lock
        })
        .collect())
}

fn commit_api_key_credential_index(old_id: &str, account: &CodexAccount) -> Result<(), String> {
    // Re-read after slow reference migration. The short global transaction merges this edit
    // with concurrent credential edits and with account lifecycle operations using this lock.
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    // Maintenance readers can discover the new detail before this transaction commits.
    // Read only the saved index here: never run repair/scans under the mutation lock.
    let path = get_accounts_storage_path();
    let content =
        fs::read_to_string(&path).map_err(|error| format!("读取账号索引失败: {}", error))?;
    let mut index: CodexAccountIndex =
        serde_json::from_str(&content).map_err(|error| format!("解析账号索引失败: {}", error))?;
    if old_id != account.id {
        // Destination IDs are protected for the entire edit; any discovered new row
        // belongs to the detail we already saved, rather than a competing edit.
        index.accounts.retain(|summary| summary.id != account.id);
    }
    let mut summary_found = false;
    for summary in &mut index.accounts {
        if summary.id == old_id {
            summary.id = account.id.clone();
            summary.email = account.email.clone();
            summary.plan_type = account.plan_type.clone();
            summary.subscription_active_until = account.subscription_active_until.clone();
            summary.last_used = account.last_used;
            summary_found = true;
            break;
        }
    }
    if !summary_found {
        // Do not resurrect an account removed while profile migration was in flight.
        load_account(old_id).ok_or_else(|| format!("账号不存在: {}", old_id))?;
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    if index.current_account_id.as_deref() == Some(old_id) {
        index.current_account_id = Some(account.id.clone());
    }
    save_account_index(&index)?;
    if old_id != account.id {
        delete_account_file_unlocked(old_id)?;
    }
    Ok(())
}
