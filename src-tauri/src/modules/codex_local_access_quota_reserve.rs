// Shared, in-memory quota evidence and admission across native gateway instances.
// No disk or network work is performed while holding this coordinator's mutex.
use crate::modules::codex_native_api::rate_limits::{QuotaSnapshot, QuotaWindow};

#[derive(Clone)]
struct ObservedQuotaWindow {
    value: Option<QuotaWindow>,
    absent: bool,
    observed_at: Instant,
    request_started: Instant,
}

#[derive(Default)]
struct QuotaAccountRuntime {
    primary: Option<ObservedQuotaWindow>,
    secondary: Option<ObservedQuotaWindow>,
    in_flight: usize,
    dispatch_generation: u64,
    completed_at: Option<Instant>,
    awaiting_confirmation: bool,
    verified: bool,
    verified_at: Option<Instant>,
    verified_query_started_at: Option<Instant>,
}

#[derive(Default)]
struct QuotaReserveCoordinator {
    accounts: HashMap<String, QuotaAccountRuntime>,
    watched_account: Option<String>,
    watched_reserve: Option<CodexLocalAccessQuotaReserve>,
    strict_policy: Option<(String, CodexLocalAccessQuotaReserve)>,
}

static QUOTA_RESERVE_COORDINATOR: OnceLock<Mutex<QuotaReserveCoordinator>> = OnceLock::new();
static QUOTA_RESERVE_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

fn quota_refresh_failure_backoff(failures: u32, error: Option<&str>) -> Duration {
    let seconds = (5_u64 << failures.min(5)).min(120);
    let hint = error
        .and_then(|e| e.split("[retry_after_seconds:").nth(1))
        .and_then(|e| e.split(']').next())
        .and_then(|e| e.parse::<u64>().ok())
        .unwrap_or(0);
    Duration::from_secs(seconds.max(hint.min(3600)))
}

fn quota_reserve_coordinator() -> &'static Mutex<QuotaReserveCoordinator> {
    QUOTA_RESERVE_COORDINATOR.get_or_init(|| Mutex::new(QuotaReserveCoordinator::default()))
}

fn notify_quota_reserve_changed(account_id: &str) {
    if crate::get_app_handle().is_none()
        || QUOTA_RESERVE_NOTIFY_PENDING.swap(true, Ordering::AcqRel)
    {
        return;
    }
    if !quota_reserve_coordinator()
        .lock()
        .ok()
        .is_some_and(|runtime| runtime.watched_account.as_deref() == Some(account_id))
    {
        QUOTA_RESERVE_NOTIFY_PENDING.store(false, Ordering::Release);
        return;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        QUOTA_RESERVE_NOTIFY_PENDING.store(false, Ordering::Release);
        emit_local_access_state_updated();
    });
}

fn sync_quota_reserve_policy(collection: &CodexLocalAccessCollection) {
    let policy = collection
        .bound_oauth_quota_reserve
        .as_ref()
        .filter(|_| collection.enabled)
        .and_then(|reserve| {
            collection
                .bound_oauth_account_id
                .as_ref()
                .map(|id| (id.clone(), reserve.clone()))
        });
    if let Ok(mut runtime) = quota_reserve_coordinator().lock() {
        runtime.watched_account = collection
            .bound_oauth_quota_reserve
            .as_ref()
            .and(collection.bound_oauth_account_id.clone());
        runtime.watched_reserve = collection.bound_oauth_quota_reserve.clone();
        runtime.strict_policy = policy;
    }
}

fn quota_reserve_policy(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
) -> Option<CodexLocalAccessQuotaReserve> {
    if let Some(reserve) = quota_reserve_coordinator()
        .lock()
        .ok()?
        .strict_policy
        .as_ref()
        .filter(|(id, _)| id == account_id)
        .map(|(_, reserve)| reserve.clone())
    {
        return Some(reserve);
    }
    if collection.bound_oauth_account_id.as_deref() == Some(account_id) {
        if let Some(reserve) = &collection.bound_oauth_quota_reserve {
            return Some(reserve.clone());
        }
    }
    None
}

fn quota_reserve_cutoff(reserve: &CodexLocalAccessQuotaReserve, threshold: i32) -> f64 {
    f64::from((threshold + reserve.safety_margin_percent.clamp(1, 20)).min(100))
}

