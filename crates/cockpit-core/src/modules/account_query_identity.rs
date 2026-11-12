//! Small, versioned identity projections. Never put credentials in list indexes.
use super::antigravity_account_index_store::{
    IndexDocument, IndexStore, BATCH_BYTES, BATCH_ROWS, BATCH_TIME,
};
use super::provider_account_index;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

pub(crate) use super::query_identity::fingerprint;
pub use super::query_identity::QueryIdentity;

pub(crate) trait IdentitySummary {
    fn id(&self) -> &str;
    fn identity(&self) -> Option<&QueryIdentity>;
}

macro_rules! summary_impl {
    ($($ty:ty),+) => { $(impl IdentitySummary for $ty {
        fn id(&self) -> &str { &self.id }
        fn identity(&self) -> Option<&QueryIdentity> { self.query_identity.as_ref().filter(|v| v.version == 1) }
    })+ };
}
summary_impl!(
    crate::models::kiro::KiroAccountSummary,
    crate::models::windsurf::WindsurfAccountSummary,
    crate::models::trae::TraeAccountSummary
);

#[derive(Default)]
pub(crate) struct BackfillCursor {
    after: Option<i64>,
    unreadable: usize,
}

pub(crate) enum BackfillStatus {
    Inactive,
    Pending,
    Done { revision: u64, version: u32 },
}

