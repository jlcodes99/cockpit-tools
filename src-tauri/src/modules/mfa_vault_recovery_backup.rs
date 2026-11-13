use std::{
    fs::{self, OpenOptions},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    account, progress, sqlite_error, LocalVaultCipher, VaultList, VaultRecord, FILE, VERSION,
};
use crate::modules::{
    background_maintenance as maintenance, secure_account_storage::local_vault_key_path,
};

const CHECKPOINT_FILE: &str = "mfa_vault.recovery.sqlite";
const STAGING_FILE: &str = ".mfa_vault.recovery.staging";
const KIND: &str = "mfa-vault-recovery-checkpoint";
const CHECKPOINT_VERSION: u32 = 1;
const TIMEOUT: Duration = Duration::from_secs(20);
const PAUSE: Duration = Duration::from_millis(25);
const PAGES_PER_STEP: i32 = 32;

pub(super) fn checkpoint_exists(root: &Path) -> Result<bool, String> {
    root.join(CHECKPOINT_FILE)
        .try_exists()
        .map_err(|error| error.to_string())
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    version: u32,
    saved_revision: u64,
    history_revision: u64,
    saved_records: u64,
    history_records: u64,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    summary: CheckpointSummary,
    digest: [u8; 32],
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(sqlite_error)?;
    conn.busy_timeout(PAUSE).map_err(sqlite_error)?;
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite_error)?;
    if version != VERSION {
        return Err("2FA 恢复检查点数据库版本不受支持".into());
    }
    Ok(conn)
}

fn with_budget<T>(
    deadline: Instant,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    loop {
        if Instant::now() >= deadline {
            return Err("2FA 恢复检查点处理超时".into());
        }
        if let Some(_resource) = maintenance::try_resource() {
            return work();
        }
        thread::sleep(PAUSE);
    }
}

fn revisions(conn: &Connection) -> Result<Option<[u64; 2]>, String> {
    let saved = progress(conn, VaultList::Saved)?;
    let history = progress(conn, VaultList::History)?;
    Ok((saved.complete && history.complete).then_some([saved.revision, history.revision]))
}

fn copy_database(
    source: &Connection,
    target: &mut Connection,
    deadline: Instant,
) -> Result<(), String> {
    let backup = Backup::new(source, target).map_err(sqlite_error)?;
    loop {
        match with_budget(deadline, || {
            backup.step(PAGES_PER_STEP).map_err(sqlite_error)
        })? {
            StepResult::Done => return Ok(()),
            _ => thread::sleep(PAUSE),
        }
    }
}

fn digest_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn inspect(
    conn: &Connection,
    cipher: &LocalVaultCipher,
    deadline: Instant,
) -> Result<Manifest, String> {
    let [saved_revision, history_revision] = revisions(conn)?.ok_or("2FA 迁移尚未完成")?;
    let mut hash = Sha256::new();
    for list in [VaultList::Saved, VaultList::History] {
        let encoded =
            serde_json::to_vec(&progress(conn, list)?).map_err(|error| error.to_string())?;
        digest_bytes(&mut hash, &encoded);
    }
    let mut summary = CheckpointSummary {
        version: CHECKPOINT_VERSION,
        saved_revision,
        history_revision,
        saved_records: 0,
        history_records: 0,
    };
    let mut query = conn
        .prepare("SELECT list,identity,time,removed,ciphertext FROM vault_records ORDER BY list,identity")
        .map_err(sqlite_error)?;
    let mut rows = query.query([]).map_err(sqlite_error)?;
    loop {
        let done = with_budget(deadline, || {
            let started = Instant::now();
            let mut bytes = 0u64;
            for _ in 0..maintenance::BATCH_ROWS {
                let Some(row) = rows.next().map_err(sqlite_error)? else {
                    return Ok(true);
                };
                let list: i64 = row.get(0).map_err(sqlite_error)?;
                let (list, count) = match list {
                    1 => (VaultList::Saved, &mut summary.saved_records),
                    2 => (VaultList::History, &mut summary.history_records),
                    _ => return Err("2FA 恢复检查点记录类型无效".into()),
                };
                let identity = row
                    .get_ref(1)
                    .map_err(sqlite_error)?
                    .as_blob()
                    .map_err(|_| "2FA 恢复检查点身份格式无效")?;
                let time: i64 = row.get(2).map_err(sqlite_error)?;
                let removed: bool = row.get(3).map_err(sqlite_error)?;
                let ciphertext = row
                    .get_ref(4)
                    .map_err(sqlite_error)?
                    .as_str()
                    .map_err(|_| "2FA 恢复检查点密文格式无效")?;
                if ciphertext.len() as u64 > maintenance::BATCH_BYTES * 4 {
                    return Err("2FA 恢复检查点单条记录超过处理预算".into());
                }
                let record: VaultRecord = cipher.unseal(list.encryption_kind(), ciphertext)?;
                if removed
                    || record.time != time
                    || super::identity(cipher, list, &record.secret).as_slice() != identity
                {
                    return Err("2FA 恢复检查点记录校验失败".into());
                }
                hash.update(list.table_id().to_le_bytes());
                digest_bytes(&mut hash, identity);
                hash.update(time.to_le_bytes());
                digest_bytes(&mut hash, ciphertext.as_bytes());
                *count += 1;
                bytes += ciphertext.len() as u64;
                if bytes >= maintenance::BATCH_BYTES || started.elapsed() >= maintenance::BATCH_TIME
                {
                    break;
                }
            }
            Ok(false)
        })?;
        if done {
            break;
        }
        thread::sleep(PAUSE);
    }
    Ok(Manifest {
        summary,
        digest: hash.finalize().into(),
    })
}

