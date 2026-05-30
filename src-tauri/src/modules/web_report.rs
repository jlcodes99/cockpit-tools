//! 本地网页查询服务
//! 提供 /report?token=... 查询多平台账号用量摘要

use crate::models::codebuddy::CodebuddyAccount;
use serde::Serialize;
use serde_json::Value;
use std::sync::{OnceLock, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{timeout, Duration};
use url::Url;

use super::config::PORT_RANGE;
use crate::models::workbuddy::WorkbuddyAccount;
use crate::models::zed::ZedAccount;

const MAX_HTTP_REQUEST_BYTES: usize = 32 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_REFRESH_STALE_THRESHOLD_SECONDS: i64 = 10 * 60;
const NEXT_AUTH_REFRESH_TRIGGER_LABEL: &str =
    "Next AuthRefresh trigger time (only trigger if access to this page )";

static ACTUAL_REPORT_PORT: OnceLock<RwLock<Option<u16>>> = OnceLock::new();
static REPORT_REFRESH_STATE: OnceLock<RwLock<ReportRefreshState>> = OnceLock::new();
static REPORT_REFRESH_LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
enum ReportFormat {
    Markdown,
    Yaml,
}

#[derive(Debug, Clone, Serialize)]
struct ReportRow {
    service: String,
    account: String,
    metric: String,
    used: String,
    remaining: String,
    reset_cycle: String,
    status: String,
    note: String,
}

#[derive(Debug, Clone)]
struct ReportMeta {
    generated_at: String,
    data_collected_at: String,
    data_collected_note: Option<String>,
    data_delayed: String,
    next_auth_refresh_trigger_time: String,
}

#[derive(Debug, Default, Clone)]
struct ReportRefreshState {
    data_collected_at: Option<chrono::DateTime<chrono::Utc>>,
    last_auth_trigger_at: Option<chrono::DateTime<chrono::Utc>>,
    last_auth_trigger_note: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ServiceRefreshPolicy {
    key: &'static str,
    interval_minutes: i32,
}

fn report_port_state() -> &'static RwLock<Option<u16>> {
    ACTUAL_REPORT_PORT.get_or_init(|| RwLock::new(None))
}

fn report_refresh_state() -> &'static RwLock<ReportRefreshState> {
    REPORT_REFRESH_STATE.get_or_init(|| RwLock::new(ReportRefreshState::default()))
}

fn report_refresh_lock() -> &'static AsyncMutex<()> {
    REPORT_REFRESH_LOCK.get_or_init(|| AsyncMutex::new(()))
}

fn set_actual_port(port: Option<u16>) {
    if let Ok(mut guard) = report_port_state().write() {
        *guard = port;
    }
}

pub fn get_actual_port() -> Option<u16> {
    report_port_state().read().ok().and_then(|guard| *guard)
}

fn build_service_refresh_policies(cfg: &super::config::UserConfig) -> Vec<ServiceRefreshPolicy> {
    vec![
        ServiceRefreshPolicy {
            key: "antigravity",
            interval_minutes: cfg.auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "codex",
            interval_minutes: cfg.codex_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "ghcp",
            interval_minutes: cfg.ghcp_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "windsurf",
            interval_minutes: cfg.windsurf_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "kiro",
            interval_minutes: cfg.kiro_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "cursor",
            interval_minutes: cfg.cursor_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "gemini",
            interval_minutes: cfg.gemini_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "codebuddy",
            interval_minutes: cfg.codebuddy_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "codebuddy_cn",
            interval_minutes: cfg.codebuddy_cn_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "qoder",
            interval_minutes: cfg.qoder_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "trae",
            interval_minutes: cfg.trae_auto_refresh_minutes,
        },
        ServiceRefreshPolicy {
            key: "zed",
            interval_minutes: cfg.zed_auto_refresh_minutes,
        },
    ]
}

fn needs_auth_refresh_trigger(now: chrono::DateTime<chrono::Utc>) -> bool {
    let Ok(state) = report_refresh_state().read() else {
        return true;
    };

    let Some(collected_at) = state.data_collected_at else {
        return true;
    };

    now.signed_duration_since(collected_at).num_seconds() > AUTH_REFRESH_STALE_THRESHOLD_SECONDS
}

async fn run_refresh_for_service(policy: ServiceRefreshPolicy) -> Result<(), String> {
    match policy.key {
        "antigravity" => {
            super::account::refresh_all_quotas_logic(super::account::QuotaRefreshTrigger::Auto)
                .await
                .map(|_| ())
        }
        "codex" => super::codex_quota::refresh_all_quotas().await.map(|_| ()),
        "ghcp" => super::github_copilot_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "windsurf" => super::windsurf_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "kiro" => super::kiro_account::refresh_all_tokens().await.map(|_| ()),
        "cursor" => super::cursor_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "gemini" => super::gemini_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "codebuddy" => super::codebuddy_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "codebuddy_cn" => super::codebuddy_cn_account::refresh_all_tokens()
            .await
            .map(|_| ()),
        "qoder" => super::qoder_oauth::refresh_all_accounts_from_openapi()
            .await
            .map(|_| ()),
        "trae" => super::trae_account::refresh_all_tokens().await.map(|_| ()),
        "zed" => super::zed_account::refresh_all_accounts().await.map(|_| ()),
        _ => Err(format!("未知服务: {}", policy.key)),
    }
}

async fn maybe_trigger_auth_refresh_check() {
    let now = chrono::Utc::now();
    if !needs_auth_refresh_trigger(now) {
        return;
    }

    let _lock = report_refresh_lock().lock().await;
    let check_started_at = chrono::Utc::now();
    if !needs_auth_refresh_trigger(check_started_at) {
        return;
    }

    let cfg = super::config::get_user_config();
    let due_services = build_service_refresh_policies(&cfg)
        .into_iter()
        .filter(|policy| policy.interval_minutes > 0)
        .collect::<Vec<_>>();

    if let Ok(mut state) = report_refresh_state().write() {
        state.last_auth_trigger_at = Some(check_started_at);
        state.last_auth_trigger_note = None;
    }

    let mut failed_services: Vec<&'static str> = Vec::new();
    for policy in due_services {
        match run_refresh_for_service(policy).await {
            Ok(()) => {}
            Err(err) => {
                super::logger::log_warn(&format!(
                    "[WebReport] AuthRefresh check 刷新失败: service={}, error={}",
                    policy.key, err
                ));
                failed_services.push(policy.key);
            }
        }
    }

    let failure_note = if failed_services.is_empty() {
        None
    } else {
        Some(
            failed_services
                .into_iter()
                .map(|service| format!("{}刷新失败,该服务数据为历史数据", service))
                .collect::<Vec<_>>()
                .join("; "),
        )
    };

    let finished_at = chrono::Utc::now();
    if let Ok(mut state) = report_refresh_state().write() {
        state.data_collected_at = Some(finished_at);
        state.last_auth_trigger_note = failure_note;
    }
}

fn format_elapsed_hours_minutes(seconds: i64) -> String {
    let total_minutes = (seconds.max(0)) / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    format!("{}h{}m", hours, minutes)
}

fn format_next_auth_refresh_trigger_time(delayed_seconds: i64) -> String {
    let remaining = AUTH_REFRESH_STALE_THRESHOLD_SECONDS - delayed_seconds.max(0);
    if remaining <= 0 {
        return "0m".to_string();
    }
    let remaining_minutes = (remaining + 59) / 60;
    format!("{}m", remaining_minutes)
}

fn build_report_meta(generated_at: chrono::DateTime<chrono::Utc>) -> ReportMeta {
    let state = report_refresh_state().read().ok();
    let collected_at = state
        .as_ref()
        .and_then(|guard| guard.data_collected_at.clone())
        .unwrap_or(generated_at);
    let delayed_seconds = generated_at
        .signed_duration_since(collected_at)
        .num_seconds()
        .max(0);

    ReportMeta {
        generated_at: generated_at.to_rfc3339(),
        data_collected_at: collected_at.to_rfc3339(),
        data_collected_note: state
            .as_ref()
            .and_then(|guard| guard.last_auth_trigger_note.clone()),
        data_delayed: format_elapsed_hours_minutes(delayed_seconds),
        next_auth_refresh_trigger_time: format_next_auth_refresh_trigger_time(delayed_seconds),
    }
}

