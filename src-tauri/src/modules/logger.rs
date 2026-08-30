use crate::modules::account::get_data_dir;
use chrono::{DateTime, Duration, Local};
use regex::{Captures, Regex};
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::{error, info, warn};
use tracing_subscriber::{
    filter::filter_fn, fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

const APP_LOG_FILE_PREFIX: &str = "app.log";
const CODEX_API_LOG_FILE_PREFIX: &str = "codex-api.log";
const CODEX_AUTH_DIAGNOSTIC_LOG_FILE_PREFIX: &str = "codex-auth-diagnostic.log";
const CODEX_CDP_DIAGNOSTIC_LOG_FILE_PREFIX: &str = "codex-cdp-diagnostic.log";
const CODEX_API_LOG_TARGET: &str = "codex_api";
const CODEX_AUTH_DIAGNOSTIC_LOG_TARGET: &str = "codex_auth_diagnostic";
const CODEX_CDP_DIAGNOSTIC_LOG_TARGET: &str = "codex_cdp_diagnostic";
const MANAGED_LOG_FILE_PREFIXES: &[&str] = &[
    APP_LOG_FILE_PREFIX,
    CODEX_API_LOG_FILE_PREFIX,
    CODEX_AUTH_DIAGNOSTIC_LOG_FILE_PREFIX,
    CODEX_CDP_DIAGNOSTIC_LOG_FILE_PREFIX,
];
const LOG_RETENTION_DAYS: i64 = 3;
const DEFAULT_LOG_TAIL_LINES: usize = 200;
const MIN_LOG_TAIL_LINES: usize = 20;
const MAX_LOG_TAIL_LINES: usize = 5000;
const LOG_TAIL_SCAN_CHUNK_BYTES: usize = 8192;
static EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b")
        .expect("email regex should be valid")
});

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.to_rfc3339())
    }
}

pub fn get_log_dir() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    let log_dir = data_dir.join("logs");

    if !log_dir.exists() {
        fs::create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败: {}", e))?;
    }

    Ok(log_dir)
}

fn is_log_file_with_prefix(name: &str, prefix: &str) -> bool {
    name == prefix
        || name
            .strip_prefix(prefix)
            .map(|suffix| suffix.starts_with('.'))
            .unwrap_or(false)
}

fn is_managed_log_file_name(name: &str) -> bool {
    MANAGED_LOG_FILE_PREFIXES
        .iter()
        .any(|prefix| is_log_file_with_prefix(name, prefix))
}

fn is_managed_log_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(is_managed_log_file_name)
        .unwrap_or(false)
}

pub fn clamp_log_tail_lines(line_limit: Option<usize>) -> usize {
    line_limit
        .unwrap_or(DEFAULT_LOG_TAIL_LINES)
        .clamp(MIN_LOG_TAIL_LINES, MAX_LOG_TAIL_LINES)
}

fn list_log_files_by_name<F>(matcher: F) -> Result<Vec<PathBuf>, String>
where
    F: Fn(&str) -> bool,
{
    let log_dir = get_log_dir()?;
    let entries = fs::read_dir(&log_dir).map_err(|e| format!("读取日志目录失败: {}", e))?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取日志目录项失败: {}", e))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_file() || !matcher(name) {
            continue;
        }
        paths.push(path);
    }

    paths.sort_by(|left, right| compare_log_paths_by_recency(left, right));
    Ok(paths)
}

fn compare_log_paths_by_recency(left: &PathBuf, right: &PathBuf) -> std::cmp::Ordering {
    let left_modified = fs::metadata(left)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let right_modified = fs::metadata(right)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    right_modified
        .cmp(&left_modified)
        .then_with(|| right.file_name().cmp(&left.file_name()))
}

pub fn list_managed_log_files() -> Result<Vec<PathBuf>, String> {
    list_log_files_by_name(is_managed_log_file_name)
}

pub fn resolve_managed_log_file(file_name: Option<&str>) -> Result<PathBuf, String> {
    let log_files = list_managed_log_files()?;
    if log_files.is_empty() {
        return Err("未找到可用日志文件".to_string());
    }

    if let Some(file_name) = file_name.map(str::trim).filter(|name| !name.is_empty()) {
        return log_files
            .into_iter()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(file_name))
            .ok_or_else(|| format!("未找到指定日志文件: {}", file_name));
    }

    log_files
        .into_iter()
        .next()
        .ok_or_else(|| "未找到可用日志文件".to_string())
}

pub fn get_latest_app_log_file() -> Result<PathBuf, String> {
    list_log_files_by_name(|name| is_log_file_with_prefix(name, APP_LOG_FILE_PREFIX))?
        .into_iter()
        .next()
        .ok_or_else(|| "未找到可用日志文件".to_string())
}

