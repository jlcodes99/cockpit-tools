// Merge asynchronous OAuth results into the latest saved account. Never recreate
// a deleted account or let an old HTTP/CLI snapshot replace a newer login.
fn update_oauth_account_if_current(
    account_id: &str,
    expected_credentials: &Option<Value>,
    update: impl Fn(&mut ClaudeAccount) -> Result<(), String>,
) -> Result<ClaudeAccount, String> {
    let _index_lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁")?;
    let path = account_file_path(account_id)?;
    for _ in 0..3 {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("读取 Claude 账号失败，账号可能已删除: {}", error))?;
        // Decode the exact bytes used by CAS; do not restore a backup here.
        let mut current: ClaudeAccount = match serde_json::from_str(&content) {
            Ok(current) => current,
            Err(_) => {
                crate::modules::secure_account_storage::deserialize_encrypted_snapshot(&content)?
            }
        };
        if current.id != account_id {
            return Err("Claude 账号标识不一致".to_string());
        }
        if !matches!(
            current.auth_mode,
            ClaudeAuthMode::OAuth | ClaudeAuthMode::SetupToken
        ) || &current.claude_credentials_raw != expected_credentials
        {
            return Ok(current);
        }
        update(&mut current)?;
        if current.id != account_id {
            return Err("Claude 同步账号标识不一致".to_string());
        }
        slim_claude_account_snapshots(&mut current);
        let serialized =
            crate::modules::secure_account_storage::serialize_account_file("claude", &current)?;
        if atomic_write::write_string_atomic_if_hash_matches(
            &path,
            Sha256::digest(content.as_bytes()).into(),
            || Ok(serialized),
        )? {
            let mut index = load_index()?;
            index.accounts.retain(|item| item.id != account_id);
            index.accounts.push(current.summary());
            index.accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
            save_index(&index)?;
            return Ok(current);
        }
    }
    Err("Claude 账号正在更新，请重试".to_string())
}

fn save_oauth_quota_result(account: &ClaudeAccount) -> Result<ClaudeAccount, String> {
    update_oauth_account_if_current(&account.id, &account.claude_credentials_raw, |current| {
        current.quota = account.quota.clone();
        current.claude_usage_raw = account.claude_usage_raw.clone();
        current.quota_error = account.quota_error.clone();
        current.usage_updated_at = account.usage_updated_at;
        current.status = account.status.clone();
        current.status_reason = account.status_reason.clone();
        Ok(())
    })
}

static CLAUDE_QUOTA_REFRESHING: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

struct ClaudeQuotaRefreshGuard(String);

impl ClaudeQuotaRefreshGuard {
    fn acquire(account_id: &str) -> Option<Self> {
        CLAUDE_QUOTA_REFRESHING
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(account_id.to_string())
            .then(|| Self(account_id.to_string()))
    }
}

impl Drop for ClaudeQuotaRefreshGuard {
    fn drop(&mut self) {
        CLAUDE_QUOTA_REFRESHING
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.0);
    }
}
