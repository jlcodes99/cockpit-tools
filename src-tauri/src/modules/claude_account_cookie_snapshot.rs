// Keep the original Chromium profile read-only. SQLite may need to roll back
// a hot journal, so databases are recovered on a disposable copy.
struct DesktopCookieSnapshot {
    connection: Option<Connection>,
    temporary_dir: Option<PathBuf>,
}

impl std::ops::Deref for DesktopCookieSnapshot {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("cookie snapshot connection")
    }
}

impl Drop for DesktopCookieSnapshot {
    fn drop(&mut self) {
        // Windows cannot remove an open SQLite file.
        self.connection.take();
        if let Some(directory) = self.temporary_dir.take() {
            if let Err(error) = fs::remove_dir_all(&directory) {
                logger::log_warn(&format!(
                    "[Claude Cookies] Temporary snapshot cleanup failed: {}",
                    error
                ));
            }
        }
    }
}

fn open_desktop_cookie_snapshot(cookies_path: &Path) -> Result<DesktopCookieSnapshot, String> {
    let sidecar_path = |suffix: &str| {
        let mut name = cookies_path.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    let directory =
        std::env::temp_dir().join(format!("cockpit-claude-cookies-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).map_err(|error| error.to_string())?;
    let mut snapshot = DesktopCookieSnapshot {
        connection: None,
        temporary_dir: Some(directory.clone()),
    };
    // Always copy: a running Chromium process can create WAL/journal files after
    // an existence check. SQLite must never recover or create SHM in the source.
    for suffix in ["", "-journal", "-wal"] {
        let source = sidecar_path(suffix);
        if suffix.is_empty() || source.exists() {
            let size = fs::metadata(&source)
                .map_err(|error| error.to_string())?
                .len();
            if size > 128 * 1024 * 1024 {
                return Err(crate::modules::i18n::translate(
                    &crate::modules::config::get_user_config().language,
                    "claude.errors.cookieSnapshotTooLarge",
                    &[],
                ));
            }
            let destination = directory.join(format!("Cookies{}", suffix));
            fs::copy(&source, &destination).map_err(|error| error.to_string())?;
            // Recovery writes only to this disposable copy, even when the source
            // has a read-only attribute. Unix copies restrict credentials to owner.
            let mut permissions = fs::metadata(&destination)
                .map_err(|error| error.to_string())?
                .permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                permissions.set_mode(0o600);
            }
            #[cfg(windows)]
            permissions.set_readonly(false);
            fs::set_permissions(&destination, permissions).map_err(|error| error.to_string())?;
        }
    }
    let read_path = directory.join("Cookies");
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE;
    let connection =
        Connection::open_with_flags(&read_path, flags).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    // Loading the schema lets SQLite recover the temporary journal/WAL without
    // the full-table scan of quick_check. Queries afterwards cannot write.
    connection
        .query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|error| error.to_string())?;
    snapshot.connection = Some(connection);
    Ok(snapshot)
}

fn ensure_desktop_verification_profile(profile_dir: &Path) -> Result<(), String> {
    if profile_dir.is_dir() {
        return Ok(());
    }
    Err(crate::modules::i18n::translate(
        &crate::modules::config::get_user_config().language,
        "claude.errors.profileMissing",
        &[],
    ))
}
