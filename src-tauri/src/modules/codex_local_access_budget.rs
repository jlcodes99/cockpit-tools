use crate::models::codex_local_access::{
    CodexApiKeyBudget, CodexApiKeyBudgetUsage, CodexBudgetBucket, CodexBudgetPeriod,
};

struct KeyConcurrencyGuard(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for KeyConcurrencyGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn acquire_key_concurrency(
    key: &CodexLocalAccessApiKey,
) -> Result<Arc<dyn Send + Sync>, native_contract::ServiceError> {
    static COUNTERS: std::sync::LazyLock<
        Mutex<HashMap<String, std::sync::Weak<std::sync::atomic::AtomicUsize>>>,
    > = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let counter = {
        let mut counters = COUNTERS
            .lock()
            .map_err(|_| native_error(503, "service_unavailable", "Key admission unavailable"))?;
        if counters.len() > 4096 {
            counters.retain(|_, value| value.strong_count() > 0);
        }
        let counter = counters
            .get(&key.id)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        counters.insert(key.id.clone(), Arc::downgrade(&counter));
        counter
    };
    let limit = key
        .budget
        .max_concurrent
        .map(|limit| limit as usize)
        .unwrap_or(usize::MAX);
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < limit).then_some(count + 1)
        })
        .map_err(|_| {
            native_error(
                429,
                "key_concurrency_reached",
                "API key concurrent request limit reached",
            )
        })?;
    Ok(Arc::new(KeyConcurrencyGuard(counter)))
}

fn budget_period_bounds(period: CodexBudgetPeriod, now: i64) -> (i64, Option<i64>) {
    let date = chrono::Utc
        .timestamp_millis_opt(now)
        .single()
        .expect("valid clock")
        .date_naive();
    let midnight = |date: chrono::NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    };
    match period {
        CodexBudgetPeriod::Lifetime => (0, None),
        CodexBudgetPeriod::Daily => (midnight(date), Some(midnight(date) + 86_400_000)),
        CodexBudgetPeriod::Weekly => {
            let start =
                midnight(date) - i64::from(date.weekday().num_days_from_monday()) * 86_400_000;
            (start, Some(start + 7 * 86_400_000))
        }
        CodexBudgetPeriod::Monthly => {
            let start = date.with_day(1).unwrap();
            let next = start.checked_add_months(chrono::Months::new(1)).unwrap();
            (midnight(start), Some(midnight(next)))
        }
    }
}

fn budget_bucket_used(bucket: &CodexBudgetBucket, period: CodexBudgetPeriod, now: i64) -> u64 {
    if bucket.start == budget_period_bounds(period, now).0 {
        bucket.used_microusd
    } else {
        0
    }
}

fn budget_used(key: &CodexLocalAccessApiKey, now: i64) -> u64 {
    match key.budget.period {
        CodexBudgetPeriod::Lifetime => key.budget_usage.lifetime_microusd,
        CodexBudgetPeriod::Daily => {
            budget_bucket_used(&key.budget_usage.daily, CodexBudgetPeriod::Daily, now)
        }
        CodexBudgetPeriod::Weekly => {
            budget_bucket_used(&key.budget_usage.weekly, CodexBudgetPeriod::Weekly, now)
        }
        CodexBudgetPeriod::Monthly => {
            budget_bucket_used(&key.budget_usage.monthly, CodexBudgetPeriod::Monthly, now)
        }
    }
}

fn validate_key_budget(budget: &CodexApiKeyBudget) -> Result<(), String> {
    if [budget.limit_microusd, budget.daily_limit_microusd]
        .into_iter()
        .flatten()
        .any(|value| value == 0 || value > 1_000_000_000_000_000)
        || budget.max_concurrent.is_some_and(|n| n == 0 || n > 1024)
        || budget.daily_image_limit == Some(0)
    {
        return Err("CODEX_KEY_BUDGET_INVALID".into());
    }
    Ok(())
}

