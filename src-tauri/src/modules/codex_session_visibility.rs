use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use toml_edit::Document;

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DEFAULT_PROVIDER_ID: &str = "openai";
const STATE_DB_FILE: &str = "state_5.sqlite";
const CONFIG_FILE_NAME: &str = "config.toml";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX: &str = "backup-";
const SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX: &str = "-session-visibility-repair";
const MAX_SESSION_VISIBILITY_REPAIR_BACKUPS: usize = 1;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairItem {
    pub instance_id: String,
    pub instance_name: String,
    pub target_provider: String,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub skipped_sqlite_file: bool,
    pub backup_dir: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairSummary {
    pub instance_count: usize,
    pub mutated_instance_count: usize,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub skipped_sqlite_file_count: usize,
    pub items: Vec<CodexSessionVisibilityRepairItem>,
    pub backup_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct CodexSyncInstance {
    id: String,
    name: String,
    data_dir: PathBuf,
    last_pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct RolloutProviderChange {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    updated_content: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct SqliteProviderScan {
    rows_to_update: usize,
    skipped_unusable_database: bool,
}

pub fn repair_session_visibility_across_instances(
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut changed_rollout_file_count = 0usize;
    let mut updated_sqlite_row_count = 0usize;
    let mut skipped_sqlite_file_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        let target_provider = read_target_provider(&instance.data_dir)?;
        let rollout_changes =
            collect_rollout_provider_changes(&instance.data_dir, &target_provider)?;
        let sqlite_scan = count_sqlite_rows_to_update(&instance.data_dir, &target_provider)?;
        let sqlite_rows_to_update = sqlite_scan.rows_to_update;
        if sqlite_scan.skipped_unusable_database {
            skipped_sqlite_file_count += 1;
        }

        if rollout_changes.is_empty() && sqlite_rows_to_update == 0 {
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: 0,
                updated_sqlite_row_count: 0,
                skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
                backup_dir: None,
                running,
            });
            continue;
        }

        let backup_dir = backup_instance_files(
            &instance.data_dir,
            &rollout_changes,
            sqlite_rows_to_update > 0,
            &instance.id,
            &instance.name,
            &target_provider,
        )?;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();

        let repaired = repair_single_instance(
            &instance.data_dir,
            &target_provider,
            &rollout_changes,
            sqlite_rows_to_update > 0,
        );
        let sqlite_rows_updated = match repaired {
            Ok(value) => value,
            Err(error) => {
                let restore_result = restore_instance_files_from_backup(
                    &instance.data_dir,
                    &backup_dir,
                    sqlite_rows_to_update > 0,
                );
                if let Err(restore_error) = restore_result {
                    return Err(format!(
                        "修复实例历史会话可见性失败 ({}): {}；自动回滚也失败: {}；备份目录: {}",
                        instance.name,
                        error,
                        restore_error,
                        backup_dir.display()
                    ));
                }
                return Err(format!(
                    "修复实例历史会话可见性失败 ({}): {}；已自动回滚，备份目录: {}",
                    instance.name,
                    error,
                    backup_dir.display()
                ));
            }
        };

        mutated_instance_count += 1;
        changed_rollout_file_count += rollout_changes.len();
        updated_sqlite_row_count += sqlite_rows_updated;
        if running {
            mutated_running_instance_count += 1;
        }
        backup_dirs.push(backup_dir_string.clone());
        items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            target_provider,
            changed_rollout_file_count: rollout_changes.len(),
            updated_sqlite_row_count: sqlite_rows_updated,
            skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
            backup_dir: Some(backup_dir_string),
            running,
        });
    }

    prune_session_visibility_repair_backups(&instances);

    let message = build_summary_message(
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        mutated_running_instance_count,
        skipped_sqlite_file_count,
    );

    Ok(CodexSessionVisibilityRepairSummary {
        instance_count: instances.len(),
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        skipped_sqlite_file_count,
        items,
        backup_dirs,
        message,
    })
}

pub fn read_history_visibility_provider_for_dir(data_dir: &Path) -> Result<String, String> {
    read_target_provider(data_dir)
}

