use super::antigravity_account_index_store::{IndexStore, StoredSummary};
use super::provider_account_index;
use crate::models::cursor::{CursorAccount, CursorAccountIndex};
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, File, ReadDir},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::{Mutex, TryLockError},
    time::{Duration, Instant},
};

const BATCH_ENTRIES: usize = 16;
const BATCH_BYTES: u64 = 2 * 1024 * 1024;
const BATCH_TIME: Duration = Duration::from_millis(25);

type Decode = fn(&str) -> Result<CursorAccount, String>;
type NormalizeId = fn(&str) -> Result<String, String>;

pub(crate) struct CursorRecovery {
    store: IndexStore<CursorAccountIndex>,
    directory: PathBuf,
    decode: Decode,
    normalize_id: NormalizeId,
    mutation_lock: &'static Mutex<()>,
    index_ready: bool,
    entries: Option<ReadDir>,
    pending: VecDeque<(String, PathBuf)>,
    staged: Vec<StoredSummary>,
    exhausted: bool,
    unreadable: usize,
    refresh_needed: bool,
}

impl CursorRecovery {
    pub(crate) fn new(
        store: IndexStore<CursorAccountIndex>,
        directory: PathBuf,
        decode: Decode,
        normalize_id: NormalizeId,
        mutation_lock: &'static Mutex<()>,
    ) -> Self {
        Self {
            store,
            directory,
            decode,
            normalize_id,
            mutation_lock,
            index_ready: false,
            entries: None,
            pending: VecDeque::new(),
            staged: Vec::new(),
            exhausted: false,
            unreadable: 0,
            refresh_needed: false,
        }
    }

    pub(crate) fn migration_key(&self) -> String {
        self.store.migration_key()
    }

    pub(crate) fn take_refresh_needed(&mut self) -> bool {
        std::mem::take(&mut self.refresh_needed)
    }

    pub(crate) fn run_batch(&mut self) -> Result<bool, String> {
        if !self.index_ready {
            if self.store.detail_recovery_complete()? {
                return Ok(true);
            }
            let has_details = directory_has_entries(&self.directory)?;
            if !has_details
                && !self.store.has_legacy_source()?
                && !self.store.database_path().exists()
            {
                return Ok(true);
            }
            self.index_ready = self.store.migrate_with_detail_fallback(has_details)?;
            return Ok(false);
        }

        if !self.staged.is_empty() {
            // Keep the batch on lock contention. Rechecks in SQLite also reject
            // a deletion or update that landed after the file was read.
            let Some(_lock) = self.try_mutation_lock()? else {
                return Ok(false);
            };
            self.store.recover_details_batch(&self.staged, false)?;
            self.staged.clear();
            self.refresh_needed = true;
        }

        if self.pending.is_empty() && !self.exhausted {
            self.read_directory_batch()?;
        }
        if self.pending.is_empty() && self.exhausted {
            if self.unreadable != 0 {
                let unreadable = std::mem::take(&mut self.unreadable);
                self.entries = None;
                self.exhausted = false;
                return Err(format!("Cursor 有 {unreadable} 份旧账号详情暂时无法恢复"));
            }
            let Some(_lock) = self.try_mutation_lock()? else {
                return Ok(false);
            };
            self.store.recover_details_batch(&[], true)?;
            self.refresh_needed = true;
            return Ok(true);
        }

        let ids: Vec<_> = self.pending.iter().map(|(id, _)| id.clone()).collect();
        let missing: HashSet<_> = self.store.missing_recovery_ids(&ids)?.into_iter().collect();
        self.pending.retain(|(id, _)| missing.contains(id));
        let mut bytes = 0;
        let started = Instant::now();
        while let Some((id, path)) = self.pending.front() {
            let Some((result, used)) = read_detail(path, id, BATCH_BYTES - bytes, self.decode)
            else {
                break;
            };
            bytes += used;
            match result {
                Ok(Some(row)) => self
                    .staged
                    .push(provider_account_index::to_summary(&row.summary())?),
                Ok(None) => {}
                Err(error) => {
                    if let Some(backup) = detail_backup(path) {
                        self.pending.front_mut().expect("pending detail").1 = backup;
                        if started.elapsed() >= BATCH_TIME || bytes == BATCH_BYTES {
                            break;
                        }
                        continue;
                    }
                    self.unreadable += 1;
                    super::logger::log_warn(&format!(
                        "[CursorRecovery] path={}, error={error}",
                        path.display()
                    ));
                }
            }
            self.pending.pop_front();
            if started.elapsed() >= BATCH_TIME || bytes == BATCH_BYTES {
                break;
            }
        }
        Ok(false)
    }

