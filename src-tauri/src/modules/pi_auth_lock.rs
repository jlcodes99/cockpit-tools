//! `auth.json` lock compatible with pi's `proper-lockfile` usage.
//!
//! pi locks `<home>/auth.json` by creating the directory `auth.json.lock`
//! (`realpath: false`) and treats a lock older than its `stale` window as
//! abandoned. Using the same directory makes cockpit and a running pi
//! serialize their read-modify-write of `auth.json`.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// pi's sync lock uses proper-lockfile's 10s default `stale`; stay below it.
const STALE_AFTER: Duration = Duration::from_secs(10);
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub struct AuthLock {
    dir: PathBuf,
}

impl Drop for AuthLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.dir);
    }
}

fn lock_dir(auth_path: &Path) -> PathBuf {
    let mut name = auth_path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

fn is_stale(dir: &Path) -> bool {
    fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_AFTER)
}

/// Block until the lock for `auth_path` is held (or time out).
pub fn lock(auth_path: &Path) -> Result<AuthLock, String> {
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败: {}", parent.display(), e))?;
    }
    let dir = lock_dir(auth_path);
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(AuthLock { dir }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                if is_stale(&dir) {
                    let _ = fs::remove_dir(&dir);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(format!("{} 正被 pi 占用，请稍后重试", auth_path.display()));
                }
                std::thread::sleep(RETRY_DELAY);
            }
            Err(e) => return Err(format!("锁定 {} 失败: {}", auth_path.display(), e)),
        }
    }
}

/// Lock several files in a stable order (avoids lock-order deadlocks).
pub fn lock_all(paths: &[PathBuf]) -> Result<Vec<AuthLock>, String> {
    let mut sorted: Vec<&PathBuf> = paths.iter().collect();
    sorted.sort();
    sorted.dedup();
    sorted.into_iter().map(|p| lock(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let dir = std::env::temp_dir().join(format!("pi-lock-test-{}", std::process::id()));
        let auth = dir.join("auth.json");
        let guard = lock(&auth).unwrap();
        assert!(lock_dir(&auth).is_dir());
        drop(guard);
        assert!(!lock_dir(&auth).exists());
        let _again = lock(&auth).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
