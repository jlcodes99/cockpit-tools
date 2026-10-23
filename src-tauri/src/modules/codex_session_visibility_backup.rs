// Codex Session Visibility：Atomic rollout/SQLite backup and restore operations。
// 通过 include! 保持原模块作用域、私有调用关系和修复事务行为。
fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("database is locked") || lowered.contains("database busy") {
        return format!(
            "Codex SQLite 会话库当前被占用，请关闭 Codex / Codex App 后重试 ({}): {}",
            path.display(),
            message
        );
    }
    format!(
        "更新 SQLite 会话可见性失败 ({}): {}",
        path.display(),
        message
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RolloutTimestampIdentity {
    created_at_ns: Option<i128>,
    device: Option<u64>,
    inode: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RolloutTimestampBackup {
    relative_path: PathBuf,
    source_size: u64,
    source_identity: RolloutTimestampIdentity,
    original_modified_at_ns: i128,
    applied_modified_at_ns: i128,
}

fn system_time_nanos(value: SystemTime) -> Option<i128> {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).ok(),
        Err(error) => i128::try_from(error.duration().as_nanos()).ok().map(|value| -value),
    }
}

fn system_time_from_nanos(value: i128) -> Option<SystemTime> {
    let magnitude = value.checked_abs()?;
    let seconds = u64::try_from(magnitude / 1_000_000_000).ok()?;
    let nanos = (magnitude % 1_000_000_000) as u32;
    let duration = Duration::new(seconds, nanos);
    if value < 0 { UNIX_EPOCH.checked_sub(duration) } else { UNIX_EPOCH.checked_add(duration) }
}

fn rollout_timestamp_identity(metadata: &fs::Metadata) -> RolloutTimestampIdentity {
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (Some(metadata.dev()), Some(metadata.ino()))
    };
    #[cfg(not(unix))]
    let (device, inode) = (None, None);
    RolloutTimestampIdentity {
        // On Unix, backdating mtime may also backdate birthtime (APFS).
        // Device/inode are the stable identity there; Windows creation time
        // remains independent of a modification-time update.
        created_at_ns: if cfg!(unix) { None } else { metadata.created().ok().and_then(system_time_nanos) },
        device,
        inode,
    }
}

fn normalize_rollout_timestamp(change: &RolloutProviderChange) -> Result<bool, String> {
    let (Some(expected), Some(target)) = (change.source_modified_at, change.target_modified_at) else {
        return Ok(false);
    };
    apply_rollout_timestamp_if_unchanged(
        &change.absolute_path,
        change.source_size,
        &change.source_identity,
        expected,
        target,
    )
}

fn apply_rollout_timestamp_if_unchanged(
    path: &Path,
    expected_size: u64,
    identity: &RolloutTimestampIdentity,
    expected_modified_at: SystemTime,
    target_modified_at: SystemTime,
) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "跳过已不可读取的 Codex rollout 文件 ({}): {}",
                path.display(),
                error
            ));
            return Ok(false);
        }
    };
    if !metadata.is_file()
        || metadata.len() != expected_size
        || metadata.modified().ok() != Some(expected_modified_at)
        || &rollout_timestamp_identity(&metadata) != identity
    {
        modules::logger::log_warn(&format!(
            "跳过修复扫描后发生变化的 Codex rollout 文件: {}",
            path.display()
        ));
        return Ok(false);
    }
    if metadata.modified().ok() == Some(target_modified_at) { return Ok(false); }
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        // Read contents/metadata and change only attributes; never request
        // write-data or truncation rights on append-only history.
        fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0100).open(path)
    };
    #[cfg(not(windows))]
    let file = fs::File::open(path);
    let file = file.map_err(|error| format!("{}: {}", path.display(), error))?;
    match file.try_lock() {
        Ok(()) => {},
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        Err(error) => return Err(format!("{}: {}", path.display(), error)),
    }
    let unchanged = |metadata: &fs::Metadata| {
        metadata.len() == expected_size
            && metadata.modified().ok() == Some(expected_modified_at)
            && &rollout_timestamp_identity(metadata) == identity
    };
    if !unchanged(&file.metadata().map_err(|error| format!("{}: {}", path.display(), error))?)
        || !unchanged(&fs::symlink_metadata(path).map_err(|error| format!("{}: {}", path.display(), error))?)
    {
        return Ok(false);
    }
    // The generic best-effort time helper deliberately ignores setter errors.
    // Repair results must count only actual successful, verified changes.
    file.set_modified(target_modified_at).map_err(|error| format!("{}: {}", path.display(), error))?;
    let applied = file.metadata().map_err(|error| format!("{}: {}", path.display(), error))?;
    if applied.len() != expected_size
        || applied.modified().ok() != Some(target_modified_at)
        || &rollout_timestamp_identity(&fs::symlink_metadata(path).map_err(|error| format!("{}: {}", path.display(), error))?) != identity
    {
        return Ok(false);
    }
    Ok(true)
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

