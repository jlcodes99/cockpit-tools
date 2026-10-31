// Parent replay evidence is indexed on disk in the same bounded parser batches.
// A large parent no longer produces a whole-file Vec or an unbounded RAM cache.
#[derive(Default, Serialize, Deserialize)]
struct ParentScanState {
    generation: i64,
    modified: i64,
    size: u64,
    offset: u64,
    line: i64,
    complete: bool,
    missing_timestamp: bool,
    anchor: String,
    #[serde(default)]
    discard_line: bool,
}

fn prepare_parent_index(conn: &Connection, path: &Path) -> Result<Option<ParentScanState>, String> {
    let path_text = path.to_string_lossy();
    let raw: Option<String> = conn
        .query_row(
            "SELECT state FROM session_parent_sync WHERE file_path=?1",
            [path_text.as_ref()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut state = raw
        .and_then(|s| serde_json::from_str::<ParentScanState>(&s).ok())
        .unwrap_or_default();
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = metadata_modified_nanos(&meta);
    let size = meta.len();
    if state.complete && state.modified == modified && state.size == size {
        return Ok(Some(state));
    }
    if state.offset > 0
        && (size < state.size
            || (size == state.size && modified != state.modified)
            || !checkpoint_anchor(path, state.offset).is_ok_and(|v| v == state.anchor))
    {
        state = ParentScanState {
            generation: state.generation + 1,
            ..Default::default()
        };
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(state.offset))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let started = std::time::Instant::now();
    let initial = state.offset;
    let mut events = Vec::new();
    loop {
        let position = reader.stream_position().map_err(|e| e.to_string())?;
        if position > initial
            && (position - initial >= crate::modules::background_maintenance::BATCH_BYTES
                || events.len() >= INSERT_BATCH_SIZE
                || started.elapsed() >= crate::modules::background_maintenance::BATCH_TIME)
        {
            break;
        }
        let Some(line) = read_usage_line(&mut reader, &mut state.discard_line)? else {
            break;
        };
        if !line.ends_with('\n') && !line.is_empty() {
            reader
                .seek(SeekFrom::Start(position))
                .map_err(|e| e.to_string())?;
            break;
        }
        state.line += 1;
        if !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        if value.get("type").and_then(JsonValue::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(JsonValue::as_str) != Some("token_count") {
            continue;
        }
        let Some(signature) = payload.get("info").and_then(parse_token_signature) else {
            continue;
        };
        let Some(timestamp) = parse_timestamp(value.get("timestamp")) else {
            state.missing_timestamp = true;
            continue;
        };
        events.push((
            state.line,
            timestamp.timestamp_millis(),
            serde_json::to_string(&signature).map_err(|e| e.to_string())?,
        ));
    }
    state.offset = reader.stream_position().map_err(|e| e.to_string())?;
    state.complete = state.offset
        >= reader
            .get_ref()
            .metadata()
            .map_err(|e| e.to_string())?
            .len();
    state.size = size;
    state.modified = modified;
    state.anchor = checkpoint_anchor(path, state.offset)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (line, timestamp, signature) in events {
        tx.execute("INSERT OR REPLACE INTO session_parent_signatures(file_path,generation,line,timestamp,signature) VALUES(?1,?2,?3,?4,?5)",
            params![path_text.as_ref(),state.generation,line,timestamp,signature]).map_err(|e|e.to_string())?;
    }
    tx.execute("INSERT INTO session_parent_sync(file_path,state) VALUES(?1,?2) ON CONFLICT(file_path) DO UPDATE SET state=excluded.state",
        params![path_text.as_ref(),serde_json::to_string(&state).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    // Rewritten generations are reclaimed gradually, never by one huge DELETE.
    tx.execute("DELETE FROM session_parent_signatures WHERE rowid IN (SELECT rowid FROM session_parent_signatures WHERE file_path=?1 AND generation!=?2 LIMIT 32)",params![path_text.as_ref(),state.generation]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    if !state.complete && state.offset == initial {
        return Err("父 rollout 尾部尚未写完".into());
    }
    Ok(state.complete.then_some(state))
}

enum ParentReplay {
    Pending,
    Matched(usize),
}
fn resolve_parent_replay(
    conn: &Connection,
    parent_id: &str,
    cutoff: DateTime<Utc>,
    index: &RolloutIndex,
    child: &[ParsedTokenEvent],
    checkpoint: &mut ParserCheckpoint,
) -> Result<ParentReplay, String> {
    let paths = index
        .get(parent_id)
        .ok_or_else(|| format!("找不到父 rollout: {parent_id}"))?;
    let mut snapshots = Vec::new();
    for path in paths {
        let Some(state) = prepare_parent_index(conn, path)? else {
            return Ok(ParentReplay::Pending);
        };
        if state.missing_timestamp {
            return Err("父 rollout 的 token_count 缺少有效 timestamp".into());
        }
        let count:i64=conn.query_row("SELECT COUNT(*) FROM session_parent_signatures WHERE file_path=?1 AND generation=?2",params![path.to_string_lossy().as_ref(),state.generation],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count == 0 {
            return Err("父 rollout 尚无可用 token_count".into());
        }
        snapshots.push((path, state));
    }
    let Some((first, first_state)) = snapshots.first() else {
        return Err("找不到父 rollout".into());
    };
    // Duplicate UUID files are unusual. Compare streams without retaining their
    // histories; only the selected parent is needed for normal indexed matches.
    for (other, state) in snapshots.iter().skip(1) {
        let mut a=conn.prepare("SELECT signature FROM session_parent_signatures WHERE file_path=?1 AND generation=?2 AND timestamp<=?3 ORDER BY line").map_err(|e|e.to_string())?;
        let mut b=conn.prepare("SELECT signature FROM session_parent_signatures WHERE file_path=?1 AND generation=?2 AND timestamp<=?3 ORDER BY line").map_err(|e|e.to_string())?;
        let mut ar = a
            .query(params![
                first.to_string_lossy().as_ref(),
                first_state.generation,
                cutoff.timestamp_millis()
            ])
            .map_err(|e| e.to_string())?;
        let mut br = b
            .query(params![
                other.to_string_lossy().as_ref(),
                state.generation,
                cutoff.timestamp_millis()
            ])
            .map_err(|e| e.to_string())?;
        let mut compared = 0;
        loop {
            let av = ar
                .next()
                .map_err(|e| e.to_string())?
                .map(|r| r.get::<_, String>(0))
                .transpose()
                .map_err(|e| e.to_string())?;
            let bv = br
                .next()
                .map_err(|e| e.to_string())?
                .map(|r| r.get::<_, String>(0))
                .transpose()
                .map_err(|e| e.to_string())?;
            if av != bv {
                return Err("父 rollout UUID 对应多个内容不一致的文件".into());
            }
            if av.is_none() {
                break;
            }
            compared += 1;
            if compared % 500 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    let mut matched = 0;
    for event in child {
        let signature = serde_json::to_string(&event.signature).map_err(|e| e.to_string())?;
        let line:Option<i64>=conn.query_row("SELECT line FROM session_parent_signatures WHERE file_path=?1 AND generation=?2 AND signature=?3 AND line>?4 AND timestamp<=?5 ORDER BY line LIMIT 1",
            params![first.to_string_lossy().as_ref(),first_state.generation,signature,checkpoint.replay_matched as i64,cutoff.timestamp_millis()],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(line) = line else { break };
        checkpoint.replay_matched = line as usize;
        matched += 1;
    }
    Ok(ParentReplay::Matched(matched))
}
