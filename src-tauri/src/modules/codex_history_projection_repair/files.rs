use super::*;
use crate::modules::codex_session_rollout_records::{Object, Records};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    time::SystemTime,
};

const MAX_CHAIN_DEPTH: usize = 128;

#[derive(Debug)]
pub(super) enum SourceError {
    Missing(String),
    Invalid(String),
}

impl From<String> for SourceError {
    fn from(value: String) -> Self {
        Self::Invalid(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Identity {
    volume: u64,
    index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    identity: Identity,
    length: u64,
    modified: SystemTime,
}

#[derive(Debug)]
pub(super) struct Evidence {
    pub path: PathBuf,
    stamp: Stamp,
    hash: [u8; 32],
    pub decoded_bytes: u64,
    pub initial_ordinal: u64,
    pub next_ordinal: u64,
    pub has_ordinals: bool,
    base: Option<Base>,
}

#[derive(Debug)]
struct Base {
    id: String,
    bytes: u64,
    ordinal: u64,
}

impl Evidence {
    pub fn check_stamp(&self) -> Result<(), String> {
        if stamp(&self.path)? != self.stamp {
            return Err(error("CHANGED", "rollout changed during repair"));
        }
        Ok(())
    }

    pub fn recheck(&self, budget: &mut Budget) -> Result<(), String> {
        self.check_stamp()?;
        if digest(&self.path, budget)? != self.hash {
            return Err(error("CHANGED", "rollout contents changed since scan"));
        }
        self.check_stamp()
    }

    pub fn backup_metadata(&self) -> Value {
        let hash = self
            .hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        serde_json::json!({"path":self.path,"sha256":hash,"decodedBytes":self.decoded_bytes,"nextOrdinal":self.next_ordinal})
    }
}

pub(super) fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| error("IO", e))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error(
            "UNSAFE_PATH",
            "expected a regular file without symlinks",
        ));
    }
    Ok(())
}

pub(super) fn profile_root(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| error("IO", e))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(error(
            "UNSAFE_PATH",
            "expected a profile directory without symlinks",
        ));
    }
    path.canonicalize().map_err(|e| error("IO", e))
}

pub(super) fn identity(path: &Path) -> Result<Identity, String> {
    regular_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).map_err(|e| error("IO", e))?;
        Ok(Identity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let file = fs::File::open(path).map_err(|e| error("IO", e))?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle() as _), &mut info) }
            .map_err(|e| error("IO", e))?;
        Ok(Identity {
            volume: info.dwVolumeSerialNumber.into(),
            index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(error(
            "UNSUPPORTED",
            "file identity is unavailable on this platform",
        ))
    }
}

fn stamp(path: &Path) -> Result<Stamp, String> {
    let identity = identity(path)?;
    let metadata = fs::metadata(path).map_err(|e| error("IO", e))?;
    Ok(Stamp {
        identity,
        length: metadata.len(),
        modified: metadata.modified().map_err(|e| error("IO", e))?,
    })
}

fn physical_id(path: &Path) -> Option<String> {
    file_ids(path).map(|(_, physical)| physical)
}

fn file_ids(path: &Path) -> Option<(String, String)> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zst").unwrap_or(name);
    let core = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    chrono::NaiveDateTime::parse_from_str(core.get(..19)?, "%Y-%m-%dT%H-%M-%S").ok()?;
    if core.get(19..20)? != "-" {
        return None;
    }
    let ids = core.get(20..)?;
    let (logical, physical) = ids.split_once('_').unwrap_or((ids, ids));
    Some((
        uuid::Uuid::parse_str(logical).ok()?.to_string(),
        uuid::Uuid::parse_str(physical).ok()?.to_string(),
    ))
}

