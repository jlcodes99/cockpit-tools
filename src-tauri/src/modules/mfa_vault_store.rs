use std::{path::Path, time::Duration};

use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{account, secure_account_storage::LocalVaultCipher};

#[path = "mfa_vault_recovery_backup.rs"]
mod recovery_backup;
pub(crate) use recovery_backup::{
    create_checkpoint, prepare_legacy_backup_rotation, restore_checkpoint, verify_checkpoint,
    CheckpointSummary,
};

const FILE: &str = "mfa_vault.sqlite";
const VERSION: u32 = 1;
const CHECKPOINT_REQUIRED: &str = "mfa_recovery_checkpoint_required";
const IMPORT_BATCH_ROWS: usize = 32;
const IMPORT_BATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultList {
    Saved,
    History,
}

impl VaultList {
    fn table_id(self) -> i64 {
        match self {
            Self::Saved => 1,
            Self::History => 2,
        }
    }

    fn encryption_kind(self) -> &'static str {
        match self {
            Self::Saved => "mfa-vault-saved",
            Self::History => "mfa-vault-history",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultRecord {
    id: String,
    account_name: String,
    secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remark: Option<String>,
    time: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultProgress {
    revision: u64,
    offset: usize,
    complete: bool,
    source_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSnapshot {
    progress: VaultProgress,
    records: Vec<VaultRecord>,
    removed: Vec<String>,
}

#[derive(Deserialize)]
pub struct VaultChanges {
    revision: u64,
    #[serde(default)]
    upsert: Vec<VaultRecord>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    replace: bool,
}

fn sqlite_error(error: rusqlite::Error) -> String {
    // The SQLite error displays structural details only, with no bound values.
    format!("2FA 数据库读写失败: {error}")
}

fn open(root: &Path) -> Result<Connection, String> {
    let path = root.join(FILE);
    let mut conn =
        match Connection::open_with_flags(&path, OpenFlags::default() & !OpenFlags::SQLITE_OPEN_CREATE) {
            Ok(conn) => conn,
            Err(error) => {
                if path.try_exists().map_err(|error| error.to_string())? {
                    return Err(sqlite_error(error));
                }
                if recovery_backup::checkpoint_exists(root)? {
                    return Err(CHECKPOINT_REQUIRED.into());
                }
                Connection::open(&path).map_err(sqlite_error)?
            }
        };
    conn.busy_timeout(Duration::from_millis(150))
        .map_err(sqlite_error)?;
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite_error)?;
    if version > VERSION {
        return Err("2FA 数据库版本不受支持".into());
    }
    if version == 0 {
        if recovery_backup::checkpoint_exists(root)? {
            return Err("2FA 密钥库结构无效，已保留原文件".into());
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS vault_progress (
                list INTEGER PRIMARY KEY,
                revision INTEGER NOT NULL DEFAULT 0,
                migration_offset INTEGER NOT NULL DEFAULT 0,
                migration_complete INTEGER NOT NULL DEFAULT 0,
                migration_source TEXT
            );
            INSERT OR IGNORE INTO vault_progress(list) VALUES (1),(2);
            CREATE TABLE IF NOT EXISTS vault_records (
                list INTEGER NOT NULL,
                identity BLOB NOT NULL,
                time INTEGER NOT NULL,
                removed INTEGER NOT NULL DEFAULT 0,
                ciphertext TEXT NOT NULL,
                PRIMARY KEY (list, identity)
            );
            CREATE INDEX IF NOT EXISTS vault_records_order ON vault_records(list, removed, time DESC);
            PRAGMA user_version=1;",
        )
        .map_err(sqlite_error)?;
        tx.commit().map_err(sqlite_error)?;
    }
    Ok(conn)
}

fn progress(conn: &Connection, list: VaultList) -> Result<VaultProgress, String> {
    conn.query_row(
        "SELECT revision, migration_offset, migration_complete, migration_source FROM vault_progress WHERE list=?1",
        [list.table_id()],
        |row| {
            Ok(VaultProgress {
                revision: row.get(0)?,
                offset: row.get(1)?,
                complete: row.get(2)?,
                source_id: row.get(3)?,
            })
        },
    )
    .map_err(sqlite_error)
}

fn cipher(root: &Path, conn: &Connection) -> Result<LocalVaultCipher, String> {
    let has_data: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM vault_records)", [], |row| {
            row.get(0)
        })
        .map_err(sqlite_error)?;
    LocalVaultCipher::open(root, !has_data)
}