fn sqlite_candidate_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = sqlite_dir_session_dbs(data_dir);
    let legacy = data_dir.join(STATE_DB_FILE);
    if !paths.iter().any(|path| path == &legacy) {
        paths.push(legacy);
    }
    paths
}

fn sqlite_candidate_paths_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
) -> Vec<PathBuf> {
    match options.sqlite_scope {
        SqliteRepairScope::LegacyStateOnly => vec![data_dir.join(STATE_DB_FILE)],
        SqliteRepairScope::OfficialStateDbs => official_state_db_candidate_paths(data_dir),
        SqliteRepairScope::AllSessionDbs => provider_sync_sqlite_paths(data_dir),
    }
}

fn provider_sync_sqlite_paths(data_dir: &Path) -> Vec<PathBuf> {
    sqlite_candidate_paths(data_dir)
        .into_iter()
        .filter(|path| has_provider_sync_table(path))
        .collect()
}

fn official_state_db_candidate_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique_path(
        &mut paths,
        data_dir.join(SQLITE_DIR_NAME).join(OFFICIAL_STATE_DB_FILE),
    );
    push_unique_path(&mut paths, data_dir.join(STATE_DB_FILE));
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn sqlite_dir_session_dbs(data_dir: &Path) -> Vec<PathBuf> {
    let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
    let Ok(entries) = fs::read_dir(&sqlite_dir) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| is_sqlite_candidate(path))
        .filter(|path| has_codex_session_table(path))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        (
            path.file_name()
                .map(|name| name != OsStr::new(PREFERRED_SQLITE_DB_FILE))
                .unwrap_or(true),
            path.file_name().map(|name| name.to_os_string()),
        )
    });
    candidates
}

fn is_sqlite_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("db") | Some("sqlite") | Some("sqlite3")
    )
}

fn has_codex_session_table(path: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    [
        "threads",
        "local_thread_catalog",
        "automation_runs",
        "inbox_items",
    ]
    .iter()
    .any(|table| {
        connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                [table],
                |_| Ok(()),
            )
            .is_ok()
    })
}

fn has_provider_sync_table(path: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    ["threads", "local_thread_catalog"].iter().any(|table| {
        connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                [table],
                |_| Ok(()),
            )
            .is_ok()
    })
}

fn relative_to_instance_root(data_dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(data_dir).unwrap_or(path).to_path_buf()
}

fn sqlite_sidecar_paths(db_path: &Path) -> Vec<PathBuf> {
    let raw = db_path.to_string_lossy();
    vec![
        PathBuf::from(format!("{}-wal", raw)),
        PathBuf::from(format!("{}-shm", raw)),
    ]
}

fn remove_sqlite_sidecar_files(db_path: &Path) -> Result<(), String> {
    for path in sqlite_sidecar_paths(db_path) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "清理 SQLite sidecar 文件失败 ({}): {}",
                    path.display(),
                    error
                ));
            }
        }
    }
    Ok(())
}