pub(super) fn registry(
    profile: &Path,
    budget: &Budget,
) -> Result<HashMap<String, PathBuf>, String> {
    let mut found = HashMap::new();
    let mut directories = vec![profile.join("sessions"), profile.join("archived_sessions")];
    let mut visited = 0;
    while let Some(directory) = directories.pop() {
        budget.check()?;
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(error("IO", e)),
        };
        if metadata.file_type().is_symlink() {
            return Err(error("UNSAFE_PATH", "symlink in rollout directories"));
        }
        for entry in fs::read_dir(&directory).map_err(|e| error("IO", e))? {
            budget.check()?;
            visited += 1;
            if visited > MAX_ENTRIES {
                return Err(error("LIMIT", "too many rollout directory entries"));
            }
            let entry = entry.map_err(|e| error("IO", e))?;
            let kind = entry.file_type().map_err(|e| error("IO", e))?;
            if kind.is_symlink() {
                return Err(error("UNSAFE_PATH", "symlink in rollout directories"));
            }
            if kind.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Some(id) = physical_id(&entry.path()) else {
                continue;
            };
            if found.insert(id, entry.path()).is_some() {
                return Err(error(
                    "AMBIGUOUS_SOURCE",
                    "multiple files have the same rollout ID",
                ));
            }
        }
        std::thread::yield_now();
    }
    Ok(found)
}

pub(super) fn selected_source(
    profile: &Path,
    path: &str,
    registry: &HashMap<String, PathBuf>,
) -> Result<String, SourceError> {
    let path = Path::new(path);
    let expected = if path.is_absolute() {
        path.to_path_buf()
    } else {
        profile.join(path)
    };
    if expected
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(error(
            "UNSAFE_PATH",
            "selected rollout contains a parent traversal",
        )
        .into());
    }
    let id = physical_id(&expected).ok_or_else(|| {
        error(
            "UNSAFE_PATH",
            "selected rollout has a noncanonical filename",
        )
    })?;
    let source = registry
        .get(&id)
        .ok_or_else(|| SourceError::Missing(id.clone()))?;
    let parent = expected
        .parent()
        .ok_or_else(|| error("UNSAFE_PATH", "selected rollout has no directory"))?
        .canonicalize()
        .map_err(|e| error("IO", e))?;
    let expected = parent.join(
        expected
            .file_name()
            .ok_or_else(|| error("UNSAFE_PATH", "selected rollout has no filename"))?,
    );
    if !expected.starts_with(profile)
        || expected
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(error("UNSAFE_PATH", "selected rollout is outside this profile").into());
    }
    let plain = |path: &Path| {
        if path.extension().is_some_and(|e| e == "zst") {
            path.with_extension("")
        } else {
            path.to_path_buf()
        }
    };
    if plain(source) != plain(&expected) {
        return Err(error(
            "AMBIGUOUS_SOURCE",
            "selected rollout path does not match its indexed source",
        )
        .into());
    }
    Ok(id)
}

pub(super) fn load_chain(
    id: &str,
    registry: &HashMap<String, PathBuf>,
    sources: &mut HashMap<String, Evidence>,
    visiting: &mut HashSet<String>,
    budget: &mut Budget,
) -> Result<(), SourceError> {
    if sources.contains_key(id) {
        return Ok(());
    }
    if visiting.len() >= MAX_CHAIN_DEPTH || !visiting.insert(id.to_owned()) {
        return Err(error("UNSAFE_HISTORY", "cyclic or excessive history ancestry").into());
    }
    let path = registry
        .get(id)
        .ok_or_else(|| SourceError::Missing(id.to_owned()))?;
    let source = read(path, budget)?;
    if let Some(base) = &source.base {
        load_chain(&base.id, registry, sources, visiting, budget)?;
        let parent = &sources[&base.id];
        // The official store pages by ordinal; old byte cutoffs need not equal an exact line boundary.
        if base.ordinal == 0
            || base.bytes > parent.decoded_bytes
            || base.ordinal > parent.next_ordinal
        {
            return Err(error(
                "UNSAFE_HISTORY",
                format!("ancestor cutoff is unavailable for {id}"),
            )
            .into());
        }
    }
    visiting.remove(id);
    sources.insert(id.to_owned(), source);
    Ok(())
}

