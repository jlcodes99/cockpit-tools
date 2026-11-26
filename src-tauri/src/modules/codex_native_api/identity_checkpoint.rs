//! Self-contained, strictly bounded hot-cache checkpoints.
//!
//! Normal turns (`<= MAX_CHECKPOINT_BYTES`) are stored as plain JSON text for
//! zero-overhead reads; oversized repetitive turns are transparently compressed
//! into a single self-contained `zstd:<base64>` cell so no external chunk table
//! (`native_rebuild_items_v1`) or manifest archive (`native_rebuild_history_v1`)
//! is ever needed.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

const ZSTD_PREFIX: &str = "zstd:";
const PARTIAL_COMPRESS_THRESHOLD_BYTES: usize = 4 * 1024;
const PARAMETER_COMPRESS_THRESHOLD_BYTES: usize = 64 * 1024;

pub(super) struct Metadata {
    pub thread: String,
    pub account: String,
    pub parameters: Option<String>,
}

struct HotPayload {
    transcript: String,
    parameters: Option<String>,
    bytes: usize,
}

pub(in crate::modules::codex_native_api::identity) struct Encoded {
    raw_transcript: String,
    raw_parameters: Option<String>,
    raw_bytes: usize,
    hot: Option<HotPayload>,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    super::super::maintenance::ensure_created_column(connection, "native_rebuild_checkpoints_v1")?;
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS native_rebuild_checkpoints_v1_evict
            ON native_rebuild_checkpoints_v1(created ASC, response ASC);
         CREATE INDEX IF NOT EXISTS native_rebuild_checkpoints_v1_thread
            ON native_rebuild_checkpoints_v1(caller, thread, created DESC, response DESC);",
    )
}

pub(crate) fn encode_zstd_text(raw: &str) -> Result<String, ServiceError> {
    let compressed = zstd::stream::encode_all(raw.as_bytes(), 1).map_err(storage)?;
    Ok(format!("{ZSTD_PREFIX}{}", STANDARD.encode(compressed)))
}

pub(crate) fn decode_text(raw: &str) -> Result<String, ServiceError> {
    if let Some(encoded) = raw.strip_prefix(ZSTD_PREFIX) {
        let compressed = STANDARD.decode(encoded).map_err(storage)?;
        let bytes = zstd::stream::decode_all(compressed.as_slice()).map_err(storage)?;
        String::from_utf8(bytes).map_err(storage)
    } else {
        Ok(raw.to_owned())
    }
}

pub(crate) fn decode_transcript(raw: &str) -> Result<Vec<Value>, ServiceError> {
    let decoded = decode_text(raw)?;
    serde_json::from_str(&decoded).map_err(storage)
}

pub(super) fn metadata(
    connection: &Connection,
    caller: &str,
    response: &str,
) -> Result<Option<Metadata>, ServiceError> {
    let row: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT thread,account,parameters
             FROM native_rebuild_checkpoints_v1
             WHERE caller=?1 AND response=?2",
            params![caller, response],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((thread, account, parameters)) = row else {
        return Ok(None);
    };
    let parameters = parameters
        .as_deref()
        .map(decode_text)
        .transpose()?;
    Ok(Some(Metadata {
        thread,
        account,
        parameters,
    }))
}

pub(super) fn transcript(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    response: &str,
) -> Result<Option<Vec<Value>>, ServiceError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT transcript
             FROM native_rebuild_checkpoints_v1
             WHERE caller=?1 AND response=?2 AND (?3 IS NULL OR thread=?3)",
            params![caller, response, thread],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    raw.as_deref().map(decode_transcript).transpose()
}

