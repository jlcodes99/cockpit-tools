//! Credential-route health never changes the user's selected service mode.
use super::{binding, status, storage, types::Registration};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, time::Duration};

#[derive(Clone, Debug)]
pub(crate) struct OfficialRouteSnapshot {
    pub registration_id: String,
    pub source_account_id: String,
    pub authorization_revision: String,
    pub source_fingerprint: String,
    pub available: bool,
    pub failure_code: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Health {
    authorization_revision: String,
    source_fingerprint: String,
    states: BTreeMap<String, Failure>,
}

#[derive(Serialize, Deserialize)]
struct Failure {
    code: String,
    paused: bool,
    retry_at_ms: i64,
    updated_at_ms: i64,
    failures: u8,
}

fn read(record: &Registration) -> Result<Health, String> {
    let path = storage::official_health_path(&record.summary.id)?;
    fn damaged() -> Health {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::modules::logger::log_warn("[ChatGPT Plan] Ignoring invalid derived official health; credentials and bindings are preserved");
        }
        Health::default()
    }
    let content = (|| {
        let mut raw = String::new();
        std::fs::File::open(path)?
            .take(256 * 1024 + 1)
            .read_to_string(&mut raw)?;
        Ok::<_, std::io::Error>(raw)
    })();
    let mut health = match content {
        Ok(raw) if raw.len() > 256 * 1024 => return Ok(damaged()),
        Ok(raw) => match serde_json::from_str::<Health>(&raw) {
            Ok(health) => health,
            Err(_) => return Ok(damaged()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Health::default()),
        Err(_) => return Ok(damaged()),
    };
    clear_request_capability_failures(&mut health);
    let fingerprint = record
        .source_binding
        .as_ref()
        .map(|binding| binding.identity_fingerprint.as_str());
    Ok(
        if health.authorization_revision == record.authorization_revision
            && Some(health.source_fingerprint.as_str()) == fingerprint
        {
            health
        } else {
            Health::default()
        },
    )
}

fn snapshot(
    record: &Registration,
    source: &str,
    model: &str,
    now_ms: i64,
) -> Result<OfficialRouteSnapshot, String> {
    binding::validate_record_source(record, source)?;
    if !record.summary.connected {
        return Err("CHATGPT_PLAN_DISCONNECTED".into());
    }
    let binding = record
        .source_binding
        .as_ref()
        .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
    let health = read(record)?;
    let blocked = health
        .states
        .get("")
        .into_iter()
        .chain(health.states.get(model))
        .find(|state| state.paused || state.retry_at_ms > now_ms);
    Ok(OfficialRouteSnapshot {
        registration_id: record.summary.id.clone(),
        source_account_id: source.into(),
        authorization_revision: record.authorization_revision.clone(),
        source_fingerprint: binding.identity_fingerprint.clone(),
        available: !record.summary.needs_reauth && blocked.is_none(),
        failure_code: if record.summary.needs_reauth {
            Some("chatgpt_plan_reauthorization_required".into())
        } else {
            blocked.map(|state| state.code.clone())
        },
    })
}

pub(crate) async fn official_route_snapshot(
    id: &str,
    source: &str,
    model: &str,
) -> Result<OfficialRouteSnapshot, String> {
    let (id, source, model) = (id.to_owned(), source.to_owned(), model.to_owned());
    storage::blocking(move || {
        snapshot(
            &storage::load(&id)?,
            &source,
            &model,
            chrono::Utc::now().timestamp_millis(),
        )
    })
    .await
}

pub(crate) async fn fallback_epoch_matches(id: &str, source: &str, revision: &str) -> bool {
    let (id, source, revision) = (id.to_owned(), source.to_owned(), revision.to_owned());
    storage::blocking(move || {
        let record = storage::load(&id)?;
        binding::validate_record_source(&record, &source)?;
        Ok(record.summary.connected && record.authorization_revision == revision)
    })
    .await
    .unwrap_or(false)
}

fn save(record: &Registration, health: &Health) -> Result<(), String> {
    let raw = serde_json::to_string(health).map_err(|_| "CHATGPT_PLAN_STORAGE")?;
    crate::modules::atomic_write::write_secret_string_atomic(
        &storage::official_health_path(&record.summary.id)?,
        &raw,
    )
    .map_err(|_| "CHATGPT_PLAN_STORAGE".into())
}

fn request_capability_failure(code: &str) -> bool {
    matches!(
        code,
        "capability_unsupported" | "subscription_sharing_unsupported_capability"
    )
}

fn clear_request_capability_failures(health: &mut Health) {
    // Older builds persisted request-specific rejects as model-wide pauses.
    // Ignore only those derived states; real eligibility/model pauses survive.
    health
        .states
        .retain(|_, state| !request_capability_failure(&state.code));
}

fn quota_exhaustion(code: &str) -> bool {
    matches!(
        code,
        "subscription_sharing_usage_limit_exceeded"
            | "usage_limit_reached"
            | "insufficient_quota"
            | "quota_exceeded"
    )
}

fn failure_delay(quota: bool, failures: u8, retry_after: Option<Duration>) -> Duration {
    if let Some(delay) = retry_after {
        // Quota resets can be hours or days away. A transport backoff cap must
        // never shorten the upstream's explicit recovery deadline.
        return if quota {
            delay.max(Duration::from_secs(1))
        } else {
            delay.clamp(Duration::from_secs(5), Duration::from_secs(1800))
        };
    }
    let base = if quota { 60 } else { 5 };
    let cap = if quota { 900 } else { 1800 };
    Duration::from_secs(base * (1u64 << failures.saturating_sub(1).min(6)))
        .min(Duration::from_secs(cap))
}

fn apply_failure(
    health: &mut Health,
    model: &str,
    status: u16,
    code: &str,
    message: Option<&str>,
    retry_after: Option<Duration>,
    now: i64,
) {
    if request_capability_failure(code) {
        clear_request_capability_failures(health);
        return;
    }
    let lower = message.unwrap_or_default().to_ascii_lowercase();
    let challenge = lower.contains("cloudflare")
        || lower.contains("cf-mitigated")
        || code.contains("challenge");
    let capability = matches!(
        code,
        "model_not_found" | "model_not_supported" | "official_model_unavailable"
    );
    let exhausted = quota_exhaustion(code);
    let grant_paused = status == 401
        || (status == 403 && !challenge)
        || code.contains("not_eligible")
        || code.contains("reauthorization")
        || code.contains("permission_required")
        || matches!(
            code,
            "eligibility_rejected"
                | "subscription_sharing_route_not_supported"
                | "chatpass_v2_scope_not_authorized"
                | "chatpass_v2_invalid_authorization_context"
                | "authorization_failed"
                | "subscription_sharing_invalid_user"
        );
    let permanent = grant_paused || capability;
    let key = if grant_paused || status == 403 || exhausted {
        ""
    } else {
        model
    };
    if let Some(current) = health.states.get_mut(key) {
        if current.paused {
            return;
        }
        if !permanent && current.retry_at_ms > now {
            // Concurrent failures share one probe window. Preserve a genuine
            // quota rejection over a less specific error and honor a later reset.
            if exhausted || !quota_exhaustion(&current.code) {
                if retry_after.is_some() || (exhausted && !quota_exhaustion(&current.code)) {
                    current.retry_at_ms = current.retry_at_ms.max(
                        now.saturating_add(
                            failure_delay(exhausted, current.failures, retry_after)
                                .as_millis()
                                .min(i64::MAX as u128) as i64,
                        ),
                    );
                }
                if exhausted {
                    current.code = code.chars().take(128).collect();
                }
                current.updated_at_ms = current.updated_at_ms.max(now);
            }
            return;
        }
    }
    let failures = health
        .states
        .get(key)
        .map_or(1, |state| state.failures.saturating_add(1));
    let delay = failure_delay(exhausted, failures, retry_after);
    if health.states.len() >= 256 && !health.states.contains_key(key) {
        if let Some(oldest) = health
            .states
            .iter()
            .filter(|(name, _)| !name.is_empty())
            .min_by_key(|(_, state)| state.updated_at_ms)
            .map(|(name, _)| name.clone())
        {
            health.states.remove(&oldest);
        }
    }
    health.states.insert(
        key.into(),
        Failure {
            code: code.chars().take(128).collect(),
            paused: permanent,
            retry_at_ms: now.saturating_add(delay.as_millis().min(i64::MAX as u128) as i64),
            updated_at_ms: now,
            failures,
        },
    );
}

fn apply_success(health: &mut Health, model: &str, started_at_ms: i64, now_ms: i64) {
    health.states.retain(|key, state| {
        state.paused
            || (key != "" && key != model)
            || state.updated_at_ms >= started_at_ms
            || (quota_exhaustion(&state.code) && state.retry_at_ms > now_ms)
    });
}

pub(crate) async fn record_official_route_failure(
    snapshot: &OfficialRouteSnapshot,
    model: &str,
    status: u16,
    code: &str,
    message: Option<&str>,
    retry_after: Option<Duration>,
    token_fingerprint: Option<&str>,
) -> Result<(), String> {
    let guard = storage::lock(&snapshot.registration_id).await?;
    let (snapshot, model, code, message, fingerprint) = (
        snapshot.clone(),
        model.to_owned(),
        code.to_owned(),
        message.map(str::to_owned),
        token_fingerprint.map(str::to_owned),
    );
    storage::blocking(move || {
        let _guard = guard;
        let record = storage::load(&snapshot.registration_id)?;
        binding::validate_record_source(&record, &snapshot.source_account_id)?;
        if !record.summary.connected
            || record.authorization_revision != snapshot.authorization_revision
            || !record
                .source_binding
                .as_ref()
                .is_some_and(|binding| binding.identity_fingerprint == snapshot.source_fingerprint)
        {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        if fingerprint.is_some_and(|expected| {
            record.credentials().map_or(true, |credentials| {
                status::token_fingerprint(&credentials.access_token) != expected
            })
        }) {
            return Ok(());
        }
        let mut health = read(&record)?;
        health.authorization_revision = snapshot.authorization_revision;
        health.source_fingerprint = snapshot.source_fingerprint;
        apply_failure(
            &mut health,
            &model,
            status,
            &code,
            message.as_deref(),
            retry_after,
            chrono::Utc::now().timestamp_millis(),
        );
        save(&record, &health)
    })
    .await
}

fn apply_check_success(health: &mut Health, started_at_ms: i64) {
    // GET /models proves permission, not inference quota recovery. Keep the
    // quota probe history even after its deadline, and never erase evidence
    // which arrived while this permission check was in flight.
    health
        .states
        .retain(|_, state| quota_exhaustion(&state.code) || state.updated_at_ms >= started_at_ms);
}

pub(super) async fn reset_after_check(
    id: &str,
    expected_revision: &str,
    expected_fingerprint: Option<&str>,
    started_at_ms: i64,
) -> Result<(), String> {
    let guard = storage::lock(id).await?;
    let id = id.to_owned();
    let expected_revision = expected_revision.to_owned();
    let expected_fingerprint = expected_fingerprint.map(str::to_owned);
    storage::blocking(move || {
        let _guard = guard;
        let record = storage::load(&id)?;
        if record.authorization_revision != expected_revision {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        if !record.summary.connected || record.summary.needs_reauth {
            return Err("CHATGPT_PLAN_REAUTH_REQUIRED".into());
        }
        if expected_fingerprint.as_ref().is_none_or(|expected| {
            record.credentials().map_or(true, |credentials| {
                status::token_fingerprint(&credentials.access_token) != *expected
            })
        }) {
            return Ok(());
        }
        let mut health = read(&record)?;
        apply_check_success(&mut health, started_at_ms);
        if !health.states.is_empty() {
            return save(&record, &health);
        }
        let path = storage::official_health_path(&id)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("CHATGPT_PLAN_STORAGE".into()),
        }
    })
    .await
}

pub(crate) async fn record_official_route_success(
    id: &str,
    source: &str,
    revision: &str,
    model: &str,
    started_at_ms: i64,
    token_fingerprint: &str,
) -> Result<(), String> {
    let guard = storage::lock(id).await?;
    let (id, source, revision, model, fingerprint) = (
        id.to_owned(),
        source.to_owned(),
        revision.to_owned(),
        model.to_owned(),
        token_fingerprint.to_owned(),
    );
    storage::blocking(move || {
        let _guard = guard;
        let record = storage::load(&id)?;
        binding::validate_record_source(&record, &source)?;
        if record.authorization_revision != revision || !record.summary.connected {
            return Ok(());
        }
        if record.credentials().map_or(true, |credentials| {
            status::token_fingerprint(&credentials.access_token) != fingerprint
        }) {
            return Ok(());
        }
        let mut health = read(&record)?;
        apply_success(
            &mut health,
            &model,
            started_at_ms,
            chrono::Utc::now().timestamp_millis(),
        );
        save(&record, &health)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXHAUSTED: &str = "subscription_sharing_usage_limit_exceeded";

    #[test]
    fn authorization_models_check_preserves_quota_and_newer_failure_evidence() {
        let mut health = Health::default();
        apply_failure(
            &mut health,
            "quota-model",
            429,
            EXHAUSTED,
            None,
            Some(Duration::from_secs(3600)),
            1000,
        );
        apply_failure(
            &mut health,
            "old-model",
            400,
            "model_not_supported",
            None,
            None,
            1000,
        );
        apply_failure(
            &mut health,
            "new-model",
            400,
            "model_not_supported",
            None,
            None,
            3000,
        );
        apply_failure(
            &mut health,
            "same-ms-model",
            400,
            "model_not_supported",
            None,
            None,
            2000,
        );
        let quota_deadline = health.states[""].retry_at_ms;
        apply_check_success(&mut health, 2000);
        assert!(!health.states.contains_key("old-model"));
        assert!(health.states["new-model"].paused);
        assert!(health.states["same-ms-model"].paused);
        assert_eq!(health.states[""].retry_at_ms, quota_deadline);
        // Even a new models check after the reset deadline does not prove a
        // successful generating probe and must retain the quota backoff level.
        apply_check_success(&mut health, quota_deadline + 1);
        assert_eq!(health.states.len(), 1);
        assert_eq!(health.states[""].code, EXHAUSTED);
        assert_eq!(health.states[""].failures, 1);
    }

    #[test]
    fn authorization_models_check_cannot_erase_a_quota_failure_while_in_flight() {
        let mut health = Health::default();
        let check_started = 1000;
        apply_failure(&mut health, "model", 429, EXHAUSTED, None, None, 2000);
        apply_check_success(&mut health, check_started);
        assert_eq!(health.states[""].updated_at_ms, 2000);
        assert_eq!(health.states[""].retry_at_ms, 62_000);
    }

    #[test]
    fn official_quota_exhaustion_honors_long_reset_and_shares_the_grant_window() {
        let mut health = Health::default();
        apply_failure(
            &mut health,
            "model-a",
            429,
            EXHAUSTED,
            None,
            Some(Duration::from_secs(24 * 60 * 60)),
            1000,
        );
        assert_eq!(health.states[""].retry_at_ms, 86_401_000);
        assert!(!health.states[""].paused);
        assert!(!health.states.contains_key("model-a"));
        // An in-flight rejection can extend the same window, never shorten it
        // or count as another controlled probe.
        apply_failure(
            &mut health,
            "model-b",
            429,
            EXHAUSTED,
            None,
            Some(Duration::from_secs(60)),
            2000,
        );
        assert_eq!(health.states[""].retry_at_ms, 86_401_000);
        assert_eq!(health.states[""].failures, 1);
        apply_failure(
            &mut health,
            "model-b",
            429,
            EXHAUSTED,
            None,
            Some(Duration::from_secs(2 * 24 * 60 * 60)),
            3000,
        );
        assert_eq!(health.states[""].retry_at_ms, 172_803_000);
        assert_eq!(health.states[""].updated_at_ms, 3000);
        assert_eq!(health.states[""].failures, 1);
    }

    #[test]
    fn official_quota_exhaustion_without_reset_uses_bounded_probe_windows() {
        let mut health = Health::default();
        let mut now = 1000;
        for expected_seconds in [60, 120, 240, 480, 900, 900, 900] {
            apply_failure(&mut health, "model", 429, EXHAUSTED, None, None, now);
            let deadline = now + expected_seconds * 1000;
            assert_eq!(health.states[""].retry_at_ms, deadline);
            let count = health.states[""].failures;
            apply_failure(
                &mut health,
                "other-model",
                429,
                EXHAUSTED,
                None,
                None,
                now + 1,
            );
            assert_eq!(health.states[""].retry_at_ms, deadline);
            assert_eq!(health.states[""].failures, count);
            now = deadline + 1;
        }
        let mut transient = Health::default();
        apply_failure(
            &mut transient,
            "model",
            429,
            "rate_limit_exceeded",
            None,
            None,
            1000,
        );
        assert_eq!(transient.states["model"].retry_at_ms, 6000);
        assert!(!transient.states.contains_key(""));
    }

    #[test]
    fn official_quota_cannot_be_cleared_by_old_or_parallel_success() {
        let mut health = Health::default();
        apply_failure(&mut health, "model", 429, EXHAUSTED, None, None, 1000);
        apply_success(&mut health, "model", 999, 2000);
        assert!(health.states.contains_key(""));
        apply_success(&mut health, "other-model", 1001, 2000);
        assert!(
            health.states.contains_key(""),
            "a parallel success cannot erase an active quota reset"
        );
        apply_success(&mut health, "model", 1000, 61_001);
        assert!(
            health.states.contains_key(""),
            "same-millisecond attempts are conservatively old"
        );
        apply_success(&mut health, "model", 61_001, 61_002);
        assert!(
            health.states.is_empty(),
            "a successful post-reset probe clears transient quota health"
        );
    }

    #[test]
    fn official_health_request_capability_rejection_never_pauses_model() {
        let mut health = Health::default();
        for code in [
            "capability_unsupported",
            "subscription_sharing_unsupported_capability",
        ] {
            apply_failure(
                &mut health,
                "model",
                400,
                code,
                Some("request unsupported"),
                None,
                1000,
            );
            assert!(health.states.is_empty());
        }
        apply_failure(
            &mut health,
            "model",
            400,
            "model_not_supported",
            None,
            None,
            1000,
        );
        assert!(health.states["model"].paused);
    }

    #[test]
    fn official_health_discards_legacy_request_pauses_only() {
        let mut health = Health::default();
        for (model, code) in [
            ("fast", "subscription_sharing_unsupported_capability"),
            ("tools", "capability_unsupported"),
            ("missing", "model_not_found"),
            ("", "subscription_sharing_user_not_eligible"),
        ] {
            health.states.insert(
                model.into(),
                Failure {
                    code: code.into(),
                    paused: true,
                    retry_at_ms: i64::MAX,
                    updated_at_ms: 0,
                    failures: 1,
                },
            );
        }
        clear_request_capability_failures(&mut health);
        assert_eq!(health.states.len(), 2);
        assert!(health.states["missing"].paused);
        assert!(health.states[""].paused);
    }

    #[test]
    fn official_health_eligibility_pauses_and_transient_backoff_is_bounded_and_idempotent() {
        let mut health = Health::default();
        apply_failure(
            &mut health,
            "model",
            503,
            "official_unavailable",
            None,
            None,
            1000,
        );
        assert_eq!(health.states["model"].retry_at_ms, 6000);
        apply_failure(
            &mut health,
            "model",
            503,
            "official_unavailable",
            None,
            None,
            2000,
        );
        assert_eq!(health.states["model"].failures, 1);
        apply_failure(
            &mut health,
            "model",
            503,
            "official_unavailable",
            None,
            None,
            6001,
        );
        assert_eq!(health.states["model"].retry_at_ms, 16001);
        apply_failure(
            &mut health,
            "model",
            403,
            "subscription_sharing_user_not_eligible",
            None,
            None,
            7000,
        );
        assert!(health.states[""].paused);
        apply_failure(
            &mut health,
            "model",
            503,
            "official_unavailable",
            None,
            Some(Duration::MAX),
            17000,
        );
        assert_eq!(health.states["model"].retry_at_ms, 1817000);
    }
}
