use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::modules::logger;

#[derive(Debug, Clone, Serialize)]
pub struct ManagedLogFile {
    pub log_file_path: String,
    pub log_file_name: String,
    pub file_size: u64,
    pub modified_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogSnapshot {
    pub log_dir_path: String,
    pub log_file_path: String,
    pub log_file_name: String,
    pub content: String,
    pub line_limit: usize,
    pub file_size: u64,
    pub modified_at_ms: Option<i64>,
    pub available_files: Vec<ManagedLogFile>,
}

fn to_unix_millis(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
        .and_then(|value| i64::try_from(value).ok())
}

fn open_directory(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    Ok(())
}

fn build_managed_log_file(path: &Path) -> Result<ManagedLogFile, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("读取日志文件元数据失败: {}", e))?;

    Ok(ManagedLogFile {
        log_file_path: path.to_string_lossy().to_string(),
        log_file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        file_size: metadata.len(),
        modified_at_ms: metadata.modified().ok().and_then(to_unix_millis),
    })
}

fn build_available_log_files(paths: Vec<PathBuf>) -> Result<Vec<ManagedLogFile>, String> {
    paths
        .into_iter()
        .map(|path| build_managed_log_file(path.as_path()))
        .collect()
}

async fn run_bounded_log_read<T: Send + 'static>(
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    timeout: std::time::Duration,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = gate.try_acquire_owned().map_err(|_| "日志读取仍在进行，请稍后重试".to_string())?;
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        task()
    });
    tokio::time::timeout(timeout, worker).await
        .map_err(|_| "日志读取超时，请稍后重试".to_string())?
        .map_err(|error| format!("日志读取任务失败: {error}"))?
}

#[tauri::command]
pub async fn logs_get_snapshot(file_name: Option<String>, line_limit: Option<usize>) -> Result<LogSnapshot, String> {
    static GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1))).clone();
    run_bounded_log_read(gate, std::time::Duration::from_secs(5), move || {
        read_log_snapshot(file_name, line_limit)
    }).await
}

fn read_log_snapshot(
    file_name: Option<String>,
    line_limit: Option<usize>,
) -> Result<LogSnapshot, String> {
    let line_limit = logger::clamp_log_tail_lines(line_limit);
    let log_dir = logger::get_log_dir()?;
    let log_file = logger::resolve_managed_log_file(file_name.as_deref())?;
    let content = logger::read_log_tail_lines(&log_file, line_limit)?;
    let metadata = fs::metadata(&log_file).map_err(|e| format!("读取日志文件元数据失败: {}", e))?;
    let available_files = build_available_log_files(logger::list_managed_log_files()?)?;

    Ok(LogSnapshot {
        log_dir_path: log_dir.to_string_lossy().to_string(),
        log_file_path: log_file.to_string_lossy().to_string(),
        log_file_name: log_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        content,
        line_limit,
        file_size: metadata.len(),
        modified_at_ms: metadata.modified().ok().and_then(to_unix_millis),
        available_files,
    })
}

#[tauri::command]
pub fn logs_open_log_directory() -> Result<(), String> {
    let log_dir = logger::get_log_dir()?;
    open_directory(&log_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn log_read_timeout_does_not_allow_overlapping_workers() {
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let result = run_bounded_log_read(gate.clone(), std::time::Duration::from_millis(10), move || {
            let _ = blocked.recv();
            Ok(1)
        }).await;
        assert!(result.unwrap_err().contains("超时"));
        assert_eq!(gate.available_permits(), 0);
        assert!(run_bounded_log_read(gate.clone(), std::time::Duration::from_secs(1), || Ok(2)).await.is_err());
        release.send(()).unwrap();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(4), gate.clone().acquire_owned()).await.unwrap().unwrap();
        drop(permit);
        assert_eq!(run_bounded_log_read(gate, std::time::Duration::from_secs(1), || Ok(3)).await.unwrap(), 3);
    }
}