fn format_data_collected_at(meta: &ReportMeta) -> String {
    let Some(note) = meta
        .data_collected_note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return meta.data_collected_at.clone();
    };
    format!("{} ({})", meta.data_collected_at, note)
}

fn format_timestamp_human_local(value: &str) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) else {
        return value.to_string();
    };
    let local_dt = parsed.with_timezone(&chrono::Local);
    let local_time = local_dt.format("%Y-%m-%d %H:%M:%S").to_string();
    let offset = local_dt.format("%:z").to_string();
    let tz_name = local_dt.format("%Z").to_string();
    if tz_name.is_empty() {
        format!("{} {}", local_time, offset)
    } else {
        format!("{} {} ({})", local_time, offset, tz_name)
    }
}

fn format_data_collected_at_human(meta: &ReportMeta) -> String {
    let base = format_timestamp_human_local(&meta.data_collected_at);
    let Some(note) = meta
        .data_collected_note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return base;
    };
    format!("{} ({})", base, note)
}

pub async fn start_server() {
    let cfg = super::config::get_user_config();
    if !cfg.report_enabled {
        set_actual_port(None);
        super::logger::log_info("[WebReport] 网页查询服务未启用，跳过启动");
        return;
    }

    let token = cfg.report_token.trim().to_string();
    if token.is_empty() {
        set_actual_port(None);
        super::logger::log_warn("[WebReport] 配置了启用但 token 为空，网页查询服务未启动");
        return;
    }

    let preferred_port = cfg.report_port;
    let mut port = preferred_port;
    let mut listener = None;

    for attempt in 0..PORT_RANGE {
        let addr = format!("0.0.0.0:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(bound) => {
                listener = Some(bound);
                if attempt > 0 {
                    super::logger::log_info(&format!(
                        "[WebReport] 配置端口 {} 被占用，已切换至 {}",
                        preferred_port, port
                    ));
                }
                break;
            }
            Err(err) => {
                if attempt < PORT_RANGE - 1 {
                    port += 1;
                } else {
                    super::logger::log_error(&format!(
                        "[WebReport] 无法绑定端口 ({}-{})，最后错误: {}",
                        preferred_port,
                        preferred_port + PORT_RANGE - 1,
                        err
                    ));
                    set_actual_port(None);
                    return;
                }
            }
        }
    }

    let listener = match listener {
        Some(v) => v,
        None => {
            set_actual_port(None);
            return;
        }
    };

    set_actual_port(Some(port));
    super::logger::log_info(&format!(
        "[WebReport] 网页查询服务已启动: http://0.0.0.0:{}/report?token=***",
        port
    ));

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, port).await {
                super::logger::log_warn(&format!("[WebReport] 请求处理失败 {}: {}", addr, err));
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, port: u16) -> Result<(), String> {
    let raw_request = read_http_request(&mut stream).await?;
    let (method, target) = parse_request_target(&raw_request)?;

    if method.eq_ignore_ascii_case("OPTIONS") {
        write_response(&mut stream, "200 OK", "text/plain; charset=utf-8", "").await?;
        return Ok(());
    }

    if !method.eq_ignore_ascii_case("GET") {
        write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "Only GET is allowed",
        )
        .await?;
        return Ok(());
    }

    let parsed_url = parse_request_url(&target, port)?;
    if parsed_url.path() != "/report" {
        write_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not Found",
        )
        .await?;
        return Ok(());
    }

    let request_token = parsed_url
        .query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let config_token = super::config::get_user_config()
        .report_token
        .trim()
        .to_string();
    if config_token.is_empty() || request_token != config_token {
        write_response(
            &mut stream,
            "401 Unauthorized",
            "text/plain; charset=utf-8",
            "Unauthorized",
        )
        .await?;
        return Ok(());
    }

    let format = parsed_url
        .query_pairs()
        .find(|(key, _)| key == "format")
        .map(|(_, value)| value.to_string())
        .unwrap_or_else(|| "md".to_string());
    let render = parsed_url
        .query_pairs()
        .find(|(key, _)| key == "render")
        .map(|(_, value)| parse_bool_query(value.as_ref()))
        .unwrap_or(false);
    let report_format = if format.eq_ignore_ascii_case("yaml") || format.eq_ignore_ascii_case("yml")
    {
        ReportFormat::Yaml
    } else {
        ReportFormat::Markdown
    };

    maybe_trigger_auth_refresh_check().await;
    let generated_at = chrono::Utc::now();
    let meta = build_report_meta(generated_at);
    let mut rows = build_report_rows();
    rows.sort_by(|left, right| {
        left.service
            .cmp(&right.service)
            .then(left.account.cmp(&right.account))
            .then(left.metric.cmp(&right.metric))
    });

    let (content_type, body) = if render {
        ("text/html; charset=utf-8", render_html(&meta, &rows))
    } else {
        match report_format {
            ReportFormat::Markdown => (
                "text/markdown; charset=utf-8",
                render_markdown(&meta, &rows),
            ),
            ReportFormat::Yaml => (
                "application/x-yaml; charset=utf-8",
                render_yaml(&meta, &rows),
            ),
        }
    };

    write_response(&mut stream, "200 OK", content_type, &body).await?;
    Ok(())
}

async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.as_bytes().len()
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|err| format!("写响应头失败: {}", err))?;
    stream
        .write_all(body.as_bytes())
        .await
        .map_err(|err| format!("写响应体失败: {}", err))?;
    stream
        .flush()
        .await
        .map_err(|err| format!("刷新响应失败: {}", err))
}

async fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取请求超时".to_string())?
            .map_err(|err| format!("读取请求失败: {}", err))?;
        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n")
            || buffer.len() >= MAX_HTTP_REQUEST_BYTES
        {
            break;
        }
    }

    if buffer.is_empty() {
        return Err("请求为空".to_string());
    }

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn parse_request_target(request: &str) -> Result<(String, String), String> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "请求行为空".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "缺少 method".to_string())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "缺少 target".to_string())?
        .to_string();
    Ok((method, target))
}

fn parse_request_url(target: &str, port: u16) -> Result<Url, String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return Url::parse(target).map_err(|err| format!("URL 解析失败: {}", err));
    }
    Url::parse(&format!("http://127.0.0.1:{}{}", port, target))
        .map_err(|err| format!("URL 解析失败: {}", err))
}

fn parse_bool_query(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "y"
    )
}

fn build_report_rows() -> Vec<ReportRow> {
    let mut rows = Vec::new();
    append_antigravity_rows(&mut rows);
    append_codex_rows(&mut rows);
    append_github_copilot_rows(&mut rows);
    append_windsurf_rows(&mut rows);
    append_kiro_rows(&mut rows);
    append_cursor_rows(&mut rows);
    append_gemini_rows(&mut rows);
    append_codebuddy_rows(
        &mut rows,
        "CodeBuddy",
        super::codebuddy_account::list_accounts(),
    );
    append_codebuddy_rows(
        &mut rows,
        "CodeBuddy CN",
        super::codebuddy_cn_account::list_accounts(),
    );
    append_qoder_rows(&mut rows);
    append_trae_rows(&mut rows);
    append_workbuddy_rows(&mut rows);
    append_zed_rows(&mut rows);

    if rows.is_empty() {
        rows.push(make_row(
            "System",
            "-",
            "-",
            "-",
            "-",
            "-",
            "normal",
            "No accounts available",
        ));
    }

    rows
}