impl BackfillCursor {
    pub(crate) fn step<I: IndexDocument, S: IdentitySummary + Serialize>(
        &mut self,
        store: &IndexStore<I>,
        lock: &Mutex<()>,
        path: impl Fn(&str) -> Result<PathBuf, String>,
        load: impl Fn(&str) -> Option<S>,
    ) -> Result<BackfillStatus, String> {
        let Some((revision, version, candidates)) =
            store.missing_identity_batch(self.after.unwrap_or(-1))?
        else {
            if !store.has_legacy_source()? {
                return Ok(BackfillStatus::Inactive);
            }
            let finished = store.migrate_batch()?;
            return Ok(if finished && store.active_revision()?.is_none() {
                BackfillStatus::Inactive
            } else {
                BackfillStatus::Pending
            });
        };
        if candidates.is_empty() {
            if self.after.take().is_none() {
                return Ok(BackfillStatus::Done { revision, version });
            }
            if self.unreadable != 0 {
                let count = std::mem::take(&mut self.unreadable);
                return Err(format!("有 {count} 份账号详情暂时无法用于身份回填"));
            }
            return Ok(BackfillStatus::Pending);
        }

        let started = Instant::now();
        let mut observed = Vec::new();
        let mut bytes = 0;
        let mut unreadable = 0;
        let mut after = self.after;
        for (position, id) in candidates {
            if started.elapsed() >= BATCH_TIME || bytes >= BATCH_BYTES {
                break;
            }
            let file = path(&id)?;
            let Ok(stamp) = file_stamp(&file) else {
                unreadable += 1;
                after = Some(position);
                continue;
            };
            if stamp.size > BATCH_BYTES {
                unreadable += 1;
                after = Some(position);
                continue;
            }
            if bytes + stamp.size > BATCH_BYTES {
                break;
            }
            bytes += stamp.size;
            let Some(next) =
                load(&id).filter(|summary| summary.id() == id && summary.identity().is_some())
            else {
                unreadable += 1;
                after = Some(position);
                continue;
            };
            observed.push((file, stamp, provider_account_index::to_summary(&next)?));
            after = Some(position);
        }

        let Ok(_local) = lock.try_lock() else {
            return Ok(BackfillStatus::Pending);
        };
        let Some(_writer) = store.try_lock_writes()? else {
            return Ok(BackfillStatus::Pending);
        };
        if store.active_revision()? != Some(revision) {
            self.after = None;
            self.unreadable = 0;
            return Ok(BackfillStatus::Pending);
        }
        let mut rows = Vec::new();
        for (path, stamp, row) in observed {
            if file_stamp(&path).ok() == Some(stamp) {
                rows.push(row);
            }
        }
        if !rows.is_empty() && !store.commit_identity_batch(revision, &rows)? {
            return Ok(BackfillStatus::Pending);
        }
        self.after = after;
        self.unreadable += unreadable;
        Ok(BackfillStatus::Pending)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    inode: (u64, u64),
}

fn file_stamp(path: &Path) -> std::io::Result<FileStamp> {
    let metadata = fs::metadata(path)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(FileStamp {
        size: metadata.len(),
        modified: metadata.modified()?,
        #[cfg(unix)]
        inode: (metadata.dev(), metadata.ino()),
    })
}

pub(crate) fn backup_duplicate(path: &Path) -> Result<[u8; 32], String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut content = String::new();
    file.take(BATCH_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|error| error.to_string())?;
    if content.len() as u64 > BATCH_BYTES {
        return Err("duplicate detail exceeds maintenance byte budget".into());
    }
    let hash: [u8; 32] = Sha256::digest(content.as_bytes()).into();
    let backup = path.with_extension(format!(
        "json.dedup-{:x}.backup",
        Sha256::digest(content.as_bytes())
    ));
    if let Ok(existing) = fs::File::open(&backup) {
        let mut bytes = Vec::new();
        existing
            .take(BATCH_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes == content.as_bytes() {
            return Ok(hash);
        }
    }
    crate::modules::atomic_write::write_secret_string_atomic(&backup, &content)?;
    Ok(hash)
}

/// Generate only identity-compatible buckets. A missing identity component is
/// a wildcard *only* where the existing platform matching rules permit it.
pub(crate) fn duplicate_keys(identity: &QueryIdentity) -> Vec<String> {
    if identity.platform == "windsurf" {
        let subject = identity.subject.as_deref().unwrap_or("");
        return match &identity.credential_digest {
            Some(digest) => vec![
                format!("w:{subject}:{digest}"),
                format!("w:{subject}:missing"),
            ],
            None => vec![format!("w:{subject}:all")],
        };
    }
    let mut keys = Vec::new();
    if let Some(subject) = &identity.subject {
        keys.push(format!("s:{subject}"));
    }
    if identity.subject.is_none() {
        if let Some(email) = &identity.email {
            keys.push(format!("e:{email}"));
        }
    }
    if let Some(digest) = &identity.credential_digest {
        keys.push(format!("c:{digest}"));
    }
    keys
}

#[derive(Default)]
pub(crate) struct DedupScan {
    revision: Option<u64>,
    position: usize,
    candidate: usize,
    buckets: std::collections::HashMap<String, Vec<usize>>,
}
pub(crate) enum ScanResult {
    Pending,
    Complete,
    Pair(usize, usize),
}
impl DedupScan {
    pub(crate) fn step<S: IdentitySummary>(
        &mut self,
        rows: &[S],
        revision: u64,
        compatible: impl Fn(&QueryIdentity, &QueryIdentity) -> bool,
    ) -> ScanResult {
        if self.revision != Some(revision) {
            *self = Self {
                revision: Some(revision),
                ..Self::default()
            };
        }
        let started = std::time::Instant::now();
        let mut work = 0;
        while self.position < rows.len() {
            if work >= BATCH_ROWS || started.elapsed() >= BATCH_TIME {
                return ScanResult::Pending;
            }
            let Some(identity) = rows[self.position].identity() else {
                return ScanResult::Pending;
            };
            let keys = duplicate_keys(identity);
            let mut offset = 0;
            for key in &keys {
                if let Some(bucket) = self.buckets.get(key) {
                    for &previous in bucket {
                        if offset < self.candidate {
                            offset += 1;
                            continue;
                        }
                        if work >= BATCH_ROWS {
                            return ScanResult::Pending;
                        }
                        work += 1;
                        offset += 1;
                        self.candidate = offset;
                        if rows[previous]
                            .identity()
                            .is_some_and(|other| compatible(other, identity))
                        {
                            return ScanResult::Pair(previous, self.position);
                        }
                    }
                }
            }
            if identity.platform == "windsurf" {
                let subject = identity.subject.as_deref().unwrap_or("");
                let specific = identity.credential_digest.as_deref().unwrap_or("missing");
                for key in [
                    format!("w:{subject}:{specific}"),
                    format!("w:{subject}:all"),
                ] {
                    self.buckets.entry(key).or_default().push(self.position);
                }
            } else {
                for key in keys {
                    self.buckets.entry(key).or_default().push(self.position);
                }
            }
            self.position += 1;
            self.candidate = 0;
            work += 1;
        }
        ScanResult::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_only_fingerprints_and_case_sensitive() {
        assert_eq!(fingerprint(Some(" secret ")), fingerprint(Some("secret")));
        assert_ne!(fingerprint(Some("secret")), fingerprint(Some("Secret")));
        assert!(!fingerprint(Some("secret")).unwrap().contains("secret"));
        assert_eq!(fingerprint(Some("  ")), None);
    }

    struct Summary(String, QueryIdentity);
    impl IdentitySummary for Summary {
        fn id(&self) -> &str {
            &self.0
        }
        fn identity(&self) -> Option<&QueryIdentity> {
            Some(&self.1)
        }
    }
    #[test]
    fn dedup_scan_is_bounded_and_restarts_after_revision_change() {
        let rows = (0..10_000)
            .map(|i| {
                Summary(
                    i.to_string(),
                    QueryIdentity {
                        version: 1,
                        subject: Some(i.to_string()),
                        ..Default::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut scan = DedupScan::default();
        let comparisons = std::cell::Cell::new(0);
        let mut batches = 0;
        loop {
            batches += 1;
            let before = scan.position;
            let result = scan.step(&rows, 1, |_, _| {
                comparisons.set(comparisons.get() + 1);
                false
            });
            assert!(scan.position - before <= BATCH_ROWS);
            if matches!(result, ScanResult::Complete) {
                break;
            }
        }
        assert!(batches >= 313);
        assert_eq!(
            comparisons.get(),
            0,
            "unique accounts must not be compared pairwise"
        );
        assert!(matches!(
            scan.step(&rows, 2, |_, _| true),
            ScanResult::Pending
        ));
        assert!(scan.position <= 32);
    }

    #[test]
    fn legacy_summaries_remain_unmigrated_not_fabricated() {
        let value =
            serde_json::json!({"id":"old","email":"user@example.com","created_at":0,"last_used":0});
        let summary: crate::models::kiro::KiroAccountSummary =
            serde_json::from_value(value).unwrap();
        assert!(summary.identity().is_none());
    }
    fn kiro_summary(id: &str, subject: Option<&str>) -> crate::models::kiro::KiroAccountSummary {
        let mut row = serde_json::from_value::<crate::models::kiro::KiroAccountSummary>(
            serde_json::json!({"id": id, "email": "fixture@example.test", "created_at": 1, "last_used": 1}),
        ).unwrap();
        row.query_identity = subject.map(|subject| QueryIdentity {
            version: 1,
            subject: Some(subject.into()),
            platform: "kiro".into(),
            ..QueryIdentity::default()
        });
        row
    }

    #[test]
    fn identity_migration_keeps_metadata_and_rejects_stale_backfill_after_delete() {
        use crate::{
            models::kiro::KiroAccountIndex, modules::antigravity_account_index_store::IndexRead,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiro_accounts.json");
        let mut legacy = KiroAccountIndex::new();
        legacy.identity_revision = 42;
        legacy.identity_maintenance_version = 1;
        legacy.accounts = vec![
            kiro_summary("missing", None),
            kiro_summary("ready", Some("old")),
        ];
        let source = serde_json::to_string(&legacy).unwrap();
        fs::write(&path, &source).unwrap();

        let store = provider_account_index::store::<KiroAccountIndex>(&path).unwrap();
        assert!(store.migrate_batch().unwrap());
        let (revision, version, pending) = store.missing_identity_batch(-1).unwrap().unwrap();
        assert_eq!(version, 1);
        assert_eq!(pending, [(0, "missing".into())]);
        let IndexRead::Active(mut edited) = store.load().unwrap() else {
            panic!("expected SQLite index")
        };
        assert_eq!(edited.identity_revision, 42);
        edited.accounts = vec![kiro_summary("ready", Some("new"))];
        store.replace(&edited).unwrap();
        let stale =
            provider_account_index::to_summary(&kiro_summary("missing", Some("stale"))).unwrap();
        assert!(!store.commit_identity_batch(revision, &[stale]).unwrap());
        let IndexRead::Active(current) = store.load().unwrap() else {
            panic!("expected SQLite index")
        };
        assert_eq!(current.accounts.len(), 1);
        assert_eq!(
            current.accounts[0].identity().unwrap().subject.as_deref(),
            Some("new")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn identity_backfill_persists_progress_past_unreadable_rows() {
        use crate::models::kiro::KiroAccountIndex;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiro_accounts.json");
        let store = provider_account_index::store::<KiroAccountIndex>(&path).unwrap();
        let mut index = KiroAccountIndex::new();
        index.identity_maintenance_version = 1;
        index.accounts = (0..40)
            .map(|i| kiro_summary(&format!("account-{i}"), None))
            .collect();
        store.replace(&index).unwrap();
        fs::write(dir.path().join("account-39"), "{}").unwrap();
        let lock = Mutex::new(());
        let mut cursor = BackfillCursor::default();
        let mut reported_missing = false;
        for _ in 0..10 {
            let result = cursor.step(
                &store,
                &lock,
                |id| Ok(dir.path().join(id)),
                |id| Some(kiro_summary(id, Some("resolved"))),
            );
            if result.is_err() {
                reported_missing = true;
                break;
            }
        }
        assert!(reported_missing);
        let crate::modules::antigravity_account_index_store::IndexRead::Active(current) =
            store.load().unwrap()
        else {
            panic!("expected SQLite index")
        };
        assert_eq!(current.identity_maintenance_version, 0);
        assert!(current.accounts[0].identity().is_none());
        assert_eq!(
            current.accounts[39].identity().unwrap().subject.as_deref(),
            Some("resolved")
        );
    }
}