fn read_manifest(conn: &Connection, cipher: &LocalVaultCipher) -> Result<Manifest, String> {
    let sealed: String = conn
        .query_row(
            "SELECT sealed FROM recovery_manifest WHERE id=1 AND length(sealed)<=16384",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    let manifest: Manifest = cipher.unseal(KIND, &sealed)?;
    if manifest.summary.version != CHECKPOINT_VERSION {
        return Err("2FA 恢复检查点格式版本不受支持".into());
    }
    Ok(manifest)
}

fn verify_at(root: &Path, deadline: Instant) -> Result<CheckpointSummary, String> {
    let conn = open_readonly(&root.join(CHECKPOINT_FILE))?;
    let cipher = LocalVaultCipher::open(root, false)?;
    verify_database(&conn, &cipher, deadline)
}

fn verify_database(
    conn: &Connection,
    cipher: &LocalVaultCipher,
    deadline: Instant,
) -> Result<CheckpointSummary, String> {
    let expected = read_manifest(conn, cipher)?;
    if inspect(conn, cipher, deadline)? != expected {
        return Err("2FA 恢复检查点完整性校验失败".into());
    }
    Ok(expected.summary)
}

fn stage(root: &Path) -> Result<tempfile::TempPath, String> {
    // Only the holder of the checkpoint lock can remove a previous interrupted
    // copy. The published file is separate from both of these paths.
    for name in [STAGING_FILE, ".mfa_vault.recovery.staging-journal"] {
        match fs::remove_file(root.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let path = root.join(STAGING_FILE);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path).map_err(|error| error.to_string())?;
    tempfile::TempPath::try_from_path(path).map_err(|error| error.to_string())
}

fn lock_checkpoint(root: &Path) -> Result<fs::File, String> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("mfa_vault.recovery.lock"))
        .map_err(|error| error.to_string())?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "2FA 恢复检查点正在使用".to_string())?;
    Ok(lock)
}

fn sync_database(path: &Path) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

fn create_at(root: &Path) -> Result<Option<CheckpointSummary>, String> {
    let deadline = Instant::now() + TIMEOUT;
    let source = open_readonly(&root.join(FILE))?;
    let Some(current_revisions) = revisions(&source)? else {
        return Ok(None);
    };
    let path = root.join(CHECKPOINT_FILE);
    if !local_vault_key_path(root)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        let has_records: bool = source
            .query_row("SELECT EXISTS(SELECT 1 FROM vault_records)", [], |row| {
                row.get(0)
            })
            .map_err(sqlite_error)?;
        if !has_records && !path.exists() {
            return Ok(None);
        }
    }
    let cipher = LocalVaultCipher::open(root, false)?;
    let _lock = lock_checkpoint(root)?;
    if let Ok(existing) = open_readonly(&path) {
        if let Ok(manifest) = read_manifest(&existing, &cipher) {
            let backup_revisions = [
                manifest.summary.saved_revision,
                manifest.summary.history_revision,
            ];
            if backup_revisions
                .iter()
                .zip(current_revisions)
                .any(|(backup, current)| *backup > current)
            {
                return Err("已有 2FA 恢复检查点比当前密钥库更新".into());
            }
            if backup_revisions == current_revisions
                && inspect(&existing, &cipher, deadline).is_ok_and(|actual| actual == manifest)
            {
                return Ok(Some(manifest.summary));
            }
        }
    }

    let temporary = stage(root)?;
    let mut target = Connection::open(&temporary).map_err(sqlite_error)?;
    target.busy_timeout(PAUSE).map_err(sqlite_error)?;
    copy_database(&source, &mut target, deadline)?;
    target
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(sqlite_error)?;
    let manifest = inspect(&target, &cipher, deadline)?;
    let sealed = cipher.seal(KIND, &manifest)?;
    target.execute_batch("CREATE TABLE IF NOT EXISTS recovery_manifest (id INTEGER PRIMARY KEY CHECK(id=1), sealed TEXT NOT NULL)")
        .map_err(sqlite_error)?;
    target
        .execute(
            "INSERT OR REPLACE INTO recovery_manifest VALUES (1,?1)",
            [sealed],
        )
        .map_err(sqlite_error)?;
    if read_manifest(&target, &cipher)? != manifest {
        return Err("2FA 恢复检查点清单校验失败".into());
    }
    drop(target);
    sync_database(&temporary)?;
    let summary = manifest.summary;
    with_budget(deadline, || {
        let mut live = super::open(root)?;
        let tx = live
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        if revisions(&tx)? != Some([summary.saved_revision, summary.history_revision]) {
            return Err("2FA 密钥库在备份期间已更新".into());
        }
        temporary
            .persist(&path)
            .map_err(|error| error.error.to_string())?;
        Ok(())
    })?;
    Ok(Some(summary))
}

