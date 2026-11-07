//! Bounded journal framing shared by import, status repair and export.
//! Version 2 chunks become visible only after their final commit record.
use super::*;

const MAX_LEGACY_COMPLETION_REFS: usize = 100_000;

const MAX_TRANSACTION_ROWS: usize = 400_000;
const MAX_TRANSACTION_CHUNKS: u64 = 4096;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Chunk {
    pub id: String,
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub commit: bool,
    #[serde(default)]
    pub recovery: bool,
}
pub(super) enum Action {
    Begin,
    Batch(State),
    Commit,
    Abort,
}
pub(super) struct Scan {
    pub seq: u64,
}
struct Pending {
    id: String,
    generation: String,
    index: u64,
    recovery: bool,
    rows: usize,
    opaque: usize,
}

pub(super) fn scan(
    path: &Path,
    snapshot: &Snapshot,
    mut observe: impl FnMut(Action, bool, u64, &[u8]) -> Result<(), String>,
) -> Result<Scan, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Scan { seq: 0 });
        }
        Err(error) => return Err(io_failure(error)),
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut expected = 1_u64;
    let mut old_expected = 1_u64;
    let mut current_started = false;
    let mut pending: Option<Pending> = None;
    let mut committed = Scan { seq: 0 };
    loop {
        line.clear();
        let count = reader
            .by_ref()
            .take((MAX_JOURNAL_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(io_failure)?;
        if count > MAX_JOURNAL_LINE_BYTES {
            return Err(failure("journal record exceeds size limit"));
        }
        if count == 0 || line.last() != Some(&b'\n') {
            break;
        }
        let entry: JournalEntry =
            serde_json::from_slice(&line).map_err(|_| failure("invalid journal record"))?;
        if !matches!(entry.schema_version, 1 | 2) || entry.scope != snapshot.scope {
            return Err(failure("unsupported version or different scope"));
        }
        if entry.sha256 != hash(entry.payload.as_bytes()) {
            return Err(failure("journal checksum mismatch"));
        }
        let previous =
            snapshot.predecessor_generation.as_deref() == Some(entry.generation.as_str());
        if previous {
            if current_started || entry.seq != old_expected {
                return Err(failure("invalid predecessor journal sequence"));
            }
            old_expected = old_expected
                .checked_add(1)
                .ok_or_else(|| failure("journal sequence overflow"))?;
        } else {
            if entry.generation != snapshot.generation
                || entry.seq != expected
                || entry.seq > i64::MAX as u64
            {
                return Err(failure("different generation or discontinuous journal"));
            }
            current_started = true;
            expected += 1;
        }
        let state: State =
            serde_json::from_str(&entry.payload).map_err(|_| failure("invalid journal payload"))?;
        validate_state(&state)?;
        if entry.schema_version == 1 {
            if entry.transaction.is_some() || pending.is_some() {
                return Err(failure("interrupted journal transaction"));
            }
            observe(Action::Batch(state), previous, entry.seq, &line)?;
            if !previous {
                committed.seq = entry.seq;
            }
            continue;
        }
        let chunk = entry
            .transaction
            .ok_or_else(|| failure("missing journal transaction"))?;
        generation(&chunk.id)?;
        if chunk.index >= MAX_TRANSACTION_CHUNKS {
            return Err(failure("too many transaction chunks"));
        }
        if pending.is_none() {
            if chunk.index != 0 {
                return Err(failure("transaction does not start at zero"));
            }
            observe(Action::Begin, previous, entry.seq, &[])?;
            pending = Some(Pending {
                id: chunk.id.clone(),
                generation: entry.generation.clone(),
                index: 0,
                recovery: chunk.recovery,
                rows: 0,
                opaque: 0,
            });
        }
        let saved = pending.as_mut().unwrap();
        if saved.id != chunk.id
            || saved.index != chunk.index
            || saved.generation != entry.generation
            || saved.recovery != chunk.recovery
        {
            return Err(failure("transaction chunk sequence mismatch"));
        }
        if !chunk.commit && !state.recovery_completed.is_empty() {
            return Err(failure("recovery marker precedes transaction commit"));
        }
        if chunk.commit && chunk.recovery != !state.recovery_completed.is_empty() {
            return Err(failure("transaction recovery marker mismatch"));
        }
        saved.rows += state.mappings.len()
            + state.affinities.len()
            + state.opaque.len()
            + state.recovery_completed.len();
        saved.opaque += state.opaque.len();
        if saved.rows > MAX_TRANSACTION_ROWS
            || saved.recovery && saved.opaque > MAX_LEGACY_COMPLETION_REFS
        {
            return Err(failure("single journal transaction exceeds resource limit"));
        }
        saved.index += 1;
        observe(Action::Batch(state), previous, entry.seq, &line)?;
        if chunk.commit {
            observe(Action::Commit, previous, entry.seq, &[])?;
            pending = None;
            if !previous {
                committed.seq = entry.seq;
            }
        }
    }
    if let Some(saved) = pending {
        observe(
            Action::Abort,
            saved.generation != snapshot.generation,
            committed.seq,
            &[],
        )?;
    }
    Ok(committed)
}
