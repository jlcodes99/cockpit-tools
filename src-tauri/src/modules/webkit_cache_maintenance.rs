use rusqlite::Connection;
use std::path::PathBuf;

fn webkit_data_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        Some(home.join("Library/WebKit/com.jlcodes.cockpit-tools/WebsiteData"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn find_localstorage_dbs(root: &std::path::Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return results;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let candidate = path.join("LocalStorage").join("localstorage.sqlite3");
            if candidate.exists() {
                results.push(candidate);
            }
            results.extend(find_localstorage_dbs(&path));
        }
    }
    results
}

/// Checkpoint WAL on all WebKit LocalStorage SQLite databases.
///
/// WebKit WKWebView uses WAL mode for LocalStorage but does not
/// periodically checkpoint. When the app writes large blobs frequently
/// (e.g. account caches with quota snapshots), the WAL file grows
/// unbounded — in production it has been observed at 13 GB for only
/// ~5 MB of actual data.
///
/// Running this at startup keeps the WAL from accumulating over time.
pub fn checkpoint_webkit_localstorage() {
    let Some(root) = webkit_data_root() else {
        return;
    };
    if !root.exists() {
        return;
    }

    let dbs = find_localstorage_dbs(&root);
    if dbs.is_empty() {
        return;
    }

    for db_path in dbs {
        let label = db_path.display();
        match Connection::open(&db_path) {
            Ok(conn) => {
                match conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
                    Ok(()) => {
                        crate::modules::logger::log_info(&format!(
                            "[WebkitCache] WAL checkpoint 成功: {}",
                            label
                        ));
                    }
                    Err(e) => {
                        crate::modules::logger::log_warn(&format!(
                            "[WebkitCache] WAL checkpoint 失败 (可能 WebView 正在占用): {} — {}",
                            label, e
                        ));
                    }
                }
                drop(conn);
            }
            Err(e) => {
                crate::modules::logger::log_warn(&format!(
                    "[WebkitCache] 无法打开数据库: {} — {}",
                    label, e
                ));
            }
        }
    }
}
