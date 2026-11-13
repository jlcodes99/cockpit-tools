// Read only the saved 2FA section of managed local backups.
const MFA_RECOVERY_READ_BUDGET: u64 = 64 * 1024 * 1024;
const MFA_RECOVERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
struct MfaRecoveryBackup {
    schema: String,
    version: u32,
    config: Option<MfaRecoveryConfig>,
}

#[derive(Deserialize)]
struct MfaRecoveryConfig {
    verification_records: Option<Vec<JsonValue>>,
    mfa_vault_records: Option<Vec<JsonValue>>,
}

fn mfa_backup_timestamp(path: &Path) -> Option<chrono::NaiveDateTime> {
    if !matches!(path.extension()?.to_str()?, "json" | "zip") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let timestamp = [
        "cockpit_auto_backup_full_",
        "cockpit_auto_backup_config_",
        "cockpit_manual_backup_full_",
        "cockpit_manual_backup_config_",
    ]
    .iter()
    .find_map(|prefix| stem.strip_prefix(prefix))?;
    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d_%H-%M-%S").ok()
}

fn read_mfa_backup_records(
    path: &Path,
    remaining_bytes: &mut u64,
) -> Result<Option<Vec<JsonValue>>, &'static str> {
    use std::io::Read;

    let file = fs::File::open(path).map_err(|_| "mfa_backup_read_failed")?;
    let size = file.metadata().map_err(|_| "mfa_backup_read_failed")?.len();
    if size > *remaining_bytes {
        return Err("mfa_backup_read_budget_exceeded");
    }

    let mut bytes = Vec::new();
    file.take(size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "mfa_backup_read_failed")?;
    if bytes.len() as u64 != size {
        return Err("mfa_backup_changed_during_read");
    }
    *remaining_bytes -= size;

    let backup: MfaRecoveryBackup = if path.extension().is_some_and(|ext| ext == "zip") {
        let json = backup_json_from_zip_bytes(&bytes).map_err(|_| "mfa_backup_invalid")?;
        serde_json::from_str(&json)
    } else {
        serde_json::from_slice(&bytes)
    }
    .map_err(|_| "mfa_backup_invalid")?;
    if backup.schema != "cockpit-tools.data-transfer" || backup.version != 1 {
        return Err("mfa_backup_invalid");
    }

    Ok(backup
        .config
        .and_then(|config| config.verification_records.or(config.mfa_vault_records))
        .filter(|records| !records.is_empty()))
}

fn find_mfa_recovery_records_in(
    dir: &Path,
    deadline: Instant,
) -> Result<Option<Vec<JsonValue>>, &'static str> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("mfa_backup_directory_read_failed"),
    };

    let mut candidates = std::collections::BTreeMap::new();
    let mut truncated = false;
    for entry in entries {
        if Instant::now() >= deadline {
            return Err("mfa_backup_search_timed_out");
        }
        let entry = entry.map_err(|_| "mfa_backup_directory_read_failed")?;
        let path = entry.path();
        if let Some(timestamp) = mfa_backup_timestamp(&path) {
            if !entry
                .file_type()
                .map_err(|_| "mfa_backup_directory_read_failed")?
                .is_file()
            {
                continue;
            }
            let stem = path.with_extension("");
            let pair: &mut (Option<PathBuf>, Option<PathBuf>) =
                candidates.entry((timestamp, stem)).or_default();
            if path.extension().is_some_and(|ext| ext == "zip") {
                pair.1 = Some(path);
            } else {
                pair.0 = Some(path);
            }
            // Scheduled backups run daily. Keep the newest candidates in a bounded window.
            if candidates.len() > 32 {
                candidates.pop_first();
                truncated = true;
            }
        }
    }

    let mut remaining_bytes = MFA_RECOVERY_READ_BUDGET;
    let mut invalid_backup = false;
    for (_, (json, zip)) in candidates.into_iter().rev() {
        if Instant::now() >= deadline {
            return Err("mfa_backup_search_timed_out");
        }
        let records = match (json, zip) {
            (Some(json), Some(zip)) => match read_mfa_backup_records(&json, &mut remaining_bytes) {
                Err(
                    "mfa_backup_invalid"
                    | "mfa_backup_read_failed"
                    | "mfa_backup_changed_during_read",
                ) => read_mfa_backup_records(&zip, &mut remaining_bytes),
                result => result,
            },
            (Some(json), None) => read_mfa_backup_records(&json, &mut remaining_bytes),
            (None, Some(zip)) => read_mfa_backup_records(&zip, &mut remaining_bytes),
            (None, None) => continue,
        };
        match records {
            Ok(Some(records)) => return Ok(Some(records)),
            Ok(None) => {}
            Err("mfa_backup_invalid") => invalid_backup = true,
            Err(error) => return Err(error),
        }
    }
    if invalid_backup {
        return Err("mfa_backup_invalid");
    }
    if truncated {
        return Err("mfa_backup_search_budget_exceeded");
    }
    Ok(None)
}

#[tauri::command]
pub async fn find_mfa_recovery_records() -> Result<Option<Vec<JsonValue>>, String> {
    static SEARCH_LOCK: Mutex<()> = Mutex::new(());
    let deadline = Instant::now() + MFA_RECOVERY_TIMEOUT;
    let search = tauri::async_runtime::spawn_blocking(move || {
        let _guard = SEARCH_LOCK
            .try_lock()
            .map_err(|_| "mfa_backup_search_in_progress")?;
        let dir = get_auto_backup_dir_path().map_err(|_| "mfa_backup_directory_read_failed")?;
        find_mfa_recovery_records_in(&dir, deadline)
    });
    tokio::time::timeout(MFA_RECOVERY_TIMEOUT, search)
        .await
        .map_err(|_| "mfa_backup_search_timed_out".to_string())?
        .map_err(|_| "mfa_backup_search_failed".to_string())?
        .map_err(str::to_string)
}

