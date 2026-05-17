use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{types::Value, Connection, OpenFlags, Transaction};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use url::Url;

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const STATE_DB_FILE: &str = "state_5.sqlite";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const BACKUP_FILE_NAMES: [&str; 3] = [STATE_DB_FILE, SESSION_INDEX_FILE, GLOBAL_STATE_FILE];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceThreadSyncItem {
    pub instance_id: String,
    pub instance_name: String,
    pub added_thread_count: usize,
    pub backup_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceThreadSyncSummary {
    pub instance_count: usize,
    pub thread_universe_count: usize,
    pub mutated_instance_count: usize,
    pub total_synced_thread_count: usize,
    pub items: Vec<CodexInstanceThreadSyncItem>,
    pub backup_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceTargetThreadSyncSummary {
    pub requested_session_count: usize,
    pub target_instance_id: String,
    pub target_instance_name: String,
    pub synced_session_count: usize,
    pub skipped_existing_count: usize,
    pub missing_session_count: usize,
    pub backup_dir: Option<String>,
    pub running: bool,
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
struct ThreadRowData {
    columns: Vec<String>,
    values: Vec<Value>,
}

impl ThreadRowData {
    fn get_value(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .position(|item| item == column)
            .and_then(|index| self.values.get(index))
    }

    fn get_text(&self, column: &str) -> Option<String> {
        match self.get_value(column)? {
            Value::Text(value) => Some(value.clone()),
            Value::Integer(value) => Some(value.to_string()),
            Value::Real(value) => Some(value.to_string()),
            _ => None,
        }
    }

    fn get_i64(&self, column: &str) -> Option<i64> {
        match self.get_value(column)? {
            Value::Integer(value) => Some(*value),
            Value::Text(value) => value.parse::<i64>().ok(),
            _ => None,
        }
    }

    fn set_text(&mut self, column: &str, value: String) {
        if let Some(index) = self.columns.iter().position(|item| item == column) {
            if let Some(slot) = self.values.get_mut(index) {
                *slot = Value::Text(value);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ThreadSnapshot {
    id: String,
    cwd: String,
    rollout_path: PathBuf,
    row_data: ThreadRowData,
    session_index_entry: JsonValue,
    source_root: PathBuf,
}

pub fn sync_threads_across_instances() -> Result<CodexInstanceThreadSyncSummary, String> {
    let instances = collect_instances()?;
    if instances.len() < 2 {
        return Err("至少需要两个 Codex 实例才能同步线程".to_string());
    }

    let mut thread_universe = HashMap::<String, ThreadSnapshot>::new();
    let mut existing_ids_by_instance = HashMap::<String, HashSet<String>>::new();

    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?;
        let ids = snapshots
            .iter()
            .map(|item| item.id.clone())
            .collect::<HashSet<_>>();
        for snapshot in snapshots {
            thread_universe
                .entry(snapshot.id.clone())
                .or_insert(snapshot);
        }
        existing_ids_by_instance.insert(instance.id.clone(), ids);
    }

    let mut universe_ids = thread_universe.keys().cloned().collect::<Vec<_>>();
    universe_ids.sort();

    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut total_synced_thread_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    for instance in &instances {
        let existing_ids = existing_ids_by_instance
            .get(&instance.id)
            .cloned()
            .unwrap_or_default();
        let missing_snapshots = universe_ids
            .iter()
            .filter(|id| !existing_ids.contains(*id))
            .filter_map(|id| thread_universe.get(id).cloned())
            .collect::<Vec<_>>();

        if missing_snapshots.is_empty() {
            items.push(CodexInstanceThreadSyncItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                added_thread_count: 0,
                backup_dir: None,
            });
            continue;
        }

        let backup_dir = sync_missing_threads_to_instance(instance, &missing_snapshots)?;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();
        backup_dirs.push(backup_dir_string.clone());
        mutated_instance_count += 1;
        total_synced_thread_count += missing_snapshots.len();
        if is_instance_running(instance, &process_entries) {
            mutated_running_instance_count += 1;
        }

        items.push(CodexInstanceThreadSyncItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            added_thread_count: missing_snapshots.len(),
            backup_dir: Some(backup_dir_string),
        });
    }

    let message = if total_synced_thread_count == 0 {
        "所有 Codex 实例已是最新，无需同步线程".to_string()
    } else if mutated_running_instance_count > 0 {
        format!(
            "已为 {} 个实例补齐 {} 条线程，运行中的实例可能需要重启后显示",
            mutated_instance_count, total_synced_thread_count
        )
    } else {
        format!(
            "已为 {} 个实例补齐 {} 条线程",
            mutated_instance_count, total_synced_thread_count
        )
    };

    Ok(CodexInstanceThreadSyncSummary {
        instance_count: instances.len(),
        thread_universe_count: thread_universe.len(),
        mutated_instance_count,
        total_synced_thread_count,
        items,
        backup_dirs,
        message,
    })
}

pub fn sync_sessions_to_instance(
    session_ids: Vec<String>,
    target_instance_id: String,
) -> Result<CodexInstanceTargetThreadSyncSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let target_id = target_instance_id.trim();
    if target_id.is_empty() {
        return Err("请选择目标实例".to_string());
    }

    let instances = collect_instances()?;
    let target = instances
        .iter()
        .find(|instance| instance.id == target_id)
        .cloned()
        .ok_or_else(|| format!("目标实例不存在: {}", target_id))?;

    let mut source_snapshots = HashMap::<String, ThreadSnapshot>::new();
    let mut target_existing_ids = HashSet::<String>::new();
    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?;
        if instance.id == target.id {
            target_existing_ids = snapshots
                .iter()
                .map(|snapshot| snapshot.id.clone())
                .collect::<HashSet<_>>();
            continue;
        }

        for snapshot in snapshots {
            if requested_ids.contains(&snapshot.id) {
                source_snapshots
                    .entry(snapshot.id.clone())
                    .or_insert(snapshot);
            }
        }
    }

    let mut snapshots_to_sync = Vec::new();
    let mut skipped_existing_count = 0usize;
    let mut missing_session_count = 0usize;
    let mut ordered_ids = requested_ids.iter().cloned().collect::<Vec<_>>();
    ordered_ids.sort();
    for session_id in ordered_ids {
        if target_existing_ids.contains(&session_id) {
            skipped_existing_count += 1;
            continue;
        }
        match source_snapshots.get(&session_id) {
            Some(snapshot) => snapshots_to_sync.push(snapshot.clone()),
            None => missing_session_count += 1,
        }
    }

    let process_entries = modules::process::collect_codex_process_entries();
    let running = is_instance_running(&target, &process_entries);

    if snapshots_to_sync.is_empty() {
        let message = if skipped_existing_count > 0 && missing_session_count == 0 {
            format!(
                "目标实例已存在所选 {} 条会话，无需恢复",
                skipped_existing_count
            )
        } else {
            "所选会话在其他实例中不存在，无法恢复到目标实例".to_string()
        };
        return Ok(CodexInstanceTargetThreadSyncSummary {
            requested_session_count: requested_ids.len(),
            target_instance_id: target.id,
            target_instance_name: target.name,
            synced_session_count: 0,
            skipped_existing_count,
            missing_session_count,
            backup_dir: None,
            running,
            message,
        });
    }

    let backup_dir = sync_missing_threads_to_instance(&target, &snapshots_to_sync)?;
    let synced_session_count = snapshots_to_sync.len();
    let message = if running {
        format!(
            "已恢复 {} 条会话到「{}」，目标实例运行中，可能需要重启后显示",
            synced_session_count, target.name
        )
    } else {
        format!(
            "已恢复 {} 条会话到「{}」",
            synced_session_count, target.name
        )
    };

    Ok(CodexInstanceTargetThreadSyncSummary {
        requested_session_count: requested_ids.len(),
        target_instance_id: target.id,
        target_instance_name: target.name,
        synced_session_count,
        skipped_existing_count,
        missing_session_count,
        backup_dir: Some(backup_dir.to_string_lossy().to_string()),
        running,
        message,
    })
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

fn load_thread_snapshots(instance: &CodexSyncInstance) -> Result<Vec<ThreadSnapshot>, String> {
    let db_path = instance.data_dir.join(STATE_DB_FILE);
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let connection = match open_readonly_connection(&db_path) {
        Ok(connection) => connection,
        Err(error) if should_skip_state_db_message(&error) => {
            log_skipped_state_db(&instance.name, &db_path, &error);
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let columns = match read_thread_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if should_skip_state_db_message(&error) => {
            log_skipped_state_db(&instance.name, &db_path, &error);
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let select_columns = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!("SELECT {} FROM threads", select_columns);
    let mut statement = match connection.prepare(&query) {
        Ok(statement) => statement,
        Err(error) if should_skip_state_db_error(&error) => {
            log_skipped_state_db(&instance.name, &db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!("读取实例线程失败 ({}): {}", instance.name, error));
        }
    };
    let mut rows = match statement.query([]) {
        Ok(rows) => rows,
        Err(error) if should_skip_state_db_error(&error) => {
            log_skipped_state_db(&instance.name, &db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!("查询实例线程失败 ({}): {}", instance.name, error));
        }
    };
    let session_index_map = read_session_index_map(&instance.data_dir)?;

    let mut snapshots = Vec::new();
    loop {
        let Some(row) = (match rows.next() {
            Ok(row) => row,
            Err(error) if should_skip_state_db_error(&error) => {
                log_skipped_state_db(&instance.name, &db_path, &error.to_string());
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(format!("迭代实例线程失败 ({}): {}", instance.name, error));
            }
        }) else {
            break;
        };

        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(
                row.get::<usize, Value>(index)
                    .map_err(|error| format!("解析线程记录失败 ({}): {}", instance.name, error))?,
            );
        }

        let row_data = ThreadRowData {
            columns: columns.clone(),
            values,
        };
        let id = row_data
            .get_text("id")
            .ok_or_else(|| format!("线程缺少 id 字段 ({})", instance.name))?;
        let rollout_path = row_data
            .get_text("rollout_path")
            .ok_or_else(|| format!("线程 {} 缺少 rollout_path ({})", id, instance.name))?;
        let title = row_data
            .get_text("title")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let cwd = row_data
            .get_text("cwd")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "未知工作区".to_string());
        let updated_at = row_data.get_i64("updated_at").and_then(format_timestamp);
        let session_index_entry = session_index_map.get(&id).cloned().unwrap_or_else(|| {
            build_fallback_session_index_entry(&id, &title, updated_at.as_deref())
        });

        snapshots.push(ThreadSnapshot {
            id,
            cwd,
            rollout_path: PathBuf::from(rollout_path),
            row_data,
            session_index_entry,
            source_root: instance.data_dir.clone(),
        });
    }

    Ok(snapshots)
}

fn sync_missing_threads_to_instance(
    target: &CodexSyncInstance,
    snapshots: &[ThreadSnapshot],
) -> Result<PathBuf, String> {
    let backup_dir = backup_instance_files(&target.data_dir)?;
    let index_map = read_session_index_map(&target.data_dir)?;
    let existing_index_ids = index_map.keys().cloned().collect::<HashSet<_>>();
    let target_provider =
        modules::codex_session_visibility::read_history_visibility_provider_for_dir(
            &target.data_dir,
        )?;
    let db_path = target.data_dir.join(STATE_DB_FILE);
    let mut connection = Connection::open(&db_path)
        .map_err(|error| format!("打开目标实例数据库失败 ({}): {}", target.name, error))?;
    let target_columns = read_thread_columns(&connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启目标实例事务失败 ({}): {}", target.name, error))?;

    for snapshot in snapshots {
        let target_rollout_path = copy_rollout_file(snapshot, &target.data_dir)?;
        rewrite_rollout_provider_for_target(&target_rollout_path, &target_provider)?;
        let mut row_data = snapshot.row_data.clone();
        row_data.set_text(
            "rollout_path",
            target_rollout_path.to_string_lossy().to_string(),
        );
        row_data.set_text("model_provider", target_provider.clone());
        insert_thread_row(&transaction, &target_columns, &row_data)?;
    }

    transaction
        .commit()
        .map_err(|error| format!("提交目标实例事务失败 ({}): {}", target.name, error))?;

    append_session_index_entries(&target.data_dir, &existing_index_ids, snapshots)?;
    update_global_state(
        &target.data_dir,
        snapshots.iter().map(|snapshot| snapshot.cwd.as_str()),
    )?;

    Ok(backup_dir)
}

fn open_readonly_connection(db_path: &Path) -> Result<Connection, String> {
    let mut uri = Url::from_file_path(db_path)
        .map_err(|_| format!("无法构建只读数据库 URI: {}", db_path.display()))?;
    uri.set_query(Some("mode=ro"));
    Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("打开只读数据库失败 ({}): {}", db_path.display(), error))
}

fn should_skip_state_db_error(error: &rusqlite::Error) -> bool {
    modules::db::is_unusable_sqlite_database_error(error)
        || error
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table: threads")
}

fn should_skip_state_db_message(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    modules::db::is_unusable_sqlite_database_message(message)
        || lowered.contains("no such table: threads")
        || message.contains("threads 表不存在或没有列定义")
}

fn log_skipped_state_db(instance_name: &str, db_path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "跳过无法读取的 Codex 线程数据库 ({} / {}): {}",
        instance_name,
        db_path.display(),
        reason
    ));
}

fn read_thread_columns(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| format!("读取 threads 表结构失败: {}", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("查询 threads 表结构失败: {}", error))?;
    let mut columns = Vec::new();

    while let Some(row) = rows
        .next()
        .map_err(|error| format!("解析 threads 表结构失败: {}", error))?
    {
        columns.push(
            row.get::<usize, String>(1)
                .map_err(|error| format!("解析 threads 列失败: {}", error))?,
        );
    }

    if columns.is_empty() {
        return Err("threads 表不存在或没有列定义".to_string());
    }

    Ok(columns)
}

fn backup_instance_files(data_dir: &Path) -> Result<PathBuf, String> {
    let backup_dir = data_dir.join(format!(
        "backup-{}-instance-thread-sync",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("创建备份目录失败 ({}): {}", data_dir.display(), error))?;

    for file_name in BACKUP_FILE_NAMES {
        let source = data_dir.join(file_name);
        if !source.exists() {
            continue;
        }
        let target = backup_dir.join(format!("{}.bak", file_name));
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "备份文件失败 ({} -> {}): {}",
                source.display(),
                target.display(),
                error
            )
        })?;
    }

    Ok(backup_dir)
}

