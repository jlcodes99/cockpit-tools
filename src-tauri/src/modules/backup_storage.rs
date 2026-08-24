//! 统一管理 Cockpit 生成的备份目录、行为快照保留策略和空间统计。
//!
//! 定时 JSON/ZIP 与行为快照共用一个可配置根目录。历史版本散落在平台目录
//! 旁边的备份仍会被统计和清理，但新生成的行为备份应通过本模块获取路径。

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::modules::{account, codex_account, codex_instance, config};

const DEFAULT_BACKUP_DIR_NAME: &str = "backups";
const BEHAVIOR_DIR_NAME: &str = "behavior";
const MIGRATION_STAGING_PREFIX: &str = ".cockpit-backup-migration-";

static ACTIVE_MIGRATION_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static MIGRATION_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
pub struct BackupUsageEntry {
    pub source: String,
    pub file_count: u64,
    pub size_bytes: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupUsageSummary {
    pub total_file_count: u64,
    pub total_size_bytes: u64,
    pub entries: Vec<BackupUsageEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupDirectoryChangeResult {
    pub old_directory: String,
    pub new_directory: String,
    pub migrated: bool,
    pub migrated_file_count: u64,
    pub migrated_size_bytes: u64,
    pub removed_file_count: u64,
    pub removed_size_bytes: u64,
    pub remaining_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupMigrationSourceSummary {
    pub source: String,
    pub file_count: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupDirectoryMigrationPreview {
    pub old_directory: String,
    pub new_directory: String,
    pub file_count: u64,
    pub size_bytes: u64,
    pub sources: Vec<BackupMigrationSourceSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupDirectoryMigrationProgress {
    pub migration_id: String,
    pub phase: String,
    pub total_file_count: u64,
    pub processed_file_count: u64,
    pub total_size_bytes: u64,
    pub processed_size_bytes: u64,
    pub current_source: Option<String>,
    pub current_path: Option<String>,
    pub cancellable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupCleanupResult {
    pub deleted_file_count: u64,
    pub deleted_directory_count: u64,
    pub deleted_size_bytes: u64,
    pub sources: Vec<String>,
}

#[derive(Default)]
struct UsageAccumulator {
    file_count: u64,
    size_bytes: u64,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct MigrationFile {
    source: String,
    source_path: PathBuf,
    relative_target: PathBuf,
    size_bytes: u64,
    modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct MigrationManifest {
    current_root: PathBuf,
    target_root: PathBuf,
    files: Vec<MigrationFile>,
    cleanup_paths: Vec<PathBuf>,
}

fn safe_component(value: &str) -> String {
    let normalized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        "default".to_string()
    } else {
        normalized
    }
}

pub fn default_backup_root_dir() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        // 现有单元测试会并行修改 COCKPIT_TOOLS_TEST_DATA_DIR。行为备份路径
        // 在测试态使用独立目录，避免一个测试清理另一个测试正在写入的 SQLite
        // 快照；正式构建仍保持默认数据目录下的 backups 行为。
        return Ok(
            std::env::temp_dir().join(format!("cockpit-tools-test-backups-{}", std::process::id()))
        );
    }

    #[cfg(not(test))]
    Ok(account::get_data_dir()?.join(DEFAULT_BACKUP_DIR_NAME))
}

pub fn get_backup_root_dir() -> Result<PathBuf, String> {
    let configured = config::get_user_config()
        .backup_directory
        .trim()
        .to_string();
    if configured.is_empty() {
        return default_backup_root_dir();
    }
    let path = PathBuf::from(configured);
    if !path.is_absolute() {
        return Err("备份目录必须是绝对路径".to_string());
    }
    Ok(path)
}

pub fn ensure_backup_root_dir() -> Result<PathBuf, String> {
    ensure_backup_write_available()?;
    let path = get_backup_root_dir()?;
    fs::create_dir_all(&path)
        .map_err(|error| format!("创建备份目录失败({}): {}", path.display(), error))?;
    Ok(path)
}

pub fn ensure_backup_write_available() -> Result<(), String> {
    let active = ACTIVE_MIGRATION_ID
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "备份迁移状态锁已损坏".to_string())?;
    if active.is_some() {
        return Err("备份目录正在迁移，请稍后重试".to_string());
    }
    Ok(())
}

pub fn behavior_backup_dir(
    source: &str,
    scope: &str,
    operation_id: &str,
) -> Result<PathBuf, String> {
    let root = ensure_backup_root_dir()?;
    let path = root
        .join(BEHAVIOR_DIR_NAME)
        .join(safe_component(source))
        .join(safe_component(scope))
        .join(safe_component(operation_id));
    fs::create_dir_all(&path)
        .map_err(|error| format!("创建行为备份目录失败({}): {}", path.display(), error))?;
    Ok(path)
}

pub fn scope_for_path(path: &Path) -> String {
    format!("{:x}", md5::compute(path.to_string_lossy().as_bytes()))
}

pub fn prune_behavior_backups(source: &str, scope: &str) -> Result<BackupCleanupResult, String> {
    let root = get_backup_root_dir()?
        .join(BEHAVIOR_DIR_NAME)
        .join(safe_component(source))
        .join(safe_component(scope));
    prune_operation_children(&root, Some(source.to_string()))
}

fn normalize_comparison_path(path: &Path) -> PathBuf {
    let mut cursor = path.to_path_buf();
    let mut missing_components = Vec::new();
    while !cursor.exists() {
        let Some(name) = cursor.file_name().map(|value| value.to_os_string()) else {
            break;
        };
        missing_components.push(name);
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent.to_path_buf();
    }

    let mut normalized = cursor.canonicalize().unwrap_or(cursor);
    for component in missing_components.into_iter().rev() {
        normalized.push(component);
    }

    #[cfg(target_os = "windows")]
    {
        return PathBuf::from(
            normalized
                .to_string_lossy()
                .replace('/', "\\")
                .to_lowercase(),
        );
    }

    #[cfg(not(target_os = "windows"))]
    normalized
}

fn validate_backup_target(target_directory: &str) -> Result<(PathBuf, PathBuf), String> {
    let target = PathBuf::from(target_directory.trim());
    if target.as_os_str().is_empty() || !target.is_absolute() {
        return Err("备份目录必须是非空绝对路径".to_string());
    }
    let current = get_backup_root_dir()?;
    let current_cmp = normalize_comparison_path(&current);
    let target_cmp = normalize_comparison_path(&target);
    if target_cmp != current_cmp
        && (target_cmp.starts_with(&current_cmp) || current_cmp.starts_with(&target_cmp))
    {
        return Err("新旧备份目录不能互相嵌套".to_string());
    }
    Ok((current, target))
}

fn migration_source_for_relative(default_source: &str, relative: &Path) -> String {
    if default_source != "managed" {
        return default_source.to_string();
    }
    let mut components = relative.components();
    if components.next().and_then(|item| item.as_os_str().to_str()) == Some(BEHAVIOR_DIR_NAME) {
        return components
            .next()
            .and_then(|item| item.as_os_str().to_str())
            .map(safe_component)
            .unwrap_or_else(|| "behavior".to_string());
    }
    "scheduled".to_string()
}

fn collect_migration_tree(
    root: &Path,
    current: &Path,
    target_prefix: &Path,
    default_source: &str,
    cancellable: bool,
    files: &mut Vec<MigrationFile>,
    destinations: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    if cancellable && migration_cancelled() {
        return Err("backup_migration_cancelled".to_string());
    }
    let metadata = fs::symlink_metadata(current)
        .map_err(|error| format!("读取备份迁移源失败({}): {}", current.display(), error))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        if current != root
            && current
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with(MIGRATION_STAGING_PREFIX))
        {
            return Ok(());
        }
        for entry in fs::read_dir(current)
            .map_err(|error| format!("读取备份迁移目录失败({}): {}", current.display(), error))?
        {
            let entry = entry.map_err(|error| format!("读取备份迁移项失败: {}", error))?;
            collect_migration_tree(
                root,
                &entry.path(),
                target_prefix,
                default_source,
                cancellable,
                files,
                destinations,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Ok(());
    }
    let relative = current
        .strip_prefix(root)
        .map_err(|_| format!("无法计算备份迁移相对路径: {}", current.display()))?;
    let relative_target = target_prefix.join(relative);
    if !destinations.insert(relative_target.clone()) {
        return Ok(());
    }
    files.push(MigrationFile {
        source: migration_source_for_relative(default_source, relative),
        source_path: current.to_path_buf(),
        relative_target,
        size_bytes: metadata.len(),
        modified_at: metadata.modified().ok(),
    });
    Ok(())
}

fn push_cleanup_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let key = normalize_comparison_path(&path)
        .to_string_lossy()
        .to_lowercase();
    if seen.insert(key) {
        paths.push(path);
    }
}

fn build_migration_manifest(
    target_directory: &str,
    cancellable: bool,
) -> Result<MigrationManifest, String> {
    let (current_root, target_root) = validate_backup_target(target_directory)?;
    let target_cmp = normalize_comparison_path(&target_root);
    if normalize_comparison_path(&current_root) == target_cmp {
        return Ok(MigrationManifest {
            current_root,
            target_root,
            files: Vec::new(),
            cleanup_paths: Vec::new(),
        });
    }
    let mut files = Vec::new();
    let mut destinations = HashSet::new();
    let mut cleanup_paths = Vec::new();
    let mut cleanup_seen = HashSet::new();

    let mut collect_root = |root: PathBuf, target_prefix: PathBuf, source: &str| {
        if !root.exists() {
            return Ok(());
        }
        let root_cmp = normalize_comparison_path(&root);
        if target_cmp.starts_with(&root_cmp) || root_cmp.starts_with(&target_cmp) {
            return Err(format!(
                "新备份目录不能与历史备份目录互相嵌套: {}",
                root.display()
            ));
        }
        collect_migration_tree(
            &root,
            &root,
            &target_prefix,
            source,
            cancellable,
            &mut files,
            &mut destinations,
        )?;
        push_cleanup_path(&mut cleanup_paths, &mut cleanup_seen, root);
        Ok(())
    };

    collect_root(current_root.clone(), PathBuf::new(), "managed")?;
    for (source, root) in legacy_backup_roots() {
        if normalize_comparison_path(&root) == normalize_comparison_path(&current_root) {
            continue;
        }
        collect_root(
            root,
            PathBuf::from(BEHAVIOR_DIR_NAME)
                .join(safe_component(&source))
                .join("legacy"),
            &source,
        )?;
    }
    for (scope, root) in legacy_codex_instance_backup_dirs() {
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("backup")
            .to_string();
        collect_root(
            root,
            PathBuf::from(BEHAVIOR_DIR_NAME)
                .join("codex")
                .join(safe_component(&scope))
                .join(safe_component(&name)),
            "codex",
        )?;
    }

    files.sort_by(|left, right| left.relative_target.cmp(&right.relative_target));
    for file in &files {
        let destination = target_root.join(&file.relative_target);
        if destination.exists() {
            return Err(format!(
                "新备份目录存在同名文件，请选择空目录或其他目录: {}",
                destination.display()
            ));
        }
    }
    Ok(MigrationManifest {
        current_root,
        target_root,
        files,
        cleanup_paths,
    })
}

fn summarize_migration_sources(files: &[MigrationFile]) -> Vec<BackupMigrationSourceSummary> {
    let mut summaries: HashMap<String, (u64, u64)> = HashMap::new();
    for file in files {
        let entry = summaries.entry(file.source.clone()).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(file.size_bytes);
    }
    let mut result = summaries
        .into_iter()
        .map(
            |(source, (file_count, size_bytes))| BackupMigrationSourceSummary {
                source,
                file_count,
                size_bytes,
            },
        )
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.source.cmp(&right.source));
    result
}

pub fn preview_backup_root_dir_change(
    target_directory: &str,
) -> Result<BackupDirectoryMigrationPreview, String> {
    let manifest = build_migration_manifest(target_directory, false)?;
    let file_count = manifest.files.len() as u64;
    let size_bytes = manifest.files.iter().map(|file| file.size_bytes).sum();
    Ok(BackupDirectoryMigrationPreview {
        old_directory: manifest.current_root.to_string_lossy().to_string(),
        new_directory: manifest.target_root.to_string_lossy().to_string(),
        file_count,
        size_bytes,
        sources: summarize_migration_sources(&manifest.files),
    })
}

struct ActiveMigrationGuard {
    migration_id: String,
}

impl Drop for ActiveMigrationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_MIGRATION_ID.get_or_init(|| Mutex::new(None)).lock() {
            if active.as_deref() == Some(self.migration_id.as_str()) {
                *active = None;
            }
        }
        MIGRATION_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    }
}