fn append_antigravity_rows(rows: &mut Vec<ReportRow>) {
    match super::account::list_accounts() {
        Ok(accounts) => {
            for account in accounts {
                let status = if account.disabled {
                    "disabled"
                } else {
                    "normal"
                };
                let account_name = account.email.clone();
                if let Some(quota) = account.quota {
                    if quota.models.is_empty() {
                        rows.push(make_row(
                            "Antigravity IDE",
                            &account_name,
                            "Models",
                            "-",
                            "-",
                            "-",
                            status,
                            "Quota exists but model list is empty",
                        ));
                        continue;
                    }

                    for model in quota.models {
                        let remaining = clamp_percent(model.percentage as f64);
                        let used = 100.0 - remaining;
                        let metric = model
                            .display_name
                            .clone()
                            .unwrap_or_else(|| model.name.clone());
                        let mut note = String::new();
                        if let Some(reason) = account.disabled_reason.as_deref() {
                            note = reason.to_string();
                        }
                        rows.push(make_row(
                            "Antigravity IDE",
                            &account_name,
                            &metric,
                            &percent_text(used),
                            &percent_text(remaining),
                            &normalize_reset_text(&model.reset_time),
                            status,
                            &note,
                        ));
                    }
                } else {
                    rows.push(make_row(
                        "Antigravity IDE",
                        &account_name,
                        "Models",
                        "-",
                        "-",
                        "-",
                        status,
                        "Quota not fetched yet",
                    ));
                }
            }
        }
        Err(err) => rows.push(make_row(
            "Antigravity IDE",
            "-",
            "-",
            "-",
            "-",
            "-",
            "error",
            &err,
        )),
    }
}

fn append_codex_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::codex_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let mut status = "normal".to_string();
        let mut note = String::new();
        if let Some(err) = account.quota_error.as_ref() {
            status = "quota_error".to_string();
            note = err.message.clone();
        }

        if let Some(quota) = account.quota {
            let main_label = quota
                .hourly_window_minutes
                .map(|mins| format!("Main window ({})", format_minutes_natural(mins)))
                .unwrap_or_else(|| "Main window".to_string());
            let weekly_label = quota
                .weekly_window_minutes
                .map(|mins| format!("Weekly window ({})", format_minutes_natural(mins)))
                .unwrap_or_else(|| "Weekly window".to_string());

            let main_remaining = clamp_percent(quota.hourly_percentage as f64);
            let weekly_remaining = clamp_percent(quota.weekly_percentage as f64);

            rows.push(make_row(
                "Codex",
                &account_name,
                &main_label,
                &percent_text(100.0 - main_remaining),
                &percent_text(main_remaining),
                &format_unix_timestamp(quota.hourly_reset_time),
                &status,
                &note,
            ));
            rows.push(make_row(
                "Codex",
                &account_name,
                &weekly_label,
                &percent_text(100.0 - weekly_remaining),
                &percent_text(weekly_remaining),
                &format_unix_timestamp(quota.weekly_reset_time),
                &status,
                &note,
            ));
        } else {
            rows.push(make_row(
                "Codex",
                &account_name,
                "Quota",
                "-",
                "-",
                "-",
                &status,
                if note.is_empty() {
                    "Quota not fetched yet"
                } else {
                    &note
                },
            ));
        }
    }
}

fn append_github_copilot_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::github_copilot_account::list_accounts();
    for account in accounts {
        let account_name = account
            .github_email
            .clone()
            .unwrap_or_else(|| account.github_login.clone());
        let reset = pick_copilot_reset_text(
            account.copilot_limited_user_reset_date,
            account.copilot_quota_reset_date.as_deref(),
        );
        let mut pushed = 0usize;
        if let Some(snapshots) = account.copilot_quota_snapshots.as_ref() {
            pushed = append_copilot_snapshot_rows(
                rows,
                "GitHub Copilot",
                &account_name,
                snapshots,
                &reset,
                "normal",
            );
        }

        if pushed == 0 {
            rows.push(make_row(
                "GitHub Copilot",
                &account_name,
                "Quota",
                "-",
                "-",
                &reset,
                "normal",
                "Quota snapshot unavailable",
            ));
        }
    }
}

fn append_windsurf_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::windsurf_account::list_accounts();
    for account in accounts {
        let account_name = account
            .github_email
            .clone()
            .unwrap_or_else(|| account.github_login.clone());
        let reset = pick_copilot_reset_text(
            account.copilot_limited_user_reset_date,
            account.copilot_quota_reset_date.as_deref(),
        );
        let mut pushed = 0usize;
        if let Some(snapshots) = account.copilot_quota_snapshots.as_ref() {
            pushed = append_copilot_snapshot_rows(
                rows,
                "Windsurf",
                &account_name,
                snapshots,
                &reset,
                "normal",
            );
        }

        if pushed == 0 {
            pushed = append_windsurf_plan_status_rows(
                rows,
                &account_name,
                account.copilot_quota_snapshots.as_ref(),
                account.windsurf_user_status.as_ref(),
                account.windsurf_plan_status.as_ref(),
                &reset,
                "normal",
            );
        }

        if pushed == 0 {
            rows.push(make_row(
                "Windsurf",
                &account_name,
                "Quota",
                "-",
                "-",
                &reset,
                "normal",
                "Quota snapshot unavailable",
            ));
        }
    }
}

fn append_windsurf_plan_status_rows(
    rows: &mut Vec<ReportRow>,
    account: &str,
    snapshots: Option<&Value>,
    windsurf_user_status: Option<&Value>,
    windsurf_plan_status: Option<&Value>,
    reset_fallback: &str,
    status: &str,
) -> usize {
    let mut candidates: Vec<&Value> = Vec::new();

    if let Some(snapshot_value) = snapshots {
        if let Some(plan_status) = get_nested_value(snapshot_value, &["windsurfPlanStatus"]) {
            candidates.push(plan_status);
        }
        if let Some(plan_status) =
            get_nested_value(snapshot_value, &["windsurfUserStatus", "planStatus"])
        {
            candidates.push(plan_status);
        }
    }

    if let Some(user_status) = windsurf_user_status {
        if let Some(plan_status) = get_nested_value(user_status, &["userStatus", "planStatus"]) {
            candidates.push(plan_status);
        }
        if let Some(plan_status) = get_nested_value(user_status, &["planStatus"]) {
            candidates.push(plan_status);
        }
    }

    if let Some(plan_status) = windsurf_plan_status {
        candidates.push(plan_status);
    }

    for candidate in candidates {
        let appended = append_windsurf_plan_status_candidate_rows(
            rows,
            account,
            candidate,
            reset_fallback,
            status,
        );
        if appended > 0 {
            return appended;
        }
    }

    0
}

fn append_windsurf_plan_status_candidate_rows(
    rows: &mut Vec<ReportRow>,
    account: &str,
    plan_status: &Value,
    reset_fallback: &str,
    status: &str,
) -> usize {
    if resolve_windsurf_billing_strategy(plan_status).as_deref() == Some("quota") {
        let daily_used_percent = pick_first_number(
            plan_status,
            &[
                &["dailyQuotaRemainingPercent"],
                &["daily_quota_remaining_percent"],
            ],
        )
        .map(|value| clamp_percent(100.0 - value))
        .or(Some(100.0));
        let weekly_used_percent = pick_first_number(
            plan_status,
            &[
                &["weeklyQuotaRemainingPercent"],
                &["weekly_quota_remaining_percent"],
            ],
        )
        .map(|value| clamp_percent(100.0 - value))
        .or(Some(100.0));
        let daily_reset = pick_first_reset_value(
            plan_status,
            &[&["dailyQuotaResetAtUnix"], &["daily_quota_reset_at_unix"]],
            reset_fallback,
        );
        let weekly_reset = pick_first_reset_value(
            plan_status,
            &[&["weeklyQuotaResetAtUnix"], &["weekly_quota_reset_at_unix"]],
            reset_fallback,
        );
        let overage_balance_micros = pick_first_number(
            plan_status,
            &[&["overageBalanceMicros"], &["overage_balance_micros"]],
        );

        let mut quota_count = 0usize;
        quota_count += push_windsurf_quota_percent_row(
            rows,
            account,
            "Daily quota usage",
            daily_used_percent,
            &daily_reset,
            status,
        );
        quota_count += push_windsurf_quota_percent_row(
            rows,
            account,
            "Weekly quota usage",
            weekly_used_percent,
            &weekly_reset,
            status,
        );
        rows.push(make_row(
            "Windsurf",
            account,
            "Extra usage balance",
            "-",
            &format_micros_usd(overage_balance_micros.unwrap_or(0.0)),
            reset_fallback,
            status,
            "",
        ));
        quota_count += 1;
        return quota_count;
    }

    let prompt_total = pick_first_number(
        plan_status,
        &[
            &["availablePromptCredits"],
            &["available_prompt_credits"],
            &["promptCredits"],
            &["prompt_credits"],
        ],
    );
    let prompt_used = pick_first_number(
        plan_status,
        &[
            &["usedPromptCredits"],
            &["used_prompt_credits"],
            &["promptCreditsUsed"],
            &["prompt_credits_used"],
            &["consumedPromptCredits"],
            &["consumed_prompt_credits"],
        ],
    );

    let flow_total = pick_first_number(
        plan_status,
        &[
            &["availableFlowCredits"],
            &["available_flow_credits"],
            &["flowCredits"],
            &["flow_credits"],
        ],
    );
    let flow_used = pick_first_number(
        plan_status,
        &[
            &["usedFlowCredits"],
            &["used_flow_credits"],
            &["flowCreditsUsed"],
            &["flow_credits_used"],
            &["consumedFlowCredits"],
            &["consumed_flow_credits"],
        ],
    );

    let reset = pick_first_reset_value(
        plan_status,
        &[
            &["planEnd"],
            &["plan_end"],
            &["cycleEnd"],
            &["cycle_end"],
            &["resetAt"],
            &["reset_at"],
        ],
        reset_fallback,
    );

    let mut count = 0usize;
    count += push_windsurf_credit_row(
        rows,
        account,
        "Prompt",
        prompt_total,
        prompt_used,
        &reset,
        status,
    );
    count += push_windsurf_credit_row(rows, account, "Flow", flow_total, flow_used, &reset, status);
    count
}