fn read_at(root: &Path, list: VaultList) -> Result<VaultSnapshot, String> {
    let mut conn = open(root)?;
    let tx = conn.transaction().map_err(sqlite_error)?;
    let progress = progress(&tx, list)?;
    let mut query = tx
        .prepare(
            "SELECT removed,ciphertext FROM vault_records WHERE list=?1 ORDER BY removed,time DESC",
        )
        .map_err(sqlite_error)?;
    let mut rows = query.query([list.table_id()]).map_err(sqlite_error)?;
    let mut records = Vec::new();
    let mut removed = Vec::new();
    let mut key = None;
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        let deleted: bool = row.get(0).map_err(sqlite_error)?;
        let content: String = row.get(1).map_err(sqlite_error)?;
        if key.is_none() {
            key = Some(LocalVaultCipher::open(root, false)?);
        }
        if let Some(key) = &key {
            if deleted {
                removed.push(key.unseal(list.encryption_kind(), &content)?);
            } else {
                records.push(key.unseal(list.encryption_kind(), &content)?);
            }
        }
    }
    Ok(VaultSnapshot {
        progress,
        records,
        removed,
    })
}

// Current WebView normalization collapses spaces and separators only for
// Base32 values and otherwise compares the trimmed upper-case value.
fn secret_identity(raw: &str) -> String {
    let upper = raw.trim().to_uppercase();
    let compact: String = upper
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect();
    let base = compact.trim_end_matches('=');
    if !base.is_empty()
        && base
            .bytes()
            .all(|ch| ch.is_ascii_uppercase() || (b'2'..=b'7').contains(&ch))
    {
        compact
    } else {
        upper
    }
}

fn identity(cipher: &LocalVaultCipher, list: VaultList, secret: &str) -> [u8; 32] {
    cipher.fingerprint(
        list.encryption_kind().as_bytes(),
        secret_identity(secret).as_bytes(),
    )
}

fn import_at(
    root: &Path,
    list: VaultList,
    source_id: &str,
    offset: usize,
    records: Vec<VaultRecord>,
    final_batch: bool,
) -> Result<VaultProgress, String> {
    if records.len() > IMPORT_BATCH_ROWS
        || records
            .iter()
            .try_fold(0usize, |size, record| {
                size.checked_add(
                    record.id.len()
                        + record.secret.len()
                        + record.account_name.len()
                        + record.remark.as_ref().map_or(0, String::len),
                )
            })
            .is_none_or(|size| size > IMPORT_BATCH_BYTES)
    {
        return Err("2FA 迁移批次超过处理预算".into());
    }
    let mut conn = open(root)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let current = progress(&tx, list)?;
    if current.complete {
        return Ok(current);
    }
    let saved_offset = if current.source_id.as_deref() == Some(source_id) {
        current.offset
    } else {
        0
    };
    if offset < saved_offset {
        return Ok(current);
    }
    if offset != saved_offset {
        return Err("2FA 迁移进度已变更，请重新读取".into());
    }
    if !records.is_empty() {
        let cipher = cipher(root, &tx)?;
        for record in &records {
            let identity = identity(&cipher, list, &record.secret);
            let content = cipher.seal(list.encryption_kind(), record)?;
            tx.execute(
                "INSERT INTO vault_records(list,identity,time,ciphertext) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(list,identity) DO NOTHING",
                params![list.table_id(), identity.as_slice(), record.time, content],
            )
            .map_err(sqlite_error)?;
        }
    }
    if final_batch {
        tx.execute(
            "DELETE FROM vault_records WHERE list=?1 AND removed=1",
            [list.table_id()],
        )
        .map_err(sqlite_error)?;
    }
    tx.execute(
        "UPDATE vault_progress SET migration_offset=?2, migration_complete=?3, migration_source=?4, revision=revision+1 WHERE list=?1",
        params![list.table_id(), offset + records.len(), final_batch, source_id],
    )
    .map_err(sqlite_error)?;
    let next = progress(&tx, list)?;
    tx.commit().map_err(sqlite_error)?;
    Ok(next)
}