fn require_missing_vault(root: &Path) -> Result<(), String> {
    if root
        .join(FILE)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("mfa_recovery_live_vault_exists".into());
    }
    for suffix in ["-journal", "-wal", "-shm"] {
        if root
            .join(format!("{FILE}{suffix}"))
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Err("mfa_recovery_live_vault_sidecar_exists".into());
        }
    }
    Ok(())
}

fn restore_at(root: &Path) -> Result<CheckpointSummary, String> {
    let deadline = Instant::now() + TIMEOUT;
    require_missing_vault(root)?;
    let _lock = lock_checkpoint(root)?;
    require_missing_vault(root)?;
    let source = open_readonly(&root.join(CHECKPOINT_FILE))?;
    let cipher = LocalVaultCipher::open(root, false)?;
    let temporary = stage(root)?;
    let mut target = Connection::open(&temporary).map_err(sqlite_error)?;
    target.busy_timeout(PAUSE).map_err(sqlite_error)?;
    copy_database(&source, &mut target, deadline)?;
    target
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(sqlite_error)?;
    let summary = verify_database(&target, &cipher, deadline)?;
    target
        .execute("DROP TABLE recovery_manifest", [])
        .map_err(sqlite_error)?;
    drop(target);
    sync_database(&temporary)?;
    with_budget(deadline, || {
        require_missing_vault(root)?;
        temporary
            .persist_noclobber(root.join(FILE))
            .map_err(|error| error.error.to_string())?;
        Ok(())
    })?;
    Ok(summary)
}

pub(crate) fn create_checkpoint() -> Result<Option<CheckpointSummary>, String> {
    create_at(&account::get_data_dir()?)
}

pub(crate) fn verify_checkpoint() -> Result<CheckpointSummary, String> {
    verify_at(&account::get_data_dir()?, Instant::now() + TIMEOUT)
}

fn prepare_legacy_backup_rotation_at(root: &Path) -> Result<bool, String> {
    if !root.join(FILE).try_exists().map_err(|error| error.to_string())? {
        return Ok(false);
    }
    let live = open_readonly(&root.join(FILE))?;
    if revisions(&live)?.is_none() {
        return Ok(false);
    }
    drop(live);
    if create_at(root)?.is_some() {
        return Ok(true);
    }
    // The vault intentionally avoids creating a key/checkpoint for two completed empty lists.
    if checkpoint_exists(root)? || local_vault_key_path(root).try_exists().map_err(|error| error.to_string())? {
        return Ok(false);
    }
    let mut live = open_readonly(&root.join(FILE))?;
    let snapshot = live.transaction().map_err(sqlite_error)?;
    Ok(revisions(&snapshot)?.is_some()
        && snapshot
            .query_row("SELECT NOT EXISTS(SELECT 1 FROM vault_records)", [], |row| row.get(0))
            .map_err(sqlite_error)?)
}

pub(crate) fn prepare_legacy_backup_rotation() -> Result<bool, String> {
    prepare_legacy_backup_rotation_at(&account::get_data_dir()?)
}

