//! A confirmation is tied to one claimed run. Nothing is sent before approval.
use crate::modules::codex_wakeup::CodexWakeupTask;
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationRequest {
    pub request_id: String,
    pub task_name: String,
    pub account_count: usize,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub expires_at: i64,
}

struct Pending {
    request: ConfirmationRequest,
    reply: oneshot::Sender<bool>,
}
static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn list_pending() -> Result<Vec<ConfirmationRequest>, String> {
    let now = chrono::Utc::now().timestamp();
    Ok(pending()
        .lock()
        .map_err(|_| "CODEX_CONFIRMATION_LOCK")?
        .values()
        .filter(|p| p.request.expires_at > now)
        .map(|p| p.request.clone())
        .collect())
}

pub fn respond(request_id: &str, approved: bool) -> Result<(), String> {
    let item = pending()
        .lock()
        .map_err(|_| "CODEX_CONFIRMATION_LOCK")?
        .remove(request_id)
        .ok_or("CODEX_CONFIRMATION_EXPIRED")?;
    let approved = approved && chrono::Utc::now().timestamp() < item.request.expires_at;
    item.reply
        .send(approved)
        .map_err(|_| "CODEX_CONFIRMATION_EXPIRED".to_string())
}

struct PendingGuard<'a> {
    id: String,
    app: Option<&'a AppHandle>,
}
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = pending().lock() {
            pending.remove(&self.id);
        }
        if let Some(app) = self.app {
            let _ = app.emit("codex:wakeup-confirmation-ended", &self.id);
        }
    }
}

// The dispatch future remains unpolled for rejected, expired, closed or cancelled
// confirmations. The same guard also covers cancellation while awaiting a reply.
async fn after_approval<T>(
    reply: oneshot::Receiver<bool>,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    dispatch: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let cancellation = async {
        loop {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let approved = tokio::select! {
        biased;
        _ = cancellation => return Err("CODEX_CONFIRMATION_CANCELLED".into()),
        result = tokio::time::timeout(timeout, reply) => matches!(result, Ok(Ok(true))),
    };
    if !approved {
        return Err("CODEX_CONFIRMATION_DECLINED_OR_EXPIRED".into());
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("CODEX_CONFIRMATION_CANCELLED".into());
    }
    dispatch.await
}

pub async fn dispatch<T>(
    app: Option<&AppHandle>,
    task: &CodexWakeupTask,
    automatic: bool,
    cancel: Arc<AtomicBool>,
    send: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if !automatic || task.execution_mode.as_deref().unwrap_or("auto") == "auto" {
        return send.await;
    }
    if task.execution_mode.as_deref() != Some("confirm") {
        return Err("CODEX_CONFIRMATION_MODE_INVALID".into());
    }
    let app = app.ok_or("CODEX_CONFIRMATION_UI_UNAVAILABLE")?;
    let timeout = Duration::from_secs(task.confirm_timeout_minutes.unwrap_or(5).max(1) as u64 * 60);
    let request = ConfirmationRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        task_name: task.name.clone(),
        account_count: task.account_ids.len(),
        model: task.model_display_name.clone().or(task.model.clone()),
        prompt: task.prompt.clone(),
        expires_at: chrono::Utc::now().timestamp() + timeout.as_secs() as i64,
    };
    let (reply, receiver) = oneshot::channel();
    pending()
        .lock()
        .map_err(|_| "CODEX_CONFIRMATION_LOCK")?
        .insert(
            request.request_id.clone(),
            Pending {
                request: request.clone(),
                reply,
            },
        );
    let _guard = PendingGuard {
        id: request.request_id.clone(),
        app: Some(app),
    };
    app.emit("codex:wakeup-confirmation", &request)
        .map_err(|e| e.to_string())?;
    after_approval(receiver, timeout, cancel, send).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejected_expired_closed_and_cancelled_confirmations_never_poll_model() {
        for outcome in ["reject", "expire", "close", "cancel"] {
            let (reply, receiver) = oneshot::channel();
            match outcome {
                "reject" => {
                    reply.send(false).unwrap();
                }
                "close" => drop(reply),
                _ => {}
            }
            let result = after_approval(
                receiver,
                Duration::from_millis(10),
                Arc::new(AtomicBool::new(outcome == "cancel")),
                async {
                    panic!("no model request before confirmation");
                    #[allow(unreachable_code)]
                    Ok::<(), String>(())
                },
            )
            .await;
            assert!(result.is_err(), "{outcome}");
        }
    }
    #[tokio::test]
    async fn approval_dispatches_once_and_cancel_racing_approval_wins() {
        let sends = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for cancelled in [true, false] {
            let (reply, receiver) = oneshot::channel();
            reply.send(true).unwrap();
            let sent = sends.clone();
            let result = after_approval(
                receiver,
                Duration::from_secs(1),
                Arc::new(AtomicBool::new(cancelled)),
                async move {
                    sent.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .await;
            assert_eq!(result.is_ok(), !cancelled);
        }
        assert_eq!(sends.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn confirmation_without_frontend_fails_closed_but_auto_and_manual_keep_their_behavior() {
        let mut task: CodexWakeupTask = serde_json::from_value(serde_json::json!({
            "id":"task","name":"Task","enabled":true,"accountIds":["synthetic"],"createdAt":1,"updatedAt":1,
            "schedule":{"kind":"interval","weeklyDays":[],"intervalHours":1},"executionMode":"confirm"
        })).unwrap();
        assert!(
            dispatch(None, &task, true, Arc::new(AtomicBool::new(false)), async {
                panic!("no frontend must not send");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            })
            .await
            .is_err()
        );
        let sends = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for automatic in [false, true] {
            if automatic {
                task.execution_mode = Some("auto".into());
            }
            let sent = sends.clone();
            dispatch(
                None,
                &task,
                automatic,
                Arc::new(AtomicBool::new(false)),
                async move {
                    sent.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(sends.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn responses_are_single_use_and_bound_to_one_run() {
        let id = uuid::Uuid::new_v4().to_string();
        let (reply, mut receiver) = oneshot::channel();
        pending().lock().unwrap().insert(
            id.clone(),
            Pending {
                request: ConfirmationRequest {
                    request_id: id.clone(),
                    task_name: "Task".into(),
                    account_count: 1,
                    model: None,
                    prompt: None,
                    expires_at: chrono::Utc::now().timestamp() + 60,
                },
                reply,
            },
        );
        respond(&id, true).unwrap();
        assert_eq!(receiver.try_recv(), Ok(true));
        assert!(respond(&id, true).is_err());
        assert!(respond("old-run", true).is_err());
    }
}