fn change_at(root: &Path, list: VaultList, changes: VaultChanges) -> Result<VaultProgress, String> {
    let mut conn = open(root)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let current = progress(&tx, list)?;
    if changes.revision != current.revision {
        return Err("2FA 密钥库已更新，请重新读取".into());
    }
    let cipher = if !changes.upsert.is_empty() || !changes.remove.is_empty() {
        Some(cipher(root, &tx)?)
    } else {
        None
    };
    if changes.replace {
        tx.execute("DELETE FROM vault_records WHERE list=?1", [list.table_id()])
            .map_err(sqlite_error)?;
    }
    if let Some(cipher) = &cipher {
        for record in changes.upsert {
            let identity = identity(cipher, list, &record.secret);
            let content = cipher.seal(list.encryption_kind(), &record)?;
            tx.execute(
                "INSERT INTO vault_records(list,identity,time,ciphertext) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(list,identity) DO UPDATE SET time=excluded.time, removed=0, ciphertext=excluded.ciphertext",
                params![list.table_id(), identity.as_slice(), record.time, content],
            )
            .map_err(sqlite_error)?;
        }
        let removed = if changes.replace {
            Vec::new()
        } else {
            changes.remove
        };
        for secret in removed {
            let identity = identity(cipher, list, &secret);
            if current.complete {
                tx.execute(
                    "DELETE FROM vault_records WHERE list=?1 AND identity=?2",
                    params![list.table_id(), identity.as_slice()],
                )
                .map_err(sqlite_error)?;
            } else {
                let content = cipher.seal(list.encryption_kind(), &secret)?;
                tx.execute(
                    "INSERT INTO vault_records(list,identity,time,removed,ciphertext) VALUES (?1,?2,0,1,?3)
                     ON CONFLICT(list,identity) DO UPDATE SET time=0, removed=1, ciphertext=excluded.ciphertext",
                    params![list.table_id(), identity.as_slice(), content],
                )
                .map_err(sqlite_error)?;
            }
        }
    }
    tx.execute(
        "UPDATE vault_progress SET revision=revision+1, migration_complete=MAX(migration_complete,?2) WHERE list=?1",
        params![list.table_id(), changes.replace],
    )
    .map_err(sqlite_error)?;
    let next = progress(&tx, list)?;
    tx.commit().map_err(sqlite_error)?;
    Ok(next)
}

pub fn read(list: VaultList) -> Result<VaultSnapshot, String> {
    read_at(&account::get_data_dir()?, list)
}

pub fn import(
    list: VaultList,
    source_id: String,
    offset: usize,
    records: Vec<VaultRecord>,
    final_batch: bool,
) -> Result<VaultProgress, String> {
    import_at(
        &account::get_data_dir()?,
        list,
        &source_id,
        offset,
        records,
        final_batch,
    )
}

pub fn change(list: VaultList, changes: VaultChanges) -> Result<VaultProgress, String> {
    change_at(&account::get_data_dir()?, list, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, secret: &str, time: i64) -> VaultRecord {
        VaultRecord {
            id: id.into(),
            account_name: id.into(),
            secret: secret.into(),
            remark: None,
            time,
        }
    }

    #[test]
    fn interrupted_import_resumes_without_reviving_a_deleted_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let saved = VaultList::Saved;
        let one = key("first", "JBSWY3DPEHPK3PXP", 1);
        let two = key("second", "GEZDGNBVGY3TQOJQ", 2);
        let first = import_at(root, saved, "fixture", 0, vec![one.clone()], false).unwrap();
        assert_eq!(first.offset, 1);
        let replay = import_at(root, saved, "fixture", 0, vec![one], false).unwrap();
        assert_eq!(replay.revision, first.revision);

        change_at(
            root,
            saved,
            VaultChanges {
                revision: first.revision,
                upsert: Vec::new(),
                remove: vec![two.secret.clone()],
                replace: false,
            },
        )
        .unwrap();
        let midway = read_at(root, saved).unwrap();
        assert_eq!(midway.progress.offset, 1);
        assert_eq!(midway.removed, vec![two.secret.clone()]);
        let finished = import_at(root, saved, "fixture", 1, vec![two.clone()], true).unwrap();
        assert!(finished.complete);
        let restored = read_at(root, saved).unwrap();
        assert_eq!(restored.records.len(), 1);
        assert!(restored.removed.is_empty());
        assert_eq!(restored.records[0].id, "first");
        let bytes = std::fs::read(root.join(FILE)).unwrap();
        assert!(!bytes
            .windows(two.secret.len())
            .any(|candidate| candidate == two.secret.as_bytes()));
    }

    #[test]
    fn replacing_history_marks_it_complete_without_touching_saved_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let saved = VaultList::Saved;
        let history = VaultList::History;
        let one = key("saved", "JBSWY3DPEHPK3PXP", 1);
        import_at(root, saved, "fixture", 0, vec![one.clone()], true).unwrap();
        change_at(
            root,
            history,
            VaultChanges {
                revision: 0,
                upsert: Vec::new(),
                remove: Vec::new(),
                replace: true,
            },
        )
        .unwrap();
        assert!(
            import_at(root, history, "fixture", 0, vec![one], true)
                .unwrap()
                .complete
        );
        assert!(read_at(root, history).unwrap().records.is_empty());
        assert_eq!(read_at(root, saved).unwrap().records.len(), 1);
    }
}
