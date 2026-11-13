pub(crate) use super::local_history_store::{checkin_timestamp, HistoryRow, HistoryStore};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

static MIGRATION_ERRORS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRead<T> {
    pub(crate) records: Vec<T>,
    migration_pending: bool,
    migration_error: Option<String>,
}

impl<T> HistoryRead<T> {
    pub(crate) fn is_complete(&self) -> bool {
        !self.migration_pending && self.migration_error.is_none()
    }
}

pub(super) fn schedule_tracked(
    key: String,
    mut migrate: impl FnMut() -> Result<bool, String> + Send + 'static,
) {
    super::background_maintenance::schedule(key.clone(), move || {
        let result = migrate();
        if let Ok(mut errors) = MIGRATION_ERRORS.lock() {
            match &result {
                Err(error) => {
                    errors.insert(key.clone(), error.clone());
                }
                Ok(_) => {
                    errors.remove(&key);
                }
            }
        }
        result
    });
}

fn schedule(store: HistoryStore, migrate: fn(&HistoryStore) -> Result<bool, String>) {
    schedule_tracked(store.migration_key(), move || migrate(&store));
}

pub(crate) fn read_available<T: HistoryRow>(
    store: HistoryStore,
    retry: bool,
) -> Result<HistoryRead<T>, String> {
    let (records, migration_pending) = store.load_available()?;
    Ok(track_read(
        records,
        migration_pending,
        &store.migration_key(),
        retry,
        || store.schedule::<T>(),
    ))
}

pub(super) fn track_read<T>(
    records: Vec<T>,
    migration_pending: bool,
    key: &str,
    retry: bool,
    schedule: impl FnOnce(),
) -> HistoryRead<T> {
    let mut result = HistoryRead {
        records,
        migration_pending,
        migration_error: None,
    };
    if !migration_pending {
        return result;
    }
    if super::background_maintenance::is_scheduled(key) {
        return result;
    }
    if let Ok(mut errors) = MIGRATION_ERRORS.lock() {
        if retry {
            errors.remove(key);
        } else if let Some(error) = errors.get(key) {
            result.migration_error = Some(error.clone());
            result.migration_pending = false;
            return result;
        }
    }
    schedule();
    result
}

pub(crate) fn wakeup(
    source: std::path::PathBuf,
    scope: &'static str,
    limit: usize,
) -> HistoryStore {
    HistoryStore::new(source, scope, limit).with_scheduler(schedule)
}

pub(crate) fn daily_checkin(source: std::path::PathBuf, scope: &'static str) -> HistoryStore {
    HistoryStore::daily_checkin(source, scope).with_scheduler(schedule)
}

pub(crate) fn switches(
    source: std::path::PathBuf,
    scope: &'static str,
    limit: usize,
) -> HistoryStore {
    HistoryStore::switches(source, scope, limit).with_scheduler(schedule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: String,
        timestamp: i64,
    }

    impl HistoryRow for Item {
        fn id(&self) -> &str {
            &self.id
        }
        fn timestamp(&self) -> i64 {
            self.timestamp
        }
    }

    #[test]
    fn history_read_preserves_available_rows_after_failed_import_and_can_retry() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("history.json");
        fs::write(&source, "invalid").unwrap();
        let store = daily_checkin(source.clone(), "fixture");
        store
            .add(&[Item {
                id: "live".into(),
                timestamp: 1,
            }])
            .unwrap();

        let initial = read_available::<Item>(store.clone(), false).unwrap();
        assert_eq!(initial.records[0].id, "live");
        assert!(initial.migration_pending);
        let deadline = Instant::now() + Duration::from_secs(15);
        let failed = loop {
            let current = read_available::<Item>(store.clone(), false).unwrap();
            if !current.migration_pending {
                break current;
            }
            assert!(
                Instant::now() < deadline,
                "invalid source never reached terminal failure"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(failed.records, initial.records);
        assert!(failed.migration_error.is_some());

        fs::write(&source, r#"[{"id":"restored","timestamp":2}]"#).unwrap();
        assert!(
            read_available::<Item>(store.clone(), true)
                .unwrap()
                .migration_pending
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let complete = loop {
            let current = read_available::<Item>(store.clone(), false).unwrap();
            if !current.migration_pending {
                break current;
            }
            assert!(Instant::now() < deadline, "retry did not finish");
            thread::sleep(Duration::from_millis(20));
        };
        assert!(complete.migration_error.is_none());
        assert_eq!(
            complete
                .records
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["restored", "live"]
        );
    }
}