fn precise_quota_remaining(quota: &CodexQuota, primary: bool) -> Option<f64> {
    let normalized = valid_quota_remaining_percent(if primary {
        quota.hourly_percentage
    } else {
        quota.weekly_percentage
    })?;
    // The persisted display uses whole percentages; retain the original
    // server precision for admission whenever a usage response provided it.
    let used = quota
        .raw_data
        .as_ref()
        .and_then(|raw| raw.get("rate_limit"))
        .and_then(|limit| {
            limit.get(if primary {
                "primary_window"
            } else {
                "secondary_window"
            })
        })
        .and_then(|window| window.get("used_percent"))
        .and_then(Value::as_f64)
        .filter(|used| used.is_finite() && (0.0..=100.0).contains(used));
    Some(used.map_or(f64::from(normalized), |used| 100.0 - used))
}

fn window_from_account(account: &CodexAccount, primary: bool) -> Option<ObservedQuotaWindow> {
    let quota = fresh_quota_for_bound_oauth_reserve(account)?;
    let (remaining, present, minutes, reset) = if primary {
        (
            quota.hourly_percentage,
            quota.hourly_window_present,
            quota.hourly_window_minutes,
            quota.hourly_reset_time,
        )
    } else {
        (
            quota.weekly_percentage,
            quota.weekly_window_present,
            quota.weekly_window_minutes,
            quota.weekly_reset_time,
        )
    };
    let age = chrono::Utc::now()
        .timestamp()
        .saturating_sub(account.usage_updated_at?);
    if present != Some(false) && valid_quota_remaining_percent(remaining).is_none() {
        return None;
    }
    let at = Instant::now().checked_sub(Duration::from_secs(age.max(0) as u64))?;
    Some(ObservedQuotaWindow {
        value: precise_quota_remaining(quota, primary).map(|remaining| QuotaWindow {
            remaining_percent: remaining,
            window_minutes: minutes,
            resets_at: reset,
        }),
        absent: present == Some(false),
        observed_at: at,
        request_started: at,
    })
}

fn effective_quota_window(
    account: &CodexAccount,
    observed: Option<&ObservedQuotaWindow>,
    primary: bool,
) -> Option<ObservedQuotaWindow> {
    observed
        .filter(|window| window.observed_at.elapsed().as_secs() <= 180)
        .cloned()
        .or_else(|| window_from_account(account, primary))
}

fn quota_window_blocks(window: Option<&ObservedQuotaWindow>, cutoff: f64) -> bool {
    match window {
        Some(window) if window.absent => false,
        Some(window) => window
            .value
            .as_ref()
            .is_none_or(|value| value.remaining_percent <= cutoff),
        None => true,
    }
}

fn quota_windows_near(
    reserve: &CodexLocalAccessQuotaReserve,
    primary: Option<&ObservedQuotaWindow>,
    secondary: Option<&ObservedQuotaWindow>,
) -> bool {
    [
        (primary, reserve.hourly_percent),
        (secondary, reserve.weekly_percent),
    ]
    .into_iter()
    .any(|(window, cutoff)| {
        window
            .filter(|w| !w.absent)
            .and_then(|w| w.value.as_ref())
            .is_some_and(|w| {
                w.remaining_percent <= (quota_reserve_cutoff(reserve, cutoff) + 10.0).max(20.0)
            })
    })
}

fn quota_reserve_decision(
    reserve: &CodexLocalAccessQuotaReserve,
    account: &CodexAccount,
    runtime: Option<&QuotaAccountRuntime>,
) -> (
    bool,
    bool,
    Option<ObservedQuotaWindow>,
    Option<ObservedQuotaWindow>,
) {
    let primary = effective_quota_window(account, runtime.and_then(|r| r.primary.as_ref()), true);
    let secondary =
        effective_quota_window(account, runtime.and_then(|r| r.secondary.as_ref()), false);
    let blocked = !runtime.is_some_and(|state| state.verified)
        || account.quota_error.is_some()
        || quota_refresh_fail_closed_for_account(&account.id)
        || quota_window_blocks(
            primary.as_ref(),
            quota_reserve_cutoff(reserve, reserve.hourly_percent),
        )
        || quota_window_blocks(
            secondary.as_ref(),
            quota_reserve_cutoff(reserve, reserve.weekly_percent),
        );
    let near = quota_windows_near(reserve, primary.as_ref(), secondary.as_ref());
    (blocked, near, primary, secondary)
}

fn quota_reserve_allows(reserve: &CodexLocalAccessQuotaReserve, account: &CodexAccount) -> bool {
    let Ok(runtime) = quota_reserve_coordinator().lock() else {
        return false;
    };
    let state = runtime.accounts.get(&account.id);
    let (blocked, near, _, _) = quota_reserve_decision(reserve, account, state);
    !blocked && !(near && state.is_some_and(|s| s.in_flight > 0 || s.awaiting_confirmation))
}

