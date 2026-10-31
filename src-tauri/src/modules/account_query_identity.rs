//! Small, versioned identity projections. Never put credentials in list indexes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryIdentity {
    pub version: u32,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub credential_digest: Option<String>,
    pub platform: String,
}

pub(crate) fn fingerprint(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| format!("{:x}", Sha256::digest(v.as_bytes())))
}

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

/// A persisted per-record version is also the backfill checkpoint. Only missing
/// records are read, and a failed detail never becomes a fabricated identity.
pub(crate) fn backfill<S: IdentitySummary>(
    summaries: &mut [S],
    path: impl Fn(&str) -> Result<std::path::PathBuf, String>,
    load_summary: impl Fn(&str) -> Option<S>,
) -> Result<(bool, bool), String> {
    let started = std::time::Instant::now();
    let mut changed = false;
    let mut bytes = 0;
    let mut count = 0;
    let mut first_error = None;
    for summary in summaries.iter_mut().filter(|s| s.identity().is_none()) {
        if count >= super::background_maintenance::BATCH_ROWS
            || bytes >= super::background_maintenance::BATCH_BYTES
            || started.elapsed() >= super::background_maintenance::BATCH_TIME
        {
            return Ok((false, changed));
        }
        let file = path(summary.id())?;
        let size = match std::fs::metadata(&file) {
            Ok(meta) => meta.len(),
            Err(error) => {
                first_error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        if size > super::background_maintenance::BATCH_BYTES {
            first_error.get_or_insert_with(|| {
                format!(
                    "identity detail exceeds maintenance byte budget: {}",
                    summary.id()
                )
            });
            continue;
        }
        if changed && bytes + size > super::background_maintenance::BATCH_BYTES {
            return Ok((false, true));
        }
        let Some(next) = load_summary(summary.id()) else {
            first_error
                .get_or_insert_with(|| format!("identity detail unavailable: {}", summary.id()));
            continue;
        };
        *summary = next;
        bytes += size;
        count += 1;
        changed = true;
    }
    if let Some(error) = first_error {
        if changed {
            Ok((false, true))
        } else {
            Err(error)
        }
    } else {
        Ok((true, changed))
    }
}

/// Build outside the platform mutex; compare the source index under the same
/// write lock before committing so newer writes and removals always win.
pub(crate) fn commit_backfill<T: Serialize>(
    path: &std::path::Path,
    before: &str,
    next: &T,
) -> Result<bool, String> {
    let hash: [u8; 32] = Sha256::digest(before.as_bytes()).into();
    let content = serde_json::to_string_pretty(next).map_err(|e| e.to_string())?;
    crate::modules::atomic_write::write_string_atomic_if_hash_matches(path, hash, || Ok(content))
}

/// Invalidate before replacing a detail whose identity changed. If the process
/// exits between the two file writes, the next reader cannot trust an old
/// identity snapshot; background backfill repairs only that incomplete record.
pub(crate) fn invalidate_before_write(
    path: &std::path::Path,
    id: &str,
    next: &QueryIdentity,
) -> Result<(), String> {
    let content = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let mut value: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let Some(items) = value
        .get_mut("accounts")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    let Some(item) = items.iter_mut().find(|v| v["id"].as_str() == Some(id)) else {
        return Ok(());
    };
    if item
        .get("query_identity")
        .and_then(|v| serde_json::from_value::<QueryIdentity>(v.clone()).ok())
        .as_ref()
        == Some(next)
    {
        return Ok(());
    }
    if item["query_identity"].is_null() {
        return Ok(());
    }
    item["query_identity"] = serde_json::Value::Null;
    if let Some(object) = value.as_object_mut() {
        let revision = object
            .get("identity_revision")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        object.insert("identity_revision".into(), revision.wrapping_add(1).into());
        object.insert("identity_maintenance_version".into(), 0.into());
    }
    if !commit_backfill(path, &content, &value)? {
        return Err("identity index changed during account write".into());
    }
    Ok(())
}

pub(crate) fn backup_duplicate(path: &std::path::Path) -> Result<[u8; 32], String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if content.len() as u64 > super::background_maintenance::BATCH_BYTES {
        return Err("duplicate detail exceeds maintenance byte budget".into());
    }
    let hash: [u8; 32] = Sha256::digest(content.as_bytes()).into();
    let backup = path.with_extension(format!(
        "json.dedup-{:x}.backup",
        Sha256::digest(content.as_bytes())
    ));
    crate::modules::atomic_write::write_string_atomic(&backup, &content)?;
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
            if work >= super::background_maintenance::BATCH_ROWS
                || started.elapsed() >= super::background_maintenance::BATCH_TIME
            {
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
                        if work >= super::background_maintenance::BATCH_ROWS {
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
            assert!(scan.position - before <= super::super::background_maintenance::BATCH_ROWS);
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

    #[test]
    fn backfill_cas_preserves_updates_deletes_and_marks_identity_write_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let identity = QueryIdentity {
            version: 1,
            subject: Some("old".into()),
            ..Default::default()
        };
        let original = serde_json::json!({"accounts":[{"id":"a","query_identity":identity}],"identity_revision":1,"identity_maintenance_version":1});
        let source = serde_json::to_string(&original).unwrap();
        std::fs::write(&path, &source).unwrap();
        let newer = serde_json::json!({"accounts":[],"identity_revision":2});
        std::fs::write(&path, newer.to_string()).unwrap();
        assert!(!commit_backfill(&path, &source, &original).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer.to_string());
        std::fs::remove_file(&path).unwrap();
        assert!(!commit_backfill(&path, &source, &original).unwrap());
        assert!(!path.exists());
        std::fs::write(&path, &source).unwrap();
        invalidate_before_write(
            &path,
            "a",
            &QueryIdentity {
                version: 1,
                subject: Some("new".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let current: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(current["accounts"][0]["query_identity"].is_null());
        assert_eq!(current["identity_maintenance_version"], 0);
        assert_eq!(current["identity_revision"], 2);
    }
}