fn backup_sqlite_databases(
    data_dir: &Path,
    backup_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
) -> Result<Vec<String>, String> {
    let mut backed_up = Vec::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        check_launch_visibility_budget("backup_sqlite")?;
        if !db_path.exists() {
            continue;
        }
        let relative = relative_to_instance_root(data_dir, &db_path);
        let backup_db_path = backup_dir.join("db").join(&relative);
        if let Some(parent) = backup_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 备份目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        let connection = Connection::open(&db_path).map_err(|error| {
            format!(
                "打开 SQLite 会话库以创建一致备份失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|error| {
                format!(
                    "设置 SQLite 备份 busy_timeout 失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;
        configure_launch_visibility_connection(&connection)?;

        if backup_db_path.exists() {
            fs::remove_file(&backup_db_path).map_err(|error| {
                format!(
                    "删除旧 SQLite 备份失败 ({}): {}",
                    backup_db_path.display(),
                    error
                )
            })?;
        }
        let backup_target = backup_db_path.to_string_lossy().to_string();
        connection
            .execute("VACUUM main INTO ?1", [backup_target.as_str()])
            .map_err(|error| {
                format!(
                    "备份 SQLite 会话库失败 ({} -> {}): {}",
                    db_path.display(),
                    backup_db_path.display(),
                    error
                )
            })?;
        backed_up.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(backed_up)
}

fn restore_sqlite_databases_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
) -> Result<Vec<String>, String> {
    let backup_db_root = backup_dir.join("db");
    if !backup_db_root.exists() {
        return Ok(Vec::new());
    }
    let backup_paths = list_backup_sqlite_files(&backup_db_root)?;
    let mut restored = Vec::new();
    for backup_db_path in backup_paths {
        let relative = backup_db_path
            .strip_prefix(&backup_db_root)
            .map_err(|_| format!("无法计算 SQLite 备份相对路径: {}", backup_db_path.display()))?;
        let target_db_path = data_dir.join(relative);
        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 恢复目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        remove_sqlite_sidecar_files(&target_db_path)?;
        fs::copy(&backup_db_path, &target_db_path).map_err(|error| {
            format!(
                "恢复 SQLite 会话库失败 ({} -> {}): {}",
                backup_db_path.display(),
                target_db_path.display(),
                error
            )
        })?;
        remove_sqlite_sidecar_files(&target_db_path)?;
        restored.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(restored)
}

fn list_backup_sqlite_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|error| format!("读取 SQLite 备份目录失败 ({}): {}", root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取 SQLite 备份目录项失败 ({}): {}", root.display(), error)
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取 SQLite 备份文件类型失败 ({}): {}",
                path.display(),
                error
            )
        })?;
        if file_type.is_dir() {
            result.extend(list_backup_sqlite_files(&path)?);
        } else if file_type.is_file() {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}

fn backup_instance_files(
    data_dir: &Path,
    rollout_changes: &[RolloutProviderChange],
    include_sqlite: bool,
    include_session_index: bool,
    include_global_state: bool,
    instance_id: &str,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
) -> Result<PathBuf, String> {
    check_launch_visibility_budget("backup_instance")?;
    let scope = modules::backup_storage::scope_for_path(data_dir);
    let backup_dir = modules::backup_storage::behavior_backup_dir(
        "codex",
        &scope,
        &format!(
            "{}{}{}",
            SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX,
            Utc::now().format("%Y%m%d-%H%M%S"),
            SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX
        ),
    )?;

    let backed_up_files: Vec<String> = Vec::new();
    let mut rollout_timestamps = Vec::new();
    let mut sqlite_backup_files = Vec::new();
    for change in rollout_changes {
        check_launch_visibility_budget("backup_rollout_timestamps")?;
        // Timestamp-only repair needs no rollout content copy. A byte copy can
        // be huge, go stale during append, and must never replace live history.
        if let (Some(original), Some(applied)) = (
            change.source_modified_at.and_then(system_time_nanos),
            change.target_modified_at.and_then(system_time_nanos),
        ) {
            rollout_timestamps.push(RolloutTimestampBackup {
                relative_path: change.relative_path.clone(),
                source_size: change.source_size,
                source_identity: change.source_identity.clone(),
                original_modified_at_ns: original,
                applied_modified_at_ns: applied,
            });
        }
    }

    if include_sqlite {
        sqlite_backup_files = backup_sqlite_databases(data_dir, &backup_dir, options)?;
    }

    let mut session_index_backup_created = false;
    if include_session_index {
        let source = data_dir.join(SESSION_INDEX_FILE);
        if source.exists() {
            let target = backup_dir.join("files").join(SESSION_INDEX_FILE);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "创建 session_index 备份目录失败 ({}): {}",
                        parent.display(),
                        error
                    )
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "备份 session_index.jsonl 失败 ({} -> {}): {}",
                    source.display(),
                    target.display(),
                    error
                )
            })?;
            session_index_backup_created = true;
        }
    }

    let mut global_state_backup_created = false;
    if include_global_state {
        let source = data_dir.join(GLOBAL_STATE_FILE);
        if source.exists() {
            let target = backup_dir.join("files").join(GLOBAL_STATE_FILE);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("创建全局状态备份目录失败 ({}): {error}", parent.display())
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "备份 Codex 全局状态失败 ({} -> {}): {error}",
                    source.display(),
                    target.display()
                )
            })?;
            global_state_backup_created = true;
        }
    }

    let manifest = json!({
        "instanceId": instance_id,
        "instanceRoot": data_dir,
        "targetProvider": target_provider,
        "createdAt": Utc::now().to_rfc3339(),
        "hasSqliteBackup": !sqlite_backup_files.is_empty(),
        "sqliteFiles": sqlite_backup_files,
        "hasSessionIndexBackup": session_index_backup_created,
        "hasGlobalStateBackup": global_state_backup_created,
        "rolloutFiles": backed_up_files,
        "rolloutTimestamps": rollout_timestamps,
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

fn parse_session_visibility_repair_backup_timestamp(name: &str) -> Option<&str> {
    let timestamp = name
        .strip_prefix(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)?
        .strip_suffix(SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX)?;
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
    Some(timestamp)
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
        let _ = restore_sqlite_databases_from_backup(data_dir, backup_dir)?;
    }

    let manifest_path = backup_dir.join("manifest.json");
    if manifest_path.exists() {
        let manifest: JsonValue = serde_json::from_slice(
            &fs::read(&manifest_path).map_err(|error| error.to_string())?,
        ).map_err(|error| error.to_string())?;
        if let Some(records) = manifest.get("rolloutTimestamps") {
            let records: Vec<RolloutTimestampBackup> = serde_json::from_value(records.clone())
                .map_err(|error| error.to_string())?;
            for record in records {
                if !record.relative_path.components().all(|component| matches!(component, std::path::Component::Normal(_))) {
                    continue;
                }
                let (Some(applied), Some(original)) = (
                    system_time_from_nanos(record.applied_modified_at_ns),
                    system_time_from_nanos(record.original_modified_at_ns),
                ) else { continue; };
                // Restore only a timestamp we applied to the same unchanged
                // file. New records, replacement files and new mtimes win.
                apply_rollout_timestamp_if_unchanged(
                    &data_dir.join(record.relative_path), record.source_size,
                    &record.source_identity, applied, original,
                )?;
            }
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

        // Legacy repair backups may contain stale rollouts. Restoring them
        // would discard appended records and invalidate downstream byte cursors.
        if source_path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl")) {
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
        modules::codex_session_file_time::restore_modified_time(
            &target_path,
            modules::codex_session_file_time::read_modified_time(&source_path),
        )?;
    }
    Ok(())
}