fn resolve_windsurf_billing_strategy(plan_status: &Value) -> Option<String> {
    let raw = pick_first_string(
        plan_status,
        &[
            &["billingStrategy"],
            &["billing_strategy"],
            &["planInfo", "billingStrategy"],
            &["planInfo", "billing_strategy"],
        ],
    )?;
    let normalized = raw.trim().to_ascii_lowercase();
    let canonical = normalized
        .trim_start_matches("billing_strategy_")
        .trim_start_matches("billing-strategy-")
        .trim_start_matches("billingstrategy")
        .trim_matches('_')
        .trim_matches('-')
        .to_string();

    if canonical == "quota" {
        return Some("quota".to_string());
    }
    if canonical.contains("credit") {
        return Some("credits".to_string());
    }
    Some(canonical)
}

fn push_windsurf_quota_percent_row(
    rows: &mut Vec<ReportRow>,
    account: &str,
    metric: &str,
    used_percent: Option<f64>,
    reset: &str,
    status: &str,
) -> usize {
    let (used, remaining, note) = if let Some(used_raw) = used_percent {
        let used = clamp_percent(used_raw);
        let remaining = clamp_percent(100.0 - used);
        (used, remaining, "")
    } else {
        return 0;
    };

    rows.push(make_row(
        "Windsurf",
        account,
        metric,
        &percent_text(used),
        &percent_text(remaining),
        reset,
        status,
        note,
    ));
    1
}

fn push_windsurf_credit_row(
    rows: &mut Vec<ReportRow>,
    account: &str,
    metric: &str,
    total: Option<f64>,
    used: Option<f64>,
    reset: &str,
    status: &str,
) -> usize {
    let Some(total) = total else {
        return 0;
    };
    if total <= 0.0 {
        return 0;
    }

    if let Some(used_value) = used {
        let used_normalized = used_value.max(0.0);
        let remaining = (total - used_normalized).max(0.0);
        let used_percent = clamp_percent((used_normalized / total) * 100.0);
        rows.push(make_row(
            "Windsurf",
            account,
            metric,
            &format!(
                "{:.0}/{:.0} ({})",
                used_normalized,
                total,
                percent_text(used_percent)
            ),
            &format!("{:.0}", remaining),
            reset,
            status,
            "",
        ));
    } else {
        rows.push(make_row(
            "Windsurf",
            account,
            metric,
            "-",
            &format!("{:.0}", total),
            reset,
            status,
            "Used credits unavailable",
        ));
    }

    1
}

fn pick_first_reset_value(value: &Value, paths: &[&[&str]], fallback: &str) -> String {
    for path in paths {
        let parsed = parse_reset_value(get_nested_value(value, path));
        if parsed != "-" {
            return parsed;
        }
    }

    normalize_reset_text(fallback)
}

fn append_kiro_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::kiro_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");
        let reset = format_unix_timestamp(account.usage_reset_at);
        let mut pushed = false;

        if let (Some(total), Some(used)) = (account.credits_total, account.credits_used) {
            if total > 0.0 {
                let used_percent = clamp_percent((used / total) * 100.0);
                let remaining = (total - used).max(0.0);
                rows.push(make_row(
                    "Kiro",
                    &account_name,
                    "Credits",
                    &format!("{:.2}/{:.2} ({})", used, total, percent_text(used_percent)),
                    &format!("{:.2}", remaining),
                    &reset,
                    status,
                    "",
                ));
                pushed = true;
            }
        }

        if let (Some(total), Some(used)) = (account.bonus_total, account.bonus_used) {
            if total > 0.0 {
                let used_percent = clamp_percent((used / total) * 100.0);
                let remaining = (total - used).max(0.0);
                rows.push(make_row(
                    "Kiro",
                    &account_name,
                    "Bonus credits",
                    &format!("{:.2}/{:.2} ({})", used, total, percent_text(used_percent)),
                    &format!("{:.2}", remaining),
                    &reset,
                    status,
                    "",
                ));
                pushed = true;
            }
        }

        if !pushed {
            rows.push(make_row(
                "Kiro",
                &account_name,
                "Usage",
                "-",
                "-",
                &reset,
                status,
                account
                    .status_reason
                    .as_deref()
                    .unwrap_or("Credits data unavailable"),
            ));
        }
    }
}

fn append_qoder_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::qoder_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let mut roots: Vec<&Value> = Vec::new();
        if let Some(raw) = account.auth_credit_usage_raw.as_ref() {
            roots.push(raw);
        }
        if let Some(raw) = account.auth_user_plan_raw.as_ref() {
            roots.push(raw);
        }
        if let Some(raw) = account.auth_user_info_raw.as_ref() {
            roots.push(raw);
        }

        let reset = pick_qoder_reset_value(&roots);
        let mut pushed = 0usize;
        pushed += push_qoder_bucket_row(
            rows,
            &account_name,
            &roots,
            &[&["userQuota"], &["user_quota"]],
            "User quota",
            &reset,
            account.plan_type.as_deref().unwrap_or(""),
        );
        pushed += push_qoder_bucket_row(
            rows,
            &account_name,
            &roots,
            &[&["addOnQuota"], &["addonQuota"], &["add_on_quota"]],
            "Add-on quota",
            &reset,
            account.plan_type.as_deref().unwrap_or(""),
        );

        if pushed == 0 {
            if let (Some(total), Some(used)) = (account.credits_total, account.credits_used) {
                if total > 0.0 {
                    let remaining = account.credits_remaining.unwrap_or((total - used).max(0.0));
                    let used_percent = account
                        .credits_usage_percent
                        .unwrap_or((used / total) * 100.0);
                    rows.push(make_row(
                        "Qoder",
                        &account_name,
                        "Credits",
                        &format!(
                            "{}/{} ({})",
                            format_number_compact(used),
                            format_number_compact(total),
                            percent_text(used_percent)
                        ),
                        &format_number_compact(remaining.max(0.0)),
                        &reset,
                        "normal",
                        account.plan_type.as_deref().unwrap_or(""),
                    ));
                    continue;
                }
            }

            rows.push(make_row(
                "Qoder",
                &account_name,
                "Usage",
                "-",
                "-",
                &reset,
                "normal",
                account
                    .plan_type
                    .as_deref()
                    .unwrap_or("Credits data unavailable"),
            ));
        }
    }
}