fn begin_migration(migration_id: &str) -> Result<ActiveMigrationGuard, String> {
    let trimmed = migration_id.trim();
    if trimmed.is_empty() {
        return Err("迁移任务 ID 不能为空".to_string());
    }
    let mut active = ACTIVE_MIGRATION_ID
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "备份迁移状态锁已损坏".to_string())?;
    if active.is_some() {
        return Err("已有备份目录迁移正在进行".to_string());
    }
    *active = Some(trimmed.to_string());
    MIGRATION_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    Ok(ActiveMigrationGuard {
        migration_id: trimmed.to_string(),
    })
}

pub fn cancel_backup_root_dir_change(migration_id: &str) -> Result<bool, String> {
    let active = ACTIVE_MIGRATION_ID
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "备份迁移状态锁已损坏".to_string())?;
    if active.as_deref() != Some(migration_id.trim()) {
        return Ok(false);
    }
    MIGRATION_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
    Ok(true)
}

fn migration_cancelled() -> bool {
    MIGRATION_CANCEL_REQUESTED.load(Ordering::SeqCst)
}

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| format!("删除迁移临时文件失败({}): {}", path.display(), error))
}

struct MigrationStageGuard {
    path: PathBuf,
}

impl Drop for MigrationStageGuard {
    fn drop(&mut self) {
        let _ = remove_path_if_exists(&self.path);
    }
}

