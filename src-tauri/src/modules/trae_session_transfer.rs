//! Trae workspace session metadata sharing across local accounts.
//!
//! Trae stores account-scoped restore keys in each workspace `state.vscdb`.
//! Builder/Agent bodies are kept in an encrypted native database, which this
//! module intentionally does not modify.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::modules::{logger, trae_account};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraeSessionTransferReport {
    pub scanned_workspaces: usize,
    pub copied_keys: usize,
    pub merged_keys: usize,
    /// True when the feature is enabled and a real source→target remap ran.
    pub attempted: bool,
    pub skipped_reason: Option<String>,
}

impl TraeSessionTransferReport {
    pub fn total_changed(&self) -> usize {
        self.copied_keys.saturating_add(self.merged_keys)
    }

    pub fn user_facing_summary(&self) -> String {
        if let Some(reason) = self.skipped_reason.as_deref() {
            return format!("会话共享未执行：{}", reason);
        }
        if !self.attempted {
            return "会话共享未开启".to_string();
        }
        if self.total_changed() == 0 {
            return format!(
                "会话共享已执行，但无可迁移项（工作区 {} 个）",
                self.scanned_workspaces
            );
        }
        format!(
            "已共享会话状态：工作区 {} 个，新增 {} 项，合并 {} 项",
            self.scanned_workspaces, self.copied_keys, self.merged_keys
        )
    }
}

static TRANSFER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LAST_SWITCH_REPORT: OnceLock<Mutex<Option<TraeSessionTransferReport>>> = OnceLock::new();
const SESSION_MEMENTO_KEY: &str = "memento/icube-ai-agent-storage";
/// Match the official Trae session-memento capacity without ballooning state.vscdb.
const SESSION_LIST_LIMIT: usize = 30;

pub fn take_last_switch_report() -> Option<TraeSessionTransferReport> {
    LAST_SWITCH_REPORT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
}

fn remember_switch_report(report: &TraeSessionTransferReport) {
    if let Ok(mut guard) = LAST_SWITCH_REPORT.get_or_init(|| Mutex::new(None)).lock() {
        *guard = Some(report.clone());
    }
}

pub fn prepare_account_switch(
    platform: trae_account::TraePlatformKind,
    user_data_dir: &Path,
    account_id: &str,
) -> Result<TraeSessionTransferReport, String> {
    if !sharing_enabled(platform) {
        logger::log_info(&format!(
            "[Trae Session Transfer] 已关闭切号共享，跳过: platform={}, account_id={}",
            platform.provider_key(),
            account_id
        ));
        let report = TraeSessionTransferReport {
            skipped_reason: Some("设置中未开启切号共享本地会话".to_string()),
            ..TraeSessionTransferReport::default()
        };
        remember_switch_report(&report);
        return Ok(report);
    }
    let target = match trae_account::load_account(account_id) {
        Some(account) => account,
        None => {
            logger::log_warn(&format!(
                "[Trae Session Transfer] 目标账号不存在，跳过共享: platform={}, account_id={}",
                platform.provider_key(),
                account_id
            ));
            let report = TraeSessionTransferReport {
                skipped_reason: Some("目标账号不存在".to_string()),
                ..TraeSessionTransferReport::default()
            };
            remember_switch_report(&report);
            return Ok(report);
        }
    };
    // Soft-skip when uid is missing: never block inject/start after close.
    // Official Trae scopes workspace keys by numeric user id
    // (e.g. `{uid}_ai-chat:sessionRelation:*`, `memento/icube-ai-agent-storage-{uid}`).
    let Some(target_uid) = trae_account::account_user_id_for_local_session(&target) else {
        logger::log_warn(&format!(
            "[Trae Session Transfer] 目标账号缺少用户 ID，跳过共享以免阻断切号启动: platform={}, account_id={}, email={}",
            platform.provider_key(),
            account_id,
            target.email
        ));
        let report = TraeSessionTransferReport {
            skipped_reason: Some(
                "目标账号缺少官方用户 ID（请用该账号在 Trae 登录一次后再导入/刷新）".to_string(),
            ),
            ..TraeSessionTransferReport::default()
        };
        remember_switch_report(&report);
        return Ok(report);
    };
    // Persist resolved uid so later switches keep working even if raw payload is sparse.
    let _ = trae_account::backfill_account_user_id_if_missing(account_id, &target_uid);

    let storage_path = user_data_dir
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    let Some(source_uid) = trae_account::read_local_trae_user_id_from_storage_path(&storage_path)?
    else {
        logger::log_info(&format!(
            "[Trae Session Transfer] 本地 storage 无当前用户 ID，跳过共享: platform={}, path={}",
            platform.provider_key(),
            storage_path.display()
        ));
        let report = TraeSessionTransferReport {
            skipped_reason: Some("本地 Trae 尚未写入当前登录用户 ID".to_string()),
            ..TraeSessionTransferReport::default()
        };
        remember_switch_report(&report);
        return Ok(report);
    };
    if source_uid == target_uid {
        logger::log_info(&format!(
            "[Trae Session Transfer] 源/目标用户 ID 相同，无需共享: uid={}",
            source_uid
        ));
        let report = TraeSessionTransferReport {
            attempted: true,
            skipped_reason: Some("源账号与目标账号为同一用户 ID".to_string()),
            ..TraeSessionTransferReport::default()
        };
        remember_switch_report(&report);
        return Ok(report);
    }
    let mut report = transfer_local_session_state(user_data_dir, &source_uid, &target_uid)?;
    report.attempted = true;
    if report.total_changed() == 0 {
        logger::log_info(&format!(
            "[Trae Session Transfer] 未发现可迁移会话键: platform={}, source_uid={}, target_uid={}, dir={}",
            platform.provider_key(),
            source_uid,
            target_uid,
            user_data_dir.display()
        ));
    }
    remember_switch_report(&report);
    Ok(report)
}