fn read_session_index_map(root_dir: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    let mut entries = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = parsed.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), parsed);
    }

    Ok(entries)
}

fn build_fallback_session_index_entry(
    id: &str,
    title: &str,
    updated_at: Option<&str>,
) -> JsonValue {
    let mut value = json!({
        "id": id,
        "thread_name": title,
    });
    if let Some(updated_at) = updated_at {
        value["updated_at"] = JsonValue::String(updated_at.to_string());
    }
    value
}

fn append_session_index_entries(
    root_dir: &Path,
    existing_ids: &HashSet<String>,
    snapshots: &[ThreadSnapshot],
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    let mut lines = Vec::new();

    for snapshot in snapshots {
        if existing_ids.contains(&snapshot.id) {
            continue;
        }
        lines.push(
            serde_json::to_string(&snapshot.session_index_entry)
                .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?,
        );
    }

    if lines.is_empty() {
        return Ok(());
    }

    let needs_prefix = path.exists() && !file_ends_with_newline(&path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "打开 session_index.jsonl 失败 ({}): {}",
                path.display(),
                error
            )
        })?;

    use std::io::Write;
    if needs_prefix {
        file.write_all(b"\n").map_err(|error| {
            format!(
                "写入 session_index 换行失败 ({}): {}",
                path.display(),
                error
            )
        })?;
    }

    for line in lines {
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| {
                format!(
                    "追加 session_index 条目失败 ({}): {}",
                    path.display(),
                    error
                )
            })?;
    }

    Ok(())
}