#[cfg(test)]
mod mfa_recovery_tests {
    use super::*;

    fn backup(config: JsonValue) -> String {
        serde_json::json!({
            "schema": "cockpit-tools.data-transfer",
            "version": 1,
            "accounts": {"untouched": true},
            "config": config,
        })
        .to_string()
    }

    #[test]
    fn restores_latest_managed_backup_with_saved_keys() {
        let dir = tempfile::tempdir().unwrap();
        let records = serde_json::json!([{"accountName": "example", "secret": "JBSWY3DPEHPK3PXP"}]);
        for (name, config) in [
            (
                "cockpit_auto_backup_full_2026-11-06_00-00-00.json",
                serde_json::json!({
                    "verification_records": [{"accountName": "older"}],
                }),
            ),
            (
                "cockpit_manual_backup_config_2026-11-07_00-00-00.json",
                serde_json::json!({
                    "verification_records": records.clone(),
                }),
            ),
            (
                "cockpit_auto_backup_full_2026-11-08_00-00-00.json",
                serde_json::json!({
                    "verification_records": [],
                }),
            ),
            (
                "cockpit_auto_backup_accounts_2026-11-09_00-00-00.json",
                serde_json::json!({
                    "verification_records": [{"accountName": "ignored"}],
                }),
            ),
        ] {
            fs::write(dir.path().join(name), backup(config)).unwrap();
        }
        fs::write(
            dir.path()
                .join("cockpit_auto_backup_full_2026-11-10_00-00-00.json"),
            b"{",
        )
        .unwrap();
        let restored =
            find_mfa_recovery_records_in(dir.path(), Instant::now() + MFA_RECOVERY_TIMEOUT)
                .unwrap()
                .unwrap();
        assert_eq!(restored, *records.as_array().unwrap());
    }

    #[test]
    fn recovers_legacy_field_from_paired_archive_if_json_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let name = "cockpit_auto_backup_full_2026-11-07_00-00-00.json";
        let records = serde_json::json!([{"accountName": "example", "secret": "JBSWY3DPEHPK3PXP"}]);
        let json = backup(serde_json::json!({"mfa_vault_records": records.clone()}));
        fs::write(dir.path().join(name), b"{").unwrap();
        fs::write(
            dir.path().join(name).with_extension("zip"),
            build_auto_backup_zip_bytes(name, &json).unwrap(),
        )
        .unwrap();
        let restored =
            find_mfa_recovery_records_in(dir.path(), Instant::now() + MFA_RECOVERY_TIMEOUT)
                .unwrap()
                .unwrap();
        assert_eq!(restored, *records.as_array().unwrap());
    }

    #[test]
    fn oversized_snapshot_is_not_reported_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("cockpit_auto_backup_full_2026-11-07_00-00-00.json");
        fs::File::create(path)
            .unwrap()
            .set_len(MFA_RECOVERY_READ_BUDGET + 1)
            .unwrap();
        let result =
            find_mfa_recovery_records_in(dir.path(), Instant::now() + MFA_RECOVERY_TIMEOUT);
        assert_eq!(result, Err("mfa_backup_read_budget_exceeded"));
    }

    #[test]
    fn implicit_rotation_preserves_recovery_candidates_until_the_vault_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let source = "cockpit_auto_backup_config_2026-10-01_00-00-00";
        let pending = "cockpit_auto_backup_config_2026-11-01_00-00-00_mfa-pending.json";
        let accounts = "cockpit_auto_backup_accounts_2026-10-01_00-00-00.json";
        for name in [format!("{source}.json"), format!("{source}.zip"), pending.into(), accounts.into()] {
            fs::write(dir.path().join(name), b"fixture").unwrap();
        }
        let cutoff = SystemTime::now() + Duration::from_secs(10);
        let mut checks = 0;
        let removed = cleanup_expired_backups_in(dir.path(), cutoff, || { checks += 1; false }).unwrap();
        assert_eq!(removed, [accounts, pending]);
        assert_eq!(checks, 1);
        let removed = cleanup_expired_backups_in(dir.path(), cutoff, || true).unwrap();
        assert_eq!(removed, [format!("{source}.json"), format!("{source}.zip")]);
    }

    #[test]
    fn incomplete_managed_backups_do_not_hide_the_legacy_recovery_source() {
        let dir = tempfile::tempdir().unwrap();
        let records = serde_json::json!([{"accountName": "fixture", "secret": "JBSWY3DPEHPK3PXP"}]);
        let source = dir.path().join("cockpit_auto_backup_full_2026-10-01_00-00-00.json");
        fs::write(source, backup(serde_json::json!({"verification_records": records.clone()}))).unwrap();
        for second in 0..40 {
            let name = format!("cockpit_auto_backup_full_2026-11-01_00-00-{second:02}_mfa-pending.json");
            fs::write(dir.path().join(name), backup(serde_json::json!({}))).unwrap();
        }
        let found = find_mfa_recovery_records_in(dir.path(), Instant::now() + MFA_RECOVERY_TIMEOUT)
            .unwrap().unwrap();
        assert_eq!(found, *records.as_array().unwrap());
    }

    #[test]
    fn rotation_preserves_a_backup_replaced_during_checkpoint_verification() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("cockpit_auto_backup_config_2026-10-01_00-00-00.json");
        fs::write(&source, b"old").unwrap();
        let removed = cleanup_expired_backups_in(dir.path(), SystemTime::now() + Duration::from_secs(10), || {
            fs::write(&source, b"newer backup").unwrap();
            true
        }).unwrap();
        assert!(removed.is_empty());
        assert_eq!(fs::read(source).unwrap(), b"newer backup");
    }
}
