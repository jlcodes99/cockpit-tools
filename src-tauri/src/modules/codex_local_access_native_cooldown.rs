// Per-model retry state mirrors the active Go scheduler without sharing its implementation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NativeCooldownKey {
    account: String,
    model: String,
}

#[derive(Clone, Debug)]
struct NativeCooldown {
    until: Instant,
    quota: bool,
    backoff_level: u8,
    reason: &'static str,
}

fn native_cooldown_key(account: &str, model: &str, credential_scope: bool) -> NativeCooldownKey {
    NativeCooldownKey {
        account: account.into(),
        model: if credential_scope { "" } else { model }.into(),
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
    if account.is_empty() || !matches!(status, 401 | 402 | 403 | 404 | 408 | 429 | 500..=599) {
        return;
    }
    let key = native_cooldown_key(account, model, credential_scope || model.is_empty());
    let previous = states.get(&key);
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
            quota: quota || retain_quota,
            backoff_level,
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
    ]
    .into_iter()
    .filter_map(|key| states.get(&key))
    .filter(|state| state.until > now && !disable_cooling)
    .map(|state| state.until.saturating_duration_since(now))
    .max()
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
            quota: false,
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
