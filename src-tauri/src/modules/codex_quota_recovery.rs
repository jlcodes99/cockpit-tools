use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tauri::Emitter;
use tokio::sync::mpsc;

use crate::models::codex::CodexQuota;
use crate::modules::{atomic_write, config, i18n, logger};

const THRESHOLD: i32 = 90;
const PUSHPLUS_URL: &str = "https://www.pushplus.plus/send";
const SETTINGS_CHANGED: &str = "codex:quota-recovery-settings-changed";
// Kept outside exported general settings. The token never crosses IPC on reads.
static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static TOKEN_REVISION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct QuotaWindow {
    present: bool,
    remaining: i32,
}

impl QuotaWindow {
    fn new(present: Option<bool>, reset: Option<i64>, remaining: i32) -> Self {
        Self {
            present: present.unwrap_or_else(|| reset.is_some_and(|time| time > 0)),
            remaining,
        }
    }

    fn recovered_to(self, next: Self) -> Option<(i32, i32)> {
        (self.present
            && next.present
            && (0..next.remaining).contains(&self.remaining)
            && (THRESHOLD..=100).contains(&next.remaining))
        .then_some((self.remaining, next.remaining))
    }
}

fn quota_windows(quota: &CodexQuota) -> [QuotaWindow; 2] {
    [
        QuotaWindow::new(
            quota.hourly_window_present,
            quota.hourly_reset_time,
            quota.hourly_percentage,
        ),
        QuotaWindow::new(
            quota.weekly_window_present,
            quota.weekly_reset_time,
            quota.weekly_percentage,
        ),
    ]
}

pub(crate) struct Recovery([Option<(i32, i32)>; 2]);

impl Recovery {
    pub(crate) fn detect(previous: &CodexQuota, next: &CodexQuota) -> Option<Self> {
        let [old_short, old_week] = quota_windows(previous);
        let [new_short, new_week] = quota_windows(next);
        let changes = [
            old_short.recovered_to(new_short),
            old_week.recovered_to(new_week),
        ];
        changes.iter().any(Option::is_some).then_some(Self(changes))
    }

    // Called in quota commit order. Network and settings I/O stay off the account lock.
    pub(crate) fn enqueue(self) {
        let revision = TOKEN_REVISION.load(Ordering::Acquire);
        if crate::get_app_handle().is_some() && QUEUE.try_send((revision, self)).is_err() {
            logger::log_warn("[Codex Quota Recovery] notification queue unavailable");
        }
    }