fn rollback_finalized_files(target_root: &Path, files: &[MigrationFile]) {
    for file in files.iter().rev() {
        let _ = fs::remove_file(target_root.join(&file.relative_target));
    }
}

fn source_file_is_unchanged(file: &MigrationFile) -> bool {
    let Ok(metadata) = fs::symlink_metadata(&file.source_path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() != file.size_bytes {
        return false;
    }
    match (file.modified_at, metadata.modified().ok()) {
        (Some(expected), Some(actual)) => expected == actual,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn remove_empty_directories(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(true);
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("读取迁移源目录失败({}): {}", path.display(), error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let entries = fs::read_dir(path)
        .map_err(|error| format!("读取迁移源目录失败({}): {}", path.display(), error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取迁移源目录项失败({}): {}", path.display(), error))?;
    for entry in entries {
        let child = entry.path();
        if child.is_dir() {
            remove_empty_directories(&child)?;
        }
    }
    let is_empty = fs::read_dir(path)
        .map_err(|error| format!("检查迁移源目录失败({}): {}", path.display(), error))?
        .next()
        .is_none();
    if !is_empty {
        return Ok(false);
    }
    fs::remove_dir(path)
        .map_err(|error| format!("删除迁移源空目录失败({}): {}", path.display(), error))?;
    Ok(true)
}

fn cleanup_migrated_sources(manifest: &MigrationManifest) -> (u64, u64, Vec<String>) {
    let mut removed_file_count = 0u64;
    let mut removed_size_bytes = 0u64;
    let mut remaining_paths = Vec::new();
    let mut remaining_seen = HashSet::new();
    let mut add_remaining = |path: &Path| {
        let value = path.to_string_lossy().to_string();
        if remaining_seen.insert(value.clone()) {
            remaining_paths.push(value);
        }
    };

    for file in &manifest.files {
        if !file.source_path.exists() {
            continue;
        }
        if !source_file_is_unchanged(file) {
            add_remaining(&file.source_path);
            continue;
        }
        match fs::remove_file(&file.source_path) {
            Ok(()) => {
                removed_file_count = removed_file_count.saturating_add(1);
                removed_size_bytes = removed_size_bytes.saturating_add(file.size_bytes);
            }
            Err(_) => add_remaining(&file.source_path),
        }
    }

    for root in &manifest.cleanup_paths {
        match remove_empty_directories(root) {
            Ok(true) => {}
            Ok(false) | Err(_) => add_remaining(root),
        }
    }

    (removed_file_count, removed_size_bytes, remaining_paths)
}

fn copy_file_with_progress<F>(source: &Path, target: &Path, mut on_chunk: F) -> Result<u64, String>
where
    F: FnMut(u64) -> Result<(), String>,
{
    let mut input = fs::File::open(source)
        .map_err(|error| format!("打开备份迁移源失败({}): {}", source.display(), error))?;
    let mut output = fs::File::create(target)
        .map_err(|error| format!("创建备份迁移文件失败({}): {}", target.display(), error))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("读取备份迁移文件失败({}): {}", source.display(), error))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入备份迁移文件失败({}): {}", target.display(), error))?;
        let chunk = read as u64;
        copied = copied.saturating_add(chunk);
        on_chunk(chunk)?;
    }
    output
        .sync_all()
        .map_err(|error| format!("同步备份迁移文件失败({}): {}", target.display(), error))?;
    if let Ok(metadata) = fs::metadata(source) {
        let _ = fs::set_permissions(target, metadata.permissions());
    }
    Ok(copied)
}

pub fn change_backup_root_dir_with_progress<F>(
    target_directory: &str,
    migrate_existing: bool,
    migration_id: &str,
    mut on_progress: F,
) -> Result<BackupDirectoryChangeResult, String>
where
    F: FnMut(BackupDirectoryMigrationProgress),
{
    let guard = begin_migration(migration_id)?;
    let migration_id = guard.migration_id.clone();
    let emit = |phase: &str,
                total_file_count: u64,
                processed_file_count: u64,
                total_size_bytes: u64,
                processed_size_bytes: u64,
                current_source: Option<String>,
                current_path: Option<String>,
                cancellable: bool,
                callback: &mut F| {
        callback(BackupDirectoryMigrationProgress {
            migration_id: migration_id.clone(),
            phase: phase.to_string(),
            total_file_count,
            processed_file_count,
            total_size_bytes,
            processed_size_bytes,
            current_source,
            current_path,
            cancellable,
        });
    };

    emit(
        "scanning",
        0,
        0,
        0,
        0,
        None,
        None,
        migrate_existing,
        &mut on_progress,
    );
    let (current, target) = validate_backup_target(target_directory)?;
    if normalize_comparison_path(&current) == normalize_comparison_path(&target) {
        return Ok(BackupDirectoryChangeResult {
            old_directory: current.to_string_lossy().to_string(),
            new_directory: target.to_string_lossy().to_string(),
            migrated: false,
            migrated_file_count: 0,
            migrated_size_bytes: 0,
            removed_file_count: 0,
            removed_size_bytes: 0,
            remaining_paths: Vec::new(),
        });
    }
    if !migrate_existing {
        fs::create_dir_all(&target)
            .map_err(|error| format!("创建新备份目录失败({}): {}", target.display(), error))?;
        emit("switching", 0, 0, 0, 0, None, None, false, &mut on_progress);
        let target_value = target.to_string_lossy().to_string();
        config::patch_user_config(move |current_config| {
            current_config.backup_directory = target_value;
            Ok(())
        })?;
        emit("completed", 0, 0, 0, 0, None, None, false, &mut on_progress);
        return Ok(BackupDirectoryChangeResult {
            old_directory: current.to_string_lossy().to_string(),
            new_directory: target.to_string_lossy().to_string(),
            migrated: false,
            migrated_file_count: 0,
            migrated_size_bytes: 0,
            removed_file_count: 0,
            removed_size_bytes: 0,
            remaining_paths: Vec::new(),
        });
    }

    let manifest = build_migration_manifest(target_directory, true)?;
    let total_file_count = manifest.files.len() as u64;
    let total_size_bytes = manifest
        .files
        .iter()
        .map(|file| file.size_bytes)
        .sum::<u64>();
    if migration_cancelled() {
        emit(
            "cancelled",
            total_file_count,
            0,
            total_size_bytes,
            0,
            None,
            None,
            false,
            &mut on_progress,
        );
        return Err("backup_migration_cancelled".to_string());
    }

    fs::create_dir_all(&manifest.target_root).map_err(|error| {
        format!(
            "创建新备份目录失败({}): {}",
            manifest.target_root.display(),
            error
        )
    })?;
    let stage_root = manifest.target_root.join(format!(
        "{}{}",
        MIGRATION_STAGING_PREFIX,
        safe_component(&migration_id)
    ));
    remove_path_if_exists(&stage_root)?;
    fs::create_dir_all(&stage_root)
        .map_err(|error| format!("创建迁移临时目录失败({}): {}", stage_root.display(), error))?;
    let _stage_guard = MigrationStageGuard {
        path: stage_root.clone(),
    };

    let mut processed_file_count = 0u64;
    let mut processed_size_bytes = 0u64;
    for file in &manifest.files {
        if migration_cancelled() {
            let _ = remove_path_if_exists(&stage_root);
            emit(
                "cancelled",
                total_file_count,
                processed_file_count,
                total_size_bytes,
                processed_size_bytes,
                Some(file.source.clone()),
                Some(file.source_path.to_string_lossy().to_string()),
                false,
                &mut on_progress,
            );
            return Err("backup_migration_cancelled".to_string());
        }
        emit(
            "copying",
            total_file_count,
            processed_file_count,
            total_size_bytes,
            processed_size_bytes,
            Some(file.source.clone()),
            Some(file.source_path.to_string_lossy().to_string()),
            true,
            &mut on_progress,
        );
        let stage_path = stage_root.join(&file.relative_target);
        if let Some(parent) = stage_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建迁移临时父目录失败({}): {}", parent.display(), error)
            })?;
        }
        let copied = match copy_file_with_progress(&file.source_path, &stage_path, |chunk| {
            processed_size_bytes = processed_size_bytes.saturating_add(chunk);
            emit(
                "copying",
                total_file_count,
                processed_file_count,
                total_size_bytes,
                processed_size_bytes,
                Some(file.source.clone()),
                Some(file.source_path.to_string_lossy().to_string()),
                true,
                &mut on_progress,
            );
            if migration_cancelled() {
                return Err("backup_migration_cancelled".to_string());
            }
            Ok(())
        }) {
            Ok(copied) => copied,
            Err(error) => {
                let _ = remove_path_if_exists(&stage_root);
                if error == "backup_migration_cancelled" {
                    emit(
                        "cancelled",
                        total_file_count,
                        processed_file_count,
                        total_size_bytes,
                        processed_size_bytes,
                        Some(file.source.clone()),
                        Some(file.source_path.to_string_lossy().to_string()),
                        false,
                        &mut on_progress,
                    );
                }
                return Err(error);
            }
        };
        if copied != file.size_bytes {
            let _ = remove_path_if_exists(&stage_root);
            return Err(format!(
                "迁移备份文件大小不一致({}): expected={}, actual={}",
                file.source_path.display(),
                file.size_bytes,
                copied
            ));
        }
        processed_file_count = processed_file_count.saturating_add(1);
        emit(
            "copying",
            total_file_count,
            processed_file_count,
            total_size_bytes,
            processed_size_bytes,
            Some(file.source.clone()),
            Some(file.source_path.to_string_lossy().to_string()),
            true,
            &mut on_progress,
        );
    }

    if migration_cancelled() {
        emit(
            "cancelled",
            total_file_count,
            processed_file_count,
            total_size_bytes,
            processed_size_bytes,
            None,
            None,
            false,
            &mut on_progress,
        );
        return Err("backup_migration_cancelled".to_string());
    }

    emit(
        "verifying",
        total_file_count,
        processed_file_count,
        total_size_bytes,
        processed_size_bytes,
        None,
        None,
        false,
        &mut on_progress,
    );
    for file in &manifest.files {
        let staged = stage_root.join(&file.relative_target);
        let actual = fs::metadata(&staged)
            .map_err(|error| format!("校验迁移文件失败({}): {}", staged.display(), error))?
            .len();
        if actual != file.size_bytes {
            let _ = remove_path_if_exists(&stage_root);
            return Err(format!(
                "迁移文件校验失败({}): expected={}, actual={}",
                staged.display(),
                file.size_bytes,
                actual
            ));
        }
    }

    emit(
        "switching",
        total_file_count,
        processed_file_count,
        total_size_bytes,
        processed_size_bytes,
        None,
        None,
        false,
        &mut on_progress,
    );
    let mut finalized = Vec::new();
    for file in &manifest.files {
        let staged = stage_root.join(&file.relative_target);
        let destination = manifest.target_root.join(&file.relative_target);
        if destination.exists() {
            rollback_finalized_files(&manifest.target_root, &finalized);
            let _ = remove_path_if_exists(&stage_root);
            return Err(format!("迁移目标文件已存在: {}", destination.display()));
        }
        if let Some(parent) = destination.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                rollback_finalized_files(&manifest.target_root, &finalized);
                let _ = remove_path_if_exists(&stage_root);
                return Err(format!(
                    "创建新备份父目录失败({}): {}",
                    parent.display(),
                    error
                ));
            }
        }
        if let Err(error) = fs::rename(&staged, &destination) {
            rollback_finalized_files(&manifest.target_root, &finalized);
            let _ = remove_path_if_exists(&stage_root);
            return Err(format!(
                "提交迁移文件失败({} -> {}): {}",
                staged.display(),
                destination.display(),
                error
            ));
        }
        finalized.push(file.clone());
    }
    let _ = remove_path_if_exists(&stage_root);

    let target_value = manifest.target_root.to_string_lossy().to_string();
    if let Err(error) = config::patch_user_config(move |current_config| {
        current_config.backup_directory = target_value;
        Ok(())
    }) {
        rollback_finalized_files(&manifest.target_root, &finalized);
        return Err(error);
    }

    emit(
        "cleaning",
        total_file_count,
        processed_file_count,
        total_size_bytes,
        processed_size_bytes,
        None,
        None,
        false,
        &mut on_progress,
    );
    // 仅删除清单中且迁移期间未发生变化的源文件。若有旧任务仍在写入或产生了
    // 新文件，则保留这些内容并将路径返回给前端，避免整目录删除造成数据丢失。
    let (mut removed_file_count, mut removed_size_bytes, mut remaining_paths) =
        cleanup_migrated_sources(&manifest);
    match cleanup_managed_behavior_backups_at_root(&manifest.target_root) {
        Ok(cleaned) => {
            removed_file_count = removed_file_count.saturating_add(cleaned.deleted_file_count);
            removed_size_bytes = removed_size_bytes.saturating_add(cleaned.deleted_size_bytes);
        }
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[BackupMigration] 目标目录历史行为备份清理失败: {}",
                error
            ));
            let behavior_path = manifest.target_root.join(BEHAVIOR_DIR_NAME);
            let behavior_path = behavior_path.to_string_lossy().to_string();
            if !remaining_paths.contains(&behavior_path) {
                remaining_paths.push(behavior_path);
            }
        }
    }

    emit(
        "completed",
        total_file_count,
        processed_file_count,
        total_size_bytes,
        processed_size_bytes,
        None,
        None,
        false,
        &mut on_progress,
    );
    Ok(BackupDirectoryChangeResult {
        old_directory: manifest.current_root.to_string_lossy().to_string(),
        new_directory: manifest.target_root.to_string_lossy().to_string(),
        migrated: true,
        migrated_file_count: processed_file_count,
        migrated_size_bytes: processed_size_bytes,
        removed_file_count,
        removed_size_bytes,
        remaining_paths,
    })
}