impl Encoded {
    /// Partial observations never enter the successful checkpoint table or
    /// activate a generation. Each row is self-contained and bounded inline.
    pub fn publish_partial(
        &self,
        transaction: &Transaction<'_>,
        binding: &Binding,
        response: &str,
    ) -> Result<(), ServiceError> {
        let transcript = if self.raw_transcript.len() > PARTIAL_COMPRESS_THRESHOLD_BYTES {
            encode_zstd_text(&self.raw_transcript)?
        } else {
            self.raw_transcript.clone()
        };
        let parameters = match &self.raw_parameters {
            Some(raw) if raw.len() > PARTIAL_COMPRESS_THRESHOLD_BYTES => {
                Some(encode_zstd_text(raw)?)
            }
            other => other.clone(),
        };
        transaction
            .execute(
                "INSERT OR IGNORE INTO native_partial_history_v1
                    (caller,response,thread,session,account,owner,transcript,parameters,bytes,created)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    binding.caller,
                    response,
                    binding.thread_id,
                    binding.session_id,
                    binding.account_id,
                    binding.owner_id,
                    transcript,
                    parameters,
                    self.raw_bytes as i64,
                    timestamp()
                ],
            )
            .map_err(storage)?;
        super::super::partial_history::evict_partial_cache(transaction)
    }

    /// Serialization and optional compression happen before acquiring the
    /// identity write lock/transaction.
    pub fn prepare(
        input: &[Value],
        output: &[Value],
        parameters: Option<&Value>,
    ) -> Result<Self, ServiceError> {
        let mut items = Vec::with_capacity(input.len() + output.len());
        items.extend(
            input
                .iter()
                .filter(|item| item["type"] != "compaction_trigger")
                .cloned(),
        );
        items.extend_from_slice(output);
        let raw_transcript = serde_json::to_string(&items).map_err(storage)?;
        let raw_parameters = parameters
            .map(serde_json::to_string)
            .transpose()
            .map_err(storage)?;
        let raw_bytes = raw_transcript
            .len()
            .saturating_add(raw_parameters.as_ref().map_or(0, String::len));

        let hot = if raw_bytes <= MAX_CHECKPOINT_BYTES {
            Some(HotPayload {
                transcript: raw_transcript.clone(),
                parameters: raw_parameters.clone(),
                bytes: raw_bytes,
            })
        } else {
            let compressed_transcript = encode_zstd_text(&raw_transcript)?;
            let compressed_parameters = match &raw_parameters {
                Some(raw) if raw.len() > PARAMETER_COMPRESS_THRESHOLD_BYTES => {
                    Some(encode_zstd_text(raw)?)
                }
                other => other.clone(),
            };
            let compressed_bytes = compressed_transcript
                .len()
                .saturating_add(compressed_parameters.as_ref().map_or(0, String::len));
            (compressed_bytes <= MAX_CHECKPOINT_BYTES).then_some(HotPayload {
                transcript: compressed_transcript,
                parameters: compressed_parameters,
                bytes: compressed_bytes,
            })
        };

        Ok(Self {
            raw_transcript,
            raw_parameters,
            raw_bytes,
            hot,
        })
    }

    pub fn publish(
        &self,
        transaction: &Transaction<'_>,
        binding: &Binding,
        generation: &Generation,
        thread: &str,
        response: &str,
    ) -> Result<(), ServiceError> {
        let Some(hot) = &self.hot else {
            return Ok(());
        };
        let created = timestamp();
        transaction
            .execute(
                "INSERT OR IGNORE INTO native_rebuild_checkpoints_v1
                    (caller,response,thread,account,generation,transcript,bytes,created,parameters)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    binding.caller,
                    response,
                    thread,
                    binding.account_id,
                    generation.id,
                    hot.transcript,
                    hot.bytes as i64,
                    created,
                    hot.parameters
                ],
            )
            .map_err(storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_contained_checkpoint_keeps_small_json_plain_and_compresses_oversized_turns() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::initialize(&connection).unwrap();

        let small_input = [serde_json::json!({"type": "message", "content": "hello"})];
        let small_output = [serde_json::json!({"type": "message", "content": "world"})];
        let small = Encoded::prepare(&small_input, &small_output, None).unwrap();
        let small_hot = small.hot.as_ref().expect("small checkpoint fits");
        assert!(
            !small_hot.transcript.starts_with(ZSTD_PREFIX),
            "normal turns remain plain JSON in native_rebuild_checkpoints_v1"
        );

        let large_input = [serde_json::json!({
            "type": "message",
            "content": "x".repeat(MAX_CHECKPOINT_BYTES + 1024)
        })];
        let large = Encoded::prepare(&large_input, &small_output, None).unwrap();
        let large_hot = large.hot.as_ref().expect("compressible large turn fits");
        assert!(
            large_hot.transcript.starts_with(ZSTD_PREFIX),
            "oversized turns compress inline without external item tables"
        );
        assert!(large_hot.bytes < 16 * 1024);

        connection
            .execute(
                "INSERT INTO native_rebuild_checkpoints_v1
                    (caller,response,thread,account,generation,transcript,bytes,created,parameters)
                 VALUES('first','response','thread','account','generation',?1,?2,1,NULL)",
                params![large_hot.transcript, large_hot.bytes as i64],
            )
            .unwrap();
        assert_eq!(
            transcript(&connection, "first", Some("thread"), "response")
                .unwrap()
                .unwrap(),
            [large_input[0].clone(), small_output[0].clone()],
        );
        assert!(transcript(&connection, "other", Some("thread"), "response")
            .unwrap()
            .is_none());
    }
}
