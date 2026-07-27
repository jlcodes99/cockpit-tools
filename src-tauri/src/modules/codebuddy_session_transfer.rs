//! CodeBuddy 本地会话跨账号迁移。
//!
//! 官方客户端将正文按 UID 存在 CodeBuddyExtension 下，同时在每个实例自己的
//! codebuddy-sessions.vscdb 中按 userId 过滤会话。切号时必须同时处理两处。

use rusqlite::Connection;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::modules::{atomic_write, logger};

static TRANSFER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodebuddySessionTransferReport {
    pub added_conversations: usize,
    pub replaced_conversations: usize,
    pub updated_session_rows: usize,
    pub scanned_workspaces: usize,
}

pub fn extract_codebuddy_uid(session_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(session_json).ok()?;
    let account_uid = value
        .pointer("/account/uid")
        .or_else(|| value.pointer("/account/id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(uid) = account_uid {
        return Some(uid.to_string());
    }

    value
        .get("accessToken")
        .and_then(Value::as_str)
        .and_then(|value| value.split_once('+').map(|(uid, _)| uid.trim()))
        .filter(|uid| !uid.is_empty())
        .map(ToOwned::to_owned)
}

pub fn transfer_local_sessions(
    user_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
) -> Result<CodebuddySessionTransferReport, String> {
    validate_uid(source_uid)?;
    validate_uid(target_uid)?;
    if source_uid == target_uid {
        return Ok(CodebuddySessionTransferReport::default());
    }

    let transfer_lock = TRANSFER_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = transfer_lock
        .try_lock()
        .map_err(|_| "CodeBuddy 本地会话合并正在进行，请稍后重试".to_string())?;

    let operation_id = operation_id();
    let extension_data_dir = codebuddy_extension_data_dir()?;
    let backup_root = extension_data_dir
        .parent()
        .unwrap_or(&extension_data_dir)
        .join("Backups")
        .join("CockpitTools")
        .join(&operation_id);

    let mut report =
        sync_history_between_accounts(&extension_data_dir, source_uid, target_uid, &backup_root)?;
    report.updated_session_rows = remap_session_database_user_id(
        &user_data_dir.join("codebuddy-sessions.vscdb"),
        source_uid,
        target_uid,
        &backup_root,
    )?;

    logger::log_info(&format!(
        "[CodeBuddy Session Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, added={}, replaced={}, db_rows={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.added_conversations,
        report.replaced_conversations,
        report.updated_session_rows
    ));
    Ok(report)
}

fn validate_uid(uid: &str) -> Result<(), String> {
    let trimmed = uid.trim();
    if trimmed.is_empty() || trimmed != uid {
        return Err("CodeBuddy UID 为空或包含首尾空白".to_string());
    }
    if uid.contains('/') || uid.contains('\\') || uid.contains("..") || uid.contains('\0') {
        return Err("CodeBuddy UID 包含不安全的路径字符".to_string());
    }
    let mut components = Path::new(uid).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("CodeBuddy UID 不是安全的单级路径".to_string()),
    }
}

fn codebuddy_extension_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    #[cfg(target_os = "macos")]
    let root = home
        .join("Library")
        .join("Application Support")
        .join("CodeBuddyExtension");
    #[cfg(target_os = "windows")]
    let root = home
        .join("AppData")
        .join("Local")
        .join("CodeBuddyExtension");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = home.join(".local").join("share").join("CodeBuddyExtension");
    Ok(root.join("Data"))
}

fn operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", timestamp, std::process::id())
}