pub fn repair_session_visibility_for_dir(
    data_dir: &Path,
    instance_id: &str,
    instance_name: &str,
    running: bool,
) -> Result<CodexSessionVisibilityRepairItem, String> {
    let target_provider = read_target_provider(data_dir)?;
    let rollout_changes = collect_rollout_provider_changes(data_dir, &target_provider)?;
    let sqlite_scan = count_sqlite_rows_to_update(data_dir, &target_provider)?;
    let sqlite_rows_to_update = sqlite_scan.rows_to_update;

    if rollout_changes.is_empty() && sqlite_rows_to_update == 0 {
        return Ok(CodexSessionVisibilityRepairItem {
            instance_id: instance_id.to_string(),
            instance_name: instance_name.to_string(),
            target_provider,
            changed_rollout_file_count: 0,
            updated_sqlite_row_count: 0,
            skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
            backup_dir: None,
            running,
        });
    }

    let backup_dir = backup_instance_files(
        data_dir,
        &rollout_changes,
        sqlite_rows_to_update > 0,
        instance_id,
        instance_name,
        &target_provider,
    )?;
    let backup_dir_string = backup_dir.to_string_lossy().to_string();

    let repaired = repair_single_instance(
        data_dir,
        &target_provider,
        &rollout_changes,
        sqlite_rows_to_update > 0,
    );
    let sqlite_rows_updated = match repaired {
        Ok(value) => value,
        Err(error) => {
            let restore_result = restore_instance_files_from_backup(
                data_dir,
                &backup_dir,
                sqlite_rows_to_update > 0,
            );
            if let Err(restore_error) = restore_result {
                return Err(format!(
                    "修复实例历史会话可见性失败 ({}): {}；自动回滚也失败: {}；备份目录: {}",
                    instance_name,
                    error,
                    restore_error,
                    backup_dir.display()
                ));
            }
            return Err(format!(
                "修复实例历史会话可见性失败 ({}): {}；已自动回滚，备份目录: {}",
                instance_name,
                error,
                backup_dir.display()
            ));
        }
    };

    if let Err(error) = prune_instance_session_visibility_repair_backups(data_dir) {
        modules::logger::log_warn(&format!(
            "清理 Codex 会话可见性修复旧备份失败 ({}): {}",
            data_dir.display(),
            error
        ));
    }

    Ok(CodexSessionVisibilityRepairItem {
        instance_id: instance_id.to_string(),
        instance_name: instance_name.to_string(),
        target_provider,
        changed_rollout_file_count: rollout_changes.len(),
        updated_sqlite_row_count: sqlite_rows_updated,
        skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
        backup_dir: Some(backup_dir_string),
        running,
    })
}

fn repair_single_instance(
    data_dir: &Path,
    target_provider: &str,
    rollout_changes: &[RolloutProviderChange],
    update_sqlite: bool,
) -> Result<usize, String> {
    let sqlite_rows_updated = if update_sqlite {
        update_sqlite_provider(data_dir, target_provider)?
    } else {
        0
    };
    for change in rollout_changes {
        rewrite_rollout_provider(change)?;
    }
    Ok(sqlite_rows_updated)
}

fn build_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    mutated_running_instance_count: usize,
    _skipped_sqlite_file_count: usize,
) -> String {
    if mutated_instance_count == 0 {
        return "所有 Codex 实例的历史会话 provider 元数据已与当前 provider 一致，无需修复"
            .to_string();
    }

    if mutated_running_instance_count > 0 {
        return format!(
            "已为 {} 个实例修复历史会话可见性：改写 {} 个 rollout 文件，更新 {} 条 SQLite 记录。运行中的实例可能需要重启后显示",
            mutated_instance_count, changed_rollout_file_count, updated_sqlite_row_count
        );
    }

    format!(
        "已为 {} 个实例修复历史会话可见性：改写 {} 个 rollout 文件，更新 {} 条 SQLite 记录",
        mutated_instance_count, changed_rollout_file_count, updated_sqlite_row_count
    )
}

