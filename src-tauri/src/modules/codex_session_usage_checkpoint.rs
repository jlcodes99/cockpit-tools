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
    #[serde(default)]
    compressed: bool,
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
            compressed: false,
        }
    }
}

fn checkpoint_anchor(path: &Path, offset: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let offset = if is_compressed_rollout(path) {
        // Decoder offsets are in the uncompressed stream. Fingerprint the
        // compressed file itself to validate an interrupted scan on restart.
        file.metadata().map_err(|e| e.to_string())?.len()
    } else {
        offset
    };
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
    let key = logical_rollout_path(path);
    let compressed = is_compressed_rollout(path);
    let value: Option<String> = conn
        .query_row(
            "SELECT state FROM session_parser_checkpoints WHERE file_path=?1",
            [key.to_string_lossy().as_ref()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let checkpoint = value.and_then(|v| serde_json::from_str::<ParserCheckpoint>(&v).ok());
    Ok(checkpoint.filter(|c| {
        let size_matches = if compressed {
            size == previous_size.max(0) as u64
        } else {
            c.byte_offset <= size && size >= previous_size.max(0) as u64
        };
        c.version == PARSER_VERSION
            && c.compressed == compressed
            && size_matches
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

#[derive(Default)]
struct UsageReaderCache {
    reader: Option<UsageReader>,
}

struct UsageReader {
    input: BufReader<Box<dyn Read>>,
    path: PathBuf,
    offset: u64,
}

impl UsageReaderCache {
    fn at(&mut self, path: &Path, offset: u64) -> Result<&mut UsageReader, String> {
        if self.reader.as_ref().is_none_or(|reader| {
            reader.path != path || reader.offset != offset || !is_compressed_rollout(path)
        }) {
            self.reader = Some(UsageReader::open(path, offset)?);
        }
        Ok(self.reader.as_mut().expect("usage reader was opened"))
    }
}

impl UsageReader {
    fn open(path: &Path, offset: u64) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|error| format!("无法打开文件: {error}"))?;
        let stream: Box<dyn Read> = if is_compressed_rollout(path) {
            let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?;
            decoder.window_log_max(27).map_err(|e| e.to_string())?;
            // Skip decoded bytes only when resuming with a fresh decoder. The
            // normal sequence of batches keeps this decoder at its last offset.
            let skipped = std::io::copy(&mut decoder.by_ref().take(offset), &mut std::io::sink())
                .map_err(|e| e.to_string())?;
            if skipped != offset {
                return Err("压缩 rollout 在用量检查点之前结束".into());
            }
            Box::new(decoder)
        } else {
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            Box::new(file)
        };
        Ok(Self {
            input: BufReader::new(stream),
            path: path.to_path_buf(),
            offset,
        })
    }

    fn at_eof(&mut self) -> Result<bool, String> {
        self.input
            .fill_buf()
            .map(|bytes| bytes.is_empty())
            .map_err(|e| e.to_string())
    }
}

/// Bounded line memory even for giant response messages. Oversized token/meta
/// lines fail explicitly rather than committing an incorrect usage checkpoint.
fn read_usage_line(
    reader: &mut UsageReader,
    discard_line: &mut bool,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    if *discard_line {
        let count = reader
            .input
            .by_ref()
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(None);
        }
        reader.offset += count as u64;
        *discard_line = bytes.last() != Some(&b'\n');
        return Ok(Some(String::new()));
    }
    let count = reader
        .input
        .by_ref()
        .take(MAX_LINE_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    reader.offset += count as u64;
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
