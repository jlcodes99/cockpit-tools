// Official quota exhaustion is independent of retryable scheduler failures.
// Keep this small snapshot in memory so state reads and routing never need to
// scan/decrypt accounts while holding the gateway lock.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountQuotaCooldown {
    exhausted: bool,
    reset_at_ms: Option<i64>,
    updated_at_ms: i64,
}

impl AccountQuotaCooldown {
    fn active(&self, now: i64) -> bool {
        self.exhausted && self.reset_at_ms.map(|reset| reset > now).unwrap_or(true)
    }
}

fn account_quota_cooldown(account: &CodexAccount, now: i64) -> Option<AccountQuotaCooldown> {
    if account.is_api_key_auth() {
        return None;
    }
    let quota = account.quota.as_ref()?;
    let has_presence_flags =
        quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
    let windows = [
        (quota.hourly_percentage, quota.hourly_window_present, quota.hourly_reset_time),
        (quota.weekly_percentage, quota.weekly_window_present, quota.weekly_reset_time),
    ];
    let mut exhausted = false;
    let mut reset_at_ms = Some(0_i64);
    for (remaining, present, reset_at) in windows {
        if remaining != 0 || (has_presence_flags && present != Some(true)) {
            continue;
        }
        let reset = reset_at.filter(|value| *value > 0).map(|value| value.saturating_mul(1000));
        if reset.is_some_and(|value| value <= now) {
            continue;
        }
        exhausted = true;
        reset_at_ms = match (reset_at_ms, reset) {
            (Some(current), Some(next)) => Some(current.max(next)),
            _ => None,
        };
    }
    Some(AccountQuotaCooldown {
        exhausted,
        reset_at_ms: if exhausted { reset_at_ms } else { None },
        updated_at_ms: account.usage_updated_at.unwrap_or_default().saturating_mul(1000),
    })
}

fn sync_runtime_quota_cooldowns(runtime: &mut GatewayRuntime, accounts: &[CodexAccount], now: i64) {
    let Some(collection) = runtime.collection.as_ref() else { return; };
    let allowed: HashSet<String> = effective_sidecar_account_ids(collection).into_iter().collect();
    runtime.account_quota_cooldowns.retain(|id, _| allowed.contains(id));
    for account in accounts.iter().filter(|account| allowed.contains(&account.id)) {
        if account.is_api_key_auth() {
            runtime.account_quota_cooldowns.remove(&account.id);
            continue;
        }
        // Missing/failed quota queries must not erase a previously confirmed
        // exhausted snapshot. New accounts with no snapshot remain unknown.
        let Some(next) = account_quota_cooldown(account, now) else { continue; };
        if runtime.account_quota_cooldowns.get(&account.id)
            .is_some_and(|current| current.updated_at_ms > next.updated_at_ms) {
            continue;
        }
        runtime.account_quota_cooldowns.insert(account.id.clone(), next);
    }
}

fn is_quota_cooldown_reason(reason: &str) -> bool {
    reason.trim().to_ascii_lowercase().contains("quota")
}

fn account_recovery_blocked_by_quota(runtime: &GatewayRuntime, account_id: &str, now: i64) -> bool {
    runtime.account_quota_cooldowns.get(account_id).is_some_and(|quota| quota.active(now))
        || runtime.model_cooldowns.iter().any(|(key, cooldown)| {
            key.starts_with(&format!("{}{}", account_id, COOLDOWN_KEY_SEPARATOR))
                && cooldown.next_retry_at_ms > now
                && is_quota_cooldown_reason(&cooldown.reason)
        })
        || runtime.account_health.get(account_id).is_some_and(|health| {
            sidecar_scheduler_blocks_account(Some(health), now)
                && health.sidecar_scheduler_reason.as_deref().is_some_and(is_quota_cooldown_reason)
        })
}

async fn account_quota_blocks_dispatch(account_id: &str, model: &str) -> bool {
    if model.trim().eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID) {
        return false;
    }
    let runtime = gateway_runtime().lock().await;
    runtime.account_quota_cooldowns.get(account_id).is_some_and(|quota| quota.active(now_ms()))
}