fn push_qoder_bucket_row(
    rows: &mut Vec<ReportRow>,
    account_name: &str,
    roots: &[&Value],
    bucket_paths: &[&[&str]],
    metric: &str,
    reset: &str,
    note: &str,
) -> usize {
    let mut used: Option<f64> = None;
    let mut total: Option<f64> = None;
    let mut remaining: Option<f64> = None;
    let mut percentage: Option<f64> = None;

    for root in roots {
        for path in bucket_paths {
            let Some(bucket) = get_nested_value(root, path) else {
                continue;
            };
            used = used.or(pick_first_number(
                bucket,
                &[&["used"], &["usage"], &["consumed"]],
            ));
            total = total.or(pick_first_number(
                bucket,
                &[&["total"], &["quota"], &["limit"]],
            ));
            remaining = remaining.or(pick_first_number(
                bucket,
                &[&["remaining"], &["available"], &["left"]],
            ));
            percentage = percentage.or(pick_first_number(
                bucket,
                &[&["percentage"], &["usagePercent"], &["usage_percentage"]],
            ));
        }
    }

    if percentage.is_none() {
        if let (Some(u), Some(t)) = (used, total) {
            if t > 0.0 {
                percentage = Some((u / t) * 100.0);
            }
        }
    }
    let used_text = if let (Some(u), Some(t)) = (used, total) {
        if t > 0.0 {
            let pct = percentage.unwrap_or((u / t) * 100.0);
            format!(
                "{}/{} ({})",
                format_number_compact(u),
                format_number_compact(t),
                percent_text(pct)
            )
        } else {
            "-".to_string()
        }
    } else if let Some(pct) = percentage {
        percent_text(pct)
    } else {
        "-".to_string()
    };

    let remaining_text = if let Some(v) = remaining {
        format_number_compact(v.max(0.0))
    } else if let (Some(t), Some(u)) = (total, used) {
        format_number_compact((t - u).max(0.0))
    } else if let Some(pct) = percentage {
        percent_text(100.0 - clamp_percent(pct))
    } else {
        "-".to_string()
    };

    if used_text == "-" && remaining_text == "-" {
        return 0;
    }

    rows.push(make_row(
        "Qoder",
        account_name,
        metric,
        &used_text,
        &remaining_text,
        reset,
        "normal",
        note,
    ));
    1
}

fn pick_qoder_reset_value(roots: &[&Value]) -> String {
    for root in roots {
        let value = pick_first_reset_value(
            root,
            &[&["expiresAt"], &["expires_at"], &["resetAt"], &["reset_at"]],
            "-",
        );
        if value != "-" {
            return value;
        }
    }
    "-".to_string()
}

fn append_cursor_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::cursor_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");

        if let Some(raw_usage) = account.cursor_usage_raw.as_ref() {
            let used_percent = pick_first_number(
                raw_usage,
                &[
                    &["individualUsage", "plan", "totalPercentUsed"],
                    &["individual_usage", "plan", "total_percent_used"],
                    &["planUsage", "totalPercentUsed"],
                    &["plan_usage", "total_percent_used"],
                ],
            );
            let fallback_used = compute_cursor_used_percent_from_amount(raw_usage);
            let resolved_used = used_percent.or(fallback_used);

            let reset =
                pick_first_string(raw_usage, &[&["billingCycleEnd"], &["billing_cycle_end"]])
                    .unwrap_or_else(|| "-".to_string());

            if let Some(percent) = resolved_used {
                let normalized = clamp_percent(percent);
                rows.push(make_row(
                    "Cursor",
                    &account_name,
                    "Plan usage",
                    &percent_text(normalized),
                    &percent_text(100.0 - normalized),
                    &normalize_reset_text(&reset),
                    status,
                    "",
                ));
                continue;
            }
        }

        rows.push(make_row(
            "Cursor",
            &account_name,
            "Usage",
            "-",
            "-",
            "-",
            status,
            account
                .status_reason
                .as_deref()
                .unwrap_or("Usage data unavailable"),
        ));
    }
}

fn append_gemini_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::gemini_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");
        let mut pushed = false;

        if let Some(raw_usage) = account.gemini_usage_raw.as_ref() {
            if let Some(buckets) =
                get_nested_value(raw_usage, &["buckets"]).and_then(Value::as_array)
            {
                for bucket in buckets {
                    let Some(model_id) = get_nested_value(bucket, &["modelId"])
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };

                    let Some(remaining_fraction) =
                        get_nested_value(bucket, &["remainingFraction"]).and_then(as_f64)
                    else {
                        continue;
                    };

                    let remaining = clamp_percent(remaining_fraction * 100.0);
                    let used = 100.0 - remaining;
                    let reset = parse_reset_value(get_nested_value(bucket, &["resetTime"]));

                    rows.push(make_row(
                        "Gemini",
                        &account_name,
                        model_id,
                        &percent_text(used),
                        &percent_text(remaining),
                        &reset,
                        status,
                        "",
                    ));
                    pushed = true;
                }
            }
        }

        if !pushed {
            rows.push(make_row(
                "Gemini",
                &account_name,
                "Usage",
                "-",
                "-",
                "-",
                status,
                account
                    .status_reason
                    .as_deref()
                    .unwrap_or("Usage data unavailable"),
            ));
        }
    }
}

fn append_codebuddy_rows(
    rows: &mut Vec<ReportRow>,
    service: &str,
    accounts: Vec<CodebuddyAccount>,
) {
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");
        let resources = extract_codebuddy_resources(&account);
        let mut pushed = false;

        for item in resources {
            let total = pick_number_in_item(
                item,
                &[
                    "CycleCapacitySizePrecise",
                    "CycleCapacitySize",
                    "CapacitySizePrecise",
                    "CapacitySize",
                ],
            );
            let remaining = pick_number_in_item(
                item,
                &[
                    "CycleCapacityRemainPrecise",
                    "CycleCapacityRemain",
                    "CapacityRemainPrecise",
                    "CapacityRemain",
                ],
            );

            let (Some(total), Some(remaining)) = (total, remaining) else {
                continue;
            };
            if total <= 0.0 {
                continue;
            }

            let used = (total - remaining).max(0.0);
            let used_percent = clamp_percent((used / total) * 100.0);
            let metric = pick_string_in_item(item, &["PackageName", "PackageCode"])
                .unwrap_or_else(|| "Package".to_string());
            let reset = pick_string_in_item(item, &["CycleEndTime", "ExpiredTime"])
                .unwrap_or_else(|| "-".to_string());

            rows.push(make_row(
                service,
                &account_name,
                &metric,
                &format!("{:.2}/{:.2} ({})", used, total, percent_text(used_percent)),
                &format!("{:.2}", remaining.max(0.0)),
                &normalize_reset_text(&reset),
                status,
                "",
            ));
            pushed = true;
        }

        if !pushed {
            let fallback_note = account
                .dosage_notify_zh
                .as_deref()
                .or(account.dosage_notify_en.as_deref())
                .or(account.status_reason.as_deref())
                .unwrap_or("Usage data unavailable");
            rows.push(make_row(
                service,
                &account_name,
                "Usage",
                "-",
                "-",
                "-",
                status,
                fallback_note,
            ));
        }
    }
}

fn append_workbuddy_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::workbuddy_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");
        let resources = extract_workbuddy_resources(&account);
        let mut pushed = false;

        for item in resources {
            let total = pick_number_in_item(
                item,
                &[
                    "CycleCapacitySizePrecise",
                    "CycleCapacitySize",
                    "CapacitySizePrecise",
                    "CapacitySize",
                ],
            );
            let remaining = pick_number_in_item(
                item,
                &[
                    "CycleCapacityRemainPrecise",
                    "CycleCapacityRemain",
                    "CapacityRemainPrecise",
                    "CapacityRemain",
                ],
            );

            let (Some(total), Some(remaining)) = (total, remaining) else {
                continue;
            };
            if total <= 0.0 {
                continue;
            }

            let used = (total - remaining).max(0.0);
            let used_percent = clamp_percent((used / total) * 100.0);
            let metric = pick_string_in_item(item, &["PackageName", "PackageCode"])
                .unwrap_or_else(|| "Package".to_string());
            let reset = pick_string_in_item(item, &["CycleEndTime", "ExpiredTime"])
                .unwrap_or_else(|| "-".to_string());

            rows.push(make_row(
                "Workbuddy",
                &account_name,
                &metric,
                &format!("{:.2}/{:.2} ({})", used, total, percent_text(used_percent)),
                &format!("{:.2}", remaining.max(0.0)),
                &normalize_reset_text(&reset),
                status,
                "",
            ));
            pushed = true;
        }

        if !pushed {
            let fallback_note = account
                .dosage_notify_zh
                .as_deref()
                .or(account.dosage_notify_en.as_deref())
                .or(account.status_reason.as_deref())
                .or(account.quota_query_last_error.as_deref())
                .unwrap_or("Usage data unavailable");
            rows.push(make_row(
                "Workbuddy",
                &account_name,
                "Usage",
                "-",
                "-",
                "-",
                status,
                fallback_note,
            ));
        }
    }
}