    fn message(&self, language: &str) -> String {
        ["codex.thresholds.shortCycle", "codex.thresholds.weekly"]
            .into_iter()
            .zip(self.0)
            .filter_map(|(key, change)| {
                let (previous, next) = change?;
                let window = i18n::translate(language, key, &[]);
                Some(format!("{window} · {previous}% → {next}%"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

static QUEUE: LazyLock<mpsc::Sender<(u64, Recovery)>> = LazyLock::new(|| {
    let (sender, receiver) = mpsc::channel(128);
    tauri::async_runtime::spawn(deliver_reminders(receiver));
    sender
});

fn token_path() -> Result<std::path::PathBuf, String> {
    Ok(config::get_data_dir()?.join("codex_quota_recovery_token"))
}

fn read_token_at(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(token) => Ok(token),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err("CODEX_QUOTA_RECOVERY_SETTINGS".into()),
    }
}

fn token() -> Result<String, String> {
    let mut token = TOKEN.lock().map_err(|_| "CODEX_QUOTA_RECOVERY_SETTINGS")?;
    if let Some(token) = token.as_ref() {
        return Ok(token.clone());
    }
    let loaded = read_token_at(&token_path()?)?;
    *token = Some(loaded.clone());
    Ok(loaded)
}

#[tauri::command]
pub async fn codex_quota_recovery_enabled() -> Result<bool, String> {
    tokio::task::spawn_blocking(|| token().map(|token| !token.is_empty()))
        .await
        .map_err(|_| "CODEX_QUOTA_RECOVERY_SETTINGS".to_string())?
}

#[tauri::command]
pub async fn codex_quota_recovery_set_token(
    app: tauri::AppHandle,
    new_token: Option<String>,
) -> Result<bool, String> {
    let enabled = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let next = match new_token {
            Some(value) if value.trim().is_empty() => {
                return Err("CODEX_QUOTA_RECOVERY_TOKEN_REQUIRED".into())
            }
            Some(value) => value.trim().to_string(),
            None => String::new(),
        };
        let mut token = TOKEN.lock().map_err(|_| "CODEX_QUOTA_RECOVERY_SETTINGS")?;
        atomic_write::write_secret_string_atomic(&token_path()?, &next)
            .map_err(|_| "CODEX_QUOTA_RECOVERY_SETTINGS".to_string())?;
        let enabled = !next.is_empty();
        *token = Some(next);
        TOKEN_REVISION.fetch_add(1, Ordering::Release);
        Ok(enabled)
    })
    .await
    .map_err(|_| "CODEX_QUOTA_RECOVERY_SETTINGS".to_string())??;
    if app.emit(SETTINGS_CHANGED, ()).is_err() {
        logger::log_warn("[Codex Quota Recovery] settings event failed");
    }
    Ok(enabled)
}

#[derive(Deserialize)]
struct PushPlusResponse {
    code: i64,
}

async fn send_message(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    title: &str,
    content: &str,
) -> Result<(), &'static str> {
    let response = client
        .post(url)
        .json(&serde_json::json!({
            "token": token,
            "title": title,
            "content": content,
            "template": "txt",
        }))
        .send()
        .await
        .map_err(|_| "PushPlus network error")?
        .error_for_status()
        .map_err(|_| "PushPlus HTTP error")?
        .json::<PushPlusResponse>()
        .await
        .map_err(|_| "PushPlus invalid response")?;
    (response.code == 200)
        .then_some(())
        .ok_or("PushPlus rejected notification")
}

async fn deliver_reminders(mut queue: mpsc::Receiver<(u64, Recovery)>) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            logger::log_warn("[Codex Quota Recovery] could not initialize PushPlus client");
            return;
        }
    };
    while let Some((revision, recovery)) = queue.recv().await {
        let config = config::get_user_config();
        if !config.external_network_enabled {
            continue;
        }
        let token = match tokio::task::spawn_blocking(token).await {
            Ok(Ok(token)) if !token.is_empty() => token,
            Ok(Ok(_)) => continue,
            _ => {
                logger::log_warn("[Codex Quota Recovery] could not read PushPlus settings");
                continue;
            }
        };
        // A queued reminder belongs to the recipient configured at detection.
        if revision != TOKEN_REVISION.load(Ordering::Acquire) {
            continue;
        }
        let title = i18n::translate(&config.language, "codex.quotaRecovery.title", &[]);
        if let Err(error) = send_message(
            &client,
            PUSHPLUS_URL,
            &token,
            &title,
            &recovery.message(&config.language),
        )
        .await
        {
            logger::log_warn(&format!("[Codex Quota Recovery] {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};

    fn quota(hourly: i32, weekly: i32) -> CodexQuota {
        serde_json::from_value(json!({
            "hourly_percentage": hourly,
            "weekly_percentage": weekly,
            "hourly_window_present": true,
            "weekly_window_present": true,
        }))
        .unwrap()
    }

    #[test]
    fn detects_each_window_crossing_and_ignores_repeat_or_missing_windows() {
        let before = quota(50, 89);
        let after = quota(90, 95);
        assert_eq!(
            Recovery::detect(&before, &after).unwrap().0,
            [Some((50, 90)), Some((89, 95))]
        );
        assert_eq!(
            Recovery::detect(&after, &quota(100, 100)).unwrap().0,
            [Some((90, 100)), Some((95, 100))]
        );
        assert!(Recovery::detect(&after, &after).is_none());
        assert!(Recovery::detect(&before, &quota(89, 89)).is_none());
        let mut missing = before.clone();
        missing.hourly_window_present = Some(false);
        missing.weekly_window_present = None;
        assert!(Recovery::detect(&missing, &after).is_none());
        missing.weekly_reset_time = Some(1_800_000_000);
        assert_eq!(
            Recovery::detect(&missing, &after).unwrap().0,
            [None, Some((89, 95))]
        );
    }

    #[test]
    fn missing_or_disabled_token_stays_empty_and_read_errors_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push-token");
        assert_eq!(read_token_at(&path).unwrap(), "");
        atomic_write::write_secret_string_atomic(&path, "fixture-token").unwrap();
        assert_eq!(read_token_at(&path).unwrap(), "fixture-token");
        atomic_write::write_secret_string_atomic(&path, "").unwrap();
        assert_eq!(read_token_at(&path).unwrap(), "");
        assert!(read_token_at(dir.path()).is_err());
    }

    #[tokio::test]
    async fn pushplus_receives_only_the_token_and_localized_quota_message() {
        let (seen, mut request) = mpsc::channel(2);
        let router = Router::new().route(
            "/send",
            post(move |Json(payload): Json<Value>| {
                let seen = seen.clone();
                async move {
                    let code = if payload["token"] == "fixture-token" {
                        200
                    } else {
                        401
                    };
                    seen.send(payload).await.unwrap();
                    Json(json!({ "code": code }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/send", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let recovery = Recovery::detect(&quota(60, 92), &quota(90, 92)).unwrap();
        let message = recovery.message("en");
        send_message(
            &client,
            &url,
            "fixture-token",
            "Quota recovery reminder",
            &message,
        )
        .await
        .unwrap();
        assert_eq!(
            request.recv().await.unwrap(),
            json!({
                "token": "fixture-token",
                "title": "Quota recovery reminder",
                "content": "Short-cycle quota · 60% → 90%",
                "template": "txt",
            })
        );
        assert_eq!(
            send_message(
                &client,
                &url,
                "bad-token",
                "Quota recovery reminder",
                &message
            )
            .await,
            Err("PushPlus rejected notification"),
        );
        server.abort();
    }
}