pub fn read_log_tail_lines(log_file: &Path, line_limit: usize) -> Result<String, String> {
    let line_limit = line_limit.max(1);
    let mut file = File::open(log_file).map_err(|e| format!("打开日志文件失败: {}", e))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("读取日志文件元数据失败: {}", e))?
        .len();

    if file_len == 0 {
        return Ok(String::new());
    }

    let mut pos = file_len;
    let mut newline_count = 0usize;
    let mut start_offset = 0u64;
    let mut buffer = [0u8; LOG_TAIL_SCAN_CHUNK_BYTES];

    'scan: while pos > 0 {
        let read_size = usize::min(LOG_TAIL_SCAN_CHUNK_BYTES, pos as usize);
        pos -= read_size as u64;

        file.seek(SeekFrom::Start(pos))
            .map_err(|e| format!("读取日志定位失败: {}", e))?;
        file.read_exact(&mut buffer[..read_size])
            .map_err(|e| format!("读取日志内容失败: {}", e))?;

        for idx in (0..read_size).rev() {
            if buffer[idx] != b'\n' {
                continue;
            }
            newline_count += 1;
            if newline_count > line_limit {
                start_offset = pos + idx as u64 + 1;
                break 'scan;
            }
        }
    }

    file.seek(SeekFrom::Start(start_offset))
        .map_err(|e| format!("读取日志定位失败: {}", e))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("读取日志内容失败: {}", e))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn cleanup_expired_logs(log_dir: &Path) {
    let cutoff = Local::now() - Duration::days(LOG_RETENTION_DAYS);
    let entries = match fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(err) => {
            warn!("读取日志目录失败，跳过清理: {}", err);
            return;
        }
    };

    let mut removed_count = 0usize;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!("读取日志文件失败，已忽略: {}", err);
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() || !is_managed_log_file(&path) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!("读取日志元数据失败，已忽略: {:?}, {}", path, err);
                continue;
            }
        };

        let modified_at = match metadata.modified() {
            Ok(time) => {
                let dt: DateTime<Local> = time.into();
                dt
            }
            Err(err) => {
                warn!("读取日志修改时间失败，已忽略: {:?}, {}", path, err);
                continue;
            }
        };

        if modified_at >= cutoff {
            continue;
        }

        match fs::remove_file(&path) {
            Ok(_) => removed_count += 1,
            Err(err) => warn!("删除过期日志失败，已忽略: {:?}, {}", path, err),
        }
    }

    if removed_count > 0 {
        info!(
            "日志清理完成：删除 {} 个超过 {} 天的日志文件",
            removed_count, LOG_RETENTION_DAYS
        );
    }
}

/// 初始化日志系统
pub fn init_logger() {
    let _ = tracing_log::LogTracer::init();

    let is_dev_profile = crate::modules::account::is_dev_profile();

    let log_dir = match get_log_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("无法初始化日志目录: {}", e);
            return;
        }
    };

    let app_file_appender = tracing_appender::rolling::daily(log_dir.clone(), APP_LOG_FILE_PREFIX);
    let (app_non_blocking, app_guard) = tracing_appender::non_blocking(app_file_appender);
    let codex_api_file_appender =
        tracing_appender::rolling::daily(log_dir.clone(), CODEX_API_LOG_FILE_PREFIX);
    let (codex_api_non_blocking, codex_api_guard) =
        tracing_appender::non_blocking(codex_api_file_appender);
    let console_layer = fmt::Layer::new()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true)
        .with_timer(LocalTimer);

    let app_file_layer = fmt::Layer::new()
        .with_writer(app_non_blocking)
        .with_ansi(false)
        .with_target(false)
        .with_level(true)
        .with_timer(LocalTimer);

    let codex_api_file_layer = fmt::Layer::new()
        .with_writer(codex_api_non_blocking)
        .with_ansi(false)
        .with_target(false)
        .with_level(true)
        .with_timer(LocalTimer);

    let filter_layer = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry()
        .with(filter_layer)
        .with(console_layer)
        .with(app_file_layer.with_filter(filter_fn(|metadata| {
            !matches!(
                metadata.target(),
                CODEX_API_LOG_TARGET
                    | CODEX_AUTH_DIAGNOSTIC_LOG_TARGET
                    | CODEX_CDP_DIAGNOSTIC_LOG_TARGET
            )
        })))
        .with(codex_api_file_layer.with_filter(filter_fn(|metadata| {
            metadata.target() == CODEX_API_LOG_TARGET
        })));

    // 授权诊断日志只在本地 dev profile 创建和写入，生产环境不打开对应文件。
    let codex_auth_diagnostic_guard = if is_dev_profile {
        let codex_auth_diagnostic_file_appender = tracing_appender::rolling::daily(
            log_dir.clone(),
            CODEX_AUTH_DIAGNOSTIC_LOG_FILE_PREFIX,
        );
        let (codex_auth_diagnostic_non_blocking, guard) =
            tracing_appender::non_blocking(codex_auth_diagnostic_file_appender);
        let codex_auth_diagnostic_file_layer = fmt::Layer::new()
            .with_writer(codex_auth_diagnostic_non_blocking)
            .with_ansi(false)
            .with_target(false)
            .with_level(true)
            .with_timer(LocalTimer);

        let codex_cdp_diagnostic_file_appender =
            tracing_appender::rolling::daily(log_dir.clone(), CODEX_CDP_DIAGNOSTIC_LOG_FILE_PREFIX);
        let (codex_cdp_diagnostic_non_blocking, cdp_guard) =
            tracing_appender::non_blocking(codex_cdp_diagnostic_file_appender);
        let codex_cdp_diagnostic_file_layer = fmt::Layer::new()
            .with_writer(codex_cdp_diagnostic_non_blocking)
            .with_ansi(false)
            .with_target(false)
            .with_level(true)
            .with_timer(LocalTimer);

        let _ = registry
            .with(
                codex_auth_diagnostic_file_layer.with_filter(filter_fn(|metadata| {
                    metadata.target() == CODEX_AUTH_DIAGNOSTIC_LOG_TARGET
                })),
            )
            .with(
                codex_cdp_diagnostic_file_layer.with_filter(filter_fn(|metadata| {
                    metadata.target() == CODEX_CDP_DIAGNOSTIC_LOG_TARGET
                })),
            )
            .try_init();
        Some((guard, cdp_guard))
    } else {
        let _ = registry.try_init();
        None
    };

    std::mem::forget(app_guard);
    std::mem::forget(codex_api_guard);
    if let Some((auth_guard, cdp_guard)) = codex_auth_diagnostic_guard {
        std::mem::forget(auth_guard);
        std::mem::forget(cdp_guard);
    }

    info!("日志系统已完成初始化");

    // 日志清理移至后台线程，不阻塞启动
    std::thread::spawn(move || {
        cleanup_expired_logs(&log_dir);
    });
}

