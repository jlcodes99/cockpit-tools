// Project the active scheduler into both API-service state views. Never keep
// another cooldown clock: expiry, reset and disabled cooling use the same table
// as routing. The gateway runtime lock is held by callers, so this must not wait
// for the scheduler or do disk/network work.
fn apply_native_health_snapshot(
    runtime: &GatewayRuntime,
    snapshot: &mut [CodexLocalAccessAccountHealth],
    now_ms: i64,
) {
    if !runtime.running
        || !runtime.collection.as_ref().is_some_and(|collection| {
            collection_gateway_mode(collection) == CodexLocalAccessGatewayMode::Native
        })
    {
        return;
    }
    let Some(host) = runtime
        .native_health_source
        .as_ref()
        .and_then(|host| host.upgrade())
    else {
        return;
    };
    let states = host.cooldowns.try_lock();
    let now = Instant::now();
    let readable = states.is_ok();
    let mut current: HashMap<String, Vec<CodexLocalAccessAccountCooldown>> = HashMap::new();
    if let Ok(states) = &states {
        let scope: HashSet<_> = snapshot
            .iter()
            .map(|item| item.account_id.as_str())
            .collect();
        if !host.cooling_disabled.load(Ordering::Acquire) {
            for (key, state) in states
                .iter()
                .filter(|(key, state)| scope.contains(key.account.as_str()) && state.until > now)
            {
                let remaining_ms = state
                    .until
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(i64::MAX as u128) as i64;
                current.entry(key.account.clone()).or_default().push(
                    CodexLocalAccessAccountCooldown {
                        model_id: if key.model.is_empty() {
                            "*".into()
                        } else {
                            key.model.clone()
                        },
                        next_retry_at: now_ms.saturating_add(remaining_ms),
                        remaining_ms,
                        reason: state.reason.into(),
                    },
                );
            }
        }
    }
    drop(states);
    for health in snapshot {
        // Old Go scheduler state must not describe the currently running Rust engine.
        health
            .cooldowns
            .retain(|cooldown| cooldown.reason == "quota_exhausted");
        health.scheduler_available = None;
        health.scheduler_reason = None;
        health.scheduler_next_retry_at = None;
        if !readable {
            // A concurrent mutation is transient, not proof of availability.
            health.available = false;
            health.scheduler_reason = Some("state_refreshing".into());
            continue;
        }
        health
            .cooldowns
            .extend(current.remove(&health.account_id).unwrap_or_default());
        health.cooldowns.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        health.available = health.cooldowns.is_empty()
            && runtime
                .account_quota_cooldowns
                .get(&health.account_id)
                .is_none_or(|quota| !quota.active(now_ms))
            && !account_health_blocks_routing(runtime.account_health.get(&health.account_id));
        if let Some(cooldown) = health
            .cooldowns
            .iter()
            .max_by_key(|item| item.next_retry_at)
        {
            health.scheduler_available = Some(false);
            health.scheduler_reason = Some(cooldown.reason.clone());
            health.scheduler_next_retry_at = Some(cooldown.next_retry_at);
        }
    }
}
