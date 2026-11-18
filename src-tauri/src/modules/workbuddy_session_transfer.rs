//! WorkBuddy 本地会话跨账号合并。
//!
//! WorkBuddy 5.x 将会话正文按 UID 存在 `WorkBuddyExtension/Data`，并在每个
//! 实例的 `workbuddy.db` 中按 `user_id` 过滤会话。旧版使用
//! `CodeBuddyExtension/Data` 与 `codebuddy-sessions.vscdb`，因此切号时需要按
//! 当前真实落盘结构同步正文并重映射索引。

use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{codebuddy_session_transfer, logger, workbuddy_account, workbuddy_instance};

static TRANSFER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkbuddySessionTransferReport {
    pub added_conversations: usize,
    pub replaced_conversations: usize,
    pub updated_session_rows: usize,
    pub scanned_workspaces: usize,
    pub added_pins: usize,
}

#[derive(Clone, Copy)]
struct WorkbuddyAccountScope<'a> {
    uid: &'a str,
    enterprise_id: Option<&'a str>,
}

/// 在写入目标账号认证前合并来源账号的本地会话。
///
/// `runtime_dir` 可以是 WorkBuddy config root，也可以是其 Electron `app` 目录。
pub fn prepare_account_switch(
    runtime_dir: &Path,
    target: &WorkbuddyAccount,
) -> Result<WorkbuddySessionTransferReport, String> {
    let mut report = WorkbuddySessionTransferReport::default();
    if crate::modules::config::get_user_config().workbuddy_share_sessions_on_switch {
        let target_uid = target
            .uid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("目标 WorkBuddy 账号缺少 UID，无法共享本地会话")?;

        if let Some(current) = workbuddy_account::import_payload_from_local_prepared()? {
            let source_uid = current
                .uid
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or("当前 WorkBuddy 账号缺少 UID，无法共享本地会话")?;
            if source_uid != target_uid {
                report = transfer_local_sessions(
                    runtime_dir,
                    WorkbuddyAccountScope { uid: source_uid, enterprise_id: current.enterprise_id.as_deref() },
                    WorkbuddyAccountScope { uid: target_uid, enterprise_id: target.enterprise_id.as_deref() },
                )?;
            }
        }
    }

    workbuddy_account::write_account_to_default_client(target)?;
    Ok(report)
}

fn transfer_local_sessions(
    runtime_dir: &Path,
    source: WorkbuddyAccountScope<'_>,
    target: WorkbuddyAccountScope<'_>,
) -> Result<WorkbuddySessionTransferReport, String> {
    let (source_uid, target_uid) = (source.uid, target.uid);
    validate_uid(source_uid)?;
    validate_uid(target_uid)?;
    if source_uid == target_uid {
        return Ok(WorkbuddySessionTransferReport::default());
    }

    let transfer_lock = TRANSFER_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = transfer_lock
        .try_lock()
        .map_err(|_| "WorkBuddy 本地会话合并正在进行，请稍后重试".to_string())?;

    let runtime_dir_string = runtime_dir.to_string_lossy().to_string();
    let (config_dir, electron_data_dir) =
        workbuddy_instance::resolve_workbuddy_runtime_dirs(&runtime_dir_string)?;
    let backup_root = crate::modules::backup_storage::behavior_backup_dir(
        "workbuddy",
        &crate::modules::backup_storage::scope_for_path(runtime_dir),
        &operation_id(),
    )?;

    let mut report = WorkbuddySessionTransferReport::default();
    let extension_roots = select_extension_data_roots(source_uid)?;
    if extension_roots.is_empty() {
        // WorkBuddy 5.x 会话统一存放在 workbuddy.db（按 user_id 区分），不存在
        // 旧版的 per-uid 本地会话目录，因此这里只跳过目录级合并，仍需执行下方
        // 的数据库层 user_id 重映射（这才是 WorkBuddy 真正生效的会话合并）。
        logger::log_warn(&format!(
            "[WorkBuddy Session Transfer] 未找到来源账号本地会话目录（uid={}），跳过目录级合并，将继续执行数据库层合并",
            source_uid
        ));
    } else {
        for (label, extension_data_dir) in extension_roots {
            let file_report = codebuddy_session_transfer::sync_history_between_accounts(
                &extension_data_dir,
                source_uid,
                target_uid,
                &backup_root.join(label),
            )
            .map_err(workbuddy_error)?;
            report.added_conversations += file_report.added_conversations;
            report.replaced_conversations += file_report.replaced_conversations;
            report.scanned_workspaces += file_report.scanned_workspaces;
        }
    }

    report.updated_session_rows += remap_workbuddy_database_user_id(
        &config_dir.join("workbuddy.db"),
        target_uid,
        &backup_root.join("database"),
    )?;
    for legacy_db in [
        electron_data_dir.join("codebuddy-sessions.vscdb"),
        config_dir.join("codebuddy-sessions.vscdb"),
    ] {
        if !legacy_db.is_file() {
            continue;
        }
        report.updated_session_rows += codebuddy_session_transfer::remap_session_database_user_id(
            &legacy_db,
            source_uid,
            target_uid,
            &backup_root.join("legacy-database"),
        )
        .map_err(workbuddy_error)?;
        break;
    }

    match transfer_local_pins(&config_dir, source, target, &backup_root.join("pins")) {
        Ok(added) => report.added_pins = added,
        Err(error) => logger::log_warn(&format!(
            "[WorkBuddy Session Transfer] 置顶任务同步失败: {error}"
        )),
    }

    logger::log_info(&format!(
        "[WorkBuddy Session Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, added={}, replaced={}, db_rows={}, pins={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.added_conversations,
        report.replaced_conversations,
        report.updated_session_rows,
        report.added_pins
    ));
    let _ = crate::modules::backup_storage::prune_behavior_backups(
        "workbuddy",
        &crate::modules::backup_storage::scope_for_path(runtime_dir),
    );
    Ok(report)
}