pub fn log_info(message: &str) {
    info!("{}", sanitize_message(message));
}

pub fn log_warn(message: &str) {
    if is_cdp_diagnostic_message(message) {
        warn!(
            target: CODEX_CDP_DIAGNOSTIC_LOG_TARGET,
            "{}",
            sanitize_message(message)
        );
    } else {
        warn!("{}", sanitize_message(message));
    }
}

pub fn log_error(message: &str) {
    error!("{}", sanitize_message(message));
}

pub fn log_codex_api_info(message: &str) {
    info!(target: CODEX_API_LOG_TARGET, "{}", sanitize_message(message));
}

pub fn log_codex_api_warn(message: &str) {
    warn!(target: CODEX_API_LOG_TARGET, "{}", sanitize_message(message));
}

pub fn log_codex_api_error(message: &str) {
    error!(target: CODEX_API_LOG_TARGET, "{}", sanitize_message(message));
}

/// Codex 授权诊断日志。只接受已经去除 bearer 凭据的诊断摘要，禁止写入原始 token。
pub fn log_codex_auth_diagnostic(message: &str) {
    if !crate::modules::account::is_dev_profile() {
        return;
    }
    if is_cdp_diagnostic_message(message) {
        info!(
            target: CODEX_CDP_DIAGNOSTIC_LOG_TARGET,
            "{}",
            sanitize_message(message)
        );
    } else {
        info!(
            target: CODEX_AUTH_DIAGNOSTIC_LOG_TARGET,
            "{}",
            sanitize_message(message)
        );
    }
}

fn is_cdp_diagnostic_message(message: &str) -> bool {
    message.starts_with("[Codex Auth CDP]") || message.starts_with("[Codex Auth Network]")
}

fn sanitize_message(message: &str) -> String {
    EMAIL_REGEX
        .replace_all(message, |caps: &Captures| mask_email(&caps[0]))
        .to_string()
}

fn mask_email(email: &str) -> String {
    let (local, domain) = match email.split_once('@') {
        Some(parts) => parts,
        None => return email.to_string(),
    };

    format!("{}@{}", mask_local_part(local), mask_domain_part(domain))
}

fn mask_local_part(local: &str) -> String {
    let chars: Vec<char> = local.chars().collect();
    match chars.len() {
        0 => "***".to_string(),
        1 => "*".to_string(),
        2 => format!("{}*", chars[0]),
        3 => format!("{}*{}", chars[0], chars[2]),
        _ => format!("{}{}***{}", chars[0], chars[1], chars[chars.len() - 1]),
    }
}

fn mask_domain_part(domain: &str) -> String {
    let mut parts = domain.split('.');
    let head = parts.next().unwrap_or_default();
    let tail = parts.collect::<Vec<&str>>();

    let masked_head = mask_domain_head(head);
    if tail.is_empty() {
        masked_head
    } else {
        format!("{}.{}", masked_head, tail.join("."))
    }
}

fn mask_domain_head(head: &str) -> String {
    let chars: Vec<char> = head.chars().collect();
    match chars.len() {
        0 => "***".to_string(),
        1 => "*".to_string(),
        2 => format!("{}*", chars[0]),
        _ => format!("{}***{}", chars[0], chars[chars.len() - 1]),
    }
}
