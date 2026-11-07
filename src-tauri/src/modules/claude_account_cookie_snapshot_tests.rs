use super::*;

struct CookieTestDirectory(PathBuf);
impl CookieTestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("cockpit-cookie-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for CookieTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Only a subprocess exits here, intentionally leaving an uncommitted hot journal.
#[test]
fn cookie_snapshot_hot_journal_child() {
    let Some(path) = std::env::var_os("COCKPIT_COOKIE_TEST_CRASH_DB") else {
        return;
    };
    let connection = Connection::open(PathBuf::from(path)).unwrap();
    connection
        .execute_batch(
            "PRAGMA cache_size=1; BEGIN IMMEDIATE; UPDATE cookies SET value=zeroblob(8192);",
        )
        .unwrap();
    std::process::exit(0);
}

#[test]
fn cookie_snapshot_recovers_hot_journal_without_writing_original() {
    let directory = CookieTestDirectory::new();
    let path = directory.0.join("Cookies");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE cookies(value TEXT); BEGIN;")
            .unwrap();
        for _ in 0..150 {
            connection
                .execute("INSERT INTO cookies VALUES (?1)", ["original".repeat(1024)])
                .unwrap();
        }
        connection.execute_batch("COMMIT").unwrap();
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .arg("cookie_snapshot_hot_journal_child")
        .arg("--nocapture")
        .env("COCKPIT_COOKIE_TEST_CRASH_DB", &path);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000);
    }
    assert!(child.status().unwrap().success());
    let journal = directory.0.join("Cookies-journal");
    let original_db = fs::read(&path).unwrap();
    let original_journal = fs::read(&journal).unwrap();
    assert!(!original_journal.is_empty());
    let snapshot = open_desktop_cookie_snapshot(&path).unwrap();
    let temporary_dir = snapshot.temporary_dir.clone().unwrap();
    let value: String = snapshot
        .query_row("SELECT value FROM cookies LIMIT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "original".repeat(1024));
    assert!(snapshot.execute("DELETE FROM cookies", []).is_err());
    drop(snapshot);
    assert!(!temporary_dir.exists());
    assert_eq!(fs::read(&path).unwrap(), original_db);
    assert_eq!(fs::read(&journal).unwrap(), original_journal);
}

#[test]
fn cookie_snapshot_reads_live_wal_and_keeps_source_untouched() {
    let directory = CookieTestDirectory::new();
    let path = directory.0.join("Cookies");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE cookies(value TEXT); INSERT INTO cookies VALUES ('live');").unwrap();
    let wal = directory.0.join("Cookies-wal");
    let originals = [fs::read(&path).unwrap(), fs::read(&wal).unwrap()];
    let snapshot = open_desktop_cookie_snapshot(&path).unwrap();
    let value: String = snapshot
        .query_row("SELECT value FROM cookies", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "live");
    drop(snapshot);
    assert_eq!(fs::read(&path).unwrap(), originals[0]);
    assert_eq!(fs::read(&wal).unwrap(), originals[1]);
}

#[test]
fn desktop_verification_allows_profile_without_login_cookie() {
    let directory = CookieTestDirectory::new();
    assert!(ensure_desktop_verification_profile(&directory.0).is_ok());
    assert!(ensure_desktop_verification_profile(&directory.0.join("missing")).is_err());
}
