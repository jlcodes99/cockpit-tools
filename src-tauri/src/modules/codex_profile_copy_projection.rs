// Only recover byte cursors when the copied durable records still cover the saved ordinal.
// An ahead-of-file SQLite snapshot with missing records must continue to fail publication.
fn validate_projection_cursors(
    root: &Path,
    target: &Path,
    budget: &CopyBudget<'_>,
) -> Result<(), String> {
    budget.check()?;
    let state_path = root.join(STATE_DB);
    let history_path = root.join("thread_history_1.sqlite");
    if !state_path.exists() || !history_path.exists() {
        return Ok(());
    }
    let state = Connection::open_with_flags(state_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut history = Connection::open(&history_path).map_err(|e| e.to_string())?;
    history
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    if !has_column(&state, "threads", "rollout_path").map_err(|e| e.to_string())?
        || !has_column(
            &history,
            "thread_history_projection_state",
            "next_rollout_byte_offset",
        )
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    let has_ordinal = has_column(
        &history,
        "thread_history_projection_state",
        "next_rollout_ordinal",
    )
    .map_err(|e| e.to_string())?;
    let rows = {
        let query = if has_ordinal {
            "SELECT thread_id, next_rollout_byte_offset, next_rollout_ordinal FROM thread_history_projection_state"
        } else {
            "SELECT thread_id, next_rollout_byte_offset, NULL FROM thread_history_projection_state"
        };
        let mut stmt = history.prepare(query).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    let mut stale_threads = Vec::new();
    for (id, offset, next_ordinal) in rows {
        budget.check()?;
        let path: Option<String> = state
            .query_row("SELECT rollout_path FROM threads WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        // Historical cache rows for deleted threads do not prove missing live history.
        let Some(path) = path else {
            continue;
        };
        let relative = relative_profile_path(Path::new(&path), target)
            .ok_or_else(|| format!("会话 {id} 的历史路径不在副本中"))?;
        let rollout_path = root.join(relative);
        let mut file = fs::File::open(&rollout_path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if offset < 0 {
            return Err(format!("会话 {id} 的历史索引超出副本文件边界"));
        }
        let mut cursor_valid = offset as u64 <= length;
        if cursor_valid && offset > 0 {
            file.seek(SeekFrom::Start(offset as u64 - 1))
                .map_err(|e| e.to_string())?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(|e| e.to_string())?;
            cursor_valid = byte[0] == b'\n';
        }
        if cursor_valid {
            continue;
        }
        let complete_history = match next_ordinal.filter(|ordinal| *ordinal >= 0) {
            Some(next_ordinal) => {
                rollout_covers_projection_ordinal(&rollout_path, next_ordinal as u64, budget)?
            }
            None => false,
        };
        if !complete_history {
            return Err(if offset as u64 > length {
                format!("会话 {id} 的历史索引超出副本文件边界")
            } else {
                format!("会话 {id} 的历史索引不在记录末尾")
            });
        }
        stale_threads.push(id);
    }
    if !stale_threads.is_empty() {
        let tx = history.transaction().map_err(|e| e.to_string())?;
        for id in stale_threads {
            budget.check()?;
            // Match official codex-thread-store's delete projection operation. These four
            // tables are derived from the durable rollout and rebuild on the next resume.
            // https://github.com/openai/codex/blob/main/codex-rs/thread-store/src/local/thread_history.rs
            for table in [
                "thread_items",
                "thread_realtime_items",
                "thread_turns",
                "thread_history_projection_state",
            ] {
                let present: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                        [table],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !present {
                    continue;
                }
                let required: &[&str] = match table {
                    "thread_items" => &[
                        "thread_id",
                        "turn_id",
                        "item_id",
                        "rollout_ordinal",
                        "item_json",
                    ],
                    "thread_realtime_items" => {
                        &["thread_id", "item_id", "rollout_ordinal", "item_json"]
                    }
                    "thread_turns" => &["thread_id", "turn_id", "rollout_ordinal", "status"],
                    _ => &[
                        "thread_id",
                        "next_rollout_byte_offset",
                        "next_rollout_ordinal",
                    ],
                };
                for column in required {
                    if !has_column(&tx, table, column).map_err(|e| e.to_string())? {
                        // Unknown cache layouts cannot be safely invalidated. Reject
                        // publication instead of leaving a partially reset projection.
                        return Err(format!("会话 {id} 的历史索引超出副本文件边界"));
                    }
                }
                tx.execute(&format!("DELETE FROM {table} WHERE thread_id = ?1"), [&id])
                    .map_err(|e| e.to_string())?;
            }
        }
        budget.check()?;
        tx.commit().map_err(|e| e.to_string())?;
        history
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn rollout_covers_projection_ordinal(
    path: &Path,
    checkpoint: u64,
    budget: &CopyBudget<'_>,
) -> Result<bool, String> {
    let mut reader = BufReader::new(fs::File::open(path).map_err(|e| e.to_string())?);
    let mut first_record = true;
    let mut expected = 0_u64;
    let mut has_ordinal = false;
    let mut line = Vec::new();
    loop {
        budget.check()?;
        line.clear();
        // Bound recovery work per record; ordinary healthy copies do not take this path.
        const MAX_RECOVERY_RECORD_BYTES: u64 = 64 * 1024 * 1024;
        let bytes = std::io::Read::by_ref(&mut reader)
            .take(MAX_RECOVERY_RECORD_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if bytes == 0 {
            break;
        }
        if bytes as u64 > MAX_RECOVERY_RECORD_BYTES || line.last() != Some(&b'\n') {
            return Ok(false);
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            return Ok(false);
        };
        if first_record {
            if record["type"].as_str() != Some("session_meta") {
                return Ok(false);
            }
            expected = record["payload"]["history_base"]["end_ordinal_exclusive"]
                .as_u64()
                .unwrap_or(0);
            first_record = false;
            if record.get("ordinal").is_none() {
                continue;
            }
        }
        let Some(ordinal) = record["ordinal"].as_u64() else {
            return Ok(false);
        };
        if ordinal != expected {
            return Ok(false);
        }
        let Some(next) = ordinal.checked_add(1) else {
            return Ok(false);
        };
        expected = next;
        has_ordinal = true;
    }
    // Unlike clamping the byte offset, this proves no projected records disappeared.
    Ok(has_ordinal && expected >= checkpoint)
}
