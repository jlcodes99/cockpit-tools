//! Mount the status capsule in Cockpit-launched Antigravity desktop windows.

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::json;

use super::{app_cdp, app_lifecycle, config, i18n, logger};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const CONNECTION_GRACE: Duration = Duration::from_secs(45);
const LOCALIZATION_SCRIPT: &str = concat!(
    "(",
    include_str!("antigravity_desktop_localization.js"),
    ")(",
    include_str!("antigravity_desktop_zh_cn.json"),
    ",",
    include_str!("antigravity_desktop_zh_cn_templates.json"),
    ")"
);

type Task = tauri::async_runtime::JoinHandle<()>;

/// Keep a user-supplied debug configuration intact. Otherwise pass a
/// Cockpit-owned loopback port before any positional arguments.
pub(super) fn launch(
    profile: &str,
    args: &[String],
    start: impl FnOnce(&[String]) -> Result<u32, String>,
) -> Result<u32, String> {
    let split = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    if args[..split].iter().any(|arg| {
        matches!(
            arg.split('=').next(),
            Some(
                "--remote-debugging-port"
                    | "--remote-debugging-address"
                    | "--remote-debugging-pipe"
            )
        )
    }) {
        return start(args);
    }
    let port = match TcpListener::bind("127.0.0.1:0").and_then(|socket| socket.local_addr()) {
        Ok(address) => address.port(),
        Err(error) => {
            logger::log_warn(&format!(
                "[Antigravity Composer] 无法准备本地调试端口: {error}"
            ));
            return start(args);
        }
    };

    let mut launch_args = args[..split].to_vec();
    launch_args.push("--remote-debugging-address=127.0.0.1".into());
    launch_args.push(format!("--remote-debugging-port={port}"));
    launch_args.extend_from_slice(&args[split..]);
    let pid = start(&launch_args)?;
    start_attachment(profile, port);
    Ok(pid)
}

fn start_attachment(profile: &str, port: u16) {
    static TASKS: OnceLock<Mutex<HashMap<String, Task>>> = OnceLock::new();
    let mut tasks = TASKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tasks.retain(|_, task| !task.inner().is_finished());
    if let Some(task) = tasks.remove(profile) {
        task.abort();
    }
    tasks.insert(
        profile.to_owned(),
        tauri::async_runtime::spawn(attach(port)),
    );
}

fn is_chat_target(target: &app_cdp::CdpTarget) -> bool {
    target.target_type.as_deref() == Some("page")
        && target.url.as_deref().is_some_and(|raw| {
            reqwest::Url::parse(raw).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some()
            })
        })
}

async fn attach(port: u16) {
    let Ok(client) = Client::builder().no_proxy().build() else {
        return;
    };
    let mut last_connection = Instant::now();
    let mut mounted = false;
    let mut preferences_pending = true;
    let preferences_deadline = Instant::now() + CONNECTION_GRACE;
    let mut preferences_target: Option<app_cdp::CdpTarget> = None;
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if app_lifecycle::is_shutdown_started() {
            return;
        }
        let Some(targets) = app_cdp::query_targets(&client, port, is_chat_target).await else {
            if last_connection.elapsed() >= CONNECTION_GRACE {
                if !mounted {
                    logger::log_warn("[Antigravity Composer] 启动后未连接到本地调试窗口");
                }
                return;
            }
            continue;
        };
        last_connection = Instant::now();
        if targets.is_empty() {
            continue;
        }
        let user_config = config::get_user_config();
        if preferences_pending && Instant::now() >= preferences_deadline {
            logger::log_warn("[Antigravity Preferences] 启动后未能确认客户端偏好设置");
            preferences_pending = false;
        }
        if preferences_pending {
            let preferences = json!({
                "layout": user_config.antigravity_desktop_layout,
                "permission": user_config.antigravity_desktop_permission,
            });
            let script = include_str!("antigravity_launch_preferences.js");
            let expression = format!("({script})({preferences})");
            let candidates = preferences_target
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or(&targets);
            for target in candidates {
                match app_cdp::evaluate_target::<String>(target, &expression, false)
                    .await
                    .as_deref()
                {
                    Some("applying") => {
                        preferences_target = Some(target.clone());
                        break;
                    }
                    Some("applied") => {
                        preferences_pending = false;
                        break;
                    }
                    Some(
                        status @ ("restricted"
                        | "layout_restricted"
                        | "permission_restricted"
                        | "unsupported"
                        | "failed"),
                    ) => {
                        logger::log_warn(&format!(
                            "[Antigravity Preferences] 客户端偏好设置结果: {status}"
                        ));
                        preferences_pending = false;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let settings = script_settings(&user_config.language);
        let chinese_enabled = user_config.antigravity_desktop_chinese_enabled;
        let refresh = format!(
            "[window.__cockpitAntigravityComposer?.refresh?.({settings}) ?? false, \
             window.__cockpitAntigravityLocalization?.setEnabled({chinese_enabled}) ?? {}]",
            !chinese_enabled
        );
        let install = build_script(&settings);
        let results = futures_util::future::join_all(targets.iter().map(|target| async {
            let (status_ready, language_ready) =
                app_cdp::evaluate_target::<(bool, bool)>(target, &refresh, false)
                    .await
                    .unwrap_or_default();
            let status_ready = status_ready
                || app_cdp::evaluate_target::<bool>(target, &install, false).await == Some(true);
            if !language_ready && chinese_enabled {
                app_cdp::evaluate_target::<bool>(target, LOCALIZATION_SCRIPT, false).await;
            }
            status_ready
        }))
        .await;
        mounted |= results.into_iter().any(|ready| ready);
    }
}

fn script_settings(locale: &str) -> serde_json::Value {
    let tr = |key| i18n::translate(locale, key, &[]);
    json!({
        "title": tr("common.codexUsage.title"),
        "context": tr("common.codexUsage.contextShort"),
        "token": tr("common.codexUsage.token"),
        "noData": tr("common.noData"),
        "missing": tr("settings.general.codexAppUiInjectionMissingLabel"),
        "responseSpeed": tr("common.codexUsage.responseSpeed"),
        "overallSpeed": tr("common.codexUsage.overallSpeedWithThinking"),
        "streamingSpeed": tr("common.codexUsage.streamingSpeed"),
        "firstResponse": tr("common.codexUsage.firstResponse"),
        "weekly": tr("settings.general.codexAppUiInjectionWeeklyLabel"),
        "fiveHour": tr("settings.general.codexAppUiInjectionFiveHourLabel"),
        "close": tr("common.close"),
        "unknown": tr("common.unknown"),
        "quota": tr("common.codexUsage.quotaTitle"),
        "resetsAt": tr("common.apiBudget.resetsAt"),
    })
}

fn build_script(settings: &serde_json::Value) -> String {
    let styles = json!([
        include_str!("codex_app_injection_footer.css"),
        include_str!("antigravity_composer_status.css"),
    ]);
    let script = include_str!("antigravity_composer_status.js");
    let data = include_str!("antigravity_composer_data.js");
    let context_usage = include_str!("composer_context_usage.js");
    format!("({script})({settings}, {data}, {styles}, {context_usage})")
}
