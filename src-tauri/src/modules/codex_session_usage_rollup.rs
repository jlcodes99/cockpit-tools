const SESSION_USAGE_ROLLUP: crate::modules::usage_rollup::Rollup =
    crate::modules::usage_rollup::Rollup {
        source: "session_usage_events",
        prefix: "session_usage_v1",
        width: 60,
        dimensions: &["instance_id", "instance_name", "model", "file_path"],
        metrics: &[
            crate::modules::usage_rollup::Metric {
                name: "input_tokens",
                expression: "MAX($input_tokens,0)",
                real: false,
            },
            crate::modules::usage_rollup::Metric {
                name: "cached_input_tokens",
                expression: "MAX($cached_input_tokens,0)",
                real: false,
            },
            crate::modules::usage_rollup::Metric {
                name: "output_tokens",
                expression: "MAX($output_tokens,0)",
                real: false,
            },
        ],
    };

fn schedule_session_rollup_backfill(path: PathBuf) {
    crate::modules::background_maintenance::schedule(
        format!("session-rollup:{}", path.display()),
        move || {
            // Session import owns this lane while writing a bounded parse batch.
            let Ok(_guard) = sync_lock().try_lock() else {
                return Ok(false);
            };
            let conn =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|e| e.to_string())?;
            conn.busy_timeout(std::time::Duration::from_millis(10))
                .map_err(|e| e.to_string())?;
            if !cleanup_rewrite_batch(&conn)? {
                return Ok(false);
            }
            SESSION_USAGE_ROLLUP.batch(&conn).map_err(|e| e.to_string())
        },
    );
}
