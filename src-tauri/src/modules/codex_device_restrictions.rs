//! Evidence-based, machine-local device restriction tracking. No active login probes.
#[path = "codex_device_restrictions_store.rs"]
mod store;
use crate::modules::{codex_account, logger};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
pub use store::Restriction;
use tauri::Emitter;

pub const DESKTOP: &str = "desktop";
pub const QUOTA: &str = "quota";
pub const API: &str = "api";
pub const OAUTH: &str = "oauth";
pub const TOKEN: &str = "token";
const EVENT: &str = "codex:device-restrictions-changed";
static IO: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        static ROOT: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());
        return Ok(ROOT.path().join("device-restrictions.sqlite"));
    }
    // Outside account storage/export/backup. This is an installation observation, not an
    // OpenAI device identity. We neither read nor alter hardware/device identifiers.
    #[cfg(not(test))]
    Ok(dirs::data_local_dir()
        .ok_or("CODEX_DEVICE_RESTRICTION_DIRECTORY")?
        .join("cockpit-tools-local")
        .join("codex-device-restrictions.sqlite"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_restriction_classifier_accepts_only_explicit_error_fields() {
        for text in [
            "Device limit reached",
            r#"{"error":{"message":"Device limit reached"}}"#,
            r#"{"error":"access_denied","error_description":"Device limit reached."}"#,
        ] {
            assert!(explicit_message(text));
        }
        for text in [
            "401 Unauthorized",
            "403 Forbidden",
            "429 Too Many Requests",
            "token_revoked",
            "Please explain Device limit reached",
            r#"{"output":"Device limit reached"}"#,
            r#"{"error":{"code":"device_limit_reached"}}"#,
            &"x".repeat(9000),
        ] {
            assert!(!explicit_message(text));
        }
    }
}

fn notify() {
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit(EVENT, ());
    }
}

pub fn paused_message() -> String {
    crate::modules::i18n::translate(
        &crate::modules::config::get_user_config().language,
        "common.deviceRestriction.paused",
        &[],
    )
}

/// Only explicit official error wording is supported until real structured error
/// samples establish a stable code. Generic status codes and arbitrary body matches
/// are deliberately not evidence. Never inspect chat/tool output with this function.
pub fn explicit_message(message: &str) -> bool {
    let text = message.trim();
    if text.eq_ignore_ascii_case("Device limit reached")
        || text.eq_ignore_ascii_case("Device limit reached.")
    {
        return true;
    }
    if text.len() > 8192 {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let error = value.get("error").unwrap_or(&value);
    let matched = [error.get("message"), value.get("error_description")]
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .any(|m| {
            m.trim().eq_ignore_ascii_case("Device limit reached")
                || m.trim().eq_ignore_ascii_case("Device limit reached.")
        });
    matched
}

/// The polling path previously discarded 403/404 bodies. Inspect only a bounded
/// error envelope, never buffer an arbitrarily large response for classification.
pub async fn bounded_error_body(mut response: reqwest::Response) -> Option<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 8192 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).ok()
}

async fn io<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    // No unbounded queues or synchronous I/O on runtime threads. DB busy waits are 50ms.
    let permit = tokio::time::timeout(Duration::from_millis(250), IO.acquire())
        .await
        .map_err(|_| "CODEX_DEVICE_RESTRICTION_BUSY")?
        .map_err(|_| "CODEX_DEVICE_RESTRICTION_BUSY")?;
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        }),
    )
    .await
    .map_err(|_| "CODEX_DEVICE_RESTRICTION_TIMEOUT".to_string())?
    .map_err(|e| e.to_string())?
}