fn collect_instances() -> Result<Vec<CodexSyncInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = modules::codex_instance::get_default_codex_home()?;
    let store = modules::codex_instance::load_instance_store()?;
    instances.push(CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
        last_pid: store.default_settings.last_pid,
    });

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(CodexSyncInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
            last_pid: instance.last_pid,
        });
    }

    Ok(instances)
}

fn is_instance_running(
    instance: &CodexSyncInstance,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let codex_home = if instance.id == DEFAULT_INSTANCE_ID {
        None
    } else {
        instance.data_dir.to_str()
    };
    modules::process::resolve_codex_pid_from_entries(instance.last_pid, codex_home, process_entries)
        .is_some()
}

fn read_target_provider(data_dir: &Path) -> Result<String, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let doc = content.parse::<Document>().map_err(|error| {
        format!(
            "解析 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn collect_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
) -> Result<Vec<RolloutProviderChange>, String> {
    let mut changes = Vec::new();

    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        let rollout_paths = list_rollout_files(&root_dir)?;
        for rollout_path in rollout_paths {
            let bytes = fs::read(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            let Some(updated_content) = rewrite_rollout_session_meta_providers(
                &bytes,
                target_provider,
                &rollout_path,
            )? else {
                continue;
            };
            let relative_path = rollout_path
                .strip_prefix(data_dir)
                .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?;
            changes.push(RolloutProviderChange {
                relative_path: relative_path.to_path_buf(),
                absolute_path: rollout_path,
                updated_content,
            });
        }
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn list_rollout_files(root_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root_dir)
        .map_err(|error| format!("读取目录失败 ({}): {}", root_dir.display(), error))?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取目录项失败 ({}): {}", root_dir.display(), error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取文件类型失败 ({}): {}", path.display(), error))?;
        if file_type.is_dir() {
            result.extend(list_rollout_files(&path)?);
            continue;
        }
        if file_type.is_file() {
            let file_name = path
                .file_name()
                .and_then(|item| item.to_str())
                .unwrap_or_default();
            if file_name.starts_with("rollout-") && file_name.ends_with(".jsonl") {
                result.push(path);
            }
        }
    }

    result.sort();
    Ok(result)
}

fn parse_session_meta_record(first_line: &str) -> Option<JsonValue> {
    if first_line.trim().is_empty() {
        return None;
    }

    let parsed = serde_json::from_str::<JsonValue>(first_line).ok()?;
    if parsed.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return None;
    }
    if !parsed.get("payload").is_some_and(JsonValue::is_object) {
        return None;
    }
    Some(parsed)
}

fn rewrite_rollout_session_meta_providers(
    bytes: &[u8],
    target_provider: &str,
    path: &Path,
) -> Result<Option<Vec<u8>>, String> {
    let mut changed = false;
    let mut next_bytes = Vec::with_capacity(bytes.len());

    for (line_bytes, separator) in split_jsonl_lines(bytes) {
        let line = String::from_utf8(line_bytes.to_vec()).map_err(|error| {
            format!(
                "解析 rollout JSONL UTF-8 失败 ({}): {}",
                path.display(),
                error
            )
        })?;
        let next_line = if let Some(mut parsed) = parse_session_meta_record(&line) {
            let current_provider = parsed["payload"]
                .get("model_provider")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            if current_provider == target_provider {
                line
            } else {
                if let Some(payload) = parsed.get_mut("payload").and_then(JsonValue::as_object_mut)
                {
                    payload.insert(
                        "model_provider".to_string(),
                        JsonValue::String(target_provider.to_string()),
                    );
                }
                changed = true;
                serde_json::to_string(&parsed)
                    .map_err(|error| format!("序列化 session_meta 失败: {}", error))?
            }
        } else {
            line
        };

        next_bytes.extend_from_slice(next_line.as_bytes());
        next_bytes.extend_from_slice(separator);
    }

    Ok(changed.then_some(next_bytes))
}

