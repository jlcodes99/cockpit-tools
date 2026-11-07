use super::types::{ChatgptPlanAccountSummary, Registration};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn directory() -> Result<PathBuf, String> {
    crate::modules::account::get_data_dir()
        .map(|path| path.join("chatgpt-plan"))
        .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())
}

pub(super) async fn blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?
}

fn valid_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "CHATGPT_PLAN_NOT_FOUND".to_string())
}

fn record_path(id: &str) -> Result<PathBuf, String> {
    valid_id(id)?;
    Ok(directory()?.join(format!("{id}.json")))
}

pub(super) fn official_health_path(id: &str) -> Result<PathBuf, String> {
    Ok(record_path(id)?.with_extension("official-health"))
}

pub(super) fn load(id: &str) -> Result<Registration, String> {
    let path = record_path(id)?;
    let content = fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "CHATGPT_PLAN_NOT_FOUND".to_string()
        } else {
            "CHATGPT_PLAN_STORAGE".to_string()
        }
    })?;
    let mut record: Registration =
        serde_json::from_str(&content).map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
    if record.summary.id != id {
        return Err("CHATGPT_PLAN_STORAGE".into());
    }
    // An on-disk or old caller snapshot cannot bless a source association.
    record.summary.source_binding_valid = false;
    record.summary.source_binding_error = None;
    Ok(record)
}

pub(super) fn save(record: &Registration) -> Result<(), String> {
    let mut durable = record.clone();
    durable.summary.source_binding_valid = false;
    durable.summary.source_binding_error = None;
    let content =
        serde_json::to_string(&durable).map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
    crate::modules::atomic_write::write_secret_string_atomic(
        &record_path(&record.summary.id)?,
        &content,
    )
    .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())
}

pub(super) async fn read(id: &str) -> Result<Registration, String> {
    let id = id.to_string();
    blocking(move || load(&id)).await
}

pub(super) async fn list() -> Result<Vec<ChatgptPlanAccountSummary>, String> {
    blocking(|| {
        Ok(load_all()?
            .into_iter()
            .map(|record| record.summary)
            .collect())
    })
    .await
}

pub(super) fn load_all() -> Result<Vec<Registration>, String> {
    let directory = directory()?;
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut accounts = Vec::new();
    for entry in fs::read_dir(directory).map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())? {
        let path = entry
            .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?
            .path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Some(id) = path.file_stem().and_then(|value| value.to_str()) {
            valid_id(id)?;
            accounts.push(load(id)?);
        }
    }
    accounts.sort_by_key(|account| account.summary.created_at);
    Ok(accounts)
}

// Closing just our descriptor is insufficient on Unix: a concurrently spawned
// child can retain the same open file description until exec. Release ownership
// explicitly before closing so a completed refresh does not leave a busy lock.
pub(super) struct RegistrationLock(File);

impl Drop for RegistrationLock {
    fn drop(&mut self) {
        if fs2::FileExt::unlock(&self.0).is_err() {
            crate::modules::logger::log_warn("[ChatGPT Plan] registration lock release failed");
        }
    }
}

fn try_file_lock(path: &Path) -> Result<Option<RegistrationLock>, String> {
    if path.is_symlink() {
        return Err("CHATGPT_PLAN_STORAGE".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(RegistrationLock(file))),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(_) => Err("CHATGPT_PLAN_STORAGE".into()),
    }
}

// A dedicated registration lock spans refresh/revoke network operations. File
// locks coordinate rotating refresh tokens across processes; no UI/global lock
// is held. Contention is bounded rather than parking a runtime thread.
pub(super) async fn lock(id: &str) -> Result<RegistrationLock, String> {
    valid_id(id)?;
    let id = id.to_string();
    let path = blocking(move || Ok(directory()?.join(format!("{id}.lock")))).await?;
    lock_path(path).await
}

/// Serializes the one-source/one-registration association across processes.
/// Always acquire this before a registration lock; refresh only takes the latter.
pub(super) async fn binding_lock() -> Result<RegistrationLock, String> {
    let path = blocking(|| Ok(directory()?.join("bindings.lock"))).await?;
    lock_path(path).await
}

async fn lock_path(path: PathBuf) -> Result<RegistrationLock, String> {
    for _ in 0..350 {
        let attempt_path = path.clone();
        if let Some(file) = blocking(move || try_file_lock(&attempt_path)).await? {
            return Ok(file);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("CHATGPT_PLAN_BUSY".into())
}

pub(super) async fn host_id() -> Result<String, String> {
    let lock_id = "00000000-0000-0000-0000-000000000000";
    let _guard = lock(lock_id).await?;
    blocking(|| {
        let path = directory()?.join("host-id");
        match fs::read_to_string(&path) {
            Ok(id) => {
                valid_id(id.trim()).map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
                Ok(id.trim().to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let id = uuid::Uuid::new_v4().to_string();
                crate::modules::atomic_write::write_secret_string_atomic(&path, &id)
                    .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
                Ok(id)
            }
            Err(_) => Err("CHATGPT_PLAN_STORAGE".into()),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_ids_cannot_escape_storage() {
        for id in ["../codex", "", "/tmp/auth", "codex-account"] {
            assert!(valid_id(id).is_err());
        }
        assert!(valid_id(&uuid::Uuid::new_v4().to_string()).is_ok());
    }

    #[test]
    fn file_lock_excludes_parallel_refresh_and_releases_on_drop() {
        let directory = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = directory.join("refresh.lock");
        let first = try_file_lock(&path).unwrap().unwrap();
        assert!(try_file_lock(&path).unwrap().is_none());
        drop(first);
        assert!(try_file_lock(&path).unwrap().is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refresh_lock_release_does_not_wait_for_inherited_descriptors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("refresh.lock");
        let first = try_file_lock(&path).unwrap().unwrap();
        // dup and fork share the open file description. CLOEXEC only closes an
        // inherited descriptor at exec, after the parent may have released it.
        let inherited = first.0.try_clone().unwrap();
        assert!(try_file_lock(&path).unwrap().is_none());
        drop(first);
        let second = try_file_lock(&path)
            .unwrap()
            .expect("owner must release its lock");
        assert!(try_file_lock(&path).unwrap().is_none());
        drop(inherited);
        assert!(try_file_lock(&path).unwrap().is_none());
        drop(second);
        assert!(try_file_lock(&path).unwrap().is_some());
    }

    #[test]
    fn registration_lock_coordinates_independent_processes() {
        const CHILD_PATH: &str = "COCKPIT_TEST_REGISTRATION_LOCK_PATH";
        const CHILD_EXPECT_AVAILABLE: &str = "COCKPIT_TEST_REGISTRATION_LOCK_AVAILABLE";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let available = try_file_lock(Path::new(&path)).unwrap().is_some();
            assert_eq!(
                available,
                std::env::var(CHILD_EXPECT_AVAILABLE).unwrap() == "1"
            );
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("registration.lock");
        let probe = |available: bool| {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "modules::chatgpt_plan::storage::tests::registration_lock_coordinates_independent_processes",
                    "--nocapture",
                ])
                .env(CHILD_PATH, &path)
                .env(CHILD_EXPECT_AVAILABLE, if available { "1" } else { "0" });
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                child.creation_flags(0x08000000);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let first = try_file_lock(&path).unwrap().unwrap();
        probe(false);
        drop(first);
        probe(true);
        let second = try_file_lock(&path).unwrap().unwrap();
        probe(false);
        drop(second);
        probe(true);
    }
}