fn legacy_backup_roots() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    if let Ok(data_dir) = account::get_data_dir() {
        roots.push((
            "claude".to_string(),
            data_dir.join("claude_desktop_backups"),
        ));
    }
    if let Some(home) = dirs::home_dir() {
        #[cfg(target_os = "macos")]
        {
            roots.push((
                "codebuddy".to_string(),
                home.join("Library/Application Support/CodeBuddyExtension/Backups/CockpitTools"),
            ));
            roots.push((
                "workbuddy".to_string(),
                home.join(".workbuddy/Backups/CockpitTools"),
            ));
        }
        #[cfg(target_os = "windows")]
        {
            roots.push((
                "codebuddy".to_string(),
                home.join("AppData/Local/CodeBuddyExtension/Backups/CockpitTools"),
            ));
            roots.push((
                "workbuddy".to_string(),
                home.join(".workbuddy/Backups/CockpitTools"),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            roots.push((
                "codebuddy".to_string(),
                home.join(".local/share/CodeBuddyExtension/Backups/CockpitTools"),
            ));
            roots.push((
                "workbuddy".to_string(),
                home.join(".workbuddy/Backups/CockpitTools"),
            ));
        }
    }
    roots
}

fn legacy_codex_instance_backup_dirs() -> Vec<(String, PathBuf)> {
    let mut instance_dirs = Vec::new();
    if let Ok(store) = codex_instance::load_instance_store() {
        instance_dirs.extend(
            store
                .instances
                .into_iter()
                .map(|instance| PathBuf::from(instance.user_data_dir)),
        );
    }
    instance_dirs.push(codex_account::get_codex_home());

    let mut seen_dirs = HashSet::new();
    let mut backups = Vec::new();
    for instance_dir in instance_dirs {
        let instance_key = instance_dir.to_string_lossy().to_lowercase();
        if !seen_dirs.insert(instance_key) {
            continue;
        }
        let scope = scope_for_path(&instance_dir);
        let Ok(entries) = fs::read_dir(&instance_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with("backup-") && path.is_dir() {
                backups.push((scope.clone(), path));
            }
        }
    }
    backups
}

fn cleanup_legacy_codex_instance_backups() -> Result<BackupCleanupResult, String> {
    let mut grouped: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for (scope, path) in legacy_codex_instance_backup_dirs() {
        grouped.entry(scope).or_default().push(path);
    }

    let mut result = BackupCleanupResult {
        deleted_file_count: 0,
        deleted_directory_count: 0,
        deleted_size_bytes: 0,
        sources: Vec::new(),
    };
    for paths in grouped.values_mut() {
        if paths.len() <= 1 {
            continue;
        }
        paths.sort_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });
        for path in paths.drain(..).rev().skip(1) {
            let file_count = directory_file_count(&path);
            let size = directory_size(&path);
            fs::remove_dir_all(&path).map_err(|error| {
                format!("清理旧 Codex 实例备份失败({}): {}", path.display(), error)
            })?;
            result.deleted_directory_count += 1;
            result.deleted_file_count = result.deleted_file_count.saturating_add(file_count);
            result.deleted_size_bytes = result.deleted_size_bytes.saturating_add(size);
        }
        if result.deleted_directory_count > 0
            && !result.sources.iter().any(|source| source == "codex")
        {
            result.sources.push("codex".to_string());
        }
    }
    Ok(result)
}