pub async fn observe_account(
    account: &crate::models::codex::CodexAccount,
    scope: &'static str,
    source: &'static str,
    started_at: i64,
    restricted: bool,
) {
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return;
    }
    let original = account.clone();
    let result = io(move || {
        let path = path()?;
        if !restricted && !path.exists() {
            return Ok(false);
        }
        let mut db = store::open(&path)?;
        if !restricted
            && store::read(&db, &original.id, scope)?.is_none_or(|r| r.status == "recovered")
        {
            return Ok(false);
        }
        if !codex_account::load_account(&original.id).is_some_and(|current| {
            current.token_generation == original.token_generation
                && current.account_id == original.account_id
                && current.organization_id == original.organization_id
        }) {
            return Ok(false);
        }
        store::observe(
            &mut db,
            store::Observation {
                subject: &original.id,
                scope,
                source,
                instance_id: None,
                started_at,
                restricted,
            },
        )
    })
    .await;
    observe_result(result);
}

fn observe_result(result: Result<bool, String>) {
    match result {
        Ok(true) => notify(),
        Ok(false) => {}
        Err(error) => logger::log_warn(&format!(
            "[Codex Device Restriction] observation not persisted: {error}"
        )),
    }
}

pub async fn observe(
    subject: String,
    scope: &'static str,
    source: &'static str,
    instance_id: Option<String>,
    started_at: i64,
    restricted: bool,
) {
    let result = io(move || {
        // Do not revive deleted accounts. OAuth attempts without proven identity use a
        // login: UUID subject and can never pause an unrelated account.
        if !subject.starts_with("login:") && codex_account::load_account(&subject).is_none() {
            return Ok(false);
        }
        let path = path()?;
        if !restricted && !path.exists() {
            return Ok(false);
        }
        let mut db = store::open(&path)?;
        store::observe(
            &mut db,
            store::Observation {
                subject: &subject,
                scope,
                source,
                instance_id: instance_id.as_deref(),
                started_at,
                restricted,
            },
        )
    })
    .await;
    observe_result(result);
}

pub async fn blocked(
    subjects: Vec<String>,
    scope: &'static str,
) -> Result<HashSet<String>, String> {
    let mut blocked = HashSet::new();
    for chunk in subjects.chunks(128) {
        let ids = chunk.to_vec();
        let found = io(move || {
            let path = path()?;
            if !path.exists() {
                return Ok(HashSet::new());
            }
            let db = store::open(&path)?;
            let mut result = HashSet::new();
            for id in ids {
                if store::read(&db, &id, scope)?.is_some_and(|r| r.suspended) {
                    result.insert(id);
                }
            }
            Ok(result)
        })
        .await?;
        blocked.extend(found);
        tokio::task::yield_now().await;
    }
    Ok(blocked)
}

pub async fn ensure_allowed(account_id: &str, scope: &'static str) -> Result<(), String> {
    if blocked(vec![account_id.to_owned()], scope)
        .await?
        .is_empty()
    {
        Ok(())
    } else {
        Err(paused_message())
    }
}

/// Called on the blocking deletion worker, after account tombstones are committed.
pub fn retire_account(account_id: &str) {
    let result = (|| {
        let path = path()?;
        if !path.exists() {
            return Ok(());
        }
        store::retire(&mut store::open(&path)?, account_id, now())
    })();
    match result {
        Ok(()) => notify(),
        Err(error) => logger::log_warn(&format!(
            "[Codex Device Restriction] cleanup pending: {error}"
        )),
    }
}

#[tauri::command]
pub async fn codex_list_device_restrictions(
    after: Option<String>,
) -> Result<Vec<Restriction>, String> {
    io(move || {
        let path = path()?;
        if !path.exists() {
            return Ok(vec![]);
        }
        store::list(&store::open(&path)?, after.as_deref().unwrap_or(""))
    })
    .await
}

#[tauri::command]
pub async fn codex_allow_device_restriction_retry(
    subject: String,
    scope: String,
    revision: i64,
) -> Result<(), String> {
    if ![DESKTOP, QUOTA, API, OAUTH, TOKEN].contains(&scope.as_str()) {
        return Err("CODEX_DEVICE_RESTRICTION_SCOPE".into());
    }
    io(move || {
        store::allow_retry(
            &mut store::open(&path()?)?,
            &subject,
            &scope,
            revision,
            now(),
        )
    })
    .await?;
    notify();
    Ok(())
}