    fn try_mutation_lock(&self) -> Result<Option<std::sync::MutexGuard<'static, ()>>, String> {
        match self.mutation_lock.try_lock() {
            Ok(lock) => Ok(Some(lock)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err("Cursor 账号写入锁不可用".into()),
        }
    }

    fn read_directory_batch(&mut self) -> Result<(), String> {
        if self.entries.is_none() {
            match fs::read_dir(&self.directory) {
                Ok(entries) => self.entries = Some(entries),
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.exhausted = true;
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            }
        }

        let mut candidates = Vec::new();
        let started = Instant::now();
        for _ in 0..BATCH_ENTRIES {
            let Some(entry) = self.entries.as_mut().and_then(Iterator::next) else {
                self.exhausted = true;
                break;
            };
            let candidate = entry.and_then(|entry| Ok((entry.file_type()?, entry.path())));
            match candidate {
                Ok((kind, path))
                    if kind.is_file()
                        && path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("json")) =>
                {
                    if let Some(id) = path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .and_then(|id| (self.normalize_id)(id).ok())
                    {
                        candidates.push((id, path));
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.unreadable += 1;
                    super::logger::log_warn(&format!("[CursorRecovery] 读取目录项失败: {error}"));
                }
            }
            if started.elapsed() >= BATCH_TIME {
                break;
            }
        }
        // Keep these entries even if an indexed lookup fails transiently.
        self.pending.extend(candidates);
        Ok(())
    }
}

