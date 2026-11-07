// Persist parser state, not just a line count. A checkpoint and its imported
// events are committed together; restarting never needs to replay an append.
use std::io::{Read, Seek, SeekFrom};

const PARSER_VERSION: u32 = 1;
const MAX_LINE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParserCheckpoint {
    version: u32,
    byte_offset: u64,
    line_offset: i64,
    root_meta_seen: bool,
    root_timestamp: Option<DateTime<Utc>>,
    parent: ParentResolution,
    parent_id: Option<String>,
    deferred_reason: Option<String>,
    current_model: String,
    total_high_water: Option<CumulativeTokens>,
    signatures: Vec<(Option<String>, TokenUsageSignature)>,
    previous_token_signature: Option<TokenUsageSignature>,
    event_index: u32,
    has_billable_tokens: bool,
    replay_matched: usize,
    replay_closed: bool,
    anchor: String,
    complete: bool,
    #[serde(default)]
    discard_line: bool,
}
impl Default for ParserCheckpoint {
    fn default() -> Self {
        Self {
            version: PARSER_VERSION,
            byte_offset: 0,
            line_offset: 0,
            root_meta_seen: false,
            root_timestamp: None,
            parent: ParentResolution::None,
            parent_id: None,
            deferred_reason: None,
            current_model: "unknown".into(),
            total_high_water: None,
            signatures: Vec::new(),
            previous_token_signature: None,
            event_index: 0,
            has_billable_tokens: false,
            replay_matched: 0,
            replay_closed: false,
            anchor: String::new(),
            complete: false,
            discard_line: false,
        }
    }
}

fn checkpoint_anchor(path: &Path, offset: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 1024];
    let head = offset.min(bytes.len() as u64) as usize;
    file.read_exact(&mut bytes[..head])
        .map_err(|e| e.to_string())?;
    hash.update(&bytes[..head]);
    let tail = offset.min(bytes.len() as u64) as usize;
    file.seek(SeekFrom::Start(offset - tail as u64))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut bytes[..tail])
        .map_err(|e| e.to_string())?;
    hash.update(&bytes[..tail]);
    Ok(format!("{:x}", hash.finalize()))
}

fn load_parser_checkpoint(
    conn: &Connection,
    path: &Path,
    size: u64,
    previous_size: i64,
) -> Result<Option<ParserCheckpoint>, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT state FROM session_parser_checkpoints WHERE file_path=?1",
            [path.to_string_lossy().as_ref()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let checkpoint = value.and_then(|v| serde_json::from_str::<ParserCheckpoint>(&v).ok());
    Ok(checkpoint.filter(|c| {
        c.version == PARSER_VERSION
            && c.byte_offset <= size
            && size >= previous_size.max(0) as u64
            && checkpoint_anchor(path, c.byte_offset).is_ok_and(|anchor| anchor == c.anchor)
    }))
}

fn save_parser_checkpoint(
    conn: &Connection,
    path: &Path,
    state: &ParserCheckpoint,
) -> Result<(), String> {
    let json = serde_json::to_string(state).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO session_parser_checkpoints(file_path,state) VALUES(?1,?2) ON CONFLICT(file_path) DO UPDATE SET state=excluded.state",
        params![path.to_string_lossy().as_ref(),json]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Bounded line memory even for giant response messages. Oversized token/meta
/// lines fail explicitly rather than committing an incorrect usage checkpoint.
fn read_usage_line(
    reader: &mut BufReader<File>,
    discard_line: &mut bool,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    if *discard_line {
        let count = reader
            .by_ref()
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(None);
        }
        *discard_line = bytes.last() != Some(&b'\n');
        return Ok(Some(String::new()));
    }
    let count = reader
        .by_ref()
        .take(MAX_LINE_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_LINE_BYTES {
        let head = String::from_utf8_lossy(&bytes);
        if head.contains("\"session_meta\"")
            || head.contains("\"turn_context\"")
            || (head.contains("\"event_msg\"") && head.contains("\"token_count\""))
        {
            return Err("usage record exceeds parser byte budget".into());
        }
        *discard_line = bytes.last() != Some(&b'\n');
        return Ok(Some(String::new()));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}
