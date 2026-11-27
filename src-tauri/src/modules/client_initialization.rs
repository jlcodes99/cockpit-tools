use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use rusqlite::{Connection, ErrorCode, OpenFlags};
use tauri::Emitter;

pub(crate) struct StartupProgress {
    target: (String, String),
}

fn active_targets() -> &'static Mutex<HashSet<(String, String)>> {
    static TARGETS: OnceLock<Mutex<HashSet<(String, String)>>> = OnceLock::new();
    TARGETS.get_or_init(|| Mutex::new(HashSet::new()))
}

impl StartupProgress {
    fn emit(&self, phase: &str) {
        if let Some(app) = crate::get_app_handle() {
            let _ = app.emit(
                "client-initialization-progress",
                serde_json::json!({
                    "platform": self.target.0, "targetId": self.target.1, "phase": phase,
                }),
            );
        }
    }
}

impl Drop for StartupProgress {
    fn drop(&mut self) {
        if let Ok(mut active) = active_targets().lock() {
            active.remove(&self.target);
        }
        self.emit("finished");
    }
}

pub(crate) async fn prepare_profile(
    platform: &'static str,
    target_id: String,
    ready: impl Fn() -> Result<bool, String> + Send + 'static,
    open: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<Option<StartupProgress>, String> {
    tokio::task::spawn_blocking(move || prepare(platform, &target_id, ready, open))
        .await
        .map_err(|error| error.to_string())?
}

/// Run on a blocking worker. The caller keeps the returned guard through the
/// normal credential write and restart, whose process scope stays with it.
pub(crate) fn prepare(
    platform: &str,
    target_id: &str,
    ready: impl Fn() -> Result<bool, String>,
    open: impl FnOnce() -> Result<(), String>,
) -> Result<Option<StartupProgress>, String> {
    let target = (platform.to_owned(), target_id.to_owned());
    {
        let mut active = active_targets()
            .lock()
            .map_err(|_| "CLIENT_INITIALIZATION_BUSY")?;
        if !active.insert(target.clone()) {
            return Err("CLIENT_INITIALIZATION_BUSY".into());
        }
    }
    let progress = StartupProgress { target };
    if ready()? {
        return Ok(None);
    }
    progress.emit("initializing");
    open()?;
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if ready()? {
            progress.emit("restarting");
            return Ok(Some(progress));
        }
        if Instant::now() >= deadline {
            return Err(super::i18n::translate(
                &super::config::get_user_config().language,
                "instances.messages.firstLaunchTimedOut",
                &[],
            ));
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub(crate) fn state_db_ready(root: &Path) -> Result<bool, String> {
    let path = root.join("User/globalStorage/state.vscdb");
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(false);
    }
    let check = || {
        let database = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        database.busy_timeout(Duration::from_millis(250))?;
        database.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ItemTable')",
            [],
            |row| row.get(0),
        )
    };
    match check() {
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            Ok(false)
        }
        result => result.map_err(|error| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn ready_client_needs_no_extra_launch() {
        let result = prepare(
            "fixture",
            "warm",
            || Ok(true),
            || panic!("unexpected launch"),
        );
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn first_launch_is_single_flight_until_restart_finishes() {
        let storage_ready = Cell::new(false);
        let launches = Cell::new(0);
        let progress = prepare(
            "fixture",
            "cold",
            || Ok(storage_ready.get()),
            || {
                launches.set(launches.get() + 1);
                storage_ready.set(true);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(launches.get(), 1);
        assert!(prepare("fixture", "cold", || Ok(true), || Ok(())).is_err());
        drop(progress);
        assert!(prepare("fixture", "cold", || Ok(true), || Ok(()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn state_file_creation_alone_is_not_ready_for_account_write() {
        let profile = tempfile::tempdir().unwrap();
        let storage = profile.path().join("User/globalStorage");
        assert!(!state_db_ready(profile.path()).unwrap());
        std::fs::create_dir_all(&storage).unwrap();
        let db = Connection::open(storage.join("state.vscdb")).unwrap();
        assert!(!state_db_ready(profile.path()).unwrap());
        db.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        assert!(state_db_ready(profile.path()).unwrap());
    }
}