pub(crate) fn sync_history_between_accounts(
    extension_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
) -> Result<CodebuddySessionTransferReport, String> {
    let mut report = CodebuddySessionTransferReport::default();
    let source_outer = extension_data_dir.join(source_uid);
    if !source_outer.is_dir() {
        return Ok(report);
    }
    reject_symlink_if_exists(&source_outer)?;
    let target_outer = extension_data_dir.join(target_uid);
    reject_symlink_if_exists(&target_outer)?;

    let ide_entries = fs::read_dir(&source_outer).map_err(|error| {
        format!(
            "读取 CodeBuddy 会话根目录失败: path={}, error={}",
            source_outer.display(),
            error
        )
    })?;

    for ide_entry in ide_entries {
        let ide_entry =
            ide_entry.map_err(|error| format!("读取 CodeBuddy IDE 目录失败: {}", error))?;
        let metadata = fs::symlink_metadata(ide_entry.path())
            .map_err(|error| format!("读取 CodeBuddy IDE 目录属性失败: {}", error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let ide_name = ide_entry.file_name();
        let source_account_root = ide_entry.path().join(source_uid);
        let source_history_root = source_account_root.join("history");
        if !source_history_root.is_dir() {
            continue;
        }
        let target_account_root = extension_data_dir
            .join(target_uid)
            .join(&ide_name)
            .join(target_uid);
        reject_symlink_if_exists(&extension_data_dir.join(target_uid).join(&ide_name))?;
        reject_symlink_if_exists(&target_account_root)?;
        let target_history_root = target_account_root.join("history");

        let workspaces = fs::read_dir(&source_history_root).map_err(|error| {
            format!(
                "读取 CodeBuddy 工作区会话目录失败: path={}, error={}",
                source_history_root.display(),
                error
            )
        })?;
        for workspace_entry in workspaces {
            let workspace_entry = workspace_entry
                .map_err(|error| format!("读取 CodeBuddy 工作区目录失败: {}", error))?;
            let metadata = fs::symlink_metadata(workspace_entry.path())
                .map_err(|error| format!("读取 CodeBuddy 工作区目录属性失败: {}", error))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let workspace_name = workspace_entry.file_name();
            let source_workspace = workspace_entry.path();
            let target_workspace = target_history_root.join(&workspace_name);
            reject_symlink_if_exists(&target_workspace)?;
            let workspace_backup = backup_root
                .join("history")
                .join(&ide_name)
                .join(&workspace_name);
            let delta = merge_workspace_history(
                &source_workspace,
                &target_workspace,
                &source_account_root,
                &target_account_root,
                &workspace_name,
                &workspace_backup,
            )?;
            report.scanned_workspaces += 1;
            report.added_conversations += delta.added_conversations;
            report.replaced_conversations += delta.replaced_conversations;
        }
    }
    Ok(report)
}

fn merge_workspace_history(
    source_workspace: &Path,
    target_workspace: &Path,
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    backup_root: &Path,
) -> Result<CodebuddySessionTransferReport, String> {
    let source_index_path = source_workspace.join("index.json");
    if !source_index_path.is_file() {
        return Ok(CodebuddySessionTransferReport::default());
    }
    let source_index = read_workspace_index(&source_index_path)?;

    if !target_workspace.exists() {
        copy_dir_atomic(source_workspace, target_workspace)?;
        copy_auxiliary_workspace_roots(source_account_root, target_account_root, workspace_name)?;
        return Ok(CodebuddySessionTransferReport {
            added_conversations: conversations(&source_index).len(),
            ..CodebuddySessionTransferReport::default()
        });
    }

    let target_index_path = target_workspace.join("index.json");
    let mut target_index = read_workspace_index(&target_index_path)?;
    let source_conversations = conversations(&source_index);
    let target_conversations = conversations(&target_index);
    let mut target_positions = HashMap::new();
    let mut merged = Vec::with_capacity(target_conversations.len());
    let mut index_changed = false;
    for conversation in target_conversations {
        if let Some(id) = conversation_id(&conversation) {
            if let Some(existing_index) = target_positions.get(id).copied() {
                if conversation_is_newer(&conversation, &merged[existing_index]) {
                    merged[existing_index] = conversation;
                }
                index_changed = true;
                continue;
            }
            target_positions.insert(id.to_string(), merged.len());
        }
        merged.push(conversation);
    }

    let mut report = CodebuddySessionTransferReport::default();
    let mut index_backup_created = false;
    for source_conversation in source_conversations {
        let Some(id) = conversation_id(&source_conversation) else {
            continue;
        };
        validate_conversation_id(id)?;
        let source_conversation_dir = source_workspace.join(id);
        if !source_conversation_dir.is_dir() {
            logger::log_warn(&format!(
                "[CodeBuddy Session Transfer] 来源会话目录不存在，已跳过: {}",
                source_conversation_dir.display()
            ));
            continue;
        }

        match target_positions.get(id).copied() {
            None => {
                ensure_workspace_index_backup(
                    &target_index_path,
                    backup_root,
                    &mut index_backup_created,
                )?;
                let target_conversation_dir = target_workspace.join(id);
                reject_symlink_if_exists(&target_conversation_dir)?;
                if target_conversation_dir.exists() && !target_conversation_dir.is_dir() {
                    return Err(format!(
                        "CodeBuddy 目标会话路径不是目录: {}",
                        target_conversation_dir.display()
                    ));
                }
                let replace_orphaned = target_conversation_dir.exists();
                if target_conversation_dir.exists() {
                    let backup = backup_root.join("orphaned-conversations").join(id);
                    if !backup.exists() {
                        copy_dir_recursive(&target_conversation_dir, &backup)?;
                    }
                    replace_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                } else {
                    copy_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                }
                copy_auxiliary_conversation(
                    source_account_root,
                    target_account_root,
                    workspace_name,
                    id,
                    replace_orphaned,
                    backup_root,
                )?;
                target_positions.insert(id.to_string(), merged.len());
                merged.push(source_conversation);
                report.added_conversations += 1;
                index_changed = true;
            }
            Some(target_index) => {
                let target_conversation_dir = target_workspace.join(id);
                reject_symlink_if_exists(&target_conversation_dir)?;
                if target_conversation_dir.exists() && !target_conversation_dir.is_dir() {
                    return Err(format!(
                        "CodeBuddy 目标会话路径不是目录: {}",
                        target_conversation_dir.display()
                    ));
                }
                if !target_conversation_dir.is_dir()
                    || conversation_is_newer(&source_conversation, &merged[target_index])
                {
                    ensure_workspace_index_backup(
                        &target_index_path,
                        backup_root,
                        &mut index_backup_created,
                    )?;
                    if target_conversation_dir.exists() {
                        let backup = backup_root.join("conversations").join(id);
                        if !backup.exists() {
                            copy_dir_recursive(&target_conversation_dir, &backup)?;
                        }
                    }
                    replace_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                    copy_auxiliary_conversation(
                        source_account_root,
                        target_account_root,
                        workspace_name,
                        id,
                        true,
                        backup_root,
                    )?;
                    merged[target_index] = source_conversation;
                    report.replaced_conversations += 1;
                    index_changed = true;
                }
            }
        }
    }

    merged.sort_by(|left, right| compare_conversation_recency(right, left));
    if let Some(object) = target_index.as_object_mut() {
        object.insert("conversations".to_string(), Value::Array(merged.clone()));
    }

    let source_current = source_index
        .get("current")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    if let Some(source_current) = source_current {
        let current_exists = merged
            .iter()
            .any(|conversation| conversation_id(conversation) == Some(source_current));
        if current_exists
            && target_index.get("current").and_then(Value::as_str) != Some(source_current)
        {
            if let Some(object) = target_index.as_object_mut() {
                object.insert(
                    "current".to_string(),
                    Value::String(source_current.to_string()),
                );
            }
            index_changed = true;
        }
    }

    if index_changed {
        ensure_workspace_index_backup(&target_index_path, backup_root, &mut index_backup_created)?;
        let serialized = serde_json::to_string_pretty(&target_index)
            .map_err(|error| format!("序列化 CodeBuddy 工作区索引失败: {}", error))?;
        atomic_write::write_string_atomic(&target_index_path, &serialized)?;
    }

    Ok(report)
}

fn ensure_workspace_index_backup(
    target_index_path: &Path,
    backup_root: &Path,
    backup_created: &mut bool,
) -> Result<(), String> {
    if *backup_created {
        return Ok(());
    }
    fs::create_dir_all(backup_root).map_err(|error| {
        format!(
            "创建 CodeBuddy 会话备份目录失败: path={}, error={}",
            backup_root.display(),
            error
        )
    })?;
    fs::copy(target_index_path, backup_root.join("index.json")).map_err(|error| {
        format!(
            "备份 CodeBuddy 工作区索引失败: path={}, error={}",
            target_index_path.display(),
            error
        )
    })?;
    *backup_created = true;
    Ok(())
}

fn read_workspace_index(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "读取 CodeBuddy 工作区索引失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|error| {
        format!(
            "解析 CodeBuddy 工作区索引失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    if !value.is_object()
        || value
            .get("conversations")
            .map(|value| !value.is_array())
            .unwrap_or(false)
    {
        return Err(format!("CodeBuddy 工作区索引结构无效: {}", path.display()));
    }
    Ok(value)
}

fn conversations(index: &Value) -> Vec<Value> {
    index
        .get("conversations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn conversation_id(conversation: &Value) -> Option<&str> {
    conversation.get("id").and_then(Value::as_str)
}

fn validate_conversation_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("CodeBuddy conversationId 包含不安全的路径字符".to_string());
    }
    Ok(())
}

fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 CodeBuddy 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "读取 CodeBuddy 会话路径属性失败: path={}, error={}",
            path.display(),
            error
        )),
    }
}

