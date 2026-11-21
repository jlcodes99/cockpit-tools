use super::{activity, is_codex_app_target, query_targets};
use crate::modules::{codex_session_resume, logger, process};
use codex_session_resume::ResumeSettings;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

const SCRIPT: &str = include_str!("codex_app_enhancements/session_resume.js");

pub(crate) fn schedule_desktop_resume(
    instance: String,
    pid: u32,
    port: u16,
    settings: ResumeSettings,
) {
    tauri::async_runtime::spawn(async move {
        let result = resume(&instance, pid, port, settings).await;
        match result {
            Ok("submitted") => logger::log_info("[Codex Session Resume] 已发送续跑提示"),
            Ok("deduplicated") => {}
            Ok(status) => logger::log_warn(&format!("[Codex Session Resume] {status}")),
            Err(error) => logger::log_warn(&format!("[Codex Session Resume] {error}")),
        }
    });
}

async fn resume(
    instance: &str,
    pid: u32,
    port: u16,
    settings: ResumeSettings,
) -> Result<&'static str, String> {
    let client = activity::local_desktop_client().ok_or("无法创建桌面连接")?;
    let previous = if settings.uses_previous_desktop_session() {
        let instance = instance.to_owned();
        let expected = settings.clone();
        let previous = tauri::async_runtime::spawn_blocking(move || {
            codex_session_resume::take_stopped_thread(&instance, &expected)
        })
        .await
        .map_err(|error| error.to_string())??;
        Some(previous.ok_or("没有可确认的上次桌面会话")?)
    } else {
        None
    };
    let prepare = format!(
        r#"({SCRIPT})("prepare", {}, {})"#,
        json!({
            "target": settings.target, "threadId": settings.thread_id
        }),
        json!(previous)
    );
    let (url, thread) = timeout(Duration::from_secs(25), async {
        loop {
            if !process::is_pid_running(pid) {
                return Err("桌面实例已退出");
            }
            for target in query_targets(&client, port)
                .await
                .into_iter()
                .filter(is_codex_app_target)
            {
                let Some(url) = target.websocket_url else {
                    continue;
                };
                let result = timeout(
                    Duration::from_secs(10),
                    activity::evaluate_owned_target(&url, &prepare),
                )
                .await
                .ok()
                .flatten();
                let Some(result) = result else { continue };
                match status(&result) {
                    Some("ready") => {
                        let id = result["threadId"]
                            .as_str()
                            .and_then(|raw| Uuid::parse_str(raw).ok())
                            .ok_or("返回的会话 ID 无效")?;
                        return Ok((url, id));
                    }
                    Some("missingThread") => return Err("找不到要恢复的会话"),
                    Some("unavailableThread") => return Err("当前实例无法打开指定会话"),
                    Some("busy") => return Err("会话已在执行"),
                    _ => {}
                }
            }
            sleep(Duration::from_millis(300)).await;
        }
    })
    .await
    .map_err(|_| "等待桌面实例就绪超时")??;

    if !process::is_pid_running(pid) {
        return Err("桌面实例已退出".into());
    }
    let launch = format!("{pid}:{port}");
    let claimed = {
        let instance = instance.to_owned();
        let launch = launch.clone();
        let settings = settings.clone();
        tauri::async_runtime::spawn_blocking(move || {
            codex_session_resume::claim(&instance, thread, &launch, &settings)
        })
        .await
        .map_err(|error| error.to_string())??
    };
    if !claimed {
        return Ok("deduplicated");
    }
    let outcome = if !process::is_pid_running(pid) {
        "exited"
    } else {
        let send = format!(
            "({SCRIPT})(\"send\", {}, {})",
            json!({"prompt": settings.prompt}),
            json!(thread)
        );
        let result = timeout(
            Duration::from_secs(10),
            activity::evaluate_owned_target(&url, &send),
        )
        .await
        .ok()
        .flatten();
        match result.as_ref().and_then(status) {
            Some("submitted") => "submitted",
            Some("busy") => "busy",
            Some("rejected") => "rejected",
            Some("unavailableThread") => "unavailableThread",
            Some("pending") => "notReady",
            _ => "uncertain",
        }
    };
    let instance = instance.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        codex_session_resume::settle(&instance, thread, &launch, outcome)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(outcome)
}

fn status(value: &Value) -> Option<&str> {
    value.get("status")?.as_str()
}