fn merge_observed_window(
    current: &mut Option<ObservedQuotaWindow>,
    value: Option<QuotaWindow>,
    absent: bool,
    request_started: Instant,
    authoritative: bool,
) {
    if value.is_none() && !absent {
        return;
    }
    if let Some(previous) = current.as_ref() {
        if authoritative && previous.observed_at > request_started {
            return;
        }
        if let (Some(old), Some(new)) = (previous.value.as_ref(), value.as_ref()) {
            let older_window = old
                .resets_at
                .zip(new.resets_at)
                .is_some_and(|(old, new)| new < old);
            let newer_window = old
                .resets_at
                .zip(new.resets_at)
                .is_some_and(|(old, new)| new > old);
            let same_window = old.resets_at == new.resets_at;
            if older_window
                || (!authoritative && !newer_window && new.remaining_percent > old.remaining_percent)
                || (!authoritative && previous.request_started > request_started && !same_window)
            {
                return;
            }
        }
    }
    *current = Some(ObservedQuotaWindow {
        value,
        absent,
        observed_at: Instant::now(),
        request_started,
    });
}

fn observe_gateway_quota(account_id: &str, snapshot: QuotaSnapshot, request_started: Instant) {
    if let Ok(mut runtime) = quota_reserve_coordinator().lock() {
        if runtime.watched_account.as_deref() != Some(account_id)
            && !runtime.accounts.contains_key(account_id)
        {
            return;
        }
        // Bound memory even when public clients rotate through many accounts.
        if !runtime.accounts.contains_key(account_id) && runtime.accounts.len() >= 4096 {
            return;
        }
        let state = runtime.accounts.entry(account_id.to_string()).or_default();
        merge_observed_window(
            &mut state.primary,
            snapshot.primary,
            false,
            request_started,
            false,
        );
        merge_observed_window(
            &mut state.secondary,
            snapshot.secondary,
            false,
            request_started,
            false,
        );
    }
    notify_quota_reserve_changed(account_id);
}

pub(crate) fn confirm_gateway_quota_refresh(
    account_id: &str,
    quota: &CodexQuota,
    started: Instant,
) {
    let from = |primary: bool| {
        let (minutes, reset) = if primary {
            (quota.hourly_window_minutes, quota.hourly_reset_time)
        } else {
            (quota.weekly_window_minutes, quota.weekly_reset_time)
        };
        precise_quota_remaining(quota, primary).map(|v| QuotaWindow {
            remaining_percent: v,
            window_minutes: minutes,
            resets_at: reset,
        })
    };
    if let Ok(mut runtime) = quota_reserve_coordinator().lock() {
        if runtime.watched_account.as_deref() != Some(account_id)
            && !runtime.accounts.contains_key(account_id)
        {
            return;
        }
        let state = runtime.accounts.entry(account_id.to_string()).or_default();
        state.verified = true;
        state.verified_at = Some(Instant::now());
        state.verified_query_started_at = Some(
            state.verified_query_started_at.map_or(started, |previous| previous.max(started)),
        );
        merge_observed_window(
            &mut state.primary,
            from(true),
            quota.hourly_window_present == Some(false),
            started,
            true,
        );
        merge_observed_window(
            &mut state.secondary,
            from(false),
            quota.weekly_window_present == Some(false),
            started,
            true,
        );
        // A query started before completion is not evidence of settled consumption.
        if state
            .completed_at
            .is_none_or(|completed| started >= completed + Duration::from_secs(1))
        {
            state.awaiting_confirmation = false;
        }
    }
    if let Ok(mut failures) = bound_oauth_quota_refresh_failures().lock() {
        failures.remove(account_id);
    }
    notify_quota_reserve_changed(account_id);
}

// The Go admission gate must distinguish live queries from persisted display snapshots
// and reject queries that started before an in-flight request settled.
fn sidecar_quota_reserve_verified_query_started_ms(account_id: &str) -> Option<i64> {
    if quota_refresh_fail_closed_for_account(account_id) {
        return None;
    }
    let runtime = quota_reserve_coordinator().lock().ok()?;
    let state = runtime.accounts.get(account_id)?;
    let started = state.verified_query_started_at.filter(|_| state.verified)?;
    Some(chrono::Utc::now().timestamp_millis().saturating_sub(started.elapsed().as_millis() as i64))
}