fn add_usage_file(
    map: &mut HashMap<String, UsageAccumulator>,
    source: &str,
    path: &Path,
    size: u64,
) {
    let entry = map.entry(source.to_string()).or_default();
    entry.file_count += 1;
    entry.size_bytes = entry.size_bytes.saturating_add(size);
    if entry.path.as_os_str().is_empty() {
        entry.path = path.to_path_buf();
    }
}

fn scan_tree(path: &Path, source: &str, map: &mut HashMap<String, UsageAccumulator>, depth: usize) {
    if depth > 14 {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        add_usage_file(map, source, path, metadata.len());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        scan_tree(&entry.path(), source, map, depth + 1);
    }
}

fn scan_managed_root(root: &Path, map: &mut HashMap<String, UsageAccumulator>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|value| value.to_str());
        if name == Some(BEHAVIOR_DIR_NAME) {
            let Ok(categories) = fs::read_dir(&path) else {
                continue;
            };
            for category in categories.flatten() {
                let source = category.file_name().to_string_lossy().to_string();
                scan_tree(&category.path(), &source, map, 0);
            }
        } else if name == Some("legacy") {
            let Ok(categories) = fs::read_dir(&path) else {
                continue;
            };
            for category in categories.flatten() {
                let source = category.file_name().to_string_lossy().to_string();
                scan_tree(&category.path(), &source, map, 0);
            }
        } else {
            scan_tree(&path, "scheduled", map, 0);
        }
    }
}