fn update_global_state<'a>(
    root_dir: &Path,
    workspaces: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    let path = root_dir.join(GLOBAL_STATE_FILE);
    let mut value = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("读取全局状态失败 ({}): {}", path.display(), error))?;
        serde_json::from_str::<JsonValue>(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !value.is_object() {
        value = json!({});
    }

    let Some(object) = value.as_object_mut() else {
        return Err("全局状态文件格式无效".to_string());
    };

    let unique_workspaces = workspaces
        .filter(|item| !item.trim().is_empty())
        .map(|item| item.to_string())
        .collect::<HashSet<_>>();

    merge_string_array(object, "project-order", &unique_workspaces);
    merge_string_array(object, "electron-saved-workspace-roots", &unique_workspaces);

    let serialized = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("序列化全局状态失败: {}", error))?;
    fs::write(&path, format!("{}\n", serialized))
        .map_err(|error| format!("写入全局状态失败 ({}): {}", path.display(), error))?;
    Ok(())
}

fn merge_string_array(
    object: &mut serde_json::Map<String, JsonValue>,
    key: &str,
    additions: &HashSet<String>,
) {
    let mut values = object
        .get(key)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.as_str().map(|value| value.to_string()))
        .collect::<Vec<_>>();

    for addition in additions {
        if !values.contains(addition) {
            values.push(addition.clone());
        }
    }

    object.insert(
        key.to_string(),
        JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
    );
}

