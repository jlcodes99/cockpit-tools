//! Bounded metadata reads for plain and compressed durable rollout files.
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

const METADATA_LIMIT: u64 = 1024 * 1024;

// Detect the frame itself: restore/import staging files deliberately use .tmp.
pub(super) fn open(path: &Path) -> io::Result<Box<dyn Read>> {
    let mut input = BufReader::new(File::open(path)?);
    let magic = input.fill_buf()?;
    let compressed = magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]);
    if compressed || path.extension().is_some_and(|extension| extension == "zst") {
        let mut decoder = zstd::stream::read::Decoder::with_buffer(input)?;
        decoder.window_log_max(27)?;
        Ok(Box::new(decoder))
    } else {
        Ok(Box::new(input))
    }
}

pub(super) fn metadata(path: &Path) -> io::Result<Option<serde_json::Value>> {
    let mut reader = BufReader::new(open(path)?.take(METADATA_LIMIT + 1));
    let mut consumed = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = reader.read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(None);
        }
        consumed += count;
        if consumed as u64 > METADATA_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rollout metadata exceeds 1 MiB",
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
            return Ok(None);
        };
        return Ok(
            (value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta"))
                .then_some(value),
        );
    }
}

pub(super) fn is_compressed(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "zst")
}

// The official state DB points at the current physical rollout after revert.
// Only use paths to choose among files already discovered under this profile.
pub(super) fn current_heads(root: &Path) -> Result<HashMap<String, PathBuf>, String> {
    let database = [
        root.join("state_5.sqlite"),
        root.join("sqlite/state_5.sqlite"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let Some(database) = database else {
        return Ok(HashMap::new());
    };
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_millis(100))
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("SELECT id, rollout_path FROM threads WHERE rollout_path IS NOT NULL LIMIT 250001")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut heads = HashMap::new();
    for row in rows {
        let (id, path) = row.map_err(|error| error.to_string())?;
        if heads.len() >= 250000 {
            return Err("rollout head catalogue exceeds bounded scan".into());
        }
        let path = PathBuf::from(path);
        heads.insert(
            id,
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            },
        );
    }
    Ok(heads)
}

pub(super) fn same_storage_path(left: &Path, right: &Path) -> bool {
    let plain = |path: &Path| {
        if is_compressed(path) {
            path.with_extension("")
        } else {
            path.to_path_buf()
        }
    };
    plain(left) == plain(right)
}

pub(crate) fn materialize_bounded(source: &Path, target: &Path) -> io::Result<()> {
    use std::io::Write;
    let before = std::fs::metadata(source)?;
    let mut input = open(source)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let started = std::time::Instant::now();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > 512 * 1024 * 1024 || started.elapsed() > std::time::Duration::from_secs(15) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rollout materialization exceeds bounded scan",
            ));
        }
        output.write_all(&buffer[..count])?;
        std::thread::yield_now();
    }
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(io::Error::other("rollout changed while preparing export"));
    }
    output.sync_all()
}