fn conversation_is_newer(source: &Value, target: &Value) -> bool {
    compare_conversation_recency(source, target) == Ordering::Greater
}

fn compare_conversation_recency(left: &Value, right: &Value) -> Ordering {
    conversation_timestamp(left).cmp(&conversation_timestamp(right))
}

fn conversation_timestamp(conversation: &Value) -> i64 {
    let value = conversation.get("lastMessageAt");
    if let Some(timestamp) = value.and_then(Value::as_i64) {
        return timestamp;
    }
    value
        .and_then(Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis())
        .unwrap_or_default()
}

fn copy_auxiliary_workspace_roots(
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
) -> Result<(), String> {
    for kind in [
        "check-point",
        "file-tree",
        "plan-task",
        "genie-cache",
        "connectors",
    ] {
        let source = source_account_root.join(kind).join(workspace_name);
        if source.is_dir() {
            copy_dir_atomic(
                &source,
                &target_account_root.join(kind).join(workspace_name),
            )?;
        }
    }
    Ok(())
}

fn copy_auxiliary_conversation(
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    conversation_id: &str,
    replace: bool,
    backup_root: &Path,
) -> Result<(), String> {
    for kind in [
        "check-point",
        "file-tree",
        "plan-task",
        "genie-cache",
        "connectors",
    ] {
        let source = source_account_root
            .join(kind)
            .join(workspace_name)
            .join(conversation_id);
        if !source.is_dir() {
            continue;
        }
        let target = target_account_root
            .join(kind)
            .join(workspace_name)
            .join(conversation_id);
        if replace && target.exists() {
            let backup = backup_root
                .join("auxiliary")
                .join(kind)
                .join(conversation_id);
            if !backup.exists() {
                copy_dir_recursive(&target, &backup)?;
            }
            replace_dir_atomic(&source, &target)?;
        } else if !target.exists() {
            copy_dir_atomic(&source, &target)?;
        }
    }
    Ok(())
}

