#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NativeCooldownKey {
    account: String,
    scope: NativeCooldownScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum NativeCooldownScope {
    Model(String),
    Credential,
    IncludedQuota,
    ReserveQuota,
    UpstreamFailure,
}

#[derive(Clone, Debug)]
struct NativeCooldown {
    until: Instant,
    recorded_at: Instant,
    quota: bool,
    backoff_level: u8,
    non_quota_failures: u8,
    reason: &'static str,
}

fn native_cooldown_key(account: &str, model: &str, credential_scope: bool) -> NativeCooldownKey {
    NativeCooldownKey {
        account: account.into(),
        scope: if credential_scope || model.is_empty() {
            NativeCooldownScope::Credential
        } else {
            NativeCooldownScope::Model(model.into())
        },
    }
}

fn native_upstream_failure_key(account: &str) -> NativeCooldownKey {
    NativeCooldownKey {
        account: account.into(),
        scope: NativeCooldownScope::UpstreamFailure,
    }
}

const NATIVE_CONFIRMED_QUOTA: &str = "quota_exhausted";

fn native_oauth_quota_key(account: &str, model: &str) -> NativeCooldownKey {
    NativeCooldownKey {
        account: account.into(),
        scope: if native_is_reserve_model(model) {
            NativeCooldownScope::ReserveQuota
        } else {
            NativeCooldownScope::IncludedQuota
        },
    }
}

fn native_confirmed_quota_active(
    states: &HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    model: &str,
    now: Instant,
) -> bool {
    [native_cooldown_key(account, "", true), native_oauth_quota_key(account, model)]
        .iter().any(|key| states.get(key).is_some_and(|state| {
            state.reason == NATIVE_CONFIRMED_QUOTA && state.until > now
        }))
}

fn native_update_upstream_failure_streak(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    status: u16,
    now: Instant,
) {
    let key = native_upstream_failure_key(account);
    if !matches!(status, 408 | 500..=599) {
        // Quota and credential/model failures break the streak without
        // shortening an account pause that was already scheduled.
        if let Some(previous) = states.get_mut(&key).filter(|state| state.until > now) {
            previous.non_quota_failures = 0;
        } else {
            states.remove(&key);
        }
        return;
    }

    let state = states.entry(key).or_insert(NativeCooldown {
        until: now,
        recorded_at: now,
        quota: false,
        backoff_level: 0,
        non_quota_failures: 0,
        reason: "transient_upstream",
    });
    state.recorded_at = now;
    state.non_quota_failures = state.non_quota_failures.saturating_add(1);
    if state.non_quota_failures >= 3 {
        state.until = now + Duration::from_secs(600);
    }
}

impl NativeCooldown {
    fn active(&self, now: Instant, cooling_disabled: bool) -> bool {
        self.until > now && (!cooling_disabled || self.reason == NATIVE_CONFIRMED_QUOTA)
    }
}

fn native_mark_quota_exhausted(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    retry_after: Option<Duration>,
    now: Instant,
) {
    native_mark_quota_exhausted_at_key(states, native_cooldown_key(account, "", true), retry_after, now);
}

fn native_mark_quota_exhausted_at_key(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    key: NativeCooldownKey,
    retry_after: Option<Duration>,
    now: Instant,
) {
    native_update_upstream_failure_streak(states, &key.account, 429, now);
    if let Some(previous) = states.get_mut(&key)
        .filter(|state| state.reason == NATIVE_CONFIRMED_QUOTA && state.until > now)
    {
        previous.recorded_at = now;
        if let Some(until) = retry_after.and_then(|wait| now.checked_add(wait)) {
            previous.until = previous.until.max(until);
        }
        return;
    }
    native_write_cooldown(states, key.clone(), 429, retry_after, now);
    if let Some(state) = states.get_mut(&key) {
        state.reason = NATIVE_CONFIRMED_QUOTA;
    }
}

fn native_clear_quota_cooldowns(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account_id: &str,
    query_started: Instant,
    included_usable: bool,
    reserve_usable: bool,
) -> bool {
    let before = states.len();
    states.retain(|key, state| {
        let reserve = match &key.scope {
            NativeCooldownScope::ReserveQuota => true,
            NativeCooldownScope::Model(model) => native_is_reserve_model(model),
            _ => false,
        };
        let usable = if reserve { reserve_usable } else { included_usable };
        key.account != account_id || !state.quota || !usable || state.recorded_at >= query_started
    });
    states.len() != before
}

pub(crate) async fn clear_native_quota_cooldown_after_refresh(
    account_id: &str,
    quota: &CodexQuota,
    query_started: Instant,
) {
    let included_usable = quota_refresh_confirms_usable_allowance(quota, query_started, now_ms() / 1000);
    let reserve_usable = query_started.elapsed().as_secs() <= BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS as u64
        && quota.raw_data.as_ref().is_some_and(|raw| {
            quota_has_allowed_additional_rate_limit(raw, is_gpt_reserve_entitlement_name)
        });
    if !included_usable && !reserve_usable {
        return;
    }
    let host = gateway_runtime().lock().await.native_health_source.as_ref()
        .and_then(|host| host.upgrade());
    let Some(host) = host else { return; };
    let cleared = host.cooldowns.lock().ok().is_some_and(|mut states| {
        native_clear_quota_cooldowns(&mut states, account_id, query_started, included_usable, reserve_usable)
    });
    if cleared {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 额度刷新确认可用，已清除旧的额度冷却: account_id={account_id}"
        ));
        emit_local_access_state_updated();
    }
}