fn check_key_budget(
    key: &CodexLocalAccessApiKey,
    now: i64,
    image: bool,
) -> Result<(), native_contract::ServiceError> {
    let exhausted = key
        .budget
        .limit_microusd
        .is_some_and(|limit| budget_used(key, now) >= limit)
        || key.budget.daily_limit_microusd.is_some_and(|limit| {
            budget_bucket_used(&key.budget_usage.daily, CodexBudgetPeriod::Daily, now) >= limit
        });
    if exhausted {
        return Err(native_error(
            429,
            "budget_limit_reached",
            "API key usage budget reached",
        ));
    }
    if image
        && key.budget.daily_image_limit.is_some_and(|limit| {
            key.budget_usage.daily.start == budget_period_bounds(CodexBudgetPeriod::Daily, now).0
                && key.budget_usage.daily.image_requests >= u64::from(limit)
        })
    {
        return Err(native_error(
            429,
            "image_limit_reached",
            "API key daily image limit reached",
        ));
    }
    Ok(())
}

fn check_budget_pricing(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    model: &str,
) -> Result<(), native_contract::ServiceError> {
    if key.is_some_and(|key| {
        key.budget.limit_microusd.is_some() || key.budget.daily_limit_microusd.is_some()
    }) && resolve_effective_model_pricing(Some(collection), Some(model), None, None).is_none()
    {
        return Err(native_error(
            403,
            "budget_pricing_unavailable",
            "This model has no configured price for budget accounting",
        ));
    }
    Ok(())
}

fn record_key_budget_usage(
    key: &mut CodexLocalAccessApiKey,
    now: i64,
    cost: f64,
    metered: bool,
    image: bool,
) {
    let amount = if cost.is_finite() && cost > 0.0 {
        (cost * 1_000_000.0).ceil() as u64
    } else {
        0
    };
    key.budget_usage.lifetime_microusd = key.budget_usage.lifetime_microusd.saturating_add(amount);
    if !metered {
        key.budget_usage.unmetered_requests = key.budget_usage.unmetered_requests.saturating_add(1);
    }
    for (period, bucket) in [
        (CodexBudgetPeriod::Daily, &mut key.budget_usage.daily),
        (CodexBudgetPeriod::Weekly, &mut key.budget_usage.weekly),
        (CodexBudgetPeriod::Monthly, &mut key.budget_usage.monthly),
    ] {
        let start = budget_period_bounds(period, now).0;
        if bucket.start < start {
            *bucket = CodexBudgetBucket {
                start,
                ..Default::default()
            };
        }
        // A backward clock change must not overwrite a newer accounting bucket.
        if bucket.start == start {
            bucket.used_microusd = bucket.used_microusd.saturating_add(amount);
            bucket.image_requests = bucket.image_requests.saturating_add(u64::from(image));
        }
    }
}

fn merge_key_budget_usage(target: &mut CodexApiKeyBudgetUsage, current: &CodexApiKeyBudgetUsage) {
    target.lifetime_microusd = target.lifetime_microusd.max(current.lifetime_microusd);
    target.unmetered_requests = target.unmetered_requests.max(current.unmetered_requests);
    for (target, current) in [
        (&mut target.daily, &current.daily),
        (&mut target.weekly, &current.weekly),
        (&mut target.monthly, &current.monthly),
    ] {
        if current.start > target.start {
            *target = current.clone();
        } else if current.start == target.start {
            target.used_microusd = target.used_microusd.max(current.used_microusd);
            target.image_requests = target.image_requests.max(current.image_requests);
        }
    }
}

fn key_budget_snapshot(key: &CodexLocalAccessApiKey, now: i64) -> Value {
    let used = budget_used(key, now);
    let daily = budget_bucket_used(&key.budget_usage.daily, CodexBudgetPeriod::Daily, now);
    json!({
        "policy":key.budget,"usedMicrousd":used,
        "remainingMicrousd":key.budget.limit_microusd.map(|limit| limit.saturating_sub(used)),
        "dailyUsedMicrousd":daily,
        "dailyRemainingMicrousd":key.budget.daily_limit_microusd.map(|limit| limit.saturating_sub(daily)),
        "resetsAt":budget_period_bounds(key.budget.period, now).1,
        "lifetimeMicrousd":key.budget_usage.lifetime_microusd,
        "unmeteredRequests":key.budget_usage.unmetered_requests,
        "dailyImageRequests":if key.budget_usage.daily.start == budget_period_bounds(CodexBudgetPeriod::Daily, now).0 {key.budget_usage.daily.image_requests} else {0},
        "timezone":"UTC", "inFlightMayExceed":true,
    })
}