fn pinned_conversation_paths(
    config_dir: &Path,
    scope: WorkbuddyAccountScope<'_>,
) -> Result<[PathBuf; 3], String> {
    let enterprise_id = scope
        .enterprise_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("personal");
    validate_uid(scope.uid)?;
    validate_uid(enterprise_id)?;
    let storage = config_dir.join("storage");
    let account = storage.join(format!("user-{}-{enterprise_id}", scope.uid));
    let legacy_account = storage.join(format!("user-{}", scope.uid));
    let global = account.join("global");
    for directory in [&storage, &account, &global, &legacy_account] {
        reject_symlink_if_exists(directory)?;
    }
    Ok([
        global.join("conversations.json"),
        account.join("conversations.json"),
        legacy_account.join("conversations.json"),
    ])
}

fn read_conversation_namespace(path: &Path) -> Result<Map<String, Value>, String> {
    reject_symlink_if_exists(path)?;
    match fs::read_to_string(path) {
        Ok(raw) => {
            let namespace: Map<String, Value> = serde_json::from_str(&raw)
                .map_err(|error| format!("解析 WorkBuddy 置顶任务失败: {error}"))?;
            if namespace.get("pinned").is_some_and(|items| !items.is_array()) {
                return Err(format!("WorkBuddy 置顶任务字段不是列表: {}", path.display()));
            }
            Ok(namespace)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(format!("读取 WorkBuddy 置顶任务失败: {error}")),
    }
}

fn pinned_items(namespace: &Map<String, Value>) -> impl Iterator<Item = &Value> {
    namespace
        .get("pinned")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn pinned_id(item: &Value) -> Option<&str> {
    item.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn transfer_local_pins(
    config_dir: &Path,
    source: WorkbuddyAccountScope<'_>,
    target: WorkbuddyAccountScope<'_>,
    backup_dir: &Path,
) -> Result<usize, String> {
    let db_path = config_dir.join("workbuddy.db");
    let [source_path, source_legacy, source_uid_legacy] =
        pinned_conversation_paths(config_dir, source)?;
    let mut source_pins = Vec::new();
    for path in [&source_path, &source_legacy, &source_uid_legacy] {
        source_pins.extend(pinned_items(&read_conversation_namespace(path)?).cloned());
    }
    if source_pins.is_empty() || !db_path.is_file() {
        return Ok(0);
    }

    // The session transfer has already assigned the local index to the target.
    // Cloud conversations may also have cached rows, with a different transport.
    reject_symlink_if_exists(&db_path)?;
    let connection =
        Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("打开 WorkBuddy 置顶任务索引失败: {error}"))?;
    let columns = connection
        .prepare("SELECT name FROM pragma_table_info('sessions')")
        .and_then(|mut query| {
            query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()
        })
        .map_err(|error| format!("读取 WorkBuddy 置顶任务索引结构失败: {error}"))?;
    if columns.is_empty() {
        return Ok(0);
    }
    let sql = if columns.contains("transport") {
        "SELECT id FROM sessions WHERE user_id = ?1 AND deleted_at IS NULL AND transport = 'local'"
    } else {
        "SELECT id FROM sessions WHERE user_id = ?1 AND deleted_at IS NULL"
    };
    let local_ids = connection
        .prepare(sql)
        .and_then(|mut query| {
            query
                .query_map([target.uid], |row| row.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()
        })
        .map_err(|error| format!("读取 WorkBuddy 本地任务失败: {error}"))?;
    source_pins.retain(|item| pinned_id(item).is_some_and(|id| local_ids.contains(id)));
    if source_pins.is_empty() {
        return Ok(0);
    }

    let [target_path, target_legacy, target_uid_legacy] =
        pinned_conversation_paths(config_dir, target)?;
    let mut namespace = read_conversation_namespace(&target_path)?;
    let mut pins: Vec<Value> = pinned_items(&namespace).cloned().collect();
    let mut seen: HashSet<String> = pins
        .iter()
        .filter_map(pinned_id)
        .map(str::to_owned)
        .collect();
    for path in [&target_legacy, &target_uid_legacy] {
        pins.extend(
            pinned_items(&read_conversation_namespace(path)?)
                .filter(|item| pinned_id(item).is_some_and(|id| seen.insert(id.to_owned())))
                .cloned(),
        );
    }
    let target_count = pins.len();
    pins.extend(
        source_pins
            .into_iter()
            .filter(|item| pinned_id(item).is_some_and(|id| seen.insert(id.to_owned()))),
    );
    let added = pins.len() - target_count;
    if added == 0 {
        return Ok(0);
    }

    namespace.insert("pinned".into(), Value::Array(pins));
    if target_path.is_file() {
        fs::create_dir_all(backup_dir)
            .and_then(|()| fs::copy(&target_path, backup_dir.join("conversations.json")))
            .map_err(|error| format!("备份 WorkBuddy 置顶任务失败: {error}"))?;
    }
    let content = serde_json::to_string_pretty(&namespace).map_err(|error| error.to_string())?;
    crate::modules::atomic_write::write_secret_string_atomic(&target_path, &content)?;
    Ok(added)
}

fn validate_uid(uid: &str) -> Result<(), String> {
    let trimmed = uid.trim();
    if trimmed.is_empty() || trimmed != uid {
        return Err("WorkBuddy UID 为空或包含首尾空白".to_string());
    }
    if uid.contains('/') || uid.contains('\\') || uid.contains("..") || uid.contains('\0') {
        return Err("WorkBuddy UID 包含不安全的路径字符".to_string());
    }
    let mut components = Path::new(uid).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("WorkBuddy UID 不是安全的单级路径".to_string()),
    }
}

fn select_extension_data_roots(source_uid: &str) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".local").join("share");

    let current = base.join("WorkBuddyExtension").join("Data");
    if account_has_history(&current, source_uid)? {
        return Ok(vec![("workbuddy-extension", current)]);
    }

    let legacy = base.join("CodeBuddyExtension").join("Data");
    if account_has_history(&legacy, source_uid)? {
        return Ok(vec![("legacy-codebuddy-extension", legacy)]);
    }
    Ok(Vec::new())
}

