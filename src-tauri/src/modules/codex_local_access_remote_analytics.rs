// Incremental, private per-key hourly aggregates. Schema work never scans history;
// the resumable worker shares the host's row/byte/time budgets and yields to writers.
const REMOTE_HOUR: i64 = 3_600_000;
const REMOTE_ANALYTICS_VERSION: i64 = 2;
const REMOTE_KEY_BYTES: usize = 256;
// Only the bounded key and fixed numeric cells enter aggregation. Bodies,
// errors, emails and arbitrary JSON never enter this projection.
const REMOTE_PROJECTION_ROW_BYTES: usize = REMOTE_KEY_BYTES + 16 * 8;
static REMOTE_ANALYTICS_REVISION: AtomicU64 = AtomicU64::new(0);

fn remote_aggregate_columns(prefix: &str) -> Vec<(&'static str, String)> {
    let p = |column: &str| format!("{prefix}.{column}");
    vec![
        ("requests", "1".into()),
        ("successes", p("success")),
        (
            "canceled",
            format!("({}='client_canceled')", p("error_category")),
        ),
        (
            "images",
            format!(
                "({} IN ('image_generation','image_edit'))",
                p("request_kind")
            ),
        ),
        ("latency", p("latency_ms")),
        ("input_tokens", p("input_tokens")),
        ("output_tokens", p("output_tokens")),
        ("total_tokens", p("total_tokens")),
        ("cached_tokens", p("cached_tokens")),
        ("estimated_usd", p("estimated_cost_usd")),
        (
            "settled_microusd",
            format!("COALESCE({},0)", p("settled_microusd")),
        ),
        (
            "unrecorded",
            format!("({}='unrecorded')", p("settlement_status")),
        ),
        (
            "unmetered",
            format!("({}='unmetered')", p("settlement_status")),
        ),
    ]
}