pub fn get_backup_usage() -> Result<BackupUsageSummary, String> {
    let root = get_backup_root_dir()?;
    let mut map = HashMap::new();
    scan_managed_root(&root, &mut map);
    let mut seen = HashSet::new();
    seen.insert(root.to_string_lossy().to_lowercase());
    for (source, path) in legacy_backup_roots() {
        let key = path.to_string_lossy().to_lowercase();
        if seen.insert(key) {
            scan_tree(&path, &source, &mut map, 0);
        }
    }
    for (_, path) in legacy_codex_instance_backup_dirs() {
        scan_tree(&path, "codex", &mut map, 0);
    }

    let mut entries = map
        .into_iter()
        .filter(|(_, value)| value.file_count > 0)
        .map(|(source, value)| BackupUsageEntry {
            source,
            file_count: value.file_count,
            size_bytes: value.size_bytes,
            path: value.path.to_string_lossy().to_string(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.source.cmp(&right.source));
    let total_file_count = entries.iter().map(|entry| entry.file_count).sum();
    let total_size_bytes = entries.iter().map(|entry| entry.size_bytes).sum();
    Ok(BackupUsageSummary {
        total_file_count,
        total_size_bytes,
        entries,
    })
}

fn prune_operation_children(
    root: &Path,
    source: Option<String>,
) -> Result<BackupCleanupResult, String> {
    let mut result = BackupCleanupResult {
        deleted_file_count: 0,
        deleted_directory_count: 0,
        deleted_size_bytes: 0,
        sources: Vec::new(),
    };
    if !root.is_dir() {
        return Ok(result);
    }
    let mut children = fs::read_dir(root)
        .map_err(|error| format!("读取行为备份目录失败({}): {}", root.display(), error))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    if children.len() <= 1 {
        return Ok(result);
    }
    children.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    for path in children.into_iter().rev().skip(1) {
        let file_count = directory_file_count(&path);
        let size = directory_size(&path);
        fs::remove_dir_all(&path)
            .map_err(|error| format!("清理历史行为备份失败({}): {}", path.display(), error))?;
        result.deleted_directory_count += 1;
        result.deleted_file_count = result.deleted_file_count.saturating_add(file_count);
        result.deleted_size_bytes = result.deleted_size_bytes.saturating_add(size);
    }
    if let Some(source) = source {
        result.sources.push(source);
    }
    Ok(result)
}

fn directory_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            total = total.saturating_add(directory_size(&entry.path()));
        }
    }
    total
}