#[derive(Deserialize)]
struct Header {
    #[serde(rename = "type")]
    kind: String,
    payload: Object<HeaderPayload>,
    #[serde(default, deserialize_with = "present_value")]
    ordinal: Option<Value>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
struct HeaderPayload {
    id: String,
    #[serde(default)]
    history_base: Option<Object<HistoryBase>>,
}

#[derive(Deserialize)]
struct HistoryBase {
    thread_id: String,
    end_byte_offset: u64,
    end_ordinal_exclusive: u64,
}

#[derive(Deserialize)]
struct OrdinalRecord {
    ordinal: u64,
}

fn next_record<T: DeserializeOwned, R: Read, F: FnMut(&[u8]) -> io::Result<()>>(
    records: &mut Records<R, F>,
    budget: &Budget,
) -> Result<Option<T>, String> {
    loop {
        budget.check()?;
        let record = records.next();
        budget.check()?;
        let Some(record) = record.map_err(|e| error("UNSAFE_HISTORY", e))? else {
            return Ok(None);
        };
        if !record.complete {
            return Err(error(
                "UNSAFE_HISTORY",
                "rollout has an incomplete trailing record",
            ));
        }
        if !record.blank {
            return record
                .value
                .map(Some)
                .map_err(|e| error("UNSAFE_HISTORY", e));
        }
    }
}

fn read(path: &Path, budget: &Budget) -> Result<Evidence, String> {
    let before = stamp(path)?;
    let hash = digest(path, budget)?;
    let file = fs::File::open(path).map_err(|e| error("IO", e))?;
    let input: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zst") {
        let mut decoder =
            zstd::stream::read::Decoder::new(file).map_err(|e| error("UNSAFE_HISTORY", e))?;
        decoder
            .window_log_max(27)
            .map_err(|e| error("UNSAFE_HISTORY", e))?;
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    let mut records = Records::new(input, |_: &[u8]| {
        std::thread::yield_now();
        budget.check().map_err(io::Error::other)
    });
    let Some(header) = next_record::<Header, _, _>(&mut records, budget)? else {
        return Err(error("UNSAFE_HISTORY", "rollout contains no ordinal evidence"));
    };
    if header.kind != "session_meta" {
        return Err(error("UNSAFE_HISTORY", "rollout has no initial session metadata"));
    }
    let expected_id = file_ids(path)
        .map(|(logical, _)| logical)
        .ok_or_else(|| error("UNSAFE_PATH", "noncanonical rollout filename"))?;
    let payload = header.payload.0;
    let metadata_id = uuid::Uuid::parse_str(&payload.id)
        .map_err(|_| error("UNSAFE_HISTORY", "invalid session metadata ID"))?;
    if metadata_id.to_string() != expected_id {
        return Err(error(
            "UNSAFE_HISTORY",
            "rollout filename and session metadata disagree",
        ));
    }
    let base = payload.history_base.map(|Object(value)| {
        let id = uuid::Uuid::parse_str(&value.thread_id)
            .map_err(|_| error("UNSAFE_HISTORY", "invalid ancestor ID"))?
            .to_string();
        Ok::<_, String>(Base {
            id,
            bytes: value.end_byte_offset,
            ordinal: value.end_ordinal_exclusive,
        })
    }).transpose()?;
    let initial_ordinal = base.as_ref().map_or(0, |base| base.ordinal);
    let mut expected = initial_ordinal;
    let mut ordinal = header.ordinal.map(|value| {
        value.as_u64().ok_or_else(|| error("UNSAFE_HISTORY", "rollout record has no ordinal"))
    }).transpose()?;
    loop {
        let next = match ordinal.take() {
            Some(ordinal) => ordinal,
            None => {
                let Some(record) = next_record::<OrdinalRecord, _, _>(&mut records, budget)? else {
                    break;
                };
                record.ordinal
            }
        };
        if next != expected {
            return Err(error("MISSING_RECORDS", "rollout ordinals are not contiguous"));
        }
        expected = next
            .checked_add(1)
            .ok_or_else(|| error("UNSAFE_HISTORY", "rollout ordinal overflow"))?;
    }
    if expected == initial_ordinal {
        return Err(error("UNSAFE_HISTORY", "rollout contains no ordinal evidence"));
    }
    if stamp(path)? != before {
        return Err(error("CHANGED", "rollout changed while scanning"));
    }
    Ok(Evidence {
        path: path.to_path_buf(),
        stamp: before,
        hash,
        decoded_bytes: records.position,
        initial_ordinal,
        next_ordinal: expected,
        has_ordinals: true,
        base,
    })
}

fn digest(path: &Path, budget: &Budget) -> Result<[u8; 32], String> {
    regular_file(path)?;
    let mut reader = fs::File::open(path).map_err(|e| error("IO", e))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| error("IO", e))?;
        if count == 0 {
            break;
        }
        budget.check()?;
        std::thread::yield_now();
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}