fn split_jsonl_lines(bytes: &[u8]) -> Vec<(&[u8], &'static [u8])> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        if index > start && bytes[index - 1] == b'\r' {
            lines.push((&bytes[start..index - 1], b"\r\n".as_slice()));
        } else {
            lines.push((&bytes[start..index], b"\n".as_slice()));
        }
        start = index + 1;
    }
    if start < bytes.len() {
        lines.push((&bytes[start..], b"".as_slice()));
    }
    lines
}

fn is_missing_threads_table_error(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table: threads")
}

fn log_skipped_sqlite_database(path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "跳过无效或损坏的 Codex state_5.sqlite ({}): {}",
        path.display(),
        reason
    ));
}

fn count_sqlite_rows_to_update(
    data_dir: &Path,
    target_provider: &str,
) -> Result<SqliteProviderScan, String> {
    let db_path = data_dir.join(STATE_DB_FILE);
    if !db_path.exists() {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    }

    let connection = match Connection::open(&db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(&db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let count = match connection.query_row(
        "SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> ?1",
        [target_provider],
        |row| row.get::<usize, i64>(0),
    ) {
        Ok(count) => count,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(&db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) if is_missing_threads_table_error(&error) => {
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: false,
            });
        }
        Err(error) => {
            return Err(format!(
                "统计 SQLite provider 差异失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    Ok(SqliteProviderScan {
        rows_to_update: count.max(0) as usize,
        skipped_unusable_database: false,
    })
}

fn update_sqlite_provider(data_dir: &Path, target_provider: &str) -> Result<usize, String> {
    let db_path = data_dir.join(STATE_DB_FILE);
    if !db_path.exists() {
        return Ok(0);
    }

    let mut connection = match Connection::open(&db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(&db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| {
            format!(
                "设置 SQLite busy_timeout 失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(&db_path, &error))?;
    let updated_rows = match transaction.execute(
        "UPDATE threads SET model_provider = ?1 WHERE COALESCE(model_provider, '') <> ?1",
        [target_provider],
    ) {
        Ok(updated_rows) => updated_rows,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(&db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) if is_missing_threads_table_error(&error) => {
            return Ok(0);
        }
        Err(error) => return Err(format_sqlite_write_error(&db_path, &error)),
    };
    if let Err(error) = transaction.commit() {
        if modules::db::is_unusable_sqlite_database_error(&error) {
            log_skipped_sqlite_database(&db_path, &error.to_string());
            return Ok(0);
        }
        return Err(format_sqlite_write_error(&db_path, &error));
    }
    Ok(updated_rows)
}

fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("database is locked") || lowered.contains("database busy") {
        return format!(
            "state_5.sqlite 当前被占用，请关闭 Codex / Codex App 后重试 ({}): {}",
            path.display(),
            message
        );
    }
    format!(
        "更新 SQLite provider 失败 ({}): {}",
        path.display(),
        message
    )
}

fn rewrite_rollout_provider(change: &RolloutProviderChange) -> Result<(), String> {
    write_bytes_atomic(&change.absolute_path, &change.updated_content)
}

fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建目录失败 ({}): {}", parent.display(), error))?;

    let temp_path = parent.join(format!(
        ".{}.provider-repair.{}.{}",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&temp_path, content)
        .map_err(|error| format!("写入临时文件失败 ({}): {}", temp_path.display(), error))?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("替换文件失败 ({}): {}", path.display(), error));
    }
    Ok(())
}

fn backup_instance_files(
    data_dir: &Path,
    rollout_changes: &[RolloutProviderChange],
    include_sqlite: bool,
    instance_id: &str,
    instance_name: &str,
    target_provider: &str,
) -> Result<PathBuf, String> {
    let backup_dir = create_session_visibility_repair_backup_dir(data_dir)?;

    let mut backed_up_files = Vec::new();
    for change in rollout_changes {
        let target = backup_dir.join("files").join(&change.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 备份目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&change.absolute_path, &target).map_err(|error| {
            format!(
                "备份 rollout 文件失败 ({} -> {}): {}",
                change.absolute_path.display(),
                target.display(),
                error
            )
        })?;
        backed_up_files.push(change.relative_path.to_string_lossy().to_string());
    }

    if include_sqlite {
        let db_path = data_dir.join(STATE_DB_FILE);
        if db_path.exists() {
            let target = backup_dir.join(STATE_DB_FILE);
            fs::copy(&db_path, &target).map_err(|error| {
                format!(
                    "备份 state_5.sqlite 失败 ({} -> {}): {}",
                    db_path.display(),
                    target.display(),
                    error
                )
            })?;
        }
    }

    let manifest = json!({
        "instanceId": instance_id,
        "instanceName": instance_name,
        "instanceRoot": data_dir,
        "targetProvider": target_provider,
        "createdAt": Utc::now().to_rfc3339(),
        "hasSqliteBackup": include_sqlite && data_dir.join(STATE_DB_FILE).exists(),
        "rolloutFiles": backed_up_files,
    });
    fs::write(
        backup_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("序列化可见性修复备份清单失败: {}", error))?
        ),
    )
    .map_err(|error| {
        format!(
            "写入可见性修复备份清单失败 ({}): {}",
            backup_dir.display(),
            error
        )
    })?;

    Ok(backup_dir)
}

fn create_session_visibility_repair_backup_dir(data_dir: &Path) -> Result<PathBuf, String> {
    for attempt in 0..16 {
        let now = Utc::now();
        let backup_dir_name = format!(
            "{}{}-{:09}-{}-{}{}",
            SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX,
            now.format("%Y%m%d-%H%M%S"),
            now.timestamp_subsec_nanos(),
            std::process::id(),
            attempt,
            SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX
        );
        let backup_dir = data_dir.join(backup_dir_name);
        match fs::create_dir(&backup_dir) {
            Ok(()) => return Ok(backup_dir),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "创建备份目录失败 ({}): {}",
                    backup_dir.display(),
                    error
                ));
            }
        }
    }

    Err(format!(
        "创建备份目录失败 ({})：连续生成的备份目录名均已存在",
        data_dir.display()
    ))
}