#[cfg(test)]
mod key_budget_tests {
    use super::*;

    fn key() -> CodexLocalAccessApiKey {
        serde_json::from_value(json!({"id":format!("budget-test-{}", uuid::Uuid::new_v4()),"label":"test","key":"fixture"})).unwrap()
    }

    #[test]
    fn budget_settlement_survives_serialization_and_resets_without_carrying() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        let mut key = key();
        key.budget.period = CodexBudgetPeriod::Daily;
        key.budget.limit_microusd = Some(1_000_000);
        record_key_budget_usage(&mut key, now, 0.6, true, true);
        assert_eq!(budget_used(&key, now), 600_000);
        record_key_budget_usage(&mut key, now, 0.5, true, false);
        assert_eq!(
            check_key_budget(&key, now, false).unwrap_err().code,
            "budget_limit_reached"
        );
        let mut restarted: CodexLocalAccessApiKey =
            serde_json::from_value(serde_json::to_value(key).unwrap()).unwrap();
        assert!(check_key_budget(&restarted, now + 86_400_000, false).is_ok());
        assert_eq!(budget_used(&restarted, now + 86_400_000), 0);
        record_key_budget_usage(&mut restarted, now + 86_400_000, 0.25, true, false);
        assert_eq!(budget_used(&restarted, now + 86_400_000), 250_000);
        assert_eq!(restarted.budget_usage.lifetime_microusd, 1_350_000);
    }

    #[test]
    fn concurrency_is_key_scoped_and_releases_after_cancel() {
        let mut first = key();
        first.budget.max_concurrent = Some(1);
        let second = key();
        let permit = acquire_key_concurrency(&first).unwrap();
        assert_eq!(
            acquire_key_concurrency(&first).err().unwrap().code,
            "key_concurrency_reached"
        );
        assert!(acquire_key_concurrency(&second).is_ok());
        drop(permit);
        assert!(acquire_key_concurrency(&first).is_ok());
    }

    #[test]
    fn daily_image_cap_and_unknown_metering_are_independent_of_text() {
        let mut key = key();
        key.budget.daily_image_limit = Some(1);
        let now = now_ms();
        record_key_budget_usage(&mut key, now, 0.0, false, true);
        assert!(check_key_budget(&key, now, false).is_ok());
        assert_eq!(
            check_key_budget(&key, now, true).unwrap_err().code,
            "image_limit_reached"
        );
        assert_eq!(key_budget_snapshot(&key, now)["unmeteredRequests"], 1);
        let mut invalid = CodexApiKeyBudget::default();
        invalid.limit_microusd = Some(0);
        assert!(validate_key_budget(&invalid).is_err());
    }

    #[test]
    fn utc_periods_and_month_year_rollover() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-12-31T23:59:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            budget_period_bounds(CodexBudgetPeriod::Monthly, now).1,
            Some(
                chrono::DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        let (start, end) = budget_period_bounds(CodexBudgetPeriod::Weekly, now);
        assert_eq!(
            chrono::Utc.timestamp_millis_opt(start).unwrap().weekday(),
            chrono::Weekday::Mon
        );
        assert_eq!(end.unwrap() - start, 7 * 86_400_000);
    }

    #[test]
    fn stale_snapshots_cannot_refund_charges_or_restore_old_period() {
        let mut old = CodexApiKeyBudgetUsage::default();
        let current = CodexApiKeyBudgetUsage {
            lifetime_microusd: 50,
            daily: CodexBudgetBucket {
                start: 100,
                used_microusd: 30,
                image_requests: 2,
            },
            ..Default::default()
        };
        merge_key_budget_usage(&mut old, &current);
        merge_key_budget_usage(&mut old, &CodexApiKeyBudgetUsage::default());
        assert_eq!(old, current);
    }
}