fn append_zed_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::zed_account::list_accounts();
    for account in accounts {
        let account_name = zed_display_name(&account);
        let reset = format_unix_timestamp(account.billing_period_end_at);
        let mut pushed = false;

        if let (Some(used_cents), Some(limit_cents)) = (
            account.token_spend_used_cents,
            account.token_spend_limit_cents,
        ) {
            if limit_cents > 0 {
                let used = used_cents.max(0) as f64 / 100.0;
                let total = limit_cents as f64 / 100.0;
                let remaining = account
                    .token_spend_remaining_cents
                    .unwrap_or(limit_cents.saturating_sub(used_cents))
                    .max(0) as f64
                    / 100.0;
                let used_percent = clamp_percent((used / total) * 100.0);

                rows.push(make_row(
                    "Zed",
                    &account_name,
                    "Token spend",
                    &format!(
                        "${:.2}/${:.2} ({})",
                        used,
                        total,
                        percent_text(used_percent)
                    ),
                    &format!("${:.2}", remaining),
                    &reset,
                    "normal",
                    "",
                ));
                pushed = true;
            }
        }

        let edit_used = account.edit_predictions_used.map(|value| value as f64);
        let edit_limit = parse_numeric_text(account.edit_predictions_limit_raw.as_deref());
        let edit_remaining = parse_numeric_text(account.edit_predictions_remaining_raw.as_deref());
        if let Some(total) = edit_limit {
            if total > 0.0 {
                let used = edit_used.unwrap_or(0.0).max(0.0);
                let remaining = edit_remaining.unwrap_or((total - used).max(0.0)).max(0.0);
                let used_percent = clamp_percent((used / total) * 100.0);
                rows.push(make_row(
                    "Zed",
                    &account_name,
                    "Edit Predictions",
                    &format!("{:.0}/{:.0} ({})", used, total, percent_text(used_percent)),
                    &format!("{:.0}", remaining),
                    &reset,
                    "normal",
                    "",
                ));
                pushed = true;
            }
        } else if account
            .edit_predictions_limit_raw
            .as_deref()
            .map(|value| value.trim().eq_ignore_ascii_case("unlimited"))
            == Some(true)
        {
            rows.push(make_row(
                "Zed",
                &account_name,
                "Edit Predictions",
                "-",
                "Unlimited",
                &reset,
                "normal",
                "",
            ));
            pushed = true;
        }

        if account.has_overdue_invoices.is_some() {
            rows.push(make_row(
                "Zed",
                &account_name,
                "Overdue invoices",
                "-",
                if account.has_overdue_invoices == Some(true) {
                    "Yes"
                } else {
                    "No"
                },
                &reset,
                "normal",
                account.subscription_status.as_deref().unwrap_or(""),
            ));
            pushed = true;
        }

        if !pushed {
            rows.push(make_row(
                "Zed",
                &account_name,
                "Usage",
                "-",
                "-",
                &reset,
                "normal",
                account
                    .subscription_status
                    .as_deref()
                    .unwrap_or("Usage data unavailable"),
            ));
        }
    }
}

fn append_trae_rows(rows: &mut Vec<ReportRow>) {
    let accounts = super::trae_account::list_accounts();
    for account in accounts {
        let account_name = account.email.clone();
        let status = account.status.as_deref().unwrap_or("normal");
        let reset_fallback = format_unix_timestamp(account.plan_reset_at);
        let mut pushed = 0usize;

        if let Some(pack) = pick_preferred_trae_pack(account.trae_usage_raw.as_ref()) {
            let basic_used =
                pick_first_number(pack, &[&["usage", "basic_usage_amount"]]).unwrap_or(0.0);
            let basic_total = pick_first_number(
                pack,
                &[&["entitlement_base_info", "quota", "basic_usage_limit"]],
            );
            if let Some(total) = basic_total {
                if total > 0.0 {
                    let used = basic_used.max(0.0);
                    let remaining = (total - used).max(0.0);
                    let used_percent = clamp_percent((used / total) * 100.0);
                    let reset =
                        pick_trae_pack_reset(pack).unwrap_or_else(|| reset_fallback.clone());
                    rows.push(make_row(
                        "Trae",
                        &account_name,
                        "Basic usage",
                        &format!(
                            "{}/{} ({})",
                            format_number_compact(used),
                            format_number_compact(total),
                            percent_text(used_percent)
                        ),
                        &format_number_compact(remaining),
                        &reset,
                        status,
                        account.plan_type.as_deref().unwrap_or(""),
                    ));
                    pushed += 1;
                }
            }

            let bonus_used =
                pick_first_number(pack, &[&["usage", "bonus_usage_amount"]]).unwrap_or(0.0);
            let bonus_total = pick_first_number(
                pack,
                &[&["entitlement_base_info", "quota", "bonus_usage_limit"]],
            );
            if let Some(total) = bonus_total {
                if total > 0.0 {
                    let used = bonus_used.max(0.0);
                    let remaining = (total - used).max(0.0);
                    let used_percent = clamp_percent((used / total) * 100.0);
                    let reset =
                        pick_trae_pack_reset(pack).unwrap_or_else(|| reset_fallback.clone());
                    rows.push(make_row(
                        "Trae",
                        &account_name,
                        "Bonus usage",
                        &format!(
                            "{}/{} ({})",
                            format_number_compact(used),
                            format_number_compact(total),
                            percent_text(used_percent)
                        ),
                        &format_number_compact(remaining),
                        &reset,
                        status,
                        account.plan_type.as_deref().unwrap_or(""),
                    ));
                    pushed += 1;
                }
            }

            if let Some(pay_go_amount) = pick_first_number(pack, &[&["usage", "pay_go_amount"]]) {
                if pay_go_amount > 0.0 {
                    rows.push(make_row(
                        "Trae",
                        &account_name,
                        "On-Demand Usage",
                        &format!("${:.2}", pay_go_amount),
                        "-",
                        &reset_fallback,
                        status,
                        account.plan_type.as_deref().unwrap_or(""),
                    ));
                    pushed += 1;
                }
            }
        }

        if pushed == 0 {
            let note = account
                .plan_type
                .as_deref()
                .or(account.status_reason.as_deref())
                .unwrap_or("Usage data unavailable");

            rows.push(make_row(
                "Trae",
                &account_name,
                "Plan",
                "-",
                "-",
                &reset_fallback,
                status,
                note,
            ));
        }
    }
}

fn pick_preferred_trae_pack<'a>(usage_raw: Option<&'a Value>) -> Option<&'a Value> {
    let packs = get_nested_value(usage_raw?, &["user_entitlement_pack_list"])?.as_array()?;
    let priority = [6.0_f64, 4.0, 1.0, 9.0, 8.0, 0.0];

    for product_type in priority {
        for pack in packs {
            let current_type = pick_first_number(
                pack,
                &[
                    &["entitlement_base_info", "product_type"],
                    &["product_type"],
                ],
            );
            if current_type == Some(3.0) {
                continue;
            }
            if current_type == Some(product_type) {
                return Some(pack);
            }
        }
    }

    packs.first()
}

fn pick_trae_pack_reset(pack: &Value) -> Option<String> {
    let raw = pick_first_number(pack, &[&["entitlement_base_info", "end_time"]])?;
    if raw <= 0.0 {
        return None;
    }
    let ts = (raw as i64).saturating_add(1);
    let text = format_unix_timestamp(Some(ts));
    if text == "-" {
        None
    } else {
        Some(text)
    }
}

fn append_copilot_snapshot_rows(
    rows: &mut Vec<ReportRow>,
    service: &str,
    account: &str,
    snapshots: &Value,
    reset: &str,
    status: &str,
) -> usize {
    let metrics = [
        ("completions", "Completions"),
        ("chat", "Chat"),
        ("premium_interactions", "Premium"),
    ];
    let mut count = 0usize;

    for (key, label) in metrics {
        let Some(snapshot) = get_nested_value(snapshots, &[key]) else {
            continue;
        };
        let Some(remaining) = get_nested_value(snapshot, &["percent_remaining"]).and_then(as_f64)
        else {
            continue;
        };

        let remaining = clamp_percent(remaining);
        rows.push(make_row(
            service,
            account,
            label,
            &percent_text(100.0 - remaining),
            &percent_text(remaining),
            reset,
            status,
            "",
        ));
        count += 1;
    }

    count
}