fn sharing_enabled(_platform: trae_account::TraePlatformKind) -> bool {
    // Trae session sharing is disabled for this release: chat bodies live in an
    // SQLCipher-encrypted ModularData/ai-agent database that we cannot safely
    // remap across accounts. Index-only transfer has no user-visible benefit.
    false
}

pub fn transfer_local_session_state(
    user_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
) -> Result<TraeSessionTransferReport, String> {
    validate_uid(source_uid)?;
    validate_uid(target_uid)?;
    if source_uid == target_uid {
        return Ok(TraeSessionTransferReport::default());
    }
    let transfer_lock = TRANSFER_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = transfer_lock
        .try_lock()
        .map_err(|_| "Trae 本地会话共享正在进行，请稍后重试".to_string())?;

    let workspace_root = user_data_dir.join("User").join("workspaceStorage");
    if !workspace_root.is_dir() {
        return Ok(TraeSessionTransferReport::default());
    }
    reject_symlink(&workspace_root)?;

    let backup_root = user_data_dir
        .join("Backups")
        .join("CockpitTools")
        .join("TraeSessions")
        .join(operation_id());
    let mut workspaces = fs::read_dir(&workspace_root)
        .map_err(|error| format!("读取 Trae workspaceStorage 失败: {}", error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    workspaces.sort();

    let mut report = TraeSessionTransferReport::default();
    for workspace in workspaces {
        let metadata = fs::symlink_metadata(&workspace)
            .map_err(|error| format!("读取 Trae workspace 目录属性失败: {}", error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let db_path = workspace.join("state.vscdb");
        if !db_path.is_file() {
            continue;
        }
        reject_symlink(&db_path)?;
        let workspace_name = workspace
            .file_name()
            .ok_or_else(|| format!("Trae workspace 路径无效: {}", workspace.display()))?;
        let delta = remap_workspace_database(
            &db_path,
            source_uid,
            target_uid,
            &backup_root.join(workspace_name).join("state.vscdb"),
        )?;
        report.scanned_workspaces += 1;
        report.copied_keys += delta.copied_keys;
        report.merged_keys += delta.merged_keys;
    }

    logger::log_info(&format!(
        "[Trae Session Transfer] workspace 会话状态共享完成: source_uid={}, target_uid={}, workspaces={}, copied_keys={}, merged_keys={}",
        source_uid, target_uid, report.scanned_workspaces, report.copied_keys, report.merged_keys
    ));
    Ok(report)
}

#[derive(Debug)]
struct PendingWrite {
    key: String,
    value: String,
    overwrite: bool,
}

fn remap_workspace_database(
    db_path: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_path: &Path,
) -> Result<TraeSessionTransferReport, String> {
    let mut connection = Connection::open(db_path).map_err(|error| {
        format!(
            "打开 Trae workspace 数据库失败({}): {}",
            db_path.display(),
            error
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| format!("设置 Trae workspace 数据库超时失败: {}", error))?;

    let source_rows = {
        let mut statement = connection
            .prepare("SELECT key, CAST(value AS TEXT) FROM ItemTable")
            .map_err(|error| format!("读取 Trae workspace 数据库结构失败: {}", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("查询 Trae workspace 会话状态失败: {}", error))?;
        rows.filter_map(Result::ok)
            .filter_map(|(key, value)| {
                remap_account_key(&key, source_uid, target_uid)
                    .map(|target_key| (target_key, value))
            })
            .collect::<Vec<_>>()
    };

    let mut writes = Vec::new();
    let mut copied_keys = 0;
    let mut merged_keys = 0;
    for (target_key, source_value) in source_rows {
        let target_value = connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1",
                [&target_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("读取 Trae 目标会话状态失败: {}", error))?;
        match target_value {
            None => {
                writes.push(PendingWrite {
                    key: target_key,
                    value: source_value,
                    overwrite: false,
                });
                copied_keys += 1;
            }
            Some(target_value) if target_key.contains("sessionRelation") => {
                if let Some(merged) = merge_json_objects(&source_value, &target_value) {
                    if merged != target_value {
                        writes.push(PendingWrite {
                            key: target_key,
                            value: merged,
                            overwrite: true,
                        });
                        merged_keys += 1;
                    }
                }
            }
            // Prefer the account we are switching away from for active agent UI state,
            // so the target account lands with the same session restore pointers.
            Some(target_value)
                if (target_key.contains("currentAgentData")
                    || target_key.contains("_AI.agent.")
                    || target_key.contains("AI.agent."))
                    && target_value != source_value =>
            {
                writes.push(PendingWrite {
                    key: target_key,
                    value: source_value,
                    overwrite: true,
                });
                merged_keys += 1;
            }
            Some(_) => {}
        }
    }

    let source_session_key = account_session_memento_key(source_uid);
    let target_session_key = account_session_memento_key(target_uid);
    let legacy_session_value = read_text_value(&connection, SESSION_MEMENTO_KEY)?;
    let source_session_value =
        read_text_value(&connection, &source_session_key)?.or_else(|| legacy_session_value.clone());
    let target_session_value = read_text_value(&connection, &target_session_key)?;

    if let Some(source_session_value) = source_session_value {
        let merged_session_value = target_session_value
            .as_deref()
            .and_then(|target| merge_session_mementos(&source_session_value, target))
            .or_else(|| normalize_session_memento(&source_session_value));

        if let Some(merged_session_value) = merged_session_value {
            queue_session_memento_write(
                &mut writes,
                &mut copied_keys,
                &mut merged_keys,
                target_session_key,
                target_session_value.as_deref(),
                &merged_session_value,
            );
            queue_session_memento_write(
                &mut writes,
                &mut copied_keys,
                &mut merged_keys,
                SESSION_MEMENTO_KEY.to_string(),
                legacy_session_value.as_deref(),
                &merged_session_value,
            );
        }
    }

    if writes.is_empty() {
        return Ok(TraeSessionTransferReport::default());
    }

    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Trae 会话备份目录失败: {}", error))?;
    }
    fs::copy(db_path, backup_path).map_err(|error| {
        format!(
            "备份 Trae workspace 数据库失败: from={}, to={}, error={}",
            db_path.display(),
            backup_path.display(),
            error
        )
    })?;

    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启 Trae 会话状态事务失败: {}", error))?;
    for write in writes {
        if write.overwrite {
            transaction
                .execute(
                    "UPDATE ItemTable SET value = ?2 WHERE key = ?1",
                    params![write.key, write.value],
                )
                .map_err(|error| format!("合并 Trae 会话状态失败: {}", error))?;
        } else {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO ItemTable(key, value) VALUES (?1, ?2)",
                    params![write.key, write.value],
                )
                .map_err(|error| format!("复制 Trae 会话状态失败: {}", error))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("提交 Trae 会话状态事务失败: {}", error))?;

    Ok(TraeSessionTransferReport {
        copied_keys,
        merged_keys,
        ..TraeSessionTransferReport::default()
    })
}

fn read_text_value(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取 Trae 会话列表失败: {}", error))
}

fn account_session_memento_key(uid: &str) -> String {
    format!("{}-{}", SESSION_MEMENTO_KEY, uid)
}

fn queue_session_memento_write(
    writes: &mut Vec<PendingWrite>,
    copied_keys: &mut usize,
    merged_keys: &mut usize,
    key: String,
    current_value: Option<&str>,
    merged_value: &str,
) {
    if current_value == Some(merged_value) {
        return;
    }
    let overwrite = current_value.is_some();
    writes.push(PendingWrite {
        key,
        value: merged_value.to_string(),
        overwrite,
    });
    if overwrite {
        *merged_keys += 1;
    } else {
        *copied_keys += 1;
    }
}

fn normalize_session_memento(value: &str) -> Option<String> {
    let mut parsed: Value = serde_json::from_str(value).ok()?;
    normalize_session_memento_value(&mut parsed);
    serde_json::to_string(&parsed).ok()
}

fn merge_session_mementos(source_text: &str, target_text: &str) -> Option<String> {
    let mut source: Value = serde_json::from_str(source_text).ok()?;
    let mut target: Value = serde_json::from_str(target_text).ok()?;
    normalize_session_memento_value(&mut source);
    normalize_session_memento_value(&mut target);

    let source_object = source.as_object()?;
    let target_object = target.as_object_mut()?;
    let source_list = source_object.get("list")?.as_array()?;
    let source_current = source_object
        .get("currentSessionId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let target_current = target_object
        .get("currentSessionId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let current = {
        let target_list = target_object.get_mut("list")?.as_array_mut()?;
        let mut seen = target_list
            .iter()
            .filter_map(session_id)
            .map(ToOwned::to_owned)
            .collect::<std::collections::HashSet<_>>();
        for session in source_list {
            let Some(id) = session_id(session) else {
                continue;
            };
            if seen.insert(id.to_string()) {
                target_list.push(session.clone());
            }
        }
        target_list.sort_by(|left, right| session_updated_at(right).cmp(&session_updated_at(left)));
        target_list.truncate(SESSION_LIST_LIMIT);

        target_current
            .filter(|id| {
                target_list
                    .iter()
                    .any(|session| session_id(session) == Some(id.as_str()))
            })
            .or_else(|| {
                source_current.filter(|id| {
                    target_list
                        .iter()
                        .any(|session| session_id(session) == Some(id.as_str()))
                })
            })
    };
    if let Some(current) = current {
        target_object.insert("currentSessionId".to_string(), Value::String(current));
    }

    serde_json::to_string(&target).ok()
}

fn normalize_session_memento_value(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if !object.get("list").is_some_and(Value::is_array) {
        object.insert("list".to_string(), Value::Array(Vec::new()));
    }
    if let Some(list) = object.get_mut("list").and_then(Value::as_array_mut) {
        list.retain(|session| session_id(session).is_some());
        list.sort_by(|left, right| session_updated_at(right).cmp(&session_updated_at(left)));
        list.truncate(SESSION_LIST_LIMIT);
    }
}

fn session_id(session: &Value) -> Option<&str> {
    session
        .as_object()?
        .get("sessionId")?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

fn session_updated_at(session: &Value) -> i64 {
    session
        .as_object()
        .and_then(|object| object.get("updatedAt"))
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

fn remap_account_key(key: &str, source_uid: &str, target_uid: &str) -> Option<String> {
    if let Some(suffix) = key.strip_prefix(source_uid) {
        if suffix.starts_with('_') {
            return Some(format!("{}{}", target_uid, suffix));
        }
    }
    if let Some(prefix) = key.strip_suffix(source_uid) {
        if prefix.ends_with('_') || prefix.ends_with('-') {
            return Some(format!("{}{}", prefix, target_uid));
        }
    }
    None
}

fn merge_json_objects(source: &str, target_text: &str) -> Option<String> {
    let source: Value = serde_json::from_str(source).ok()?;
    let mut target_value: Value = serde_json::from_str(target_text).ok()?;
    let source = source.as_object()?;
    let target = target_value.as_object_mut()?;
    let mut changed = false;
    for (key, value) in source {
        if !target.contains_key(key) {
            target.insert(key.clone(), value.clone());
            changed = true;
        }
    }
    if changed {
        serde_json::to_string(&target_value).ok()
    } else {
        Some(target_text.to_string())
    }
}

fn validate_uid(uid: &str) -> Result<(), String> {
    if uid.trim().is_empty() || uid.trim() != uid || uid.len() > 256 {
        return Err("Trae 用户 ID 为空、过长或包含首尾空白".to_string());
    }
    if uid.chars().any(char::is_control) {
        return Err("Trae 用户 ID 包含不安全控制字符".to_string());
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("读取 Trae 会话路径属性失败: {}", error))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "拒绝通过符号链接读写 Trae 会话: {}",
            path.display()
        ));
    }
    Ok(())
}

fn operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", timestamp, std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_uid_scoped_workspace_state_without_touching_shared_keys() {
        let root =
            std::env::temp_dir().join(format!("trae-session-transfer-{}", uuid::Uuid::new_v4()));
        let workspace = root
            .join("User")
            .join("workspaceStorage")
            .join("workspace-a");
        fs::create_dir_all(&workspace).expect("create workspace");
        let db_path = workspace.join("state.vscdb");
        let connection = Connection::open(&db_path).expect("open db");
        connection
            .execute("CREATE TABLE ItemTable (key TEXT UNIQUE, value BLOB)", [])
            .expect("create ItemTable");
        for (key, value) in [
            ("source_AI.agent.mode", "solo"),
            (
                "source_ai-chat:sessionRelation:globalModeMap",
                r#"{"source-session":"agent","shared":"source"}"#,
            ),
            (
                "target_ai-chat:sessionRelation:globalModeMap",
                r#"{"target-session":"agent","shared":"target"}"#,
            ),
            ("currentAgentData_source", "source-current"),
            ("currentAgentData_target", "target-current"),
            (
                "memento/icube-ai-agent-storage",
                r#"{"list":[{"sessionId":"source-session","updatedAt":10,"messages":[]}],"currentSessionId":"source-session"}"#,
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO ItemTable(key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .expect("insert row");
        }
        drop(connection);

        let report = transfer_local_session_state(&root, "source", "target").expect("transfer");
        assert_eq!(report.scanned_workspaces, 1);
        assert_eq!(report.copied_keys, 2);
        assert_eq!(report.merged_keys, 3);

        let connection = Connection::open(&db_path).expect("reopen db");
        let copied: String = connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'target_AI.agent.mode'",
                [],
                |row| row.get(0),
            )
            .expect("copied key");
        assert_eq!(copied, "solo");
        let merged: String = connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'target_ai-chat:sessionRelation:globalModeMap'",
                [],
                |row| row.get(0),
            )
            .expect("merged key");
        let merged: Value = serde_json::from_str(&merged).expect("parse merged value");
        assert_eq!(merged["source-session"], "agent");
        assert_eq!(merged["target-session"], "agent");
        assert_eq!(merged["shared"], "target");
        let current: String = connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'currentAgentData_target'",
                [],
                |row| row.get(0),
            )
            .expect("current agent data");
        assert_eq!(current, "source-current");
        let target_sessions: String = connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'memento/icube-ai-agent-storage-target'",
                [],
                |row| row.get(0),
            )
            .expect("target session memento");
        let target_sessions: Value =
            serde_json::from_str(&target_sessions).expect("parse target sessions");
        assert_eq!(target_sessions["list"][0]["sessionId"], "source-session");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unsafe_user_ids() {
        let error = transfer_local_session_state(Path::new("/tmp/unused"), " source", "target")
            .expect_err("unsafe uid should fail");
        assert!(error.contains("用户 ID"));
    }

    #[test]
    fn merges_official_account_session_mementos_and_legacy_fallback() {
        let source = serde_json::json!({
            "list": [
                {"sessionId": "source-new", "updatedAt": 30, "messages": []},
                {"sessionId": "shared", "updatedAt": 10, "messages": []}
            ],
            "currentSessionId": "source-new"
        });
        let target = serde_json::json!({
            "list": [
                {"sessionId": "target-current", "updatedAt": 20, "messages": []},
                {"sessionId": "shared", "updatedAt": 5, "messages": []}
            ],
            "currentSessionId": "target-current"
        });

        let merged = merge_session_mementos(&source.to_string(), &target.to_string())
            .expect("merge session mementos");
        let merged: Value = serde_json::from_str(&merged).expect("parse merged memento");
        let ids = merged["list"]
            .as_array()
            .expect("session list")
            .iter()
            .filter_map(session_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["source-new", "target-current", "shared"]);
        assert_eq!(merged["currentSessionId"], "target-current");
    }

    #[test]
    fn limits_shared_session_memento_to_official_capacity() {
        let source = serde_json::json!({
            "list": (0..35)
                .map(|index| serde_json::json!({
                    "sessionId": format!("session-{index}"),
                    "updatedAt": index,
                    "messages": []
                }))
                .collect::<Vec<_>>(),
            "currentSessionId": "session-34"
        });
        let normalized = normalize_session_memento(&source.to_string()).expect("normalize");
        let normalized: Value = serde_json::from_str(&normalized).expect("parse normalized");
        assert_eq!(normalized["list"].as_array().expect("list").len(), 30);
        assert_eq!(normalized["list"][0]["sessionId"], "session-34");
    }
}
