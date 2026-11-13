use super::{
    normalize_antigravity_runtime_target, refresh_all_quotas, refresh_current_quota_for_target,
};
use crate::modules;
use serde_json::Value;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::time::{Instant, MissedTickBehavior};

const SETTINGS_POLL_INTERVAL: Duration = Duration::from_secs(10);
const RUNTIME_TARGET_KEY: &str = "agtools.antigravity.runtime_target.v1";
const CURRENT_MINUTES_KEY: &str = "agtools.current_account_refresh_minutes.v1";
const ACCOUNT_OVERRIDES_KEY: &str = "agtools.account_refresh_overrides.v1";

#[derive(Default)]
struct RefreshTimer {
    interval: Option<Duration>,
    next: Option<Instant>,
}

impl RefreshTimer {
    fn set_interval(&mut self, now: Instant, interval: Option<Duration>) {
        if self.interval != interval {
            self.interval = interval;
            self.next = interval.map(|period| now + period);
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        self.next.is_some_and(|deadline| deadline <= now)
    }

    fn advance(&mut self, now: Instant) {
        self.next = self.interval.map(|period| now + period);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RefreshJob {
    Full,
    Current,
}

#[derive(Default)]
struct Schedule {
    full: RefreshTimer,
    current: RefreshTimer,
    target: Option<&'static str>,
}

impl Schedule {
    fn next_job(
        &mut self,
        now: Instant,
        target: Option<&'static str>,
        full_minutes: i32,
        current_minutes: i32,
    ) -> Option<RefreshJob> {
        if self.target != target {
            self.target = target;
            self.current = RefreshTimer::default();
        }
        let period = |minutes: i32| {
            (target.is_some() && full_minutes > 0 && minutes > 0)
                .then(|| Duration::from_secs(minutes as u64 * 60))
        };
        self.full.set_interval(now, period(full_minutes));
        self.current.set_interval(now, period(current_minutes));
        if self.full.is_due(now) {
            self.full.advance(now);
            Some(RefreshJob::Full)
        } else if self.current.is_due(now) {
            self.current.advance(now);
            Some(RefreshJob::Current)
        } else {
            None
        }
    }

    fn finish(&mut self, now: Instant, job: &RefreshJob) {
        let timer = match job {
            RefreshJob::Full => &mut self.full,
            RefreshJob::Current => &mut self.current,
        };
        if timer.is_due(now) {
            timer.advance(now);
        }
    }
}

fn preference_number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn current_minutes(default: &Value, overrides: &Value, email: Option<&str>) -> i32 {
    let normalize = |minutes: f64| minutes.floor().clamp(1.0, 999.0) as i32;
    if let Some(minutes) = preference_number(email.and_then(|email| overrides.get(email))) {
        if minutes == -1.0 {
            return -1;
        }
        if minutes.is_finite() {
            return normalize(minutes);
        }
    }
    preference_number(default.get("antigravity"))
        .filter(|minutes| minutes.is_finite())
        .map_or(1, normalize)
}

fn load_schedule_preferences() -> Result<(Option<&'static str>, i32), String> {
    let preferences = modules::ui_preferences::load_ui_preferences()?;
    let target = normalize_antigravity_runtime_target(
        preferences
            .values
            .get(RUNTIME_TARGET_KEY)
            .map(String::as_str),
    );
    let platform = target.platform_id();
    if !modules::tray_layout::is_platform_enabled(platform) {
        return Ok((None, -1));
    }
    let parse_preference = |key| {
        preferences
            .values
            .get(key)
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or(Value::Null)
    };
    let default = parse_preference(CURRENT_MINUTES_KEY);
    let overrides = parse_preference(ACCOUNT_OVERRIDES_KEY);
    let overrides = &overrides["antigravity"];
    let email = if overrides
        .as_object()
        .is_some_and(|entries| !entries.is_empty())
    {
        let index = modules::account::load_account_index()?;
        target
            .bound_account_id()
            .as_ref()
            .and_then(|id| index.accounts.iter().find(|account| &account.id == id))
            .or_else(|| {
                index
                    .current_account_id
                    .as_ref()
                    .and_then(|id| index.accounts.iter().find(|account| &account.id == id))
            })
            .map(|account| account.email.clone())
    } else {
        None
    };
    Ok((
        Some(platform),
        current_minutes(&default, overrides, email.as_deref()),
    ))
}

pub(crate) async fn run(app: AppHandle) {
    let mut tick = tokio::time::interval(SETTINGS_POLL_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut schedule = Schedule::default();
    loop {
        tick.tick().await;
        let full_minutes = modules::config::get_user_config().auto_refresh_minutes;
        if full_minutes <= 0 {
            schedule = Schedule::default();
            continue;
        }
        let (target, current_minutes) =
            match tauri::async_runtime::spawn_blocking(load_schedule_preferences).await {
                Ok(Ok(settings)) => settings,
                Ok(Err(error)) => {
                    modules::logger::log_warn(&format!(
                        "[Antigravity AutoRefresh] 读取刷新设置失败: {error}"
                    ));
                    continue;
                }
                Err(error) => {
                    modules::logger::log_warn(&format!(
                        "[Antigravity AutoRefresh] 刷新设置任务失败: {error}"
                    ));
                    continue;
                }
            };
        let Some(job) = schedule.next_job(Instant::now(), target, full_minutes, current_minutes)
        else {
            continue;
        };
        let runtime_target = target.map(str::to_owned);
        let result = match job {
            RefreshJob::Full => {
                refresh_all_quotas(app.clone(), Some("auto".into()), runtime_target)
                    .await
                    .map(|_| true)
            }
            RefreshJob::Current => {
                refresh_current_quota_for_target(app.clone(), runtime_target).await
            }
        };
        schedule.finish(Instant::now(), &job);
        match result {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => modules::logger::log_warn(&format!(
                "[Antigravity AutoRefresh] 自动刷新失败: {error}"
            )),
        }
        // A failed refresh may still update the account's status or error details.
        let _ = app.emit(
            "accounts:changed",
            serde_json::json!({
                "platformId": "antigravity",
                "reason": "automatic-quota-refresh"
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_antigravity_refresh_keeps_both_jobs_due_without_window_events() {
        let start = Instant::now();
        let mut schedule = Schedule::default();
        let mut job_at = |seconds| {
            schedule.next_job(
                start + Duration::from_secs(seconds),
                Some("antigravity_ide"),
                2,
                1,
            )
        };
        assert_eq!(job_at(0), None);
        assert_eq!(job_at(60), Some(RefreshJob::Current));
        assert_eq!(job_at(120), Some(RefreshJob::Full));
        assert_eq!(job_at(130), Some(RefreshJob::Current));
        assert_eq!(job_at(131), None);
    }

    #[test]
    fn native_antigravity_refresh_respects_overrides_target_changes_and_long_runs() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut schedule = Schedule::default();
        let defaults = json!({"antigravity": 2});
        let overrides = json!({"off@example.test": -1, "fast@example.test": 1});
        let interval = |email| current_minutes(&defaults, &overrides, Some(email));
        assert_eq!(interval("unknown@example.test"), 2);
        assert_eq!(interval("off@example.test"), -1);
        assert_eq!(interval("fast@example.test"), 1);

        assert_eq!(
            schedule.next_job(at(0), Some("antigravity_ide"), 5, -1),
            None
        );
        assert_eq!(
            schedule.next_job(at(120), Some("antigravity_ide"), 5, -1),
            None
        );
        assert_eq!(
            schedule.next_job(at(130), Some("antigravity_ide"), 5, 1),
            None
        );
        assert_eq!(schedule.next_job(at(180), Some("antigravity"), 5, 1), None);
        assert_eq!(
            schedule.next_job(at(240), Some("antigravity"), 5, 1),
            Some(RefreshJob::Current)
        );
        schedule.finish(at(315), &RefreshJob::Current);
        assert_eq!(
            schedule.next_job(at(315), Some("antigravity"), 5, 1),
            Some(RefreshJob::Full)
        );
        assert_eq!(schedule.next_job(at(320), None, 5, 1), None);
        assert_eq!(schedule.next_job(at(380), Some("antigravity"), -1, 1), None);
    }
}