fn note_gateway_quota_refresh_failure(account_id: &str, started: Instant) {
    // Subscription maintenance may still be running after a valid quota was
    // published. Do not misclassify that wait as missing quota evidence.
    let newer_verified = quota_reserve_coordinator()
        .lock()
        .ok()
        .is_some_and(|runtime| {
            runtime
                .accounts
                .get(account_id)
                .and_then(|state| state.verified_at)
                .is_some_and(|at| at >= started)
        });
    if newer_verified {
        return;
    }
    if let Ok(mut failures) = bound_oauth_quota_refresh_failures().lock() {
        failures.insert(account_id.to_string());
    }
    notify_quota_reserve_changed(account_id);
}

fn note_gateway_quota_request(account_id: &str) {
    if let Ok(mut runtime) = quota_reserve_coordinator().lock() {
        if let Some(state) = runtime.accounts.get_mut(account_id) {
            state.dispatch_generation = state.dispatch_generation.wrapping_add(1);
        }
    }
}

struct QuotaAdmissionPermit {
    account_id: String,
    dispatch_generation: u64,
}

impl Drop for QuotaAdmissionPermit {
    fn drop(&mut self) {
        let mut refresh = false;
        if let Ok(mut runtime) = quota_reserve_coordinator().lock() {
            if let Some(state) = runtime.accounts.get_mut(&self.account_id) {
                state.in_flight = state.in_flight.saturating_sub(1);
                if state.dispatch_generation != self.dispatch_generation {
                    state.completed_at = Some(Instant::now());
                    state.awaiting_confirmation = true;
                    refresh = true;
                }
            }
        }
        notify_quota_reserve_changed(&self.account_id);
        if refresh && crate::get_app_handle().is_some() {
            tauri::async_runtime::spawn(async {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                trigger_bound_oauth_quota_refresh_in_background(
                    "绑定账号请求结束或取消",
                    Duration::from_secs(2),
                );
            });
        }
    }
}