fn pick_copilot_reset_text(reset_unix: Option<i64>, reset_iso: Option<&str>) -> String {
    if let Some(ts) = reset_unix {
        let formatted = format_unix_timestamp(Some(ts));
        if formatted != "-" {
            return formatted;
        }
    }

    reset_iso
        .map(normalize_reset_text)
        .filter(|text| text != "-")
        .unwrap_or_else(|| "-".to_string())
}

fn extract_codebuddy_resources(account: &CodebuddyAccount) -> Vec<&serde_json::Map<String, Value>> {
    let mut out = Vec::new();

    if let Some(quota_raw) = account.quota_raw.as_ref() {
        if let Some(list) = get_nested_value(
            quota_raw,
            &["userResource", "data", "Response", "Data", "Accounts"],
        )
        .and_then(Value::as_array)
        {
            for item in list {
                if let Some(obj) = item.as_object() {
                    out.push(obj);
                }
            }
        }
    }

    if out.is_empty() {
        if let Some(usage_raw) = account.usage_raw.as_ref() {
            if let Some(list) =
                get_nested_value(usage_raw, &["data", "Response", "Data", "Accounts"])
                    .and_then(Value::as_array)
            {
                for item in list {
                    if let Some(obj) = item.as_object() {
                        out.push(obj);
                    }
                }
            }
        }
    }

    out
}

fn extract_workbuddy_resources(account: &WorkbuddyAccount) -> Vec<&serde_json::Map<String, Value>> {
    let mut out = Vec::new();

    if let Some(quota_raw) = account.quota_raw.as_ref() {
        if let Some(list) = get_nested_value(
            quota_raw,
            &["userResource", "data", "Response", "Data", "Accounts"],
        )
        .and_then(Value::as_array)
        {
            for item in list {
                if let Some(obj) = item.as_object() {
                    out.push(obj);
                }
            }
        }
    }

    if out.is_empty() {
        if let Some(usage_raw) = account.usage_raw.as_ref() {
            if let Some(list) =
                get_nested_value(usage_raw, &["data", "Response", "Data", "Accounts"])
                    .and_then(Value::as_array)
            {
                for item in list {
                    if let Some(obj) = item.as_object() {
                        out.push(obj);
                    }
                }
            }
        }
    }

    out
}

fn zed_display_name(account: &ZedAccount) -> String {
    if let Some(name) = account
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if !account.github_login.trim().is_empty() {
        return account.github_login.clone();
    }
    if !account.user_id.trim().is_empty() {
        return account.user_id.clone();
    }
    account.id.clone()
}

fn parse_numeric_text(value: Option<&str>) -> Option<f64> {
    value.and_then(|raw| raw.trim().parse::<f64>().ok())
}

fn pick_number_in_item(item: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = item.get(*key).and_then(as_f64) {
            return Some(value);
        }
    }
    None
}

fn pick_string_in_item(item: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = item.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn compute_cursor_used_percent_from_amount(raw_usage: &Value) -> Option<f64> {
    let used = pick_first_number(
        raw_usage,
        &[
            &["individualUsage", "plan", "used"],
            &["individual_usage", "plan", "used"],
            &["planUsage", "used"],
            &["plan_usage", "used"],
            &["individualUsage", "plan", "totalSpend"],
            &["individual_usage", "plan", "total_spend"],
        ],
    )?;
    let limit = pick_first_number(
        raw_usage,
        &[
            &["individualUsage", "plan", "limit"],
            &["individual_usage", "plan", "limit"],
            &["planUsage", "limit"],
            &["plan_usage", "limit"],
        ],
    )?;
    if limit <= 0.0 {
        return None;
    }
    Some((used / limit) * 100.0)
}

fn pick_first_number(value: &Value, paths: &[&[&str]]) -> Option<f64> {
    for path in paths {
        if let Some(parsed) = get_nested_value(value, path).and_then(as_f64) {
            return Some(parsed);
        }
    }
    None
}

fn pick_first_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    for path in paths {
        if let Some(text) = get_nested_value(value, path).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn get_nested_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn make_row(
    service: &str,
    account: &str,
    metric: &str,
    used: &str,
    remaining: &str,
    reset_cycle: &str,
    status: &str,
    note: &str,
) -> ReportRow {
    ReportRow {
        service: normalize_text(service),
        account: normalize_text(account),
        metric: normalize_text(metric),
        used: normalize_text(used),
        remaining: normalize_text(remaining),
        reset_cycle: normalize_text(reset_cycle),
        status: normalize_text(status),
        note: normalize_text(note),
    }
}

fn normalize_text(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn clamp_percent(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 100.0)
}

fn format_micros_usd(value: f64) -> String {
    if !value.is_finite() {
        return "-".to_string();
    }
    let usd = value / 1_000_000.0;
    format!("${:.2}", usd)
}

fn format_number_compact(value: f64) -> String {
    if !value.is_finite() {
        return "-".to_string();
    }
    let normalized = if value.abs() < 0.000_001 { 0.0 } else { value };
    if (normalized.fract()).abs() < f64::EPSILON {
        format!("{:.0}", normalized)
    } else {
        format!("{:.2}", normalized)
    }
}

fn percent_text(value: f64) -> String {
    format!("{}%", value.round() as i64)
}

fn normalize_reset_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "-".to_string();
    }

    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return parsed.with_timezone(&chrono::Utc).to_rfc3339();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        let parsed = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc);
        return parsed.to_rfc3339();
    }

    trimmed.to_string()
}

fn parse_reset_value(value: Option<&Value>) -> String {
    let Some(raw) = value else {
        return "-".to_string();
    };

    if let Some(number) = as_f64(raw) {
        return format_unix_timestamp(Some(number as i64));
    }
    if let Some(text) = raw.as_str() {
        return normalize_reset_text(text);
    }
    "-".to_string()
}

fn format_unix_timestamp(value: Option<i64>) -> String {
    let Some(raw) = value else {
        return "-".to_string();
    };
    if raw <= 0 {
        return "-".to_string();
    }

    let seconds = if raw > 10_000_000_000 {
        raw / 1000
    } else {
        raw
    };
    let Some(dt) = chrono::DateTime::from_timestamp(seconds, 0) else {
        return "-".to_string();
    };
    dt.to_rfc3339()
}

fn render_markdown(meta: &ReportMeta, rows: &[ReportRow]) -> String {
    let now = chrono::Utc::now();
    let data_collected_at = format_data_collected_at(meta);
    let mut output = String::new();
    output.push_str("# Cockpit Tools Usage Report\n\n");
    output.push_str(&format!(
        "- Generated at: {}\n",
        markdown_cell(&meta.generated_at)
    ));
    output.push_str(&format!(
        "- Data collected at: {}\n",
        markdown_cell(&data_collected_at)
    ));
    output.push_str(&format!(
        "- Data delayed: {}\n",
        markdown_cell(&meta.data_delayed)
    ));
    output.push_str(&format!(
        "- {}: {}\n",
        NEXT_AUTH_REFRESH_TRIGGER_LABEL,
        markdown_cell(&meta.next_auth_refresh_trigger_time)
    ));
    output.push_str(&format!("- Rows: {}\n\n", rows.len()));
    output.push_str(
        "| Service | Account | Metric | Used | Remaining | Reset Cycle | Reset Friendly | Status | Note |\n",
    );
    output.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");

    for row in rows {
        let reset_friendly = format_reset_friendly(&row.reset_cycle, now);
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&row.service),
            markdown_cell(&row.account),
            markdown_cell(&row.metric),
            markdown_cell(&row.used),
            markdown_cell(&row.remaining),
            markdown_cell(&row.reset_cycle),
            markdown_cell(&reset_friendly),
            markdown_cell(&row.status),
            markdown_cell(&row.note),
        ));
    }

    output
}

fn markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br/>")
}

