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

fn json_finite_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64().filter(|value| value.is_finite()),
        serde_json::Value::String(raw) => raw.trim().parse::<f64>().ok().filter(|value| value.is_finite()),
        _ => None,
    }
}

fn json_bool(value: Option<&serde_json::Value>) -> Option<bool> {
    match value? {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::String(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        serde_json::Value::Number(number) => number.as_i64().map(|value| value != 0),
        _ => None,
    }
}

fn object_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    object.get(key).or_else(|| {
        object.iter().find_map(|(name, value)| {
            name.eq_ignore_ascii_case(key).then_some(value)
        })
    })
}

fn quota_has_usable_workspace_credits(
    raw: &serde_json::Map<String, serde_json::Value>,
    credits: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    if !["remaining", "balance"]
        .iter()
        .all(|field| object_field(credits, field).is_none_or(serde_json::Value::is_null))
    {
        return false;
    }
    json_bool(object_field(credits, "has_credits")).unwrap_or_else(|| {
        raw.get("ambient_usage")
            .and_then(|usage| usage.pointer("/default/source/kind"))
            .and_then(serde_json::Value::as_str)
            == Some("credits")
    })
}

fn quota_has_usable_credits(quota: &CodexQuota) -> bool {
    let Some(raw) = quota.raw_data.as_ref().and_then(serde_json::Value::as_object) else {
        return false;
    };
    let workspace = matches!(
        object_field(raw, "plan_type").and_then(serde_json::Value::as_str),
        Some("team" | "business")
    );
    let spend = object_field(raw, "spend_control").and_then(serde_json::Value::as_object);
    let credits = object_field(raw, "credits").and_then(serde_json::Value::as_object);
    if workspace
        && (json_bool(spend.and_then(|spend| object_field(spend, "reached"))) == Some(true)
            || json_bool(credits.and_then(|credits| object_field(credits, "overage_limit_reached")))
                == Some(true))
    {
        return false;
    }
    if let Some(limit) = spend
        .and_then(|spend| object_field(spend, "individual_limit"))
        .and_then(serde_json::Value::as_object)
    {
        if json_bool(object_field(limit, "unlimited")).unwrap_or(false) {
            return true;
        }
        if json_finite_f64(object_field(limit, "remaining")).is_some_and(|value| value > 0.0) {
            return true;
        }
        if json_finite_f64(object_field(limit, "remaining_percent")).is_some_and(|value| value > 0.0)
        {
            return true;
        }
        if let (Some(total), Some(used)) = (
            json_finite_f64(object_field(limit, "limit")),
            json_finite_f64(object_field(limit, "used")),
        ) {
            if (total - used) > 0.0 {
                return true;
            }
        }
    }
    let Some(credits) = credits else {
        return false;
    };
    if json_bool(object_field(credits, "unlimited")).unwrap_or(false) {
        return true;
    }
    json_finite_f64(object_field(credits, "remaining")).is_some_and(|value| value > 0.0)
        || json_finite_f64(object_field(credits, "balance")).is_some_and(|value| value > 0.0)
        || (workspace && quota_has_usable_workspace_credits(raw, credits))
}

fn account_quota_cooldown(account: &CodexAccount, now: i64) -> Option<AccountQuotaCooldown> {
    account_quota_cooldown_at_floor(account, now, 0)
}

fn prune_account_quota_floors(collection: &mut CodexLocalAccessCollection) -> bool {
    let len = collection.account_quota_floors.len();
    collection.account_quota_floors.retain(|id, floor| {
        collection.account_ids.contains(id) && (1..=100).contains(floor)
    });
    len != collection.account_quota_floors.len()
}

fn prune_account_concurrency_limits(collection: &mut CodexLocalAccessCollection) -> bool {
    let mut changed = false;
    let len = collection.account_concurrency_limits.len();
    collection
        .account_concurrency_limits
        .retain(|id, _| collection.account_ids.contains(id));
    if collection.account_concurrency_limits.len() != len {
        changed = true;
    }
    for limit in collection.account_concurrency_limits.values_mut() {
        let clamped = (*limit).min(MAX_ACCOUNT_CONCURRENCY_LIMIT);
        if clamped != *limit {
            *limit = clamped;
            changed = true;
        }
    }
    changed
}

fn effective_account_concurrency_limit(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
) -> u16 {
    collection
        .account_concurrency_limits
        .get(account_id)
        .copied()
        .unwrap_or(collection.max_account_concurrency)
        .min(MAX_ACCOUNT_CONCURRENCY_LIMIT)
}