fn acquire_quota_admission(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Result<Option<Arc<dyn Send + Sync>>, native_contract::ServiceError> {
    let Some(reserve) = quota_reserve_policy(collection, &account.id) else {
        return Ok(None);
    };
    let mut runtime = quota_reserve_coordinator()
        .lock()
        .map_err(|_| native_error(503, "quota_unavailable", "Quota admission is unavailable"))?;
    let state = runtime.accounts.entry(account.id.clone()).or_default();
    // Keep the admission's disk snapshot available to the final send fence.
    if state.primary.is_none() {
        state.primary = window_from_account(account, true);
    }
    if state.secondary.is_none() {
        state.secondary = window_from_account(account, false);
    }
    let (blocked, near, _, _) = quota_reserve_decision(&reserve, account, Some(state));
    if blocked || (near && (state.in_flight > 0 || state.awaiting_confirmation)) {
        return Err(native_error(
            429,
            "quota_reserved",
            "OAuth quota is reserved or awaiting confirmation",
        )
        .with_retry_after(Some(Duration::from_secs(5))));
    }
    state.in_flight += 1;
    let permit = QuotaAdmissionPermit {
        account_id: account.id.clone(),
        dispatch_generation: state.dispatch_generation,
    };
    drop(runtime);
    notify_quota_reserve_changed(&account.id);
    Ok(Some(Arc::new(permit)))
}

fn check_gateway_quota_before_send(
    account_id: &str,
    global: bool,
    local: Option<&CodexLocalAccessQuotaReserve>,
) -> Result<(), native_contract::ServiceError> {
    let runtime = quota_reserve_coordinator()
        .lock()
        .map_err(|_| native_error(503, "quota_unavailable", "Quota admission is unavailable"))?;
    let policy = runtime
        .strict_policy
        .as_ref()
        .filter(|(id, _)| id == account_id)
        .map(|(_, p)| p)
        .or(local)
        .or_else(|| {
            (global && runtime.watched_account.as_deref() == Some(account_id))
                .then_some(runtime.watched_reserve.as_ref())
                .flatten()
        });
    let Some(reserve) = policy else {
        return Ok(());
    };
    let state = runtime.accounts.get(account_id);
    fn fresh(window: Option<&ObservedQuotaWindow>) -> Option<&ObservedQuotaWindow> {
        window.filter(|w| w.observed_at.elapsed().as_secs() <= 180)
    }
    let blocked = !state.is_some_and(|state| state.verified)
        || quota_refresh_fail_closed_for_account(account_id)
        || (quota_windows_near(
                reserve,
                fresh(state.and_then(|s| s.primary.as_ref())),
                fresh(state.and_then(|s| s.secondary.as_ref())),
            )
            && state.is_some_and(|s| s.awaiting_confirmation || s.in_flight > 1))
        || quota_window_blocks(
            fresh(state.and_then(|s| s.primary.as_ref())),
            quota_reserve_cutoff(reserve, reserve.hourly_percent),
        )
        || quota_window_blocks(
            fresh(state.and_then(|s| s.secondary.as_ref())),
            quota_reserve_cutoff(reserve, reserve.weekly_percent),
        );
    if blocked {
        return Err(
            native_error(429, "quota_reserved", "OAuth quota is reserved")
                .with_retry_after(Some(Duration::from_secs(5))),
        );
    }
    Ok(())
}

fn bound_oauth_dynamic_refresh_interval(account: Option<&CodexAccount>) -> Duration {
    let Some(account) = account else {
        return Duration::from_secs(5);
    };
    let Ok(runtime) = quota_reserve_coordinator().lock() else {
        return Duration::from_secs(5);
    };
    let state = runtime.accounts.get(&account.id);
    let windows = [
        effective_quota_window(account, state.and_then(|s| s.primary.as_ref()), true),
        effective_quota_window(account, state.and_then(|s| s.secondary.as_ref()), false),
    ];
    let remaining = windows
        .iter()
        .filter_map(|w| w.as_ref())
        .filter(|w| !w.absent)
        .filter_map(|w| w.value.as_ref())
        .map(|w| w.remaining_percent)
        .fold(100.0_f64, f64::min);
    if windows.iter().any(Option::is_none) {
        return Duration::from_secs(5);
    }
    Duration::from_millis((30_000.0 * remaining / 100.0).clamp(5_000.0, 30_000.0) as u64)
}

fn build_quota_reserve_status(
    collection: &CodexLocalAccessCollection,
) -> Option<CodexLocalAccessQuotaReserveStatus> {
    let reserve = collection.bound_oauth_quota_reserve.as_ref()?;
    let account_id = normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())?;
    let runtime = quota_reserve_coordinator().lock().ok()?;
    let state = runtime.accounts.get(&account_id);
    let fresh = |window: Option<&ObservedQuotaWindow>| {
        window.filter(|window| window.observed_at.elapsed().as_secs() <= 180).cloned()
    };
    let primary = fresh(state.and_then(|state| state.primary.as_ref()));
    let secondary = fresh(state.and_then(|state| state.secondary.as_ref()));
    let near = quota_windows_near(reserve, primary.as_ref(), secondary.as_ref());
    let pending = primary.is_none() || secondary.is_none()
        || state.is_none_or(|state| !state.verified || (near && state.awaiting_confirmation));
    let refresh_failed = quota_refresh_fail_closed_for_account(&account_id);
    let blocked = refresh_failed
        || quota_window_blocks(primary.as_ref(), quota_reserve_cutoff(reserve, reserve.hourly_percent))
        || quota_window_blocks(secondary.as_ref(), quota_reserve_cutoff(reserve, reserve.weekly_percent));
    let effective = [
        ("hourly", primary.as_ref(), reserve.hourly_percent),
        ("weekly", secondary.as_ref(), reserve.weekly_percent),
    ]
    .into_iter()
    .filter_map(|(name, window, threshold)| {
        let window = window.filter(|w| !w.absent)?.value.as_ref()?;
        (window.remaining_percent <= quota_reserve_warning_threshold(threshold) as f64
            || window.remaining_percent <= quota_reserve_cutoff(reserve, threshold) + 5.0)
            .then_some((name, window.remaining_percent, threshold))
    })
    .min_by(|a, b| a.1.total_cmp(&b.1));
    let age = [primary.as_ref(), secondary.as_ref()]
        .into_iter()
        .flatten()
        .map(|w| w.observed_at.elapsed().as_secs() as i64)
        .max();
    Some(CodexLocalAccessQuotaReserveStatus {
        account_id,
        snapshot_updated_at: age.map(|age| chrono::Utc::now().timestamp() - age),
        snapshot_fresh: primary.is_some()
            && secondary.is_some()
            && !refresh_failed,
        blocked: blocked || pending,
        warning: effective.is_some() || pending,
        effective_window: effective.map(|e| e.0.to_string()),
        effective_remaining_percent: effective.map(|e| e.1),
        effective_reserve_percent: effective.map(|e| e.2),
        strict: true,
        safety_margin_percent: reserve.safety_margin_percent.clamp(1, 20),
        in_flight: state.map_or(0, |s| s.in_flight),
        awaiting_confirmation: pending,
    })
}