fn copy_rollout_file(snapshot: &ThreadSnapshot, target_root: &Path) -> Result<PathBuf, String> {
    let relative_path = snapshot
        .rollout_path
        .strip_prefix(&snapshot.source_root)
        .map_err(|_| {
            format!(
                "线程 {} 的 rollout 路径不在实例目录下: {}",
                snapshot.id,
                snapshot.rollout_path.display()
            )
        })?;
    let target_path = target_root.join(relative_path);
    let parent = target_path
        .parent()
        .ok_or_else(|| format!("无法解析目标 rollout 父目录: {}", target_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 rollout 目录失败 ({}): {}", parent.display(), error))?;
    fs::copy(&snapshot.rollout_path, &target_path).map_err(|error| {
        format!(
            "复制 rollout 文件失败 ({} -> {}): {}",
            snapshot.rollout_path.display(),
            target_path.display(),
            error
        )
    })?;
    Ok(target_path)
}

fn rewrite_rollout_provider_for_target(
    rollout_path: &Path,
    target_provider: &str,
) -> Result<(), String> {
    let content = fs::read_to_string(rollout_path).map_err(|error| {
        format!(
            "读取目标 rollout 文件失败 ({}): {}",
            rollout_path.display(),
            error
        )
    })?;
    let Some(newline_index) = content.find('\n') else {
        return Ok(());
    };
    let first_line = &content[..newline_index];
    let rest = &content[newline_index..];
    let Ok(mut parsed) = serde_json::from_str::<JsonValue>(first_line) else {
        return Ok(());
    };
    if parsed.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return Ok(());
    }
    let Some(payload) = parsed.get_mut("payload").and_then(JsonValue::as_object_mut) else {
        return Ok(());
    };
    if payload.get("model_provider").and_then(JsonValue::as_str) == Some(target_provider) {
        return Ok(());
    }

    payload.insert(
        "model_provider".to_string(),
        JsonValue::String(target_provider.to_string()),
    );
    let updated_first_line = serde_json::to_string(&parsed)
        .map_err(|error| format!("序列化 rollout provider 元数据失败: {}", error))?;
    fs::write(rollout_path, format!("{}{}", updated_first_line, rest)).map_err(|error| {
        format!(
            "写入目标 rollout provider 元数据失败 ({}): {}",
            rollout_path.display(),
            error
        )
    })?;
    Ok(())
}

fn insert_thread_row(
    transaction: &Transaction<'_>,
    target_columns: &[String],
    row_data: &ThreadRowData,
) -> Result<(), String> {
    let mut columns = Vec::new();
    let mut values = Vec::new();

    for column in target_columns {
        if let Some(value) = row_data.get_value(column) {
            columns.push(quote_identifier(column));
            values.push(to_sql_literal(value));
        }
    }

    if columns.is_empty() {
        return Err("没有可写入的 threads 列".to_string());
    }

    let sql = format!(
        "INSERT OR REPLACE INTO threads ({}) VALUES ({})",
        columns.join(", "),
        values.join(", ")
    );

    transaction
        .execute(&sql, [])
        .map_err(|error| format!("写入 threads 表失败: {}", error))?;
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn to_sql_literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => {
            if number.is_finite() {
                number.to_string()
            } else {
                "NULL".to_string()
            }
        }
        Value::Text(text) => format!("'{}'", text.replace('\'', "''")),
        Value::Blob(bytes) => format!(
            "X'{}'",
            bytes
                .iter()
                .map(|byte| format!("{:02X}", byte))
                .collect::<String>()
        ),
    }
}

fn file_ends_with_newline(path: &Path) -> Result<bool, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("读取文件失败 ({}): {}", path.display(), error))?;
    Ok(bytes.is_empty() || bytes.last() == Some(&b'\n'))
}

fn format_timestamp(timestamp: i64) -> Option<String> {
    if timestamp > 1_000_000_000_000 {
        chrono::DateTime::<Utc>::from_timestamp_millis(timestamp)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true))
    } else {
        chrono::DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true))
    }
}