fn parse_session_visibility_repair_backup_timestamp(name: &str) -> Option<&str> {
    let sort_key = name
        .strip_prefix(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)?
        .strip_suffix(SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX)?;
    if sort_key.len() < 15 {
        return None;
    }
    let timestamp = &sort_key[..15];
    if timestamp.len() != 15 {
        return None;
    }
    if !timestamp.chars().enumerate().all(|(index, value)| {
        if index == 8 {
            value == '-'
        } else {
            value.is_ascii_digit()
        }
    }) {
        return None;
    }
    let extra = &sort_key[15..];
    if !extra.is_empty()
        && (!extra.starts_with('-')
            || extra.len() == 1
            || !extra[1..]
                .chars()
                .all(|value| value.is_ascii_digit() || value == '-'))
    {
        return None;
    }
    Some(sort_key)
}

fn prune_session_visibility_repair_backups(instances: &[CodexSyncInstance]) {
    for instance in instances {
        if let Err(error) = prune_instance_session_visibility_repair_backups(&instance.data_dir) {
            modules::logger::log_warn(&format!(
                "清理 Codex 会话可见性修复旧备份失败 ({}): {}",
                instance.data_dir.display(),
                error
            ));
        }
    }
}

fn prune_instance_session_visibility_repair_backups(data_dir: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "读取实例目录失败 ({}): {}",
                data_dir.display(),
                error
            ));
        }
    };
    let mut backups: Vec<(String, PathBuf)> = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|error| format!("读取实例目录项失败 ({}): {}", data_dir.display(), error))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取实例目录项类型失败 ({}): {}",
                entry.path().display(),
                error
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(timestamp) = parse_session_visibility_repair_backup_timestamp(file_name) else {
            continue;
        };
        backups.push((timestamp.to_string(), entry.path()));
    }

    if backups.len() <= MAX_SESSION_VISIBILITY_REPAIR_BACKUPS {
        return Ok(());
    }

    backups.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in backups
        .into_iter()
        .skip(MAX_SESSION_VISIBILITY_REPAIR_BACKUPS)
    {
        fs::remove_dir_all(&path)
            .map_err(|error| format!("删除旧备份失败 ({}): {}", path.display(), error))?;
    }

    Ok(())
}