pub(crate) fn restore_checkpoint() -> Result<CheckpointSummary, String> {
    restore_at(&account::get_data_dir()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::mfa_vault_store::{
        change_at, import_at, open, read_at, VaultChanges, CHECKPOINT_REQUIRED,
    };

    fn record(id: &str, secret: &str) -> VaultRecord {
        VaultRecord {
            id: id.into(),
            account_name: id.into(),
            secret: secret.into(),
            remark: None,
            time: 42,
        }
    }

    #[test]
    fn rotation_requires_complete_progress_and_refreshes_a_stale_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!prepare_legacy_backup_rotation_at(root).unwrap());
        let item = record("rotation", "JBSWY3DPEHPK3PXP");
        let saved = import_at(root, VaultList::Saved, "saved", 0, vec![item], true).unwrap();
        assert!(!prepare_legacy_backup_rotation_at(root).unwrap());
        import_at(root, VaultList::History, "history", 0, Vec::new(), true).unwrap();
        assert!(prepare_legacy_backup_rotation_at(root).unwrap());
        let before = verify_at(root, Instant::now() + TIMEOUT).unwrap();
        assert_eq!(before.saved_records, 1);

        let cleared = change_at(root, VaultList::Saved, VaultChanges {
            revision: saved.revision, upsert: Vec::new(), remove: Vec::new(), replace: true,
        }).unwrap();
        assert!(prepare_legacy_backup_rotation_at(root).unwrap());
        let after = verify_at(root, Instant::now() + TIMEOUT).unwrap();
        assert_eq!(after.saved_revision, cleared.revision);
        assert_eq!(after.saved_records, 0);
    }

    #[test]
    fn rotation_releases_completed_empty_vaults_without_creating_a_key() {
        let dir = tempfile::tempdir().unwrap();
        for list in [VaultList::Saved, VaultList::History] {
            import_at(dir.path(), list, "empty", 0, Vec::new(), true).unwrap();
        }
        assert!(prepare_legacy_backup_rotation_at(dir.path()).unwrap());
        assert!(!local_vault_key_path(dir.path()).exists());
        assert!(!checkpoint_exists(dir.path()).unwrap());
    }

    #[test]
    fn restore_recovers_both_lists_and_never_replaces_an_existing_vault() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let saved = record("saved-fixture", "JBSWY3DPEHPK3PXP");
        let history = record("history-fixture", "GEZDGNBVGY3TQOJQ");
        let old = import_at(
            root,
            VaultList::Saved,
            "saved",
            0,
            vec![saved.clone()],
            true,
        )
        .unwrap();
        import_at(
            root,
            VaultList::History,
            "history",
            0,
            vec![history.clone()],
            true,
        )
        .unwrap();
        let checkpoint = create_at(root).unwrap().unwrap();
        change_at(
            root,
            VaultList::Saved,
            VaultChanges {
                revision: old.revision,
                upsert: Vec::new(),
                remove: Vec::new(),
                replace: true,
            },
        )
        .unwrap();
        assert_eq!(
            restore_at(root).unwrap_err(),
            "mfa_recovery_live_vault_exists"
        );
        assert!(read_at(root, VaultList::Saved).unwrap().records.is_empty());

        // Restore is a deliberate recovery of the last checkpoint if the live file was lost.
        fs::remove_file(root.join(FILE)).unwrap();
        assert_eq!(open(root).err().as_deref(), Some(CHECKPOINT_REQUIRED));
        assert!(!root.join(FILE).exists());
        assert_eq!(restore_at(root).unwrap(), checkpoint);
        let restored_saved = read_at(root, VaultList::Saved).unwrap();
        let restored_history = read_at(root, VaultList::History).unwrap();
        assert!(restored_saved.progress.complete && restored_history.progress.complete);
        assert_eq!(restored_saved.records[0].secret, saved.secret);
        assert_eq!(restored_history.records[0].secret, history.secret);
        assert_eq!(
            verify_at(root, Instant::now() + TIMEOUT).unwrap(),
            checkpoint
        );
    }

    #[test]
    fn restore_keeps_a_pending_sqlite_journal_and_rejects_a_damaged_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        import_at(
            root,
            VaultList::Saved,
            "saved",
            0,
            vec![record("saved-fixture", "JBSWY3DPEHPK3PXP")],
            true,
        )
        .unwrap();
        import_at(root, VaultList::History, "history", 0, Vec::new(), true).unwrap();
        create_at(root).unwrap().unwrap();
        fs::remove_file(root.join(FILE)).unwrap();
        let journal = root.join(format!("{FILE}-journal"));
        fs::write(&journal, b"preserve").unwrap();
        assert_eq!(
            restore_at(root).unwrap_err(),
            "mfa_recovery_live_vault_sidecar_exists"
        );
        assert_eq!(fs::read(&journal).unwrap(), b"preserve");
        fs::remove_file(journal).unwrap();
        Connection::open(root.join(CHECKPOINT_FILE))
            .unwrap()
            .execute("DELETE FROM vault_records", [])
            .unwrap();
        assert!(restore_at(root).is_err());
        assert!(!root.join(FILE).exists());
    }

    #[test]
    fn checkpoint_validates_across_multiple_copy_and_record_batches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let total = 2 * maintenance::BATCH_ROWS + 1;
        for offset in (0..total).step_by(maintenance::BATCH_ROWS) {
            let end = (offset + maintenance::BATCH_ROWS).min(total);
            let batch = (offset..end)
                .map(|index| {
                    let mut item = record(
                        &format!("large-fixture-{index}"),
                        &format!("synthetic-credential-{index}"),
                    );
                    item.remark = Some("r".repeat(4096));
                    item
                })
                .collect();
            import_at(root, VaultList::Saved, "many", offset, batch, end == total).unwrap();
        }
        import_at(root, VaultList::History, "history", 0, Vec::new(), true).unwrap();
        let pages: i32 = open(root)
            .unwrap()
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        assert!(pages > PAGES_PER_STEP);
        let summary = create_at(root).unwrap().unwrap();
        assert_eq!(summary.saved_records, total as u64);
        assert_eq!(verify_at(root, Instant::now() + TIMEOUT).unwrap(), summary);
    }

    #[test]
    fn checkpoint_copies_both_completed_lists_and_recovers_from_interrupted_staging() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let saved = record("saved-fixture", "JBSWY3DPEHPK3PXP");
        let history = record("history-fixture", "GEZDGNBVGY3TQOJQ");
        import_at(
            root,
            VaultList::Saved,
            "saved",
            0,
            vec![saved.clone()],
            true,
        )
        .unwrap();
        assert!(create_at(root).unwrap().is_none());
        import_at(
            root,
            VaultList::History,
            "history",
            0,
            vec![history.clone()],
            true,
        )
        .unwrap();
        fs::write(root.join(STAGING_FILE), b"interrupted").unwrap();
        let summary = create_at(root).unwrap().unwrap();
        assert_eq!((summary.saved_records, summary.history_records), (1, 1));
        assert_eq!(verify_at(root, Instant::now() + TIMEOUT).unwrap(), summary);
        assert!(!root.join(STAGING_FILE).exists());
        let bytes = fs::read(root.join(CHECKPOINT_FILE)).unwrap();
        for record in [saved, history] {
            assert!(!bytes
                .windows(record.secret.len())
                .any(|part| part == record.secret.as_bytes()));
        }
        assert_eq!(create_at(root).unwrap().unwrap(), summary);
        assert_eq!(fs::read(root.join(CHECKPOINT_FILE)).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(CHECKPOINT_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn checkpoint_rejects_tampering_and_keeps_last_verified_copy_on_failed_update() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let saved = record("saved-fixture", "JBSWY3DPEHPK3PXP");
        let progress = import_at(root, VaultList::Saved, "saved", 0, vec![saved], true).unwrap();
        import_at(root, VaultList::History, "history", 0, Vec::new(), true).unwrap();
        let previous = create_at(root).unwrap().unwrap();
        Connection::open(root.join(CHECKPOINT_FILE))
            .unwrap()
            .execute("DELETE FROM vault_records", [])
            .unwrap();
        assert!(verify_at(root, Instant::now() + TIMEOUT).is_err());
        assert_eq!(create_at(root).unwrap().unwrap(), previous);

        let next = change_at(
            root,
            VaultList::Saved,
            VaultChanges {
                revision: progress.revision,
                upsert: vec![record("new-fixture", "GEZDGNBVGY3TQOJQ")],
                remove: Vec::new(),
                replace: true,
            },
        )
        .unwrap();
        open(root)
            .unwrap()
            .execute("UPDATE vault_records SET ciphertext='corrupt'", [])
            .unwrap();
        assert!(create_at(root).is_err());
        assert_eq!(verify_at(root, Instant::now() + TIMEOUT).unwrap(), previous);

        change_at(
            root,
            VaultList::Saved,
            VaultChanges {
                revision: next.revision,
                upsert: Vec::new(),
                remove: Vec::new(),
                replace: true,
            },
        )
        .unwrap();
        let cleared = create_at(root).unwrap().unwrap();
        assert_eq!(cleared.saved_records, 0);
        assert!(cleared.saved_revision > previous.saved_revision);
        assert_eq!(verify_at(root, Instant::now() + TIMEOUT).unwrap(), cleared);
    }
}