fn copy_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "创建 CodeBuddy 会话目标目录失败: path={}, error={}",
            parent.display(),
            error
        )
    })?;
    let temp = parent.join(format!(
        ".cockpit-session-tmp-{}-{}",
        std::process::id(),
        operation_id()
    ));
    let result = copy_dir_recursive(source, &temp).and_then(|_| {
        fs::rename(&temp, target).map_err(|error| {
            format!(
                "提交 CodeBuddy 会话目录失败: from={}, to={}, error={}",
                temp.display(),
                target.display(),
                error
            )
        })
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

fn replace_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("创建目标目录失败: {}", error))?;
    let temp = parent.join(format!(
        ".cockpit-session-replace-{}-{}",
        std::process::id(),
        operation_id()
    ));
    copy_dir_recursive(source, &temp)?;
    let old = parent.join(format!(
        ".cockpit-session-old-{}-{}",
        std::process::id(),
        operation_id()
    ));
    if target.exists() {
        fs::rename(target, &old).map_err(|error| {
            format!(
                "暂存旧 CodeBuddy 会话目录失败: from={}, to={}, error={}",
                target.display(),
                old.display(),
                error
            )
        })?;
    }
    if let Err(error) = fs::rename(&temp, target) {
        let _ = fs::remove_dir_all(&temp);
        if old.exists() {
            let _ = fs::rename(&old, target);
        }
        return Err(format!(
            "替换 CodeBuddy 会话目录失败: path={}, error={}",
            target.display(),
            error
        ));
    }
    if old.exists() {
        if let Err(error) = fs::remove_dir_all(&old) {
            logger::log_warn(&format!(
                "[CodeBuddy Session Transfer] 清理旧会话临时目录失败: path={}, error={}",
                old.display(),
                error
            ));
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "读取 CodeBuddy 会话文件属性失败: path={}, error={}",
            source.display(),
            error
        )
    })?;
    if source_metadata.file_type().is_symlink() {
        return Err(format!("拒绝复制符号链接: {}", source.display()));
    }
    if source_metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建会话文件目录失败: {}", error))?;
        }
        fs::copy(source, target).map_err(|error| {
            format!(
                "复制 CodeBuddy 会话文件失败: from={}, to={}, error={}",
                source.display(),
                target.display(),
                error
            )
        })?;
        return Ok(());
    }
    if !source_metadata.is_dir() {
        return Err(format!(
            "不支持的 CodeBuddy 会话文件类型: {}",
            source.display()
        ));
    }
    fs::create_dir_all(target)
        .map_err(|error| format!("创建 CodeBuddy 会话目录失败: {}", error))?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("读取 CodeBuddy 会话目录失败: {}", error))?
    {
        let entry = entry.map_err(|error| format!("读取 CodeBuddy 会话条目失败: {}", error))?;
        copy_dir_recursive(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

pub(crate) fn remap_session_database_user_id(
    db_path: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = Connection::open(db_path).map_err(|error| {
        format!(
            "打开 CodeBuddy 会话数据库失败: path={}, error={}",
            db_path.display(),
            error
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("设置 CodeBuddy 会话数据库超时失败: {}", error))?;

    let mut updates = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT key, value FROM ItemTable WHERE key LIKE 'session:%'")
            .map_err(|error| format!("读取 CodeBuddy 会话数据库失败: {}", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("查询 CodeBuddy 会话数据库失败: {}", error))?;
        for row in rows {
            let (key, raw_value) =
                row.map_err(|error| format!("读取 CodeBuddy 会话记录失败: {}", error))?;
            let Ok(mut value) = serde_json::from_str::<Value>(&raw_value) else {
                continue;
            };
            let Some(object) = value.as_object_mut() else {
                continue;
            };
            if object.get("userId").and_then(Value::as_str) != Some(source_uid) {
                continue;
            }
            object.insert("userId".to_string(), Value::String(target_uid.to_string()));
            let serialized = serde_json::to_string(&value)
                .map_err(|error| format!("序列化 CodeBuddy 会话记录失败: {}", error))?;
            updates.push((key, serialized));
        }
    }
    if updates.is_empty() {
        return Ok(0);
    }

    fs::create_dir_all(backup_root).map_err(|error| {
        format!(
            "创建 CodeBuddy 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            error
        )
    })?;
    fs::copy(db_path, backup_root.join("codebuddy-sessions.vscdb")).map_err(|error| {
        format!(
            "备份 CodeBuddy 会话数据库失败: path={}, error={}",
            db_path.display(),
            error
        )
    })?;

    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启 CodeBuddy 会话数据库事务失败: {}", error))?;
    for (key, value) in &updates {
        transaction
            .execute(
                "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
                rusqlite::params![value, key],
            )
            .map_err(|error| format!("更新 CodeBuddy 会话 userId 失败: {}", error))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交 CodeBuddy 会话数据库事务失败: {}", error))?;
    Ok(updates.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = TEST_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cockpit-codebuddy-session-transfer-{}-{}-{}",
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

    fn conversation(id: &str, timestamp: &str) -> Value {
        serde_json::json!({
            "id": id,
            "type": "chat",
            "name": id,
            "createdAt": timestamp,
            "lastMessageAt": timestamp,
            "unknownField": "preserved"
        })
    }

    fn write_workspace(root: &Path, items: Vec<Value>, current: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("index.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "conversations": items,
                "current": current,
                "futureField": true
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_conversation(root: &Path, id: &str, marker: &str) {
        let directory = root.join(id).join("messages");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("message.json"), marker).unwrap();
        fs::write(
            root.join(id).join("index.json"),
            br#"{"messages":[],"requests":[]}"#,
        )
        .unwrap();
    }

    #[test]
    fn extracts_uid_from_account_or_legacy_access_token() {
        assert_eq!(
            extract_codebuddy_uid(r#"{"account":{"uid":"uid-a"}}"#).as_deref(),
            Some("uid-a")
        );
        assert_eq!(
            extract_codebuddy_uid(r#"{"accessToken":"uid-b+token"}"#).as_deref(),
            Some("uid-b")
        );
    }

    #[test]
    fn rejects_unsafe_uids() {
        for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
            assert!(validate_uid(uid).is_err(), "uid should be rejected: {uid}");
        }
        assert!(validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn merges_unique_conversations_and_uses_source_current_idempotently() {
        let temp = TestDir::new();
        let source = temp.0.join("source");
        let target = temp.0.join("target");
        let backup = temp.0.join("backup");
        write_workspace(
            &source,
            vec![conversation("source-only", "2026-01-02T00:00:00Z")],
            "source-only",
        );
        write_conversation(&source, "source-only", "source");
        write_workspace(
            &target,
            vec![conversation("target-only", "2026-01-01T00:00:00Z")],
            "target-only",
        );
        write_conversation(&target, "target-only", "target");

        let first = merge_workspace_history(
            &source,
            &target,
            &temp.0.join("source-account"),
            &temp.0.join("target-account"),
            std::ffi::OsStr::new("workspace"),
            &backup,
        )
        .unwrap();
        assert_eq!(first.added_conversations, 1);
        let merged = read_workspace_index(&target.join("index.json")).unwrap();
        assert_eq!(conversations(&merged).len(), 2);
        assert_eq!(
            merged.get("current").and_then(Value::as_str),
            Some("source-only")
        );
        assert_eq!(merged.get("futureField"), Some(&Value::Bool(true)));

        let second = merge_workspace_history(
            &source,
            &target,
            &temp.0.join("source-account"),
            &temp.0.join("target-account"),
            std::ffi::OsStr::new("workspace"),
            &backup,
        )
        .unwrap();
        assert_eq!(second, CodebuddySessionTransferReport::default());
        assert_eq!(
            conversations(&read_workspace_index(&target.join("index.json")).unwrap()).len(),
            2
        );
    }

    #[test]
    fn newer_conversation_replaces_older_and_older_source_does_not_replace() {
        let temp = TestDir::new();
        let source = temp.0.join("source");
        let target = temp.0.join("target");
        let backup = temp.0.join("backup");
        write_workspace(
            &source,
            vec![conversation("same", "2026-01-03T00:00:00Z")],
            "same",
        );
        write_conversation(&source, "same", "newer");
        write_workspace(
            &target,
            vec![conversation("same", "2026-01-01T00:00:00Z")],
            "same",
        );
        write_conversation(&target, "same", "older");

        let replaced = merge_workspace_history(
            &source,
            &target,
            &temp.0.join("source-account"),
            &temp.0.join("target-account"),
            std::ffi::OsStr::new("workspace"),
            &backup,
        )
        .unwrap();
        assert_eq!(replaced.replaced_conversations, 1);
        assert_eq!(
            fs::read_to_string(target.join("same/messages/message.json")).unwrap(),
            "newer"
        );
        assert_eq!(
            fs::read_to_string(backup.join("conversations/same/messages/message.json")).unwrap(),
            "older"
        );

        let older_source = temp.0.join("older-source");
        write_workspace(
            &older_source,
            vec![conversation("same", "2025-12-31T00:00:00Z")],
            "same",
        );
        write_conversation(&older_source, "same", "oldest");
        let unchanged = merge_workspace_history(
            &older_source,
            &target,
            &temp.0.join("source-account"),
            &temp.0.join("target-account"),
            std::ffi::OsStr::new("workspace"),
            &temp.0.join("backup-2"),
        )
        .unwrap();
        assert_eq!(unchanged.replaced_conversations, 0);
        assert_eq!(
            fs::read_to_string(target.join("same/messages/message.json")).unwrap(),
            "newer"
        );
    }

    #[test]
    fn remaps_only_matching_session_user_ids() {
        let temp = TestDir::new();
        let db_path = temp.0.join("codebuddy-sessions.vscdb");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
                [],
            )
            .unwrap();
        for (key, value) in [
            (
                "session:a",
                r#"{"conversationId":"a","userId":"source","revision":7}"#,
            ),
            ("session:b", r#"{"conversationId":"b","userId":"target"}"#),
            ("session:c", r#"{"conversationId":"c","userId":""}"#),
            ("session:bad", "not-json"),
        ] {
            connection
                .execute(
                    "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, value],
                )
                .unwrap();
        }
        drop(connection);

        let updated =
            remap_session_database_user_id(&db_path, "source", "target", &temp.0.join("backup"))
                .unwrap();
        assert_eq!(updated, 1);
        let connection = Connection::open(&db_path).unwrap();
        let value: String = connection
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'session:a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let parsed: Value = serde_json::from_str(&value).unwrap();
        assert_eq!(parsed.get("userId").and_then(Value::as_str), Some("target"));
        assert_eq!(parsed.get("revision").and_then(Value::as_i64), Some(7));
        let empty_user_id: String = connection
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'session:c'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&empty_user_id)
                .unwrap()
                .get("userId")
                .and_then(Value::as_str),
            Some("")
        );
        assert!(temp.0.join("backup/codebuddy-sessions.vscdb").is_file());
    }
}