pub(crate) fn directory_has_entries(path: &Path) -> Result<bool, String> {
    match fs::read_dir(path) {
        Ok(mut entries) => entries
            .next()
            .transpose()
            .map(|entry| entry.is_some())
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn detail_backup(path: &Path) -> Option<PathBuf> {
    if !path.extension()?.to_str()?.eq_ignore_ascii_case("json") {
        return None;
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    let backup = PathBuf::from(name);
    let metadata = fs::symlink_metadata(&backup).ok()?;
    metadata.is_file().then_some(backup)
}

fn read_detail(
    path: &Path,
    expected_id: &str,
    budget: u64,
    decode: Decode,
) -> Option<(Result<Option<CursorAccount>, String>, u64)> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Some((Ok(None), 0)),
        Err(error) => return Some((Err(error.to_string()), 0)),
    };
    let size = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => return Some((Err(error.to_string()), 0)),
    };
    if size > BATCH_BYTES {
        return Some((Err("单份详情超过迁移预算".into()), 0));
    }
    if size > budget {
        return None;
    }
    let result = (|| {
        let mut content = String::new();
        file.take(size + 1)
            .read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        if content.len() as u64 != size {
            return Err("旧账号详情在读取时发生变化".into());
        }
        let account = decode(&content)?;
        if account.id != expected_id {
            return Err("账号 ID 与文件名不一致".into());
        }
        Ok(Some(account))
    })();
    Some((result, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::antigravity_account_index_store::IndexRead;
    use serde_json::json;

    static LOCK: Mutex<()> = Mutex::new(());

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("cursor_accounts.json");
        let details = root.path().join("cursor_accounts");
        fs::create_dir(&details).unwrap();
        (root, index, details)
    }

    fn worker(path: &Path, directory: &Path) -> CursorRecovery {
        CursorRecovery::new(
            provider_account_index::store(path).unwrap(),
            directory.to_owned(),
            |content| serde_json::from_str(content).map_err(|error| error.to_string()),
            |id| Ok(id.to_owned()),
            &LOCK,
        )
    }

    fn seed(directory: &Path, id: &str) {
        let account = json!({
            "id": id, "email": format!("{id}@example.test"), "access_token": "fixture-token",
            "created_at": 1, "last_used": 2
        });
        fs::write(directory.join(format!("{id}.json")), account.to_string()).unwrap();
    }

    fn active(index: &Path) -> CursorAccountIndex {
        match provider_account_index::store(index)
            .unwrap()
            .load()
            .unwrap()
        {
            IndexRead::Active(index) => index,
            _ => panic!("expected SQLite account index"),
        }
    }

    fn finish(worker: &mut CursorRecovery) {
        for _ in 0..30 {
            if worker.run_batch().unwrap() {
                return;
            }
        }
        panic!("Cursor recovery did not finish");
    }

    #[test]
    fn cursor_recovery_resumes_without_duplicating_accounts() {
        let (_root, index, details) = setup();
        let legacy = r#"{"version":"1.0","accounts":[]}"#;
        fs::write(&index, legacy).unwrap();
        for number in 0..48 {
            seed(&details, &format!("account-{number}"));
        }
        let mut first = worker(&index, &details);
        for _ in 0..4 {
            assert!(!first.run_batch().unwrap());
        }
        let interrupted = active(&index);
        assert!(!interrupted.accounts.is_empty());
        assert!(interrupted.accounts.len() < 48);
        assert!(!interrupted.detail_recovery_complete);
        drop(first);
        finish(&mut worker(&index, &details));
        let recovered = active(&index);
        assert_eq!(recovered.accounts.len(), 48);
        assert!(recovered.detail_recovery_complete);
        assert_eq!(fs::read_to_string(index).unwrap(), legacy);
        assert_eq!(fs::read_dir(details).unwrap().count(), 48);
    }

    #[test]
    fn cursor_recovery_does_not_reinsert_a_concurrently_deleted_detail() {
        let (_root, index, details) = setup();
        fs::write(&index, r#"{"version":"1.0","accounts":[]}"#).unwrap();
        seed(&details, "retired");
        seed(&details, "orphan");
        let mut job = worker(&index, &details);
        assert!(!job.run_batch().unwrap());
        assert!(!job.run_batch().unwrap());
        assert!(job.staged.iter().any(|row| row.id == "retired"));

        let store = provider_account_index::store::<CursorAccountIndex>(&index).unwrap();
        store
            .replace_with_removals(&active(&index), &["retired".into()])
            .unwrap();
        finish(&mut job);
        let recovered = active(&index);
        assert_eq!(recovered.accounts.len(), 1);
        assert_eq!(recovered.accounts[0].id, "orphan");
        assert!(details.join("retired.json").exists());
        assert!(worker(&index, &details).run_batch().unwrap());
    }

    #[test]
    fn cursor_recovery_preserves_corrupt_index_and_recovers_readable_detail() {
        let (_root, index, details) = setup();
        fs::write(&index, "{broken").unwrap();
        seed(&details, "orphan");
        finish(&mut worker(&index, &details));
        let recovered = active(&index);
        assert!(recovered.detail_recovery_complete);
        assert_eq!(recovered.accounts[0].id, "orphan");
        assert_eq!(fs::read_to_string(&index).unwrap(), "{broken");
    }

    #[test]
    fn cursor_recovery_reads_a_detail_backup_without_rewriting_source() {
        let (_root, index, details) = setup();
        seed(&details, "orphan");
        let primary = details.join("orphan.json");
        let backup = details.join("orphan.json.bak");
        fs::rename(&primary, &backup).unwrap();
        fs::write(&primary, "{broken").unwrap();

        finish(&mut worker(&index, &details));
        assert_eq!(active(&index).accounts[0].id, "orphan");
        assert_eq!(fs::read_to_string(&primary).unwrap(), "{broken");
        assert!(backup.is_file());
    }

    #[test]
    fn cursor_recovery_does_not_treat_invalid_legacy_data_as_empty() {
        let (_root, index, details) = setup();
        fs::write(&index, "{broken").unwrap();
        fs::write(details.join("unreadable.json"), "{broken").unwrap();
        let mut job = worker(&index, &details);
        assert!(!job.run_batch().unwrap());
        assert!(!job.run_batch().unwrap());
        assert!(job.run_batch().is_err());
        let snapshot = active(&index);
        assert!(snapshot.detail_recovery_requires_account);
        assert!(!snapshot.detail_recovery_complete);
        assert!(snapshot.accounts.is_empty());
        seed(&details, "unreadable");
        finish(&mut worker(&index, &details));
        assert!(active(&index).detail_recovery_complete);
    }
}