fn render_yaml(meta: &ReportMeta, rows: &[ReportRow]) -> String {
    let now = chrono::Utc::now();
    let data_collected_at = format_data_collected_at(meta);
    let mut output = String::new();
    output.push_str(&format!(
        "generated_at: {}\n",
        yaml_quote(&meta.generated_at)
    ));
    output.push_str(&format!(
        "data_collected_at: {}\n",
        yaml_quote(&data_collected_at)
    ));
    output.push_str(&format!(
        "data_delayed: {}\n",
        yaml_quote(&meta.data_delayed)
    ));
    output.push_str(&format!(
        "next_auth_refresh_trigger_time: {}\n",
        yaml_quote(&meta.next_auth_refresh_trigger_time)
    ));
    output.push_str(&format!("row_count: {}\n", rows.len()));
    output.push_str("rows:\n");
    for row in rows {
        let reset_friendly = format_reset_friendly(&row.reset_cycle, now);
        output.push_str("  - service: ");
        output.push_str(&yaml_quote(&row.service));
        output.push('\n');
        output.push_str("    account: ");
        output.push_str(&yaml_quote(&row.account));
        output.push('\n');
        output.push_str("    metric: ");
        output.push_str(&yaml_quote(&row.metric));
        output.push('\n');
        output.push_str("    used: ");
        output.push_str(&yaml_quote(&row.used));
        output.push('\n');
        output.push_str("    remaining: ");
        output.push_str(&yaml_quote(&row.remaining));
        output.push('\n');
        output.push_str("    reset_cycle: ");
        output.push_str(&yaml_quote(&row.reset_cycle));
        output.push('\n');
        output.push_str("    reset_friendly: ");
        output.push_str(&yaml_quote(&reset_friendly));
        output.push('\n');
        output.push_str("    status: ");
        output.push_str(&yaml_quote(&row.status));
        output.push('\n');
        output.push_str("    note: ");
        output.push_str(&yaml_quote(&row.note));
        output.push('\n');
    }
    output
}

fn render_html(meta: &ReportMeta, rows: &[ReportRow]) -> String {
    let now = chrono::Utc::now();
    let generated_at = format_timestamp_human_local(&meta.generated_at);
    let data_collected_at = format_data_collected_at_human(meta);
    let mut output = String::new();
    output.push_str(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>",
    );
    output.push_str("<title>Cockpit Tools Usage Report</title>");
    output.push_str(
        "<style>body{margin:0;background:#f6f8fb;color:#0f172a;font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif}main{max-width:1120px;margin:24px auto;padding:0 16px 24px}h1{font-size:22px;margin:0 0 12px}table{width:100%;border-collapse:collapse;background:#fff;border:1px solid #e2e8f0;border-radius:10px;overflow:hidden;margin-top:16px}th,td{font-size:13px;padding:10px 12px;border-bottom:1px solid #e2e8f0;text-align:left;vertical-align:top}th{background:#e8eafe;color:#334155;font-weight:600;position:sticky;top:0}tr:last-child td{border-bottom:none}.meta-table{margin-top:0}.meta-key{width:360px;color:#334155;font-weight:600;background:#e8eafe}.group-even td{background:#ffffff}.group-odd td{background:#f0faff}.status-disabled{color:#b45309}.status-normal{color:#166534}.mono{font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace}.reset-friendly-col{min-width:240px;width:240px;white-space:nowrap}</style>",
    );
    output.push_str("</head><body><main>");
    output.push_str("<h1>Cockpit Tools Usage Report</h1>");
    output.push_str("<table class=\"meta-table\"><tbody>");
    output.push_str(&format!(
        "<tr><th class=\"meta-key\">Generated at</th><td class=\"mono\">{}</td></tr>",
        html_escape(&generated_at)
    ));
    output.push_str(&format!(
        "<tr><th class=\"meta-key\">Data collected at</th><td class=\"mono\">{}</td></tr>",
        html_escape(&data_collected_at)
    ));
    output.push_str(&format!(
        "<tr><th class=\"meta-key\">Data delayed</th><td class=\"mono\">{}</td></tr>",
        html_escape(&meta.data_delayed)
    ));
    output.push_str(&format!(
        "<tr><th class=\"meta-key\">{}</th><td class=\"mono\">{}</td></tr>",
        html_escape(NEXT_AUTH_REFRESH_TRIGGER_LABEL),
        html_escape(&meta.next_auth_refresh_trigger_time)
    ));
    output.push_str(&format!(
        "<tr><th class=\"meta-key\">Rows</th><td class=\"mono\">{}</td></tr>",
        rows.len()
    ));
    output.push_str("</tbody></table>");
    output.push_str("<table><thead><tr><th>Service</th><th>Account</th><th>Metric</th><th>Used</th><th>Remaining</th><th class=\"reset-friendly-col\">Reset Friendly</th><th>Status</th><th>Note</th></tr></thead><tbody>");

    let mut previous_group_key = String::new();
    let mut group_index: usize = 0;
    for row in rows {
        let reset_friendly = format_reset_friendly(&row.reset_cycle, now);
        let current_group_key = format!("{}|{}", row.service, row.account);
        if !previous_group_key.is_empty() && current_group_key != previous_group_key {
            group_index += 1;
        }
        previous_group_key = current_group_key;
        let status_class = if row.status.eq_ignore_ascii_case("disabled") {
            "status-disabled"
        } else {
            "status-normal"
        };
        let group_class = if group_index % 2 == 0 {
            "group-even"
        } else {
            "group-odd"
        };
        output.push_str(&format!("<tr class=\"{}\">", group_class));
        output.push_str(&format!("<td>{}</td>", html_escape(&row.service)));
        output.push_str(&format!("<td>{}</td>", html_escape(&row.account)));
        output.push_str(&format!("<td>{}</td>", html_escape(&row.metric)));
        output.push_str(&format!(
            "<td class=\"mono\">{}</td>",
            html_escape(&row.used)
        ));
        output.push_str(&format!(
            "<td class=\"mono\">{}</td>",
            html_escape(&row.remaining)
        ));
        output.push_str(&format!(
            "<td class=\"mono reset-friendly-col\">{}</td>",
            html_escape(&reset_friendly)
        ));
        output.push_str(&format!(
            "<td class=\"{}\">{}</td>",
            status_class,
            html_escape(&row.status)
        ));
        output.push_str(&format!("<td>{}</td>", html_escape(&row.note)));
        output.push_str("</tr>");
    }

    output.push_str("</tbody></table></main></body></html>");
    output
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn format_minutes_natural(total_minutes: i64) -> String {
    if total_minutes <= 0 {
        return "0m".to_string();
    }

    let mut remaining = total_minutes;
    let days = remaining / (24 * 60);
    remaining %= 24 * 60;
    let hours = remaining / 60;
    let minutes = remaining % 60;

    let mut parts: Vec<String> = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}m", minutes));
    }

    if parts.is_empty() {
        "0m".to_string()
    } else {
        parts.join("")
    }
}

fn format_reset_friendly(reset_cycle: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(target) = parse_reset_cycle_to_utc(reset_cycle) else {
        return "-".to_string();
    };

    let delta = target.signed_duration_since(now).num_seconds();
    let countdown = format_countdown_compact(delta);
    let display_time = target
        .with_timezone(&chrono::Local)
        .format("%m/%d %H:%M")
        .to_string();
    format!("{} ({})", countdown, display_time)
}

fn parse_reset_cycle_to_utc(reset_cycle: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = reset_cycle.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&chrono::Utc));
    }

    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            naive,
            chrono::Utc,
        ));
    }

    if let Ok(raw) = trimmed.parse::<i64>() {
        let ts = if raw > 10_000_000_000 {
            raw / 1000
        } else {
            raw
        };
        if ts > 0 {
            if let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0) {
                return Some(dt);
            }
        }
    }

    None
}

fn format_countdown_compact(seconds: i64) -> String {
    if seconds <= 0 {
        return "expired".to_string();
    }

    let total_minutes = (seconds + 59) / 60;
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;

    if days > 0 {
        format!("{}d{}h", days, hours)
    } else if hours > 0 {
        format!("{}h{}m", hours, minutes)
    } else {
        format!("{}m", minutes.max(1))
    }
}

fn yaml_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}
