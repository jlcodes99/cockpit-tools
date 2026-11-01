// Login-owned config keys only; preserve device/UI preferences and other settings.
fn is_desktop_auth_config_key(key: &str) -> bool {
    matches!(
        key,
        "oauth:tokenCache"
            | "oauth:tokenCacheV2"
            | "lastKnownAccountUuid"
            | "coworkTrustedDeviceToken"
    ) || key.starts_with("coworkTrustedDeviceToken:")
}

fn merge_desktop_config_token(snapshot_path: &Path, target_path: &Path) -> Result<(), String> {
    let snapshot = read_config_file(snapshot_path)?.unwrap_or_else(|| json!({}));
    let mut target = read_config_file(target_path)?.unwrap_or_else(|| json!({}));
    let object = target
        .as_object_mut()
        .ok_or_else(|| "Claude config.json 结构非法".to_string())?;
    object.retain(|key, _| !is_desktop_auth_config_key(key));
    if let Some(source) = snapshot.as_object() {
        for (key, value) in source {
            if is_desktop_auth_config_key(key) {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    let content = serde_json::to_string_pretty(&target).map_err(|e| e.to_string())?;
    atomic_write::write_secret_string_atomic(target_path, &content)
}

// Missing login state in a snapshot means absent, not "keep the previous account".
fn is_desktop_login_state_path(item: &str) -> bool {
    matches!(
        item,
        "config.json.bak"
            | "Cookies"
            | "Cookies-journal"
            | "Cookies-wal"
            | "Cookies-shm"
            | "Network"
            | "Local Storage"
            | "IndexedDB"
            | "Session Storage"
            | "Service Worker"
            | "WebStorage"
            | "DIPS"
            | "DIPS-wal"
            | "SharedStorage"
            | "SharedStorage-wal"
    )
}

fn apply_desktop_profile_snapshot(snapshot_dir: &Path, target_dir: &Path) -> Result<(), String> {
    for item in CLAUDE_DESKTOP_PROFILE_ITEMS {
        let source = snapshot_dir.join(item);
        let target = target_dir.join(item);
        if *item == "config.json" {
            merge_desktop_config_token(&source, &target)?;
        } else if *item == "config.json.bak" {
            remove_path_if_exists(&target)?;
        } else if source.exists() {
            copy_path_overwrite(&source, &target)?;
        } else if is_desktop_login_state_path(item) {
            remove_path_if_exists(&target)?;
        }
    }
    Ok(())
}

static CLAUDE_PROFILE_RESTORE_LOCK: Mutex<()> = Mutex::new(());

fn recover_desktop_profile_transaction(target: &Path, transaction: &Path) -> Result<(), String> {
    let Some(journal) = read_config_file(&transaction.join("journal.json"))? else {
        // Staging was interrupted before any live file was touched.
        return fs::remove_dir_all(transaction).map_err(|e| e.to_string());
    };
    if journal.get("committed").and_then(Value::as_bool) != Some(true) {
        let originals = journal
            .get("originals")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!(
                    "CLAUDE_PROFILE_RECOVERY_FAILED: Invalid journal: {}",
                    transaction.display()
                )
            })?;
        for item in CLAUDE_DESKTOP_PROFILE_ITEMS {
            let saved = transaction.join("rollback").join(item);
            if saved.exists() {
                remove_path_if_exists(&target.join(item))?;
                fs::rename(saved, target.join(item)).map_err(|e| e.to_string())?;
            } else if journal.get("applying").and_then(Value::as_bool) == Some(true)
                && !originals.iter().any(|v| v.as_str() == Some(item))
            {
                remove_path_if_exists(&target.join(item))?;
            }
        }
    }
    fs::remove_dir_all(transaction).map_err(|e| e.to_string())
}

fn restore_desktop_profile_snapshot(
    snapshot: &Path,
    target: &Path,
    account_uuid: Option<&str>,
) -> Result<(), String> {
    let _guard = CLAUDE_PROFILE_RESTORE_LOCK
        .try_lock()
        .map_err(|_| "CLAUDE_PROFILE_RESTORE_BUSY".to_string())?;
    restore_desktop_profile_transaction(
        snapshot,
        target,
        account_uuid,
        apply_desktop_profile_snapshot,
    )
}

fn restore_desktop_profile_transaction(
    snapshot: &Path,
    target: &Path,
    account_uuid: Option<&str>,
    apply: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if !snapshot.is_dir() || snapshot == target {
        return Err("CLAUDE_PROFILE_INVALID".to_string());
    }
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".cockpit-auth-")
            && entry.path().is_dir()
        {
            recover_desktop_profile_transaction(target, &entry.path())?;
        }
    }
    let transaction = target.join(format!(".cockpit-auth-{}", uuid::Uuid::new_v4()));
    let staged = transaction.join("staged");
    let rollback = transaction.join("rollback");
    let result = (|| {
        copy_desktop_profile_snapshot(snapshot, &staged)?;
        let mut config =
            read_config_file(&staged.join("config.json"))?.unwrap_or_else(|| json!({}));
        if let Some(id) = account_uuid {
            let obj = config
                .as_object_mut()
                .ok_or_else(|| "Claude config.json 结构非法".to_string())?;
            obj.insert("lastKnownAccountUuid".into(), Value::String(id.into()));
            obj.retain(|key, _| {
                !key.starts_with("coworkTrustedDeviceToken:")
                    || key == &format!("coworkTrustedDeviceToken:{}", id)
            });
            obj.remove("coworkTrustedDeviceToken");
        }
        write_config_file(&staged.join("config.json"), &config)?;
        read_config_file(&target.join("config.json"))?;
        fs::create_dir_all(&rollback).map_err(|e| e.to_string())?;
        let originals: Vec<_> = CLAUDE_DESKTOP_PROFILE_ITEMS
            .iter()
            .copied()
            .filter(|item| target.join(item).exists())
            .collect();
        let mut journal = json!({"originals": originals, "applying": false, "committed": false});
        write_config_file(&transaction.join("journal.json"), &journal)?;
        for item in CLAUDE_DESKTOP_PROFILE_ITEMS {
            let path = target.join(item);
            if path.exists()
                && (*item == "config.json"
                    || staged.join(item).exists()
                    || is_desktop_login_state_path(item))
            {
                fs::rename(&path, rollback.join(item)).map_err(|e| e.to_string())?;
            }
        }
        journal["applying"] = json!(true);
        write_config_file(&transaction.join("journal.json"), &journal)?;
        if rollback.join("config.json").exists() {
            copy_path_overwrite(&rollback.join("config.json"), &target.join("config.json"))?;
        }
        apply(&staged, target)?;
        journal["committed"] = json!(true);
        write_config_file(&transaction.join("journal.json"), &journal)?;
        Ok(())
    })();
    if let Err(error) = result {
        if transaction.exists() {
            if let Err(rollback_error) = recover_desktop_profile_transaction(target, &transaction) {
                return Err(format!(
                    "CLAUDE_PROFILE_RECOVERY_FAILED: {}; rollback: {}; backup={}",
                    error,
                    rollback_error,
                    transaction.display()
                ));
            }
        }
        return Err(error);
    }
    if let Err(error) = fs::remove_dir_all(&transaction) {
        logger::log_warn(&format!(
            "[Claude] committed profile transaction cleanup failed: {}",
            error
        ));
    }
    Ok(())
}