fn native_upsert_cooldown(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    model: &str,
    status: u16,
    retry_after: Option<Duration>,
    credential_scope: bool,
    now: Instant,
) {
    native_update_cooldown(states, account, model, status, retry_after, credential_scope, now, true);
}

fn native_update_cooldown(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    model: &str,
    status: u16,
    retry_after: Option<Duration>,
    credential_scope: bool,
    now: Instant,
    count_failure: bool,
) {
    if account.is_empty() || !matches!(status, 401 | 402 | 403 | 404 | 408 | 429 | 500..=599) {
        return;
    }
    if count_failure {
        native_update_upstream_failure_streak(states, account, status, now);
    }
    let key = native_cooldown_key(account, model, credential_scope || model.is_empty());
    native_write_cooldown(states, key, status, retry_after, now);
}

fn native_write_cooldown(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    key: NativeCooldownKey,
    status: u16,
    retry_after: Option<Duration>,
    now: Instant,
) {
    let previous = states.get(&key);
    if previous.is_some_and(|state| state.reason == NATIVE_CONFIRMED_QUOTA && state.until > now) {
        return;
    }
    let mut backoff_level = previous.map_or(0, |state| state.backoff_level);
    let (mut until, quota) = if status == 429 {
        let until = if let Some(hint) = retry_after {
            now.checked_add(hint.max(Duration::from_secs(10)))
                .unwrap_or_else(|| now + Duration::from_secs(10 * 365 * 86400))
        } else if let Some(current) = previous.filter(|state| state.quota && state.until > now) {
            current.until
        } else {
            let delay =
                Duration::from_secs(1u64 << backoff_level.min(11)).min(Duration::from_secs(1800));
            if delay < Duration::from_secs(1800) {
                backoff_level = backoff_level.saturating_add(1);
            }
            now + delay
        };
        (until, true)
    } else {
        let duration = match status {
            401 | 402 | 403 => Duration::from_secs(1800),
            404 => Duration::from_secs(12 * 3600),
            _ => Duration::from_secs(60),
        };
        (now + duration, false)
    };
    // A late transient failure must never shorten an established quota window.
    let retain_quota = previous.is_some_and(|state| state.quota && state.until > now);
    if let Some(current) = previous.filter(|state| state.quota && state.until > now) {
        until = until.max(current.until);
    }
    states.insert(
        key,
        NativeCooldown {
            until,
            recorded_at: now,
            quota: quota || retain_quota,
            backoff_level,
            non_quota_failures: 0,
            reason: if quota || retain_quota {
                "quota"
            } else {
                match status {
                    401 => "unauthorized",
                    402 => "payment_required",
                    403 => "forbidden",
                    404 => "model_not_available",
                    _ => "transient_upstream",
                }
            },
        },
    );
}

fn native_cooldown_wait(
    states: &HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    model: &str,
    disable_cooling: bool,
    now: Instant,
) -> Option<Duration> {
    [
        native_cooldown_key(account, model, false),
        native_cooldown_key(account, model, true),
        native_oauth_quota_key(account, model),
        native_upstream_failure_key(account),
    ]
    .into_iter()
    .filter_map(|key| states.get(&key))
    .filter(|state| state.active(now, disable_cooling))
    .map(|state| state.until.saturating_duration_since(now))
    .max()
}

fn native_upsert_transient_rate_limit(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>, account: &str, model: &str,
    retry_after: Option<Duration>, now: Instant,
) {
    native_update_upstream_failure_streak(states, account, 429, now);
    let key = native_cooldown_key(account, model, false);
    if states.get(&key).is_some_and(|state| state.quota && state.until > now) { return; }
    let until = now.checked_add(retry_after.unwrap_or(Duration::from_secs(1)))
        .unwrap_or_else(|| now + Duration::from_secs(10 * 365 * 86400));
    let until = states.get(&key).map_or(until, |state| state.until.max(until));
    states.insert(key, NativeCooldown { until, recorded_at: now, quota: false, backoff_level: 0, non_quota_failures: 0, reason: "transient_rate_limit" });
}

fn native_upsert_challenge_cooldown(
    states: &mut HashMap<NativeCooldownKey, NativeCooldown>,
    account: &str,
    model: &str,
    now: Instant,
) {
    if account.is_empty() {
        return;
    }
    let key = native_cooldown_key(account, model, false);
    let previous = states.get(&key);
    // Recording the final request after an attempted failover must not count
    // the same failure twice or shorten an existing quota/cooling window.
    if previous.is_some_and(|state| state.until > now) {
        return;
    }
    let level = previous.map_or(0, |state| state.backoff_level);
    let delay = Duration::from_secs(1u64 << level.min(11))
        .max(Duration::from_secs(10))
        .min(Duration::from_secs(1800));
    states.insert(
        key,
        NativeCooldown {
            until: now + delay,
            recorded_at: now,
            quota: false,
            non_quota_failures: 0,
            backoff_level: level.saturating_add(u8::from(delay < Duration::from_secs(1800))),
            reason: "cloudflare_challenge",
        },
    );
}

fn native_report_credential_scope(report: &native_contract::RequestReport) -> bool {
    report.credential_scope
        || matches!(
            report.error.as_deref(),
            Some(
                "usage_limit_reached"
                    | "insufficient_quota"
                    | "invalid_grant"
                    | "reauthorization_required"
            )
        )
}