fn persist_account_concurrency_limit(
    path: &Path,
    account_id: &str,
    max_concurrency: Option<u16>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut collection: CodexLocalAccessCollection = serde_json::from_str(
            current.ok_or("本地接入集合尚未创建")?,
        ).map_err(|error| error.to_string())?;
        if !collection.account_ids.iter().any(|id| id == account_id) {
            return Err("CODEX_LOCAL_ACCESS_CONCURRENCY_LIMIT_ACCOUNT_UNAVAILABLE".into());
        }
        if let Some(limit) = max_concurrency {
            collection
                .account_concurrency_limits
                .insert(account_id.to_owned(), limit.min(MAX_ACCOUNT_CONCURRENCY_LIMIT));
        } else {
            collection.account_concurrency_limits.remove(account_id);
        }
        collection.updated_at = collection.updated_at.max(now_ms());
        let content = serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())?;
        published = Some(collection);
        Ok(content)
    })?;
    published.ok_or_else(|| "CODEX_LOCAL_ACCESS_CONCURRENCY_LIMIT_STORAGE".into())
}

fn persist_account_quota_floor(
    path: &Path,
    account_id: &str,
    minimum_remaining_percent: Option<i32>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut collection: CodexLocalAccessCollection = serde_json::from_str(
            current.ok_or("本地接入集合尚未创建")?,
        ).map_err(|error| error.to_string())?;
        if !collection.account_ids.iter().any(|id| id == account_id)
            || (minimum_remaining_percent.is_some() && collection.official_account_bindings.contains_key(account_id))
        {
            return Err("CODEX_LOCAL_ACCESS_QUOTA_FLOOR_ACCOUNT_UNAVAILABLE".into());
        }
        if let Some(floor) = minimum_remaining_percent {
            collection.account_quota_floors.insert(account_id.to_owned(), floor);
        } else {
            collection.account_quota_floors.remove(account_id);
        }
        collection.updated_at = collection.updated_at.max(now_ms());
        let content = serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())?;
        published = Some(collection);
        Ok(content)
    })?;
    published.ok_or_else(|| "CODEX_LOCAL_ACCESS_QUOTA_FLOOR_STORAGE".into())
}

fn configured_pool_quota_floor(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Option<i32> {
    collection.account_quota_floors.get(&account.id).copied()
        .filter(|floor| (1..=100).contains(floor))
        .filter(|_| !account.is_api_key_auth() && !account.is_agent_identity_auth() && !account.is_web_session_auth()
            && !collection.official_account_bindings.contains_key(&account.id))
}

fn pool_account_quota_cooldown(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    now: i64,
) -> Option<AccountQuotaCooldown> {
    if let Some(floor) = configured_pool_quota_floor(collection, account) {
        account_quota_cooldown_at_floor(account, now, floor)
    } else {
        account_quota_cooldown(account, now)
    }
}

fn account_quota_cooldown_at_floor(account: &CodexAccount, now: i64, floor: i32) -> Option<AccountQuotaCooldown> {
    if account.is_api_key_auth() {
        return None;
    }
    let quota = account.quota.as_ref()?;
    let updated_at_ms = account.usage_updated_at.unwrap_or_default().saturating_mul(1000);
    if floor == 0 && quota_has_usable_credits(quota) {
        return Some(AccountQuotaCooldown {
            exhausted: false,
            reset_at_ms: None,
            updated_at_ms,
        });
    }
    let has_presence_flags =
        quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
    let windows = [
        (quota.hourly_percentage, quota.hourly_window_present, quota.hourly_reset_time),
        (quota.weekly_percentage, quota.weekly_window_present, quota.weekly_reset_time),
    ];
    let mut exhausted = false;
    let mut reset_at_ms = Some(0_i64);
    for (remaining, present, reset_at) in windows {
        if !(0..=floor).contains(&remaining) || (has_presence_flags && present != Some(true)) {
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
        updated_at_ms,
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
        let Some(next) = pool_account_quota_cooldown(collection, account, now) else { continue; };
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

fn mark_quota_cooldowns_recovered(runtime: &mut GatewayRuntime, account_ids: &[String], now: i64) {
    for account_id in account_ids
        .iter()
        .map(|account_id| account_id.trim())
        .filter(|account_id| !account_id.is_empty())
    {
        runtime.account_quota_cooldowns.insert(
            account_id.to_string(),
            AccountQuotaCooldown {
                exhausted: false,
                reset_at_ms: None,
                updated_at_ms: now,
            },
        );
    }
}

fn clear_runtime_quota_cooldowns(runtime: &mut GatewayRuntime, account_ids: &[String]) {
    for account_id in account_ids
        .iter()
        .map(|account_id| account_id.trim())
        .filter(|account_id| !account_id.is_empty())
    {
        runtime.account_quota_cooldowns.remove(account_id);
    }
}

async fn account_quota_blocks_dispatch(account_id: &str, model: &str) -> bool {
    if model.trim().eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID) {
        return false;
    }
    let runtime = gateway_runtime().lock().await;
    runtime.account_quota_cooldowns.get(account_id).is_some_and(|quota| quota.active(now_ms()))
}