fn restore_instance_files_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
    include_sqlite: bool,
) -> Result<(), String> {
    let files_root = backup_dir.join("files");
    if files_root.exists() {
        restore_directory_contents(&files_root, data_dir)?;
    }

    if include_sqlite {
        let backup_db_path = backup_dir.join(STATE_DB_FILE);
        if backup_db_path.exists() {
            let target_db_path = data_dir.join(STATE_DB_FILE);
            fs::copy(&backup_db_path, &target_db_path).map_err(|error| {
                format!(
                    "恢复 state_5.sqlite 失败 ({} -> {}): {}",
                    backup_db_path.display(),
                    target_db_path.display(),
                    error
                )
            })?;
        }
    }

    Ok(())
}

fn restore_directory_contents(source_root: &Path, target_root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(source_root)
        .map_err(|error| format!("读取备份目录失败 ({}): {}", source_root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取备份目录项失败 ({}): {}", source_root.display(), error)
        })?;
        let source_path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取备份文件类型失败 ({}): {}",
                source_path.display(),
                error
            )
        })?;
        let relative = source_path
            .strip_prefix(source_root)
            .map_err(|_| format!("无法计算备份相对路径: {}", source_path.display()))?;
        let target_path = target_root.join(relative);

        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                format!("创建恢复目录失败 ({}): {}", target_path.display(), error)
            })?;
            restore_directory_contents(&source_path, &target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建恢复父目录失败 ({}): {}", parent.display(), error))?;
        }
        fs::copy(&source_path, &target_path).map_err(|error| {
            format!(
                "恢复备份文件失败 ({} -> {}): {}",
                source_path.display(),
                target_path.display(),
                error
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cockpit-tools-codex-session-visibility-{}-{}-{}",
            name,
            std::process::id(),
            nonce
        ))
    }

    fn create_threads_db(data_dir: &Path, provider: &str) {
        let connection =
            Connection::open(data_dir.join(STATE_DB_FILE)).expect("state db should open");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("threads table should be created");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES (?1, ?2)",
                rusqlite::params!["thread-1", provider],
            )
            .expect("thread row should be inserted");
    }

    fn read_thread_provider(data_dir: &Path) -> String {
        let connection =
            Connection::open(data_dir.join(STATE_DB_FILE)).expect("state db should open");
        connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                ["thread-1"],
                |row| row.get::<usize, String>(0),
            )
            .expect("thread provider should be readable")
    }

    fn read_rollout_session_meta_providers(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .expect("rollout should be readable")
            .lines()
            .filter_map(|line| serde_json::from_str::<JsonValue>(line).ok())
            .filter(|value| value.get("type").and_then(JsonValue::as_str) == Some("session_meta"))
            .filter_map(|value| {
                value["payload"]["model_provider"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect()
    }

    #[test]
    fn repair_profile_dir_rewrites_rollout_and_sqlite_to_target_provider() {
        let data_dir = unique_temp_dir("rewrite");
        fs::create_dir_all(data_dir.join("sessions/2026/05/09"))
            .expect("session dir should be created");
        fs::write(data_dir.join(CONFIG_FILE_NAME), "model_provider = \"relay\"\n")
            .expect("config should be written");
        let rollout_path = data_dir.join("sessions/2026/05/09/rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"openai\"}}\n",
                "{\"type\":\"event\",\"payload\":{}}\n"
            ),
        )
        .expect("rollout should be written");
        create_threads_db(&data_dir, "openai");

        let item = repair_session_visibility_for_dir(&data_dir, "test", "Test", true)
            .expect("repair should succeed");

        assert_eq!(item.target_provider, "relay");
        assert_eq!(item.changed_rollout_file_count, 1);
        assert_eq!(item.updated_sqlite_row_count, 1);
        assert!(item.backup_dir.is_some());
        assert!(item.running);
        let backup_dir = PathBuf::from(item.backup_dir.as_deref().expect("backup dir"));
        let manifest: JsonValue = serde_json::from_str(
            &fs::read_to_string(backup_dir.join("manifest.json"))
                .expect("backup manifest should be readable"),
        )
        .expect("backup manifest should be valid json");
        assert_eq!(manifest["instanceId"].as_str(), Some("test"));
        assert_eq!(manifest["instanceName"].as_str(), Some("Test"));
        assert_eq!(read_thread_provider(&data_dir), "relay");

        let first_line = fs::read_to_string(&rollout_path)
            .expect("rollout should be readable")
            .lines()
            .next()
            .expect("rollout should have first line")
            .to_string();
        let parsed: JsonValue =
            serde_json::from_str(&first_line).expect("session meta should stay valid json");
        assert_eq!(
            parsed["payload"]["model_provider"].as_str(),
            Some("relay")
        );

        fs::remove_dir_all(&data_dir).expect("temp dir should be removed");
    }

    #[test]
    fn repair_profile_dir_rewrites_every_session_meta_in_rollout_jsonl() {
        let data_dir = unique_temp_dir("multi-session-meta");
        fs::create_dir_all(data_dir.join("sessions/2026/05/09"))
            .expect("session dir should be created");
        fs::write(data_dir.join(CONFIG_FILE_NAME), "model_provider = \"relay\"\n")
            .expect("config should be written");
        let rollout_path = data_dir.join("sessions/2026/05/09/rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"relay\"}}\n",
                "{\"type\":\"event\",\"payload\":{}}\n",
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"openai\"}}\n"
            ),
        )
        .expect("rollout should be written");
        create_threads_db(&data_dir, "relay");

        let item = repair_session_visibility_for_dir(&data_dir, "test", "Test", false)
            .expect("repair should succeed");

        assert_eq!(item.target_provider, "relay");
        assert_eq!(item.changed_rollout_file_count, 1);
        assert_eq!(item.updated_sqlite_row_count, 0);
        assert_eq!(
            read_rollout_session_meta_providers(&rollout_path),
            vec!["relay".to_string(), "relay".to_string()]
        );

        fs::remove_dir_all(&data_dir).expect("temp dir should be removed");
    }

    #[test]
    fn repair_profile_dir_noops_when_provider_metadata_already_matches() {
        let data_dir = unique_temp_dir("noop");
        fs::create_dir_all(&data_dir).expect("temp dir should be created");
        fs::write(data_dir.join(CONFIG_FILE_NAME), "model_provider = \"relay\"\n")
            .expect("config should be written");
        create_threads_db(&data_dir, "relay");

        let item = repair_session_visibility_for_dir(&data_dir, "test", "Test", false)
            .expect("repair should succeed");

        assert_eq!(item.target_provider, "relay");
        assert_eq!(item.changed_rollout_file_count, 0);
        assert_eq!(item.updated_sqlite_row_count, 0);
        assert!(item.backup_dir.is_none());
        assert!(!item.running);
        assert_eq!(read_thread_provider(&data_dir), "relay");

        fs::remove_dir_all(&data_dir).expect("temp dir should be removed");
    }

    #[test]
    fn backup_dir_names_are_collision_resistant_and_prunable() {
        let data_dir = unique_temp_dir("backup");
        fs::create_dir_all(&data_dir).expect("temp dir should be created");

        let first = create_session_visibility_repair_backup_dir(&data_dir)
            .expect("first backup dir should be created");
        let second = create_session_visibility_repair_backup_dir(&data_dir)
            .expect("second backup dir should be created");

        assert_ne!(first, second);
        assert!(
            parse_session_visibility_repair_backup_timestamp(
                first
                    .file_name()
                    .and_then(|item| item.to_str())
                    .expect("backup dir should have utf-8 name"),
            )
            .is_some()
        );
        assert!(
            parse_session_visibility_repair_backup_timestamp(
                second
                    .file_name()
                    .and_then(|item| item.to_str())
                    .expect("backup dir should have utf-8 name"),
            )
            .is_some()
        );
        assert!(
            parse_session_visibility_repair_backup_timestamp(
                "backup-20260509-152944-session-visibility-repair",
            )
            .is_some()
        );

        fs::remove_dir_all(&data_dir).expect("temp dir should be removed");
    }
}
