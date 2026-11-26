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

fn copy_readonly_snapshot_file_with_fallback<F>(
    source: &Path,
    destination: &Path,
    copy_fn: F,
) -> Result<(), String>
where
    F: FnOnce(&Path, &Path) -> std::io::Result<u64>,
{
    match copy_fn(source, destination) {
        Ok(_) => Ok(()),
        #[cfg(any(windows, test))]
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33 | 6000)) => {
            if source == destination
                || fs::canonicalize(source)
                    .ok()
                    .zip(fs::canonicalize(destination).ok())
                    .is_some_and(|(src, dst)| src == dst)
            {
                return Err("snapshot copy source equals destination".to_string());
            }
            #[cfg(windows)]
            let mut input = {
                use std::os::windows::fs::OpenOptionsExt;
                // FILE_SHARE_READ (1) | FILE_SHARE_WRITE (2) | FILE_SHARE_DELETE (4)
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0x0000_0007)
                    .open(source)
                    .map_err(|open_err| open_err.to_string())?
            };
            #[cfg(not(windows))]
            let mut input = File::open(source).map_err(|open_err| open_err.to_string())?;
            let mut output =
                File::create(destination).map_err(|create_err| create_err.to_string())?;
            std::io::copy(&mut input, &mut output).map_err(|copy_err| copy_err.to_string())?;
            output.sync_all().map_err(|sync_err| sync_err.to_string())?;
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn copy_readonly_snapshot_file(source: &Path, destination: &Path) -> Result<(), String> {
    copy_readonly_snapshot_file_with_fallback(source, destination, |src, dst| fs::copy(src, dst))
}

fn copy_desktop_cookie_files(cookies_path: &Path, destination_path: &Path) -> Result<(), String> {
    let sidecar_path = |path: &Path, suffix: &str| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    // Always copy: a running Chromium process can create WAL/journal files after
    // an existence check. SQLite must never recover or create SHM in the source.
    for suffix in ["", "-journal", "-wal"] {
        let source = sidecar_path(cookies_path, suffix);
        if suffix.is_empty() || source.exists() {
            let size = match fs::metadata(&source) {
                Ok(metadata) => metadata.len(),
                Err(error)
                    if !suffix.is_empty() && error.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            };
            if size > 128 * 1024 * 1024 {
                return Err(crate::modules::i18n::translate(
                    &crate::modules::config::get_user_config().language,
                    "claude.errors.cookieSnapshotTooLarge",
                    &[],
                ));
            }
            let destination = sidecar_path(destination_path, suffix);
            copy_readonly_snapshot_file(&source, &destination)?;
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
    Ok(())
}

fn open_desktop_cookie_snapshot_once(cookies_path: &Path) -> Result<DesktopCookieSnapshot, String> {
    let directory =
        std::env::temp_dir().join(format!("cockpit-claude-cookies-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).map_err(|error| error.to_string())?;
    let mut snapshot = DesktopCookieSnapshot {
        connection: None,
        temporary_dir: Some(directory.clone()),
    };
    let read_path = directory.join("Cookies");
    copy_desktop_cookie_files(cookies_path, &read_path)?;
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

fn open_desktop_cookie_snapshot(cookies_path: &Path) -> Result<DesktopCookieSnapshot, String> {
    let mut last_error = None;
    for attempt in 0..3 {
        match open_desktop_cookie_snapshot_once(cookies_path) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 && cookies_path.exists() {
                    std::thread::sleep(Duration::from_millis(30));
                } else {
                    break;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "打开 Claude Cookies 快照失败".to_string()))
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
