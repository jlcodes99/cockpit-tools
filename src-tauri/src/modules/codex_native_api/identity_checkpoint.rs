//! Durable accepted history, independent of the bounded optional hot cache.
//! Wire items are content-addressed within a caller and compressed losslessly;
//! a response stores only an ordered manifest instead of duplicating its prefix.
use super::*;

pub(super) struct Metadata {
    pub thread: String,
    pub account: String,
    pub parameters: Option<String>,
}

pub(super) struct Encoded {
    manifest: String,
    chunks: Vec<(String, Vec<u8>)>,
    parameters: Option<String>,
    hot: Option<String>,
    bytes: usize,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS native_rebuild_history_v1 (
           caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
           account TEXT NOT NULL, generation TEXT NOT NULL, manifest TEXT NOT NULL,
           parameters TEXT, PRIMARY KEY(caller,response));
         CREATE TABLE IF NOT EXISTS native_rebuild_items_v1 (
           caller TEXT NOT NULL, digest TEXT NOT NULL, body BLOB NOT NULL,
           PRIMARY KEY(caller,digest));",
    )
}

/// Prefer an existing legacy/hot record, including its missing or invalid
/// parameter manifest. Archival fallback must not conceal inconsistent evidence.
pub(super) fn metadata(
    connection: &Connection,
    caller: &str,
    response: &str,
) -> Result<Option<Metadata>, ServiceError> {
    connection.query_row(
        "SELECT thread,account,parameters FROM (
           SELECT thread,account,parameters,0 AS priority FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2
           UNION ALL SELECT thread,account,parameters,1 AS priority FROM native_rebuild_history_v1 WHERE caller=?1 AND response=?2)
         ORDER BY priority LIMIT 1",
        params![caller,response],
        |row| Ok(Metadata { thread: row.get(0)?, account: row.get(1)?, parameters: row.get(2)? }),
    ).optional().map_err(storage)
}

pub(super) fn transcript(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    response: &str,
) -> Result<Option<Vec<Value>>, ServiceError> {
    let raw: Option<String> = connection.query_row(
        "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2 AND (?3 IS NULL OR thread=?3)",
        params![caller,response,thread], |row| row.get(0),
    ).optional().map_err(storage)?;
    if let Some(raw) = raw {
        return serde_json::from_str(&raw).map(Some).map_err(storage);
    }
    let raw: Option<String> = connection.query_row(
        "SELECT manifest FROM native_rebuild_history_v1 WHERE caller=?1 AND response=?2 AND (?3 IS NULL OR thread=?3)",
        params![caller,response,thread], |row| row.get(0),
    ).optional().map_err(storage)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let manifest: Vec<String> = serde_json::from_str(&raw).map_err(storage)?;
    let mut result = Vec::with_capacity(manifest.len());
    let mut query = connection
        .prepare_cached("SELECT body FROM native_rebuild_items_v1 WHERE caller=?1 AND digest=?2")
        .map_err(storage)?;
    for digest in manifest {
        let compressed: Vec<u8> = query
            .query_row(params![caller, digest], |row| row.get(0))
            .map_err(storage)?;
        let bytes = zstd::stream::decode_all(compressed.as_slice()).map_err(storage)?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err(storage("checkpoint item digest mismatch"));
        }
        result.push(serde_json::from_slice(&bytes).map_err(storage)?);
    }
    Ok(Some(result))
}

impl Encoded {
    /// Serialization, hashing and compression happen before the identity write
    /// lock/transaction. Cancellation still gates publication of all these bytes.
    pub fn prepare(
        input: &[Value],
        output: &[Value],
        parameters: Option<&Value>,
    ) -> Result<Self, ServiceError> {
        let mut manifest = Vec::new();
        let mut chunks = Vec::new();
        let mut unique = HashSet::new();
        let parameters = parameters
            .map(serde_json::to_string)
            .transpose()
            .map_err(storage)?;
        let mut bytes = 2usize.saturating_add(parameters.as_ref().map_or(0, String::len));
        let mut hot = (bytes <= MAX_CHECKPOINT_BYTES).then(|| String::from("["));
        for item in input
            .iter()
            .filter(|item| item["type"] != "compaction_trigger")
            .chain(output)
        {
            let raw = serde_json::to_vec(item).map_err(storage)?;
            let separator = usize::from(!manifest.is_empty());
            bytes = bytes.saturating_add(separator).saturating_add(raw.len());
            if bytes > MAX_CHECKPOINT_BYTES {
                hot = None;
            } else if let Some(hot) = &mut hot {
                if separator != 0 {
                    hot.push(',');
                }
                hot.push_str(std::str::from_utf8(&raw).map_err(storage)?);
            }
            let digest = format!("{:x}", Sha256::digest(&raw));
            if unique.insert(digest.clone()) {
                chunks.push((
                    digest.clone(),
                    zstd::stream::encode_all(raw.as_slice(), 1).map_err(storage)?,
                ));
            }
            manifest.push(digest);
        }
        if let Some(hot) = &mut hot {
            hot.push(']');
        }
        Ok(Self {
            manifest: serde_json::to_string(&manifest).map_err(storage)?,
            chunks,
            parameters,
            hot,
            bytes,
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
        // The first committed response wins, just as with the original cache.
        // A pre-upgrade hot-only row is already that response's first record.
        // Do not create a competing archive from a duplicate completion, since
        // later cache eviction would silently replace its accepted transcript.
        if transaction
            .query_row(
                "SELECT 1 FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2",
                params![binding.caller, response],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage)?
            .is_some()
        {
            return Ok(());
        }
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO native_rebuild_history_v1(caller,response,thread,account,generation,manifest,parameters) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![binding.caller,response,thread,binding.account_id,generation.id,self.manifest,self.parameters],
        ).map_err(storage)?;
        if inserted == 0 {
            return Ok(());
        }
        let mut insert = transaction.prepare_cached("INSERT OR IGNORE INTO native_rebuild_items_v1(caller,digest,body) VALUES(?1,?2,?3)").map_err(storage)?;
        for (digest, body) in &self.chunks {
            insert
                .execute(params![binding.caller, digest, body])
                .map_err(storage)?;
        }
        if let Some(raw) = &self.hot {
            transaction.execute("INSERT OR IGNORE INTO native_rebuild_checkpoints_v1(caller,response,thread,account,generation,transcript,bytes,created,parameters) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![binding.caller,response,thread,binding.account_id,generation.id,raw,self.bytes as i64,timestamp(),self.parameters]).map_err(storage)?;
        }
        Ok(())
    }
}