fn account_has_history(extension_data_dir: &Path, uid: &str) -> Result<bool, String> {
    let account_root = extension_data_dir.join(uid);
    if !account_root.is_dir() {
        return Ok(false);
    }
    reject_symlink_if_exists(&account_root)?;
    for entry in fs::read_dir(&account_root).map_err(|error| {
        format!(
            "读取 WorkBuddy 会话根目录失败: path={}, error={}",
            account_root.display(),
            error
        )
    })? {
        let entry = entry.map_err(|error| format!("读取 WorkBuddy 会话目录失败: {}", error))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("读取 WorkBuddy 会话目录属性失败: {}", error))?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && entry.path().join(uid).join("history").is_dir()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn remap_workbuddy_database_user_id(
    db_path: &Path,
    target_uid: &str,
    backup_root: &Path,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = Connection::open(db_path).map_err(|error| {
        format!(
            "打开 WorkBuddy 会话数据库失败: path={}, error={}",
            db_path.display(),
            error
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("设置 WorkBuddy 会话数据库超时失败: {}", error))?;

    let has_sessions_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查 WorkBuddy 会话数据库结构失败: {}", error))?;
    if !has_sessions_table {
        return Ok(0);
    }

    let update_count: usize = connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id != ?1 AND deleted_at IS NULL",
            [target_uid],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取 WorkBuddy 会话索引失败: {}", error))?;
    if update_count == 0 {
        return Ok(0);
    }

    fs::create_dir_all(backup_root).map_err(|error| {
        format!(
            "创建 WorkBuddy 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            error
        )
    })?;
    backup_sqlite_database(db_path, backup_root)?;

    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启 WorkBuddy 会话数据库事务失败: {}", error))?;
    let changed = transaction
        .execute(
            "UPDATE sessions SET user_id = ?1 WHERE user_id != ?1 AND deleted_at IS NULL",
            [target_uid],
        )
        .map_err(|error| format!("更新 WorkBuddy 会话 user_id 失败: {}", error))?;
    transaction
        .commit()
        .map_err(|error| format!("提交 WorkBuddy 会话数据库事务失败: {}", error))?;
    Ok(changed)
}

fn backup_sqlite_database(db_path: &Path, backup_root: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let source = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            let mut path = db_path.as_os_str().to_os_string();
            path.push(suffix);
            PathBuf::from(path)
        };
        if !source.is_file() {
            continue;
        }
        reject_symlink_if_exists(&source)?;
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("无法解析 WorkBuddy 数据库文件名: {}", source.display()))?;
        fs::copy(&source, backup_root.join(file_name)).map_err(|error| {
            format!(
                "备份 WorkBuddy 会话数据库失败: path={}, error={}",
                source.display(),
                error
            )
        })?;
    }
    Ok(())
}

fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 WorkBuddy 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "读取 WorkBuddy 会话路径属性失败: path={}, error={}",
            path.display(),
            error
        )),
    }
}

fn operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", timestamp, std::process::id())
}

fn workbuddy_error(error: String) -> String {
    error
        .replace("CodeBuddy 本地会话", "WorkBuddy 本地会话")
        .replace("CodeBuddy 会话", "WorkBuddy 会话")
        .replace("CodeBuddy IDE", "WorkBuddy IDE")
        .replace("CodeBuddy 工作区", "WorkBuddy 工作区")
        .replace("CodeBuddy 目标", "WorkBuddy 目标")
        .replace("CodeBuddy conversationId", "WorkBuddy conversationId")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cockpit-workbuddy-session-transfer-{}-{}-{}",
                std::process::id(),
                operation_id(),
                id
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rejects_unsafe_uids() {
        for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
            assert!(validate_uid(uid).is_err(), "uid should be rejected: {uid}");
        }
        assert!(validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn remaps_all_active_sessions_and_preserves_deleted_sessions() {
        let temp = TestDir::new();
        let db_path = temp.0.join("workbuddy.db");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    cwd TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER
                );
                INSERT INTO sessions VALUES ('a', '/', 'source', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('b', '/', 'other', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('c', '/', 'source', 'Done', 1, 2, 3);
                INSERT INTO sessions VALUES ('d', '/', 'target', 'Done', 1, 2, NULL);",
            )
            .unwrap();
        drop(connection);

        let changed =
            remap_workbuddy_database_user_id(&db_path, "target", &temp.0.join("backup")).unwrap();
        assert_eq!(changed, 2);

        let connection = Connection::open(&db_path).unwrap();
        let active_non_target: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE deleted_at IS NULL AND user_id != 'target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let deleted_user: String = connection
            .query_row("SELECT user_id FROM sessions WHERE id = 'c'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(active_non_target, 0);
        assert_eq!(deleted_user, "source");
        assert!(temp.0.join("backup/workbuddy.db").is_file());
    }

    #[test]
    fn transfers_local_pins_after_session_remap_without_changing_target_order() {
        let temp = TestDir::new();
        let source = WorkbuddyAccountScope {
            uid: "source",
            enterprise_id: Some("org"),
        };
        let target = WorkbuddyAccountScope {
            uid: "target",
            enterprise_id: None,
        };
        let source_path = temp
            .0
            .join("storage/user-source-org/global/conversations.json");
        let legacy_path = temp.0.join("storage/user-source/conversations.json");
        let target_path = temp
            .0
            .join("storage/user-target-personal/global/conversations.json");
        for (path, body) in [
            (
                &source_path,
                json!({"pinned":[
                    {"id":"local","groupKey":"workspace-1"},
                    {"id":"duplicate","groupKey":"source"},
                    {"id":"cloud","groupKey":"cloud"},
                    {"id":"deleted","groupKey":"workspace-1"},
                    {"id":"missing","groupKey":"workspace-1"}
                ]}),
            ),
            (
                &legacy_path,
                json!({"pinned":[{"id":"legacy","groupKey":"workspace-2"}]}),
            ),
            (
                &target_path,
                json!({"pinned":[{"id":"duplicate","groupKey":"target"}],"keep":true}),
            ),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body.to_string()).unwrap();
        }
        let original_source = fs::read(&source_path).unwrap();
        let original_target = fs::read(&target_path).unwrap();
        let db_path = temp.0.join("workbuddy.db");
        Connection::open(&db_path).unwrap().execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE sessions (id TEXT PRIMARY KEY, user_id TEXT, deleted_at INTEGER, transport TEXT);
             INSERT INTO sessions VALUES ('local', 'source', NULL, 'local'),
                 ('legacy', 'source', NULL, 'local'), ('duplicate', 'target', NULL, 'local'),
                 ('cloud', 'source', -1, 'cloud'), ('deleted', 'source', 8, 'local');",
        ).unwrap();

        let backup = temp.0.join("backup");
        assert_eq!(
            remap_workbuddy_database_user_id(&db_path, target.uid, &backup.join("database"))
                .unwrap(),
            2
        );
        for unreadable_target in [
            "invalid JSON",
            r#"{"pinned":{"id":"preserve-me","groupKey":"target"},"keep":true}"#,
        ] {
            fs::write(&target_path, unreadable_target).unwrap();
            assert!(transfer_local_pins(&temp.0, source, target, &backup.join("pins")).is_err());
            assert_eq!(fs::read_to_string(&target_path).unwrap(), unreadable_target);
        }
        fs::write(&target_path, &original_target).unwrap();
        assert_eq!(
            transfer_local_pins(&temp.0, source, target, &backup.join("pins")).unwrap(),
            2
        );
        let expected = json!({"pinned":[
            {"id":"duplicate","groupKey":"target"},
            {"id":"local","groupKey":"workspace-1"},
            {"id":"legacy","groupKey":"workspace-2"},
        ],"keep":true});
        assert_eq!(
            Value::Object(read_conversation_namespace(&target_path).unwrap()),
            expected
        );
        assert_eq!(fs::read(&source_path).unwrap(), original_source);
        assert_eq!(
            fs::read(backup.join("pins/conversations.json")).unwrap(),
            original_target
        );
        assert_eq!(
            transfer_local_pins(&temp.0, source, target, &backup.join("pins")).unwrap(),
            0
        );
    }

    #[test]
    fn missing_database_is_a_noop() {
        let temp = TestDir::new();
        assert_eq!(
            remap_workbuddy_database_user_id(
                &temp.0.join("missing.db"),
                "target",
                &temp.0.join("backup")
            )
            .unwrap(),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cold_switch_merges_before_replacing_local_login() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        const FIXTURE_HOME: &str = "COCKPIT_WORKBUDDY_SWITCH_FIXTURE_HOME";
        let Some(root) = std::env::var_os(FIXTURE_HOME).map(PathBuf::from) else {
            let temp = TestDir::new();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "modules::workbuddy_session_transfer::tests::cold_switch_merges_before_replacing_local_login",
                    "--nocapture",
                ])
                .env(FIXTURE_HOME, &temp.0)
                .env("HOME", &temp.0)
                .env("COCKPIT_TOOLS_DATA_DIR", temp.0.join("cockpit"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };

        assert_eq!(dirs::home_dir().as_deref(), Some(root.as_path()));
        let auth_file = workbuddy_account::get_default_workbuddy_auth_file_path().unwrap();
        assert!(auth_file.starts_with(&root));
        let executable = root.join("WorkBuddy.app/Contents/MacOS/WorkBuddy");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s' 'BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc='\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        crate::modules::config::patch_user_config(|config| {
            config.workbuddy_app_path = executable.to_string_lossy().into_owned();
            config.workbuddy_share_sessions_on_switch = true;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            crate::modules::process::resolve_workbuddy_launch_path().unwrap(),
            executable
        );

        fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        crate::modules::workbuddy_auth_crypto::with_test_key(|| {
            let login = serde_json::json!({
                "account": {"uid": "source"}, "auth": {"accessToken": "source+fixture-token"}
            });
            let encrypted = crate::modules::workbuddy_auth_crypto::encode_session(&login).unwrap();
            fs::write(&auth_file, encrypted.to_string()).unwrap();
        });

        let runtime = root.join(".workbuddy");
        fs::create_dir_all(&runtime).unwrap();
        let database = runtime.join("workbuddy.db");
        let conn = Connection::open(&database).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, user_id TEXT, deleted_at INTEGER);
             INSERT INTO sessions VALUES ('session', 'source', NULL);",
        )
        .unwrap();
        drop(conn);
        let target: WorkbuddyAccount = serde_json::from_value(serde_json::json!({
            "id": "target", "uid": "target", "email": "fixture@example.invalid",
            "access_token": "fixture-target-token", "created_at": 0, "last_used": 0
        }))
        .unwrap();

        let report = prepare_account_switch(&runtime, &target).unwrap();
        assert_eq!(
            report.updated_session_rows, 1,
            "cold switch skipped source sessions"
        );
        let actual_uid: String = Connection::open(database)
            .unwrap()
            .query_row(
                "SELECT user_id FROM sessions WHERE id = 'session'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual_uid, "target");

        let invalid_login = br#"{"auth":{}}"#;
        fs::write(&auth_file, invalid_login).unwrap();
        assert!(prepare_account_switch(&runtime, &target).is_err());
        assert_eq!(fs::read(&auth_file).unwrap(), invalid_login);

        fs::remove_file(&auth_file).unwrap();
        assert_eq!(
            prepare_account_switch(&runtime, &target)
                .unwrap()
                .updated_session_rows,
            0
        );
        assert!(auth_file.is_file());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn treats_workbuddy_junctions_to_the_same_history_as_already_shared() {
        let temp = TestDir::new();
        let shared = temp.0.join("shared");
        let history = shared.join("history/workspace");
        fs::create_dir_all(&history).unwrap();
        let index = br#"{"conversations":[{"id":"existing"}]}"#;
        fs::write(history.join("index.json"), index).unwrap();
        let data_root = temp.0.join("CodeBuddyExtension/Data");
        let source = data_root.join("source/CodeBuddyIDE/source");
        let target = data_root.join("target/CodeBuddyIDE/target");
        for path in [&source, &target] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            junction::create(&shared, path).unwrap();
        }
        assert!(account_has_history(&data_root, "source").unwrap());
        let backup = temp.0.join("backup");
        let report = codebuddy_session_transfer::sync_history_between_accounts(
            &data_root, "source", "target", &backup,
        )
        .unwrap();
        assert_eq!(report.added_conversations, 0);
        assert_eq!(fs::read(history.join("index.json")).unwrap(), index);
        assert!(!backup.exists());

        junction::delete(&target).unwrap();
        fs::remove_dir(&target).unwrap();
        let different = temp.0.join("different");
        fs::create_dir_all(&different).unwrap();
        junction::create(&different, &target).unwrap();
        assert!(codebuddy_session_transfer::sync_history_between_accounts(
            &data_root, "source", "target", &backup,
        )
        .is_err());
        assert!(!different.join("history").exists());
    }

    #[test]
    fn merges_current_workbuddy_uid_layout_without_dropping_target_sessions() {
        let temp = TestDir::new();
        let data_root = temp.0.join("WorkBuddyExtension/Data");
        let source_account = data_root.join("source/VSCode/source");
        let target_account = data_root.join("target/VSCode/target");
        let source_history = source_account.join("history/workspace");
        let target_history = target_account.join("history/workspace");

        for (history, id, timestamp) in [
            (&source_history, "source-only", "2026-07-02T00:00:00Z"),
            (&target_history, "target-only", "2026-07-01T00:00:00Z"),
        ] {
            fs::create_dir_all(history.join(id).join("messages")).unwrap();
            fs::write(history.join(id).join("index.json"), br#"{"messages":[]}"#).unwrap();
            fs::write(
                history.join("index.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "conversations": [{
                        "id": id,
                        "createdAt": timestamp,
                        "lastMessageAt": timestamp
                    }],
                    "current": id
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::create_dir_all(source_account.join("check-point/workspace/source-only")).unwrap();
        fs::write(
            source_account.join("check-point/workspace/source-only/meta.json"),
            "{}",
        )
        .unwrap();

        assert!(account_has_history(&data_root, "source").unwrap());
        let report = codebuddy_session_transfer::sync_history_between_accounts(
            &data_root,
            "source",
            "target",
            &temp.0.join("backup"),
        )
        .unwrap();
        assert_eq!(report.added_conversations, 1);

        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(target_history.join("index.json")).unwrap()).unwrap();
        assert_eq!(
            merged
                .get("conversations")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(target_history.join("target-only").is_dir());
        assert!(target_history.join("source-only").is_dir());
        assert!(target_account
            .join("check-point/workspace/source-only/meta.json")
            .is_file());
    }
}
