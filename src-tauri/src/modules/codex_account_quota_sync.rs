// Quota writes merge only the fields owned by this request, under the account
// transaction lock. A reauthorization, workspace change or deletion invalidates it.
static QUOTA_QUERY_COMMITS: std::sync::LazyLock<Mutex<HashMap<String, Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn quota_request_identity_matches(current: &CodexAccount, original: &CodexAccount) -> bool {
    current.token_generation == original.token_generation
        && current.tokens.access_token == original.tokens.access_token
        && current.account_id == original.account_id
        && current.organization_id == original.organization_id
}

pub(crate) fn save_refreshed_quota(
    original: &CodexAccount,
    quota: crate::models::codex::CodexQuota,
    plan_type: Option<String>,
    query_started: Instant,
) -> Result<CodexAccount, String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "CODEX_QUOTA_WRITE_LOCK_POISONED".to_string())?;
    let mut current =
        load_account(&original.id).ok_or_else(|| "CODEX_STALE_ACCOUNT".to_string())?;
    if !quota_request_identity_matches(&current, original) {
        return Err("CODEX_STALE_QUOTA_REQUEST".to_string());
    }
    if QUOTA_QUERY_COMMITS
        .lock()
        .map_err(|_| "CODEX_QUOTA_WRITE_LOCK_POISONED".to_string())?
        .get(&original.id)
        .is_some_and(|last| *last > query_started)
    {
        return Err("CODEX_STALE_QUOTA_REQUEST".to_string());
    }
    let now = chrono::Utc::now().timestamp();
    current.replace_quota_preserving_team_history(quota, now);
    current.quota_error = None;
    current.usage_updated_at = Some(now);
    if plan_type.is_some() && current.plan_type == original.plan_type {
        current.plan_type = plan_type;
    }
    save_account_with_tombstone_guard(&current)?;
    if let Ok(mut commits) = QUOTA_QUERY_COMMITS.lock() {
        commits.retain(|_, at| at.elapsed() < Duration::from_secs(600));
        commits.insert(current.id.clone(), query_started);
    }
    // Commit and notification have the same order, even if two callers await
    // different blocking workers. Emitting only queues a small local UI event.
    crate::modules::codex_quota_sync::publish(&current);
    if current.plan_type != original.plan_type {
        if let Err(error) = update_account_plan_type_in_index(
            &current.id,
            &current.plan_type,
            &current.subscription_active_until,
        ) {
            logger::log_warn(&format!(
                "[Codex Quota Sync] plan index update failed: {error}"
            ));
        }
    }
    Ok(current)
}

pub(crate) fn save_quota_subscription_maintenance(
    original: &CodexAccount,
    refreshed: &CodexAccount,
) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "CODEX_QUOTA_WRITE_LOCK_POISONED".to_string())?;
    let mut current =
        load_account(&original.id).ok_or_else(|| "CODEX_STALE_ACCOUNT".to_string())?;
    if !quota_request_identity_matches(&current, original) {
        return Err("CODEX_STALE_QUOTA_REQUEST".to_string());
    }
    if original.account_id.is_some() && refreshed.account_id != original.account_id {
        return Err("CODEX_QUOTA_IDENTITY_MISMATCH".to_string());
    }
    // Subscription enrichment must never write an older quota or credentials back.
    macro_rules! merge { ($($field:ident),+) => { $(
        if current.$field == original.$field { current.$field = refreshed.$field.clone(); }
    )+ }; }
    merge!(
        account_id,
        plan_type,
        subscription_active_until,
        subscription_query_last_attempt_at,
        subscription_query_last_success_at,
        subscription_query_next_retry_at,
        subscription_query_last_error
    );
    save_account_with_tombstone_guard(&current)?;
    if current.plan_type != original.plan_type
        || current.subscription_active_until != original.subscription_active_until
    {
        update_account_plan_type_in_index(
            &current.id,
            &current.plan_type,
            &current.subscription_active_until,
        )?;
    }
    Ok(())
}

pub(crate) fn save_quota_sync_failure(
    original: &CodexAccount,
    message: String,
    query_started: Instant,
) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "CODEX_QUOTA_WRITE_LOCK_POISONED".to_string())?;
    let Some(mut current) = load_account(&original.id) else {
        return Ok(());
    };
    if !quota_request_identity_matches(&current, original)
        || QUOTA_QUERY_COMMITS
            .lock()
            .map_err(|_| "CODEX_QUOTA_WRITE_LOCK_POISONED".to_string())?
            .get(&original.id)
            .is_some_and(|at| *at > query_started)
    {
        return Ok(());
    }
    if crate::modules::codex_quota::write_quota_error(&mut current, message) {
        save_account_with_tombstone_guard(&current)?;
    }
    Ok(())
}

/// A custom group interval still works when platform refresh is disabled.
/// Explicitly disabled groups always win, including overlapping legacy groups.
pub(crate) fn quota_sync_refresh_allowed_ids(
    platform_enabled: bool,
) -> (bool, HashSet<String>, HashSet<String>) {
    let mut custom = HashSet::new();
    let mut disabled = HashSet::new();
    for group in load_codex_account_group_records() {
        match group.policy() {
            CodexGroupQuotaRefreshPolicy::Disabled => disabled.extend(group.account_ids),
            CodexGroupQuotaRefreshPolicy::Minutes(_) => custom.extend(group.account_ids),
            CodexGroupQuotaRefreshPolicy::Inherit => {}
        }
    }
    (platform_enabled, custom, disabled)
}

impl CodexQuotaRuntimeSnapshot {
    pub(crate) fn quota_activity_profiles(
        &self,
        accounts: &[CodexAccount],
    ) -> Vec<(PathBuf, String)> {
        let mut dirs = HashSet::new();
        // CLI activity can exist without a desktop process. Configured homes
        // are observed too, but start idle until new local activity is seen.
        dirs.insert(get_codex_home());
        for (_, home) in self.process_entries() {
            dirs.insert(
                home.as_ref()
                    .map(PathBuf::from)
                    .unwrap_or_else(get_codex_home),
            );
        }
        if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
            for instance in store.instances {
                dirs.insert(PathBuf::from(instance.user_data_dir));
            }
        }
        dirs.into_iter()
            .filter_map(|dir| {
                let id = oauth_account_id_for_runtime_snapshot(&dir, accounts)?;
                Some((dir, id))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "codex_account_quota_sync_tests.rs"]
mod quota_sync_tests;