fn directory_file_count(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return 1;
    }
    fs::read_dir(path)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| directory_file_count(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

fn cleanup_managed_behavior_backups_at_root(
    backup_root: &Path,
) -> Result<BackupCleanupResult, String> {
    let root = backup_root.join(BEHAVIOR_DIR_NAME);
    let mut result = BackupCleanupResult {
        deleted_file_count: 0,
        deleted_directory_count: 0,
        deleted_size_bytes: 0,
        sources: Vec::new(),
    };
    if root.is_dir() {
        for source_dir in fs::read_dir(&root)
            .map_err(|error| format!("读取行为备份目录失败({}): {}", root.display(), error))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
        {
            let Ok(scope_entries) = fs::read_dir(&source_dir) else {
                continue;
            };
            for scope_dir in scope_entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
            {
                let source = source_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("behavior")
                    .to_string();
                let cleaned = prune_operation_children(&scope_dir, Some(source.clone()))?;
                result.deleted_directory_count += cleaned.deleted_directory_count;
                result.deleted_file_count = result
                    .deleted_file_count
                    .saturating_add(cleaned.deleted_file_count);
                result.deleted_size_bytes = result
                    .deleted_size_bytes
                    .saturating_add(cleaned.deleted_size_bytes);
                if cleaned.deleted_directory_count > 0 && !result.sources.contains(&source) {
                    result.sources.push(source);
                }
            }
        }
    }
    Ok(result)
}

pub fn cleanup_behavior_backups() -> Result<BackupCleanupResult, String> {
    let backup_root = get_backup_root_dir()?;
    let mut result = cleanup_managed_behavior_backups_at_root(&backup_root)?;
    for (source, path) in legacy_backup_roots() {
        let cleaned = prune_operation_children(&path, Some(source.clone()))?;
        result.deleted_directory_count += cleaned.deleted_directory_count;
        result.deleted_file_count = result
            .deleted_file_count
            .saturating_add(cleaned.deleted_file_count);
        result.deleted_size_bytes = result
            .deleted_size_bytes
            .saturating_add(cleaned.deleted_size_bytes);
        if cleaned.deleted_directory_count > 0 && !result.sources.contains(&source) {
            result.sources.push(source);
        }
    }
    let legacy_codex = cleanup_legacy_codex_instance_backups()?;
    result.deleted_directory_count += legacy_codex.deleted_directory_count;
    result.deleted_file_count = result
        .deleted_file_count
        .saturating_add(legacy_codex.deleted_file_count);
    result.deleted_size_bytes = result
        .deleted_size_bytes
        .saturating_add(legacy_codex.deleted_size_bytes);
    if legacy_codex.deleted_directory_count > 0 && !result.sources.contains(&"codex".to_string()) {
        result.sources.push("codex".to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cockpit-backup-storage-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn migration_manifest_classifies_scheduled_and_behavior_files() {
        let root = make_temp_dir("manifest");
        fs::write(root.join("cockpit_auto_backup.json"), b"scheduled").expect("write scheduled");
        let behavior = root.join("behavior/codex/scope/operation");
        fs::create_dir_all(&behavior).expect("create behavior dir");
        fs::write(behavior.join("state.sqlite"), b"codex").expect("write behavior");

        let mut files = Vec::new();
        let mut destinations = HashSet::new();
        collect_migration_tree(
            &root,
            &root,
            Path::new(""),
            "managed",
            false,
            &mut files,
            &mut destinations,
        )
        .expect("collect migration tree");

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|file| file.source == "scheduled"));
        assert!(files.iter().any(|file| file.source == "codex"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migration_copy_reports_real_bytes_and_preserves_content() {
        let root = make_temp_dir("copy");
        let source = root.join("source.bin");
        let target = root.join("target.bin");
        let content = vec![0x5au8; 2 * 1024 * 1024 + 37];
        fs::write(&source, &content).expect("write source");
        let mut reported = 0u64;

        let copied = copy_file_with_progress(&source, &target, |chunk| {
            reported = reported.saturating_add(chunk);
            Ok(())
        })
        .expect("copy file");

        assert_eq!(copied, content.len() as u64);
        assert_eq!(reported, copied);
        assert_eq!(fs::read(&target).expect("read target"), content);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migration_cancel_request_matches_only_active_task() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let guard = begin_migration("backup-migration-test").expect("begin migration");
        assert!(!cancel_backup_root_dir_change("another-task").expect("cancel other"));
        assert!(!migration_cancelled());
        assert!(cancel_backup_root_dir_change("backup-migration-test").expect("cancel active"));
        assert!(migration_cancelled());
        drop(guard);
        assert!(!migration_cancelled());
    }

    #[test]
    fn migration_cleanup_removes_only_unchanged_manifest_files() {
        let root = make_temp_dir("cleanup-sources");
        let unchanged = root.join("unchanged.json");
        let changed = root.join("changed.json");
        let created_later = root.join("created-later.json");
        fs::write(&unchanged, b"unchanged").expect("write unchanged source");
        fs::write(&changed, b"before").expect("write changed source");

        let mut files = Vec::new();
        let mut destinations = HashSet::new();
        collect_migration_tree(
            &root,
            &root,
            Path::new(""),
            "managed",
            false,
            &mut files,
            &mut destinations,
        )
        .expect("collect migration tree");
        fs::write(&changed, b"changed-after-scan").expect("change source after scan");
        fs::write(&created_later, b"new").expect("create source after scan");

        let manifest = MigrationManifest {
            current_root: root.clone(),
            target_root: root.with_extension("target"),
            files,
            cleanup_paths: vec![root.clone()],
        };
        let (removed_count, removed_size, remaining) = cleanup_migrated_sources(&manifest);

        assert_eq!(removed_count, 1);
        assert_eq!(removed_size, b"unchanged".len() as u64);
        assert!(!unchanged.exists());
        assert_eq!(
            fs::read(&changed).expect("read changed"),
            b"changed-after-scan"
        );
        assert_eq!(fs::read(&created_later).expect("read new"), b"new");
        assert!(remaining.contains(&changed.to_string_lossy().to_string()));
        assert!(remaining.contains(&root.to_string_lossy().to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migration_stage_guard_removes_temporary_tree() {
        let root = make_temp_dir("stage-guard");
        let stage = root.join("stage");
        fs::create_dir_all(stage.join("nested")).expect("create stage tree");
        fs::write(stage.join("nested/file.bin"), b"temporary").expect("write stage file");
        {
            let _guard = MigrationStageGuard {
                path: stage.clone(),
            };
        }
        assert!(!stage.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrated_legacy_behavior_backups_are_pruned_to_one_snapshot() {
        let root = make_temp_dir("prune-migrated-legacy");
        let legacy = root.join("behavior/claude/legacy");
        for name in ["1781364037372", "1781364046301", "1781364056864"] {
            let snapshot = legacy.join(name);
            fs::create_dir_all(&snapshot).expect("create legacy snapshot");
            fs::write(snapshot.join("state.json"), name.as_bytes()).expect("write snapshot");
        }

        let cleaned = cleanup_managed_behavior_backups_at_root(&root)
            .expect("cleanup migrated behavior backups");
        let remaining = fs::read_dir(&legacy)
            .expect("read remaining snapshots")
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .count();

        assert_eq!(remaining, 1);
        assert_eq!(cleaned.deleted_directory_count, 2);
        assert_eq!(cleaned.deleted_file_count, 2);
        let _ = fs::remove_dir_all(root);
    }
}
