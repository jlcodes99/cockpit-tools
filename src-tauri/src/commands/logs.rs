use serde::Serialize;
use std::fs::{self, File, Metadata};
use std::hash::{DefaultHasher, Hash, Hasher};
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
    pub content: Option<String>,
    pub content_revision: Option<String>,
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
            .map(crate::modules::process::background_child_pid)
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map(crate::modules::process::background_child_pid)
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(crate::modules::process::background_child_pid)
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    Ok(())
}

fn build_managed_log_file(path: &Path, metadata: &Metadata) -> ManagedLogFile {
    ManagedLogFile {
        log_file_path: path.to_string_lossy().to_string(),
        log_file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        file_size: metadata.len(),
        modified_at_ms: metadata.modified().ok().and_then(to_unix_millis),
    }
}

fn build_available_log_files(paths: Vec<PathBuf>) -> Result<Vec<ManagedLogFile>, String> {
    paths
        .into_iter()
        .map(|path| {
            let metadata =
                fs::metadata(&path).map_err(|e| format!("读取日志文件元数据失败: {}", e))?;
            Ok(build_managed_log_file(&path, &metadata))
        })
        .collect()
}

fn log_content_revision(
    path: &Path,
    _file: &File,
    metadata: &Metadata,
    line_limit: usize,
) -> Option<String> {
    let mut hash = DefaultHasher::new();
    (path, line_limit, metadata.len(), metadata.modified().ok()?).hash(&mut hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ).hash(&mut hash);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(_file.as_raw_handle() as _), &mut info) }.ok()?;
        (
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
            metadata.created().ok()?,
        ).hash(&mut hash);
    }
    Some(format!("{:016x}", hash.finish()))
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
pub async fn logs_get_snapshot(
    file_name: Option<String>,
    line_limit: Option<usize>,
    known_content_revision: Option<String>,
) -> Result<LogSnapshot, String> {
    static GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1))).clone();
    run_bounded_log_read(gate, std::time::Duration::from_secs(5), move || {
        read_log_snapshot(file_name, line_limit, known_content_revision.as_deref())
    }).await
}

fn read_log_snapshot(
    file_name: Option<String>,
    line_limit: Option<usize>,
    known_content_revision: Option<&str>,
) -> Result<LogSnapshot, String> {
    let line_limit = logger::clamp_log_tail_lines(line_limit);
    let log_dir = logger::get_log_dir()?;
    let log_files = logger::list_managed_log_files()?;
    read_log_snapshot_from_files(
        &log_dir,
        log_files,
        file_name.as_deref(),
        line_limit,
        known_content_revision,
    )
}

fn read_log_snapshot_from_files(
    log_dir: &Path,
    log_files: Vec<PathBuf>,
    file_name: Option<&str>,
    line_limit: usize,
    known_content_revision: Option<&str>,
) -> Result<LogSnapshot, String> {
    let log_file = logger::resolve_managed_log_file(&log_files, file_name)?;
    let mut file = File::open(log_file).map_err(|e| format!("打开日志文件失败: {}", e))?;
    let metadata = file.metadata().map_err(|e| format!("读取日志文件元数据失败: {}", e))?;
    let selected_file = build_managed_log_file(log_file, &metadata);
    let mut content_revision = log_content_revision(log_file, &file, &metadata, line_limit);
    let content = if content_revision
        .as_deref()
        .is_some_and(|revision| Some(revision) == known_content_revision)
    {
        None
    } else {
        let content = logger::read_log_tail_lines(&mut file, metadata.len(), line_limit)?;
        let revision_after_read = file
            .metadata()
            .ok()
            .and_then(|metadata| log_content_revision(log_file, &file, &metadata, line_limit));
        if revision_after_read != content_revision {
            content_revision = None;
        }
        Some(content)
    };
    let available_files = build_available_log_files(log_files)?;

    Ok(LogSnapshot {
        log_dir_path: log_dir.to_string_lossy().to_string(),
        log_file_path: selected_file.log_file_path,
        log_file_name: selected_file.log_file_name,
        content,
        content_revision,
        line_limit,
        file_size: selected_file.file_size,
        modified_at_ms: selected_file.modified_at_ms,
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

    #[test]
    fn log_snapshot_reuses_unchanged_content_and_still_reports_available_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("app.log");
        let second = dir.path().join("codex-api.log");
        fs::write(&first, "initial line\n").unwrap();
        let read = |paths, limit, known| {
            read_log_snapshot_from_files(dir.path(), paths, Some("app.log"), limit, known)
                .unwrap()
        };
        let snapshot = read(vec![first.clone()], 200, None);
        assert_eq!(snapshot.content.as_deref(), Some("initial line\n"));
        let revision = snapshot.content_revision.as_deref().unwrap();

        fs::write(&second, "another log").unwrap();
        let unchanged = read(vec![first.clone(), second], 200, Some(revision));
        assert_eq!(unchanged.content, None);
        assert_eq!(unchanged.content_revision.as_deref(), Some(revision));
        assert_eq!(unchanged.available_files.len(), 2);
        assert!(read(vec![first.clone()], 5000, Some(revision)).content.is_some());
        assert!(read(vec![first.clone()], 200, None).content.is_some());

        fs::write(&first, "initial line\nappended line\n").unwrap();
        let changed = read(vec![first], 200, Some(revision));
        assert_eq!(changed.content.as_deref(), Some("initial line\nappended line\n"));
        assert_ne!(changed.content_revision.as_deref(), Some(revision));
    }

    #[test]
    fn log_snapshot_detects_rotation_and_a_new_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("app.log");
        let rotated = dir.path().join("app.log.2026-11-16");
        fs::write(&first, "before\n").unwrap();
        let snapshot = read_log_snapshot_from_files(
            dir.path(), vec![first.clone()], None, 200, None,
        ).unwrap();
        let revision = snapshot.content_revision.as_deref().unwrap();

        let old_timestamp = fs::metadata(&first).unwrap().modified().unwrap();
        fs::rename(&first, &rotated).unwrap();
        fs::write(&first, "rotated").unwrap();
        File::options().write(true).open(&first).unwrap()
            .set_modified(old_timestamp).unwrap();
        let replacement = read_log_snapshot_from_files(
            dir.path(), vec![first.clone(), rotated.clone()], None, 200, Some(revision),
        ).unwrap();
        assert_eq!(replacement.content.as_deref(), Some("rotated"));
        let revision = replacement.content_revision.as_deref().unwrap();
        fs::write(&first, "").unwrap();
        let next = read_log_snapshot_from_files(
            dir.path(), vec![first, rotated], None, 200, Some(revision),
        ).unwrap();
        assert_eq!(next.content.as_deref(), Some(""));
        assert_eq!(next.available_files.len(), 2);
        assert_ne!(next.content_revision.as_deref(), Some(revision));
    }

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