fn remote_rollup_delta(prefix: &str, subtract: bool) -> String {
    let fields = remote_aggregate_columns(prefix);
    let names = fields
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(",");
    let values = fields
        .iter()
        .map(|(_, expression)| {
            if subtract {
                format!("-({expression})")
            } else {
                expression.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let updates = fields
        .iter()
        .map(|(name, _)| format!("{name}={name}+excluded.{name}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "INSERT INTO cockpit_key_hourly_v2(generation,key_id,bucket,{names})
        VALUES ((SELECT generation FROM cockpit_key_analytics_state WHERE id=1),
        {prefix}.api_key_id,({prefix}.timestamp/{REMOTE_HOUR})*{REMOTE_HOUR},{values})
        ON CONFLICT(generation,key_id,bucket) DO UPDATE SET {updates};"
    )
}

fn ensure_remote_analytics_schema(conn: &Connection) -> Result<(), SqliteError> {
    // This marker is created last in the same transaction as the entire schema.
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
        WHERE type='table' AND name='cockpit_key_analytics_state')",
        [],
        |row| row.get(0),
    )?;
    if exists && remote_analytics_schema_current(conn)? {
        return Ok(());
    }
    // This may be called from the first live writer. Schema creation has no
    // history scan; backfill and retirement always stay in scheduled batches.
    let tx = conn.unchecked_transaction()?;
    for (name, declaration) in [
        ("settled_microusd", "settled_microusd INTEGER"),
        (
            "settlement_status",
            "settlement_status TEXT NOT NULL DEFAULT 'unrecorded'",
        ),
        ("settled_at", "settled_at INTEGER"),
    ] {
        ensure_request_logs_column(&tx, name, declaration)?;
    }
    let columns = remote_aggregate_columns("NEW")
        .iter()
        .map(|(name, _)| {
            format!(
                "{name} {} NOT NULL DEFAULT 0",
                if *name == "estimated_usd" {
                    "REAL"
                } else {
                    "INTEGER"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    tx.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS cockpit_key_hourly_v2(
        generation INTEGER NOT NULL,key_id TEXT NOT NULL,bucket INTEGER NOT NULL,{columns},
        PRIMARY KEY(generation,key_id,bucket)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS cockpit_key_analytics_state(
            id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL,high_water INTEGER NOT NULL);
        INSERT OR IGNORE INTO cockpit_key_analytics_state(id,cursor,high_water) VALUES(1,0,0);"
    ))?;
    for (name, definition) in [
        ("version", "INTEGER NOT NULL DEFAULT 1"),
        ("generation", "INTEGER NOT NULL DEFAULT 0"),
        ("complete", "INTEGER NOT NULL DEFAULT 0"),
        ("oversized", "INTEGER NOT NULL DEFAULT 0"),
        ("previous_generation", "INTEGER"),
        ("previous_complete", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('cockpit_key_analytics_state') WHERE name=?1)", [name], |r|r.get(0))?;
        if !present {
            tx.execute_batch(&format!(
                "ALTER TABLE cockpit_key_analytics_state ADD COLUMN {name} {definition}"
            ))?;
        }
    }
    let previous: i64 = tx.query_row(
        "SELECT version FROM cockpit_key_analytics_state WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if previous > REMOTE_ANALYTICS_VERSION {
        return Err(SqliteError::InvalidQuery);
    }
    tx.execute("UPDATE cockpit_key_analytics_state SET previous_generation=generation,
        previous_complete=(cursor>=high_water),version=?1,generation=generation+1,cursor=0,
        high_water=(SELECT COALESCE(MAX(id),0) FROM request_logs),complete=NOT EXISTS(SELECT 1 FROM request_logs),oversized=0 WHERE id=1",
        [REMOTE_ANALYTICS_VERSION])?;
    let processed = |prefix: &str| {
        format!(
            "{prefix}.api_key_id<>'' AND length(CAST({prefix}.api_key_id AS BLOB))<={REMOTE_KEY_BYTES} AND
        ({prefix}.id>(SELECT high_water FROM cockpit_key_analytics_state WHERE id=1)
        OR {prefix}.id<=(SELECT cursor FROM cockpit_key_analytics_state WHERE id=1))"
        )
    };
    let add = remote_rollup_delta("NEW", false);
    let remove = remote_rollup_delta("OLD", true);
    let watched = "api_key_id,timestamp,success,error_category,request_kind,latency_ms,input_tokens,
        output_tokens,total_tokens,cached_tokens,estimated_cost_usd,settled_microusd,settlement_status";
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS cockpit_key_insert;
        DROP TRIGGER IF EXISTS cockpit_key_delete;DROP TRIGGER IF EXISTS cockpit_key_update_remove;
        DROP TRIGGER IF EXISTS cockpit_key_update_add;DROP TRIGGER IF EXISTS cockpit_key_empty;",
    )?;
    tx.execute_batch(&format!("
        CREATE TRIGGER cockpit_key_insert AFTER INSERT ON request_logs WHEN {} BEGIN {add} END;
        CREATE TRIGGER cockpit_key_delete BEFORE DELETE ON request_logs WHEN {} BEGIN {remove} END;
        CREATE TRIGGER cockpit_key_update_remove BEFORE UPDATE OF {watched} ON request_logs WHEN {} BEGIN {remove} END;
        CREATE TRIGGER cockpit_key_update_add AFTER UPDATE OF {watched} ON request_logs WHEN {} BEGIN {add} END;",
        processed("NEW"),processed("OLD"),processed("OLD"),processed("NEW")))?;
    // A full clear can reuse row IDs. Move to a new generation atomically;
    // stale checkpoints can never omit or subtract newly inserted rows.
    tx.execute_batch("CREATE TRIGGER cockpit_key_empty AFTER DELETE ON request_logs
        WHEN NOT EXISTS(SELECT 1 FROM request_logs) BEGIN
            UPDATE cockpit_key_analytics_state SET generation=generation+1,cursor=0,high_water=0,complete=1,oversized=0,previous_complete=0 WHERE id=1;
        END;")?;
    tx.commit()?;
    REMOTE_ANALYTICS_REVISION.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

fn remote_analytics_backfill_batch(conn: &Connection) -> Result<bool, String> {
    with_request_log_sql_budget(
        conn,
        crate::modules::background_maintenance::BATCH_TIME,
        || {
            ensure_remote_analytics_schema(conn).map_err(|e| e.to_string())?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let (cursor, high, complete): (i64, i64, bool) = tx
                .query_row(
                    "SELECT cursor,high_water,complete FROM cockpit_key_analytics_state WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|e| e.to_string())?;
            if complete {
                return Ok(true);
            }
            let rows = crate::modules::background_maintenance::BATCH_ROWS
                .min(
                    crate::modules::background_maintenance::BATCH_BYTES as usize
                        / REMOTE_PROJECTION_ROW_BYTES,
                )
                .max(1);
            let next: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(id),?2) FROM
            (SELECT id FROM request_logs WHERE id>?1 AND id<=?2 ORDER BY id LIMIT ?3)",
                    params![cursor, high, rows as i64],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let fields = remote_aggregate_columns("r");
            let names = fields
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(",");
            let sums = fields
                .iter()
                .map(|(_, expr)| format!("SUM({expr})"))
                .collect::<Vec<_>>()
                .join(",");
            let updates = fields
                .iter()
                .map(|(name, _)| format!("{name}={name}+excluded.{name}"))
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(&format!("INSERT INTO cockpit_key_hourly_v2(generation,key_id,bucket,{names})
            SELECT (SELECT generation FROM cockpit_key_analytics_state WHERE id=1),r.api_key_id,
            (r.timestamp/{REMOTE_HOUR})*{REMOTE_HOUR},{sums} FROM request_logs r
            WHERE r.id>?1 AND r.id<=?2 AND r.api_key_id<>'' AND length(CAST(r.api_key_id AS BLOB))<={REMOTE_KEY_BYTES}
            GROUP BY r.api_key_id,r.timestamp/{REMOTE_HOUR}
            ON CONFLICT(generation,key_id,bucket) DO UPDATE SET {updates}"), params![cursor,next]).map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE cockpit_key_analytics_state SET cursor=?2,complete=(?2>=high_water),
                oversized=oversized+(SELECT COUNT(*) FROM request_logs WHERE id>?1 AND id<=?2
                AND length(CAST(api_key_id AS BLOB))>?3) WHERE id=1",
                params![cursor, next, REMOTE_KEY_BYTES as i64],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            REMOTE_ANALYTICS_REVISION.fetch_add(1, Ordering::AcqRel);
            Ok(next >= high)
        },
    )
}

fn remote_analytics_schema_current(conn: &Connection) -> Result<bool, SqliteError> {
    let versioned:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('cockpit_key_analytics_state') WHERE name='version')",[],|r|r.get(0))?;
    if !versioned {
        return Ok(false);
    }
    let version: i64 = conn.query_row(
        "SELECT version FROM cockpit_key_analytics_state WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if version > REMOTE_ANALYTICS_VERSION {
        return Err(SqliteError::InvalidQuery);
    }
    Ok(version == REMOTE_ANALYTICS_VERSION)
}

fn remote_analytics_cleanup_batch(conn: &Connection) -> Result<bool, String> {
    with_request_log_sql_budget(
        conn,
        crate::modules::background_maintenance::BATCH_TIME,
        || {
            let rows = crate::modules::background_maintenance::BATCH_ROWS as i64;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let changed = tx
                .execute(
                    "DELETE FROM cockpit_key_hourly_v2 WHERE (generation,key_id,bucket) IN
            (SELECT generation,key_id,bucket FROM cockpit_key_hourly_v2 WHERE generation<
            (SELECT generation FROM cockpit_key_analytics_state WHERE id=1) LIMIT ?1)",
                    [rows],
                )
                .map_err(|e| e.to_string())?;
            // The previous unversioned table remains intact until the new projection
            // is complete. Retire it in small batches rather than DROP a large tree.
            let legacy: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cockpit_key_hourly')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut old = 0;
            if legacy {
                old=tx.execute("DELETE FROM cockpit_key_hourly WHERE (key_id,bucket) IN (SELECT key_id,bucket FROM cockpit_key_hourly LIMIT ?1)",[rows]).map_err(|e|e.to_string())?;
                if old == 0 {
                    tx.execute_batch("DROP TABLE cockpit_key_hourly")
                        .map_err(|e| e.to_string())?;
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(changed == 0 && old == 0)
        },
    )
}

fn ensure_remote_analytics_started() {
    let Ok(path) = local_access_logs_db_path() else {
        return;
    };
    schedule_request_summary_session_repair(path.clone());
    crate::modules::background_maintenance::schedule(
        format!("cockpit-key-analytics:{}", path.display()),
        move || {
            let Ok(_guard) = local_access_logs_db_write_lock().try_lock() else {
                return Ok(false);
            };
            let conn = if !path.is_file() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                open_local_access_logs_db_once(&path, true).map_err(|e| e.to_string())?
            } else {
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|e| e.to_string())?
            };
            conn.busy_timeout(Duration::from_millis(10))
                .map_err(|e| e.to_string())?;
            if !remote_analytics_backfill_batch(&conn)? {
                return Ok(false);
            }
            let oversized: i64 = conn
                .query_row(
                    "SELECT oversized FROM cockpit_key_analytics_state WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let complete = remote_analytics_cleanup_batch(&conn)?;
            if complete && oversized > 0 {
                logger::log_codex_api_warn(&format!("Key analytics preserved {oversized} historical rows with oversized key IDs outside aggregates"));
            }
            Ok(complete)
        },
    );
}

fn schedule_request_summary_session_repair(path: PathBuf) {
    crate::modules::background_maintenance::schedule(format!("request-summary-session:{}",path.display()),move|| {
        if !path.is_file() {return Ok(true);}
        let Ok(_guard)=local_access_logs_db_write_lock().try_lock() else{return Ok(false);};
        let conn=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|e|e.to_string())?;
        conn.busy_timeout(Duration::from_millis(10)).map_err(|e|e.to_string())?;
        let before=conn.total_changes();
        let result=with_request_log_sql_budget(&conn,crate::modules::background_maintenance::BATCH_TIME,||
            crate::modules::codex_request_summary::repair_session_scope_batch(&conn));
        if result.is_ok() && conn.total_changes()!=before {REMOTE_ANALYTICS_REVISION.fetch_add(1,Ordering::AcqRel);}
        result
    });
}

#[cfg(test)]
#[path = "codex_local_access_remote_analytics_tests.rs"]
mod remote_analytics_tests;
