//! Metadata-only local Code handoff. The caller owns Desktop/storage-contract guards.
//! No transcript is parsed or written, and no account/authentication state is used.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[path = "continuity.rs"]
mod continuity;

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub account: String,
    pub org: String,
}

#[derive(Clone, Debug)]
pub struct Roots {
    pub records: PathBuf,
    pub pool: PathBuf,
    pub state: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub session_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub fingerprint: String,
    pub baseline_changed: bool,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub missing: usize,
    pub stale: usize,
    pub replaced_branches: usize,
    pub issues: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub quota_pauses_cleared: usize,
    #[serde(default)]
    pub preserved_branches: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub state: String,
    pub created_at: i64,
    pub created: usize,
    pub updated: usize,
    #[serde(default)]
    pub skipped_missing: usize,
    #[serde(default)]
    pub skipped_stale: usize,
    #[serde(default)]
    pub replaced_branches: usize,
    pub source: Identity,
    pub target: Identity,
    pub backup_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

type Result<T> = std::result::Result<T, String>;
const MAX_JSON: u64 = 10 * 1024 * 1024;
const MAX_PRIOR_IDS: usize = 200;
const GROUPS: &[(&str, &[&str])] = &[
    (
        "history",
        &[
            "sessionId",
            "cliSessionId",
            "priorCliSessionIds",
            "forkedFromSessionId",
            "forkedAtMessageUuid",
            "lineageDetached",
            "rewindEdges",
            "transcriptModelStates",
        ],
    ),
    (
        "workspace",
        &[
            "cwd",
            "originCwd",
            "worktreePath",
            "worktreeName",
            "worktreeLazy",
            "worktreePinned",
            "sourceBranch",
            "branch",
            "writtenBranches",
            "keptDirtyWorktree",
            "keptDirtyAt",
            "keptWorktreeLeftover",
        ],
    ),
    ("title", &["title", "titleSource", "previousTitles"]),
    ("model", &["model", "effort", "classifierSummaryEnabled"]),
];
const SCALARS: &[&str] = &["isArchived", "isStarred", "color", "createdAt"];

fn check(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(code.into())
    }
}

fn io_error(e: std::io::Error) -> String {
    match e.raw_os_error() {
        Some(libc::ENOENT) => "MISSING_PATH",
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => "UNSAFE_PATH",
        Some(libc::EEXIST) => "ALREADY_EXISTS",
        _ => "IO_ERROR",
    }
    .into()
}

fn safe_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code.as_bytes()[0].is_ascii_uppercase()
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn guard_call(guard: &mut dyn FnMut() -> Result<()>) -> Result<()> {
    guard().map_err(|code| {
        if safe_code(&code) {
            code
        } else {
            public_process_diagnostic(&code).unwrap_or_else(|| "GUARD_FAILED".into())
        }
    })
}

fn public_process_diagnostic(error: &str) -> Option<String> {
    if error.len() > 1024 {
        return None;
    }
    let value: Value = serde_json::from_str(error).ok()?;
    let code = value.get("code")?.as_str()?;
    if ![
        "CLAUDE_WRITER_RUNNING",
        "DESKTOP_UPDATE_TIMEOUT",
        "DESKTOP_CONTRACT_CHANGED",
    ]
    .contains(&code)
    {
        return None;
    }
    let pid = value.get("processId")?.as_u64()?;
    let role = value.get("processRole")?.as_str()?;
    let name = value.get("processName")?.as_str()?;
    if pid == 0
        || pid > u32::MAX as u64
        || ![
            "desktop-main",
            "desktop-helper",
            "desktop-updater",
            "claude-cli",
        ]
        .contains(&role)
        || name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b" ._()-".contains(&c))
    {
        return None;
    }
    Some(json!({"code":code,"processId":pid,"processRole":role,"processName":name}).to_string())
}

fn guard_call_quick(
    full: &mut dyn FnMut() -> Result<()>,
    quick: &mut Option<&mut dyn FnMut() -> Result<()>>,
) -> Result<()> {
    match quick {
        Some(check) => guard_call(*check),
        None => guard_call(full),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| "INVALID_JSON")?;
    bytes.push(b'\n');
    check(bytes.len() as u64 <= MAX_JSON, "JSON_TOO_LARGE")?;
    Ok(bytes)
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && Uuid::parse_str(value)
            .map(|u| u.hyphenated().to_string().eq_ignore_ascii_case(value))
            .unwrap_or(false)
}

fn identity_key(identity: &Identity) -> Result<String> {
    check(
        uuid(&identity.account) && uuid(&identity.org),
        "INVALID_IDENTITY",
    )?;
    Ok(format!(
        "{}/{}",
        identity.account.to_ascii_lowercase(),
        identity.org.to_ascii_lowercase()
    ))
}

fn session_id(value: &str) -> bool {
    value.strip_prefix("local_").map(uuid).unwrap_or(false)
}
fn record_name(value: &str) -> bool {
    value.strip_suffix(".json").map(session_id).unwrap_or(false)
}
fn valid_run_id(value: &str) -> bool {
    value.strip_prefix("run-").map(uuid).unwrap_or(false)
}

fn baseline_path(roots: &Roots, source: &Identity, target: &Identity) -> Result<PathBuf> {
    let mut keys = [identity_key(source)?, identity_key(target)?];
    keys.sort();
    Ok(roots
        .state
        .join("baselines")
        .join(format!("{}.json", digest(keys.join("|").as_bytes()))))
}

fn safe_path(path: &Path) -> Result<()> {
    check(path.is_absolute(), "UNSAFE_PATH")?;
    check(
        path.components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
        "UNSAFE_PATH",
    )?;
    let text = path.to_str().ok_or("UNSAFE_PATH")?;
    check(
        !text.contains('\0') && !text.split('/').any(|s| s == "." || s == ".."),
        "UNSAFE_PATH",
    )
}

fn disjoint(paths: &[&Path]) -> Result<()> {
    for (i, a) in paths.iter().enumerate() {
        safe_path(a)?;
        for b in paths.iter().skip(i + 1) {
            check(!a.starts_with(b) && !b.starts_with(a), "OVERLAPPING_ROOTS")?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    hash: String,
    id: FileId,
}

fn file_id(file: &File) -> Result<FileId> {
    #[cfg(unix)]
    {
        let m = file.metadata().map_err(io_error)?;
        Ok(FileId {
            dev: m.dev(),
            ino: m.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err("UNSUPPORTED_PLATFORM".into())
    }
}

// All filesystem access is anchored to open directory descriptors. O_NOFOLLOW
// applies to every ancestor, not just the leaf, including during publication.
struct Directory(File);

impl Directory {
    #[cfg(unix)]
    fn child(&self, name: &str) -> Result<Self> {
        check(
            !name.is_empty() && !name.contains('/') && name != "." && name != "..",
            "UNSAFE_PATH",
        )?;
        let name = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    #[cfg(not(unix))]
    fn child(&self, _name: &str) -> Result<Self> {
        Err("UNSUPPORTED_PLATFORM".into())
    }

    #[cfg(unix)]
    fn open(path: &Path, create: bool) -> Result<Self> {
        use std::ffi::CString;
        safe_path(path)?;
        let root = CString::new("/").unwrap();
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let mut current = Self(unsafe { File::from_raw_fd(fd) });
        for component in path.components() {
            if let Component::Normal(name) = component {
                let name = CString::new(name.as_encoded_bytes()).map_err(|_| "UNSAFE_PATH")?;
                let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                let mut next = unsafe { libc::openat(current.0.as_raw_fd(), name.as_ptr(), flags) };
                if next < 0
                    && create
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
                {
                    let made =
                        unsafe { libc::mkdirat(current.0.as_raw_fd(), name.as_ptr(), 0o700) };
                    if made < 0
                        && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
                    {
                        return Err(io_error(std::io::Error::last_os_error()));
                    }
                    current.sync()?;
                    next = unsafe { libc::openat(current.0.as_raw_fd(), name.as_ptr(), flags) };
                }
                if next < 0 {
                    return Err(io_error(std::io::Error::last_os_error()));
                }
                current = Self(unsafe { File::from_raw_fd(next) });
            }
        }
        Ok(current)
    }

    #[cfg(not(unix))]
    fn open(_path: &Path, _create: bool) -> Result<Self> {
        Err("UNSUPPORTED_PLATFORM".into())
    }

    fn private(&self) -> Result<()> {
        #[cfg(unix)]
        {
            check(
                self.0.metadata().map_err(io_error)?.mode() & 0o777 == 0o700,
                "INSECURE_DIRECTORY",
            )
        }
        #[cfg(not(unix))]
        {
            Err("UNSUPPORTED_PLATFORM".into())
        }
    }

    fn make_private(&self) -> Result<()> {
        #[cfg(unix)]
        {
            check(
                unsafe { libc::fchmod(self.0.as_raw_fd(), 0o700) } == 0,
                "IO_ERROR",
            )?;
            self.sync()
        }
        #[cfg(not(unix))]
        {
            Err("UNSUPPORTED_PLATFORM".into())
        }
    }

    fn is_directory(&self, name: &str) -> Result<bool> {
        #[cfg(unix)]
        {
            let name = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            check(
                unsafe {
                    libc::fstatat(
                        self.0.as_raw_fd(),
                        name.as_ptr(),
                        stat.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } == 0,
                "IO_ERROR",
            )?;
            let mode = unsafe { stat.assume_init() }.st_mode;
            check(mode & libc::S_IFMT != libc::S_IFLNK, "UNSAFE_PATH")?;
            Ok(mode & libc::S_IFMT == libc::S_IFDIR)
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            Err("UNSUPPORTED_PLATFORM".into())
        }
    }

    fn sync(&self) -> Result<()> {
        self.0.sync_all().map_err(io_error)
    }

    #[cfg(unix)]
    fn file(&self, name: &str, flags: i32) -> Result<File> {
        check(
            !name.is_empty() && name != "." && name != ".." && !name.contains('/'),
            "UNSAFE_PATH",
        )?;
        let name = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(
            file.metadata().map_err(io_error)?.is_file(),
            "NOT_REGULAR_FILE",
        )?;
        Ok(file)
    }

    #[cfg(not(unix))]
    fn file(&self, _name: &str, _flags: i32) -> Result<File> {
        Err("UNSUPPORTED_PLATFORM".into())
    }

    #[cfg(unix)]
    fn names(&self) -> Result<Vec<String>> {
        let fd = unsafe { libc::dup(self.0.as_raw_fd()) };
        if fd < 0 {
            return Err("IO_ERROR".into());
        }
        let dir = unsafe { libc::fdopendir(fd) };
        if dir.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err("IO_ERROR".into());
        }
        struct Close(*mut libc::DIR);
        impl Drop for Close {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let _close = Close(dir);
        let mut names = Vec::new();
        loop {
            // readdir reports end and errors through the same null pointer.
            #[cfg(target_os = "macos")]
            let errno = unsafe { libc::__error() };
            #[cfg(not(target_os = "macos"))]
            let errno = unsafe { libc::__errno_location() };
            unsafe {
                *errno = 0;
            }
            let entry = unsafe { libc::readdir(dir) };
            if entry.is_null() {
                check(unsafe { *errno } == 0, "IO_ERROR")?;
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| "UNSAFE_PATH")?;
            if name != "." && name != ".." {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    #[cfg(not(unix))]
    fn names(&self) -> Result<Vec<String>> {
        Err("UNSUPPORTED_PLATFORM".into())
    }

    #[cfg(unix)]
    fn publish(&self, temp: &str, name: &str, replace: bool) -> Result<()> {
        let temp = std::ffi::CString::new(temp).map_err(|_| "UNSAFE_PATH")?;
        let name = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
        let fd = self.0.as_raw_fd();
        let result = unsafe {
            if replace {
                libc::renameat(fd, temp.as_ptr(), fd, name.as_ptr())
            } else {
                libc::linkat(fd, temp.as_ptr(), fd, name.as_ptr(), 0)
            }
        };
        check(result == 0, "PUBLISH_FAILED")?;
        self.sync()
    }

    #[cfg(not(unix))]
    fn publish(&self, _temp: &str, _name: &str, _replace: bool) -> Result<()> {
        Err("UNSUPPORTED_PLATFORM".into())
    }

    #[cfg(unix)]
    fn unlink(&self, name: &str) -> Result<()> {
        let name = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ENOENT) {
                return Err(io_error(error));
            }
        }
        self.sync()
    }

    #[cfg(not(unix))]
    fn unlink(&self, _name: &str) -> Result<()> {
        Err("UNSUPPORTED_PLATFORM".into())
    }
}

fn parent(path: &Path) -> Result<(Directory, String)> {
    safe_path(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("UNSAFE_PATH")?;
    Ok((
        Directory::open(path.parent().ok_or("UNSAFE_PATH")?, false)?,
        name.into(),
    ))
}

#[derive(Clone)]
struct Blob {
    bytes: Vec<u8>,
    stamp: Stamp,
    metadata: TranscriptMetadata,
}

fn read_blob(path: &Path) -> Result<Option<Blob>> {
    let (dir, name) = match parent(path) {
        Ok(p) => p,
        Err(e) if e == "MISSING_PATH" => return Ok(None),
        Err(e) => return Err(e),
    };
    read_blob_from(&dir, &name, path)
}

fn read_blob_from(dir: &Directory, name: &str, path: &Path) -> Result<Option<Blob>> {
    let mut file = match dir.file(name, libc::O_RDONLY) {
        Ok(f) => f,
        Err(e) if e == "MISSING_PATH" => return Ok(None),
        Err(e) => return Err(e),
    };
    check(
        file.metadata().map_err(io_error)?.len() <= MAX_JSON,
        "JSON_TOO_LARGE",
    )?;
    let mut bytes = Vec::new();
    let metadata = transcript_metadata(path.to_path_buf(), &file)?;
    (&mut file)
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    check(bytes.len() as u64 <= MAX_JSON, "JSON_TOO_LARGE")?;
    check(
        metadata.size == bytes.len() as u64
            && transcript_metadata(path.to_path_buf(), &file)? == metadata,
        "SNAPSHOT_DRIFT",
    )?;
    Ok(Some(Blob {
        stamp: Stamp {
            hash: digest(&bytes),
            id: file_id(&file)?,
        },
        bytes,
        metadata,
    }))
}

fn required_blob(path: &Path) -> Result<Blob> {
    read_blob(path)?.ok_or_else(|| "MISSING_PATH".into())
}
fn stamp(path: &Path) -> Result<Option<Stamp>> {
    Ok(read_blob(path)?.map(|b| b.stamp))
}

fn decode<T: serde::de::DeserializeOwned>(blob: &Blob) -> Result<T> {
    serde_json::from_slice(&blob.bytes).map_err(|_| "INVALID_JSON".into())
}

fn exclusive(path: &Path, bytes: &[u8]) -> Result<FileId> {
    let (dir, name) = parent(path)?;
    let mut file = dir.file(&name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    dir.sync()?;
    file_id(&file)
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = json_bytes(value)?;
    let (dir, name) = parent(path)?;
    let before = stamp(path)?;
    let temp = format!(".handoff-{}.tmp", Uuid::new_v4());
    exclusive(&path.with_file_name(&temp), &bytes)?;
    let result = (|| {
        check(stamp(path)? == before, "JOURNAL_DRIFT")?;
        dir.publish(&temp, &name, before.is_some())
    })();
    let cleanup = dir.unlink(&temp);
    result.and(cleanup)
}

struct Lock(File);
impl Lock {
    fn acquire(state: &Path) -> Result<Self> {
        let dir = Directory::open(state, true)?;
        dir.private()?;
        let file = dir.file("lock", libc::O_RDWR | libc::O_CREAT)?;
        #[cfg(unix)]
        {
            check(
                file.metadata().map_err(io_error)?.mode() & 0o777 == 0o600,
                "INSECURE_FILE",
            )?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            check(result == 0, "LOCKED")?;
            dir.sync()?;
            Ok(Self(file))
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            Err("UNSUPPORTED_PLATFORM".into())
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
        // Keep the lock inode: unlinking permits two different lock owners.
    }
}

#[derive(Clone)]
struct Record {
    value: Value,
    blob: Blob,
}
type Records = BTreeMap<String, Record>;

fn records(dir: &Path) -> Result<Records> {
    let mut out = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let opened = Directory::open(dir, false)?;
    let before = transcript_metadata(dir.to_path_buf(), &opened.0)?;
    for name in opened.names()? {
        if !name.starts_with("local_") || !name.ends_with(".json") {
            continue;
        }
        check(record_name(&name), "INVALID_RECORD_NAME")?;
        check(ids.insert(name.to_ascii_lowercase()), "DUPLICATE_RECORD")?;
        let blob = read_blob_from(&opened, &name, &dir.join(&name))?.ok_or("MISSING_PATH")?;
        let value: Value = decode(&blob)?;
        check(
            value.is_object() && value["sessionId"].as_str() == name.strip_suffix(".json"),
            "INVALID_RECORD_IDENTITY",
        )?;
        out.insert(name, Record { value, blob });
    }
    check(
        transcript_metadata(dir.to_path_buf(), &opened.0)? == before
            && file_id(&Directory::open(dir, false)?.0)? == before.id,
        "SNAPSHOT_DRIFT",
    )?;
    Ok(out)
}

// Namespace ingestion includes pending, cleared and remote rows. Only ordinary
// candidates are subject to the local resume contract, on both affected sides.
fn validate_candidate(value: &Value) -> Result<()> {
    check(
        value["cliSessionId"].as_str().map(uuid).unwrap_or(false)
            && value["cwd"]
                .as_str()
                .map(|p| Path::new(p).is_absolute())
                .unwrap_or(false),
        "INVALID_RECORD_IDENTITY",
    )?;
    if let Some(parent) = value.get("forkedFromSessionId") {
        check(
            parent.is_null() || parent.as_str().map(session_id).unwrap_or(false),
            "INVALID_LINEAGE",
        )?;
    }
    if let Some(prior) = value.get("priorCliSessionIds") {
        check(
            prior.is_null()
                || prior
                    .as_array()
                    .map(|a| a.iter().all(|v| v.as_str().map(uuid).unwrap_or(false)))
                    .unwrap_or(false),
            "INVALID_LINEAGE",
        )?;
    }
    Ok(())
}

fn record_stamps(records: &Records) -> BTreeMap<String, Stamp> {
    records
        .iter()
        .map(|(n, r)| (n.clone(), r.blob.stamp.clone()))
        .collect()
}

fn record_metadata(records: &Records) -> BTreeMap<String, TranscriptMetadata> {
    records
        .iter()
        .map(|(name, row)| (name.clone(), row.blob.metadata.clone()))
        .collect()
}

fn check_record_metadata(
    path: &Path,
    directory_id: &FileId,
    expected: &BTreeMap<String, TranscriptMetadata>,
) -> Result<()> {
    let dir = Directory::open(path, false)?;
    let before = transcript_metadata(path.to_path_buf(), &dir.0)?;
    check(&before.id == directory_id, "SNAPSHOT_DRIFT")?;
    let names: BTreeSet<_> = dir
        .names()?
        .into_iter()
        .filter(|name| name.starts_with("local_") && name.ends_with(".json"))
        .collect();
    check(
        names == expected.keys().cloned().collect(),
        "SNAPSHOT_DRIFT",
    )?;
    for (name, metadata) in expected {
        let file = dir.file(name, libc::O_RDONLY)?;
        check(
            transcript_metadata(path.join(name), &file)? == *metadata,
            "SNAPSHOT_DRIFT",
        )?;
    }
    check(
        transcript_metadata(path.to_path_buf(), &dir.0)? == before
            && file_id(&Directory::open(path, false)?.0)? == before.id,
        "SNAPSHOT_DRIFT",
    )
}

fn portable(record: &Value) -> Value {
    let mut out = Map::new();
    for (group, fields) in GROUPS {
        let values: Map<String, Value> = fields
            .iter()
            .filter_map(|k| {
                let value = record.get(*k)?;
                if ["rewindEdges", "transcriptModelStates"].contains(k) && !nonempty(value) {
                    None
                } else {
                    Some(((*k).into(), value.clone()))
                }
            })
            .collect();
        out.insert((*group).into(), Value::Object(values));
    }
    for key in SCALARS {
        if let Some(value) = record.get(*key) {
            out.insert((*key).into(), value.clone());
        }
    }
    out.insert(
        "isArchived".into(),
        Value::Bool(record["isArchived"] == true),
    );
    Value::Object(out)
}

fn merge(source: &Value, target: &Value, base: Option<&Value>) -> Option<Value> {
    let mut out = Map::new();
    for key in GROUPS.iter().map(|g| g.0).chain(SCALARS.iter().copied()) {
        let s = source.get(key);
        let t = target.get(key);
        let selected = if s == t {
            t
        } else if base.is_some() && s == base.and_then(|b| b.get(key)) {
            t
        } else if base.is_some() && t == base.and_then(|b| b.get(key)) {
            s
        } else {
            return None;
        };
        if let Some(value) = selected {
            out.insert(key.into(), value.clone());
        }
    }
    Some(Value::Object(out))
}

// The matching local session ID identifies the same sidebar row. A
// user-selected source can supersede a different active pointer only when its
// recorded activity is later. The target preimage is journaled before any
// replacement. Equal/unknown timestamps and other metadata conflicts retain
// the conservative three-way behavior; no transcript is rewritten.
fn source_rollover(source: &Value, target: &Value) -> bool {
    let Some(source_id) = source["cliSessionId"].as_str() else {
        return false;
    };
    let Some(target_id) = target["cliSessionId"].as_str() else {
        return false;
    };
    if source_id.eq_ignore_ascii_case(target_id) {
        return false;
    }
    source["lastActivityAt"]
        .as_f64()
        .zip(target["lastActivityAt"].as_f64())
        .is_some_and(|(source_at, target_at)| {
            source_at.is_finite() && target_at.is_finite() && source_at > target_at
        })
}

fn target_exclusion(
    target: &Value,
    tasks: &BTreeSet<String>,
    source: &Value,
) -> Option<&'static str> {
    if !source_rollover(source, target)
        || !quit_marked(target)
        || ["error", "errorAt", "errorCategory", "tccFolderKind"]
            .iter()
            .any(|key| target.get(*key).is_some())
        || target
            .get("interruptedByQuitAt")
            .is_some_and(|value| !value.as_f64().is_some_and(f64::is_finite))
        || target
            .get("interruptedUnseenResume")
            .is_some_and(|value| !value.is_boolean())
    {
        return exclusion(target, tasks, source);
    }
    // A previous quit marker is scoped to the target's old active pointer.
    // The new pointer clears it in materialize(). Keep all other exclusion
    // checks, including staged transcripts, queued starts, and task ownership.
    let mut old_pointer_retired = target.clone();
    if let Some(record) = old_pointer_retired.as_object_mut() {
        record.remove("interruptedByQuitAt");
        record.remove("interruptedUnseenResume");
    }
    exclusion(&old_pointer_retired, tasks, source)
}

fn merge_selected_history(source: &Value, target: &Value, base: Option<&Value>) -> Option<Value> {
    let mut target = target.clone();
    target["history"] = source["history"].clone();
    merge(source, &target, base)
}

fn word_match(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(i, _)| {
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        !text[..i].chars().next_back().map(word).unwrap_or(false)
            && !text[i + needle.len()..]
                .chars()
                .next()
                .map(word)
                .unwrap_or(false)
    })
}

fn quota_error_at(record: &Value) -> Option<f64> {
    let recognized = record["error"]
        .as_str()
        .map(|s| word_match(s, "session limit") && word_match(&s.to_ascii_lowercase(), "resets"))
        .unwrap_or(false)
        && [
            "errorCategory",
            "tccFolderKind",
            "armedWorkAtQuit",
            "pendingFirstStart",
        ]
        .iter()
        .all(|k| record.get(*k).is_none());
    if recognized {
        record["errorAt"].as_f64()
    } else {
        None
    }
}

fn quota_activity_covers(record: &Value, activity: &Value) -> bool {
    quota_error_at(record)
        .zip(activity["lastActivityAt"].as_f64())
        .map(|(error, latest)| latest >= error)
        .unwrap_or(false)
}

fn stale_quota(target: &Value, source: &Value) -> bool {
    source["cliSessionId"] == target["cliSessionId"] && quota_activity_covers(target, source)
}

fn quit_marked(record: &Value) -> bool {
    ["interruptedByQuitAt", "interruptedUnseenResume"]
        .iter()
        .any(|k| record.get(*k).is_some())
}

fn known_quota_quit(record: &Value, activity: &Value) -> bool {
    stale_quota(record, activity)
        && record
            .get("interruptedByQuitAt")
            .map(|v| v.as_f64().is_some())
            .unwrap_or(true)
        && record
            .get("interruptedUnseenResume")
            .map(Value::is_boolean)
            .unwrap_or(true)
}

fn materialize(p: &Value, source: &Value, target: Option<&Value>) -> Value {
    let mut out = target
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (group, fields) in GROUPS {
        for key in *fields {
            out.remove(*key);
        }
        if let Some(values) = p[*group].as_object() {
            out.extend(values.clone());
        }
    }
    for key in SCALARS {
        out.remove(*key);
        if let Some(value) = p.get(*key) {
            out.insert((*key).into(), value.clone());
        }
    }
    let pointer_changed = target
        .map(|t| t.get("cliSessionId") != out.get("cliSessionId"))
        .unwrap_or(true);
    if pointer_changed {
        for key in [
            "lastActivityAt",
            "lastFocusedAt",
            "completedTurns",
            "contextExceededCount",
        ] {
            out.remove(key);
            if let Some(value) = source.get(key) {
                out.insert(key.into(), value.clone());
            }
        }
        out.insert("bridgeSessionIds".into(), json!([]));
        out.insert("remoteControlAutoEligible".into(), json!(false));
        out.insert("remoteControlUserEnabled".into(), json!(false));
        for key in [
            "armedWorkAtQuit",
            "pendingFirstStart",
            // These are scoped to the previous active transcript. Keeping the
            // destination's summary after a pointer change would mislabel the
            // imported conversation; the source summary is not portable.
            "postTurnSummary",
            "postTurnSummaryFor",
            "lastAssistantUuid",
            "turnWrapUp",
            "subagentsTruncatedFor",
            "error",
            "errorAt",
            "errorCategory",
            "tccFolderKind",
            "interruptedByQuitAt",
            "interruptedUnseenResume",
        ] {
            out.remove(key);
        }
        if let Some(marker) = source.get("subagentsTruncatedFor") {
            if marker.as_str() == source["cliSessionId"].as_str() {
                out.insert("subagentsTruncatedFor".into(), marker.clone());
            }
        }
    } else if out.get("cliSessionId") == source.get("cliSessionId") {
        for key in ["lastActivityAt", "completedTurns"] {
            if let Some(n) = source[key].as_f64() {
                if n > out
                    .get(key)
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::NEG_INFINITY)
                {
                    out.insert(key.into(), source[key].clone());
                }
            }
        }
        if target.map(|t| stale_quota(t, source)).unwrap_or(false) {
            for key in [
                "error",
                "errorAt",
                "interruptedByQuitAt",
                "interruptedUnseenResume",
            ] {
                out.remove(key);
            }
        }
    }
    if target.is_none() {
        let defaults = json!({"permissionMode":"default", "bypassChosenInApp":false,
            "chromePermissionMode":"ask", "chromeAllowedDomains":[], "cuAllowedApps":[],
            "cuGrantFlags":{"clipboardRead":false,"clipboardWrite":false,"systemKeyCombos":false},
            "enabledMcpTools":{}, "remoteMcpServersConfig":[], "alwaysAllowedReasons":[], "sessionPermissionUpdates":[]});
        out.extend(defaults.as_object().unwrap().clone());
        if let Some(value) = source.get("contextExceededCount") {
            out.insert("contextExceededCount".into(), value.clone());
        }
    }
    Value::Object(out)
}

fn meaningful(value: &Value) -> bool {
    !value.is_null() && value.as_bool() != Some(false) && value.as_str() != Some("")
}

fn nonempty(value: &Value) -> bool {
    meaningful(value)
        && !value.as_array().map(Vec::is_empty).unwrap_or(false)
        && !value.as_object().map(Map::is_empty).unwrap_or(false)
}

fn exclusion(record: &Value, tasks: &BTreeSet<String>, activity: &Value) -> Option<&'static str> {
    if tasks.contains(record["sessionId"].as_str().unwrap_or_default())
        || [
            "scheduledTaskId",
            "scheduledTask",
            "notifySessionId",
            "spawnedFrom",
            "dispatchParentId",
            "dispatchParentOrigin",
        ]
        .iter()
        .any(|k| record.get(*k).map(meaningful).unwrap_or(false))
    {
        return Some("OWNED_OR_SPAWNED_SESSION");
    }
    if [
        "sshHost",
        "sshConfig",
        "wslConfig",
        "wslDistro",
        "wslDistribution",
        "cloudSessionId",
        "remoteSessionId",
        "remoteControlSpawn",
        "remote",
        "cloud",
        "ssh",
        "wsl",
    ]
    .iter()
    .any(|k| record.get(*k).map(meaningful).unwrap_or(false))
        || record.get("backend").map(|v| v != "local").unwrap_or(false)
    {
        return Some("NONLOCAL_SESSION");
    }
    if ["movedToCloud", "transcriptCuts"]
        .iter()
        .any(|k| record.get(*k).map(nonempty).unwrap_or(false))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    // Claude Desktop 2.9939.2–2.9939.4 persists additional state that this bounded
    // handoff does not know how to project across account namespaces. Skip an
    // affected record instead of silently dropping or inheriting that state.
    if [
        "gitAnchors",
        "gitAnchorsLookupOnly",
        "withheldConnectorHosts",
        "autoFixDelivered",
        "titleOffers",
        "titleSuggestionsOff",
        "_startedThroughHostCliLauncher",
        "ranInSandboxVm",
        "scratchCarried",
        "sideSessionReportOwed",
        "remoteControlDescendant",
        "steeredByRemoteClient",
        "turnBoxDeclared",
        "sideSessionOffersMuted",
        "asides",
        "turnBoxMounted",
        "devIntentTriggers",
    ]
    .iter()
    .any(|k| record.get(*k).map(nonempty).unwrap_or(false))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    // The server-fallback field is a per-window boolean capability, not a
    // persisted prompt. It is deliberately not projected to a new account.
    if record
        .get("autoModeServerFallbackPrompt")
        .is_some_and(|value| !value.is_boolean() && !value.is_null())
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    // The marker is recomputed from the shared transcript and subagent files.
    // A marker for a different active transcript remains unsupported.
    if record
        .get("subagentsTruncatedFor")
        .is_some_and(|value| !value.is_null() && value.as_str() != record["cliSessionId"].as_str())
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    if record
        .get("stagedTranscriptPath")
        .map(nonempty)
        .unwrap_or(false)
        || ["pendingFirstStart", "armedWorkAtQuit"]
            .iter()
            .any(|k| record.get(*k).is_some())
    {
        return Some("UNFINISHED_SESSION");
    }
    if record.get("importedFrom").map(meaningful).unwrap_or(false) {
        match record.get("resumeConfirmed") {
            Some(Value::Bool(true)) => (),
            None | Some(Value::Null) | Some(Value::Bool(false)) => {
                return Some("UNFINISHED_SESSION")
            }
            Some(_) => return Some("UNSUPPORTED_HISTORY_STATE"),
        }
    }
    if record.get("cliSessionId").is_none() {
        return Some("NON_RESUMABLE_SESSION");
    }
    if quit_marked(record) && !known_quota_quit(record, activity) {
        return Some("UNFINISHED_SESSION");
    }
    None
}

fn prior_ids(record: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if let Some(prior) = record["priorCliSessionIds"].as_array() {
        // Mirror the vendor's bounded, deduplicated last-200 inventory without
        // rewriting the persisted history group or guessing missing handles.
        for value in prior.iter().rev() {
            if let Some(id) = value.as_str().filter(|s| uuid(s)) {
                ids.insert(id.to_ascii_lowercase());
                if ids.len() == MAX_PRIOR_IDS {
                    break;
                }
            }
        }
    }
    ids
}

fn record_issue(name: &str, reason: &str, source: &Records, target: &Records) -> Issue {
    let title = source
        .get(name)
        .and_then(|r| r.value["title"].as_str())
        .or_else(|| target.get(name).and_then(|r| r.value["title"].as_str()))
        .map(str::to_owned);
    Issue {
        session_id: name.trim_end_matches(".json").into(),
        reason: reason.into(),
        title,
    }
}

fn registry_tasks(blob: Option<&Blob>) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    let Some(blob) = blob else {
        return Ok(ids);
    };
    let value: Value = decode(blob)?;
    if value.as_array().map(Vec::is_empty).unwrap_or(false) {
        return Ok(ids);
    }
    let object = value.as_object().ok_or("TASK_REGISTRY_REQUIRES_REVIEW")?;
    if object.is_empty() {
        return Ok(ids);
    }
    let tasks = object
        .get("scheduledTasks")
        .and_then(Value::as_array)
        .ok_or("TASK_REGISTRY_REQUIRES_REVIEW")?;
    for task in tasks {
        let id = task
            .get("notifySessionId")
            .and_then(Value::as_str)
            .ok_or("TASK_REGISTRY_REQUIRES_REVIEW")?;
        check(session_id(id), "TASK_REGISTRY_REQUIRES_REVIEW")?;
        ids.insert(id.into());
    }
    Ok(ids)
}

fn relationship_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) if session_id(s) => {
            out.insert(s.clone());
        }
        Value::Array(a) => {
            for v in a {
                relationship_ids(v, out);
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                relationship_ids(v, out);
            }
        }
        _ => (),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct Transcript {
    path: PathBuf,
    hash: String,
    size: u64,
    id: FileId,
    modified: (i64, i64),
    changed: (i64, i64),
}
type Transcripts = BTreeMap<String, Vec<Transcript>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TranscriptMetadata {
    path: PathBuf,
    size: u64,
    id: FileId,
    modified: (i64, i64),
    changed: (i64, i64),
}

#[cfg(unix)]
fn transcript_metadata(path: PathBuf, file: &File) -> Result<TranscriptMetadata> {
    let meta = file.metadata().map_err(io_error)?;
    Ok(TranscriptMetadata {
        path,
        size: meta.len(),
        id: file_id(file)?,
        modified: (meta.mtime(), meta.mtime_nsec()),
        changed: (meta.ctime(), meta.ctime_nsec()),
    })
}

#[cfg(not(unix))]
fn transcript_metadata(_path: PathBuf, _file: &File) -> Result<TranscriptMetadata> {
    Err("UNSUPPORTED_PLATFORM".into())
}

fn expected_transcript_metadata(index: &Transcripts) -> BTreeMap<String, Vec<TranscriptMetadata>> {
    index
        .iter()
        .map(|(key, items)| {
            (
                key.clone(),
                items
                    .iter()
                    .map(|item| TranscriptMetadata {
                        path: item.path.clone(),
                        size: item.size,
                        id: item.id.clone(),
                        modified: item.modified,
                        changed: item.changed,
                    })
                    .collect(),
            )
        })
        .collect()
}

// Directory timestamps witness additions/removals (including a second copy of
// a referenced transcript). Known files are checked separately for in-place
// writes. Full namespace images and transcript hashes remain commit gates.
struct FastWitness {
    sources: BTreeMap<PathBuf, TranscriptMetadata>,
    pool: BTreeMap<PathBuf, TranscriptMetadata>,
    target_id: FileId,
}

fn pool_witness(pool: &Path) -> Result<BTreeMap<PathBuf, TranscriptMetadata>> {
    let root = Directory::open(pool, false)?;
    let before = transcript_metadata(pool.to_path_buf(), &root.0)?;
    let mut result = BTreeMap::new();
    for name in root.names()? {
        if root.is_directory(&name)? {
            let child = root.child(&name)?;
            let path = pool.join(name);
            result.insert(path.clone(), transcript_metadata(path, &child.0)?);
        }
    }
    check(
        transcript_metadata(pool.to_path_buf(), &root.0)? == before,
        "TRANSCRIPT_DRIFT",
    )?;
    result.insert(pool.to_path_buf(), before);
    Ok(result)
}

fn source_witness(
    inputs: &BTreeMap<PathBuf, Records>,
    target: &Path,
) -> Result<BTreeMap<PathBuf, TranscriptMetadata>> {
    let mut result = BTreeMap::new();
    for (path, rows) in inputs {
        if path == target {
            continue;
        }
        let dir = Directory::open(path, false)?;
        let before = transcript_metadata(path.clone(), &dir.0)?;
        let names: BTreeSet<_> = dir
            .names()?
            .into_iter()
            .filter(|name| name.starts_with("local_") && name.ends_with(".json"))
            .collect();
        check(names == rows.keys().cloned().collect(), "SNAPSHOT_DRIFT")?;
        for (name, row) in rows {
            let file = dir.file(name, libc::O_RDONLY)?;
            check(
                transcript_metadata(path.join(name), &file)? == row.blob.metadata,
                "SNAPSHOT_DRIFT",
            )?;
        }
        check(
            transcript_metadata(path.clone(), &dir.0)? == before,
            "SNAPSHOT_DRIFT",
        )?;
        result.insert(path.clone(), before);
    }
    Ok(result)
}

fn check_fast_witness(roots: &Roots, plan: &Plan, witness: &FastWitness) -> Result<()> {
    for (path, expected) in &witness.sources {
        let dir = Directory::open(path, false)?;
        check(
            transcript_metadata(path.clone(), &dir.0)? == *expected,
            "SNAPSHOT_DRIFT",
        )?;
        for (name, row) in &plan.inputs[path] {
            let file = dir.file(name, libc::O_RDONLY)?;
            check(
                transcript_metadata(path.join(name), &file)? == row.blob.metadata,
                "SNAPSHOT_DRIFT",
            )?;
        }
        check(
            transcript_metadata(path.clone(), &dir.0)? == *expected,
            "SNAPSHOT_DRIFT",
        )?;
    }
    let root = Directory::open(&roots.pool, false)?;
    check(
        transcript_metadata(roots.pool.clone(), &root.0)? == witness.pool[&roots.pool],
        "TRANSCRIPT_DRIFT",
    )?;
    let mut files: BTreeMap<PathBuf, Vec<TranscriptMetadata>> = BTreeMap::new();
    for metadata in expected_transcript_metadata(&plan.transcripts)
        .into_values()
        .flatten()
    {
        files
            .entry(metadata.path.parent().ok_or("UNSAFE_PATH")?.to_path_buf())
            .or_default()
            .push(metadata);
    }
    for (path, expected) in &witness.pool {
        if path == &roots.pool {
            continue;
        }
        let project = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("UNSAFE_PATH")?;
        let dir = root.child(project)?;
        check(
            transcript_metadata(path.clone(), &dir.0)? == *expected,
            "TRANSCRIPT_DRIFT",
        )?;
        for metadata in files.remove(path).unwrap_or_default() {
            let name = metadata
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("UNSAFE_PATH")?;
            let file = dir.file(name, libc::O_RDONLY)?;
            check(
                transcript_metadata(metadata.path.clone(), &file)? == metadata,
                "TRANSCRIPT_DRIFT",
            )?;
        }
        check(
            transcript_metadata(path.clone(), &dir.0)? == *expected,
            "TRANSCRIPT_DRIFT",
        )?;
    }
    check(files.is_empty(), "TRANSCRIPT_DRIFT")?;
    check(
        transcript_metadata(roots.pool.clone(), &root.0)? == witness.pool[&roots.pool],
        "TRANSCRIPT_DRIFT",
    )
}

// A cheap per-operation witness. Hashing every transcript at every record
// publication makes a small sidebar copy repeatedly read gigabytes of history.
// Full content hashes are still checked before the first write and at commit.
fn transcripts_metadata(
    pool: &Path,
    wanted: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<TranscriptMetadata>>> {
    let mut out: BTreeMap<String, Vec<TranscriptMetadata>> =
        wanted.iter().map(|s| (s.clone(), Vec::new())).collect();
    let pool_dir = Directory::open(pool, false)?;
    let root_before = transcript_metadata(pool.to_path_buf(), &pool_dir.0)?;
    for project in pool_dir.names()? {
        if !pool_dir.is_directory(&project)? {
            continue;
        }
        let opened = pool_dir.child(&project)?;
        let dir = pool.join(&project);
        let before = transcript_metadata(dir.clone(), &opened.0)?;
        for name in opened.names()? {
            let Some(id) = name.strip_suffix(".jsonl").filter(|s| uuid(s)) else {
                continue;
            };
            let key = id.to_ascii_lowercase();
            if !wanted.contains(&key) {
                continue;
            }
            let path = dir.join(&name);
            let file = opened.file(&name, libc::O_RDONLY)?;
            out.get_mut(&key)
                .unwrap()
                .push(transcript_metadata(path, &file)?);
        }
        check(
            transcript_metadata(dir, &opened.0)? == before
                && file_id(&pool_dir.child(&project)?.0)? == before.id,
            "TRANSCRIPT_DRIFT",
        )?;
    }
    check(
        transcript_metadata(pool.to_path_buf(), &pool_dir.0)? == root_before
            && file_id(&Directory::open(pool, false)?.0)? == root_before.id,
        "TRANSCRIPT_DRIFT",
    )?;
    for items in out.values_mut() {
        items.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(out)
}

fn transcripts(pool: &Path, wanted: &BTreeSet<String>) -> Result<Transcripts> {
    let mut out: Transcripts = wanted.iter().map(|s| (s.clone(), Vec::new())).collect();
    let pool_dir = Directory::open(pool, false)?;
    let root_before = transcript_metadata(pool.to_path_buf(), &pool_dir.0)?;
    for project in pool_dir.names()? {
        if !pool_dir.is_directory(&project)? {
            continue;
        }
        let opened = pool_dir.child(&project)?;
        let dir = pool.join(&project);
        let dir_before = transcript_metadata(dir.clone(), &opened.0)?;
        for name in opened.names()? {
            let Some(id) = name.strip_suffix(".jsonl").filter(|s| uuid(s)) else {
                continue;
            };
            let key = id.to_ascii_lowercase();
            if !wanted.contains(&key) {
                continue;
            }
            let path = dir.join(&name);
            let mut file = opened.file(&name, libc::O_RDONLY)?;
            let before = transcript_metadata(path.clone(), &file)?;
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut size = 0;
            loop {
                let n = file.read(&mut buffer).map_err(io_error)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                size += n as u64;
            }
            let after = transcript_metadata(path.clone(), &file)?;
            check(before.size == size && after == before, "TRANSCRIPT_DRIFT")?;
            out.get_mut(&key).unwrap().push(Transcript {
                path,
                hash: format!("{:x}", hasher.finalize()),
                size,
                id: before.id,
                modified: before.modified,
                changed: before.changed,
            });
        }
        check(
            transcript_metadata(dir, &opened.0)? == dir_before
                && file_id(&pool_dir.child(&project)?.0)? == dir_before.id,
            "TRANSCRIPT_DRIFT",
        )?;
    }
    check(
        transcript_metadata(pool.to_path_buf(), &pool_dir.0)? == root_before
            && file_id(&Directory::open(pool, false)?.0)? == root_before.id,
        "TRANSCRIPT_DRIFT",
    )?;
    for items in out.values_mut() {
        items.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(out)
}

fn transcript_issue(index: &Transcripts, id: &str) -> Option<&'static str> {
    match index
        .get(&id.to_ascii_lowercase())
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        [] => Some("MISSING_TRANSCRIPT"),
        [one] if one.size == 0 => Some("EMPTY_TRANSCRIPT"),
        [_] => None,
        _ => Some("AMBIGUOUS_TRANSCRIPT"),
    }
}

fn rewind_refs(record: &Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    if let Some(edges) = record
        .get("rewindEdges")
        .and_then(Value::as_array)
        .filter(|edges| edges.len() <= MAX_PRIOR_IDS)
    {
        for edge in edges {
            for key in ["parent", "child"] {
                if let Some(id) = edge.get(key).and_then(Value::as_str).filter(|id| uuid(id)) {
                    refs.insert(id.to_ascii_lowercase());
                }
            }
        }
    }
    refs
}

// Desktop's exact-version loader normalizes rewind edges and model state by
// transcript ID. Copy only the known local graph shape, and only while every
// referenced branch has a unique, nonempty transcript in the shared pool.
fn rewind_issue(record: &Value, index: &Transcripts) -> Option<&'static str> {
    let raw_edges = record.get("rewindEdges");
    let raw_states = record.get("transcriptModelStates");
    let edges = match raw_edges {
        None | Some(Value::Null) => None,
        Some(Value::Array(edges)) => Some(edges),
        Some(Value::Object(empty)) if empty.is_empty() => None,
        _ => return Some("UNSUPPORTED_HISTORY_STATE"),
    };
    let states = match raw_states {
        None | Some(Value::Null) => None,
        Some(Value::Object(states)) => Some(states),
        Some(Value::Array(empty)) if empty.is_empty() => None,
        _ => return Some("UNSUPPORTED_HISTORY_STATE"),
    };
    let Some(edges) = edges.filter(|edges| !edges.is_empty()) else {
        return states
            .filter(|states| !states.is_empty())
            .map(|_| "UNSUPPORTED_HISTORY_STATE");
    };
    if edges.len() > MAX_PRIOR_IDS {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    let mut ids = BTreeSet::new();
    let mut children = BTreeSet::new();
    let mut refs = BTreeSet::new();
    for edge in edges {
        let Some(edge) = edge.as_object() else {
            return Some("UNSUPPORTED_HISTORY_STATE");
        };
        if !edge.keys().all(|key| {
            ["id", "parent", "child", "at", "cwd", "forkPoint", "anchor"].contains(&key.as_str())
        }) {
            return Some("UNSUPPORTED_HISTORY_STATE");
        }
        let Some(id) = edge.get("id").and_then(Value::as_str).filter(|id| uuid(id)) else {
            return Some("UNSUPPORTED_HISTORY_STATE");
        };
        let Some(parent) = edge
            .get("parent")
            .and_then(Value::as_str)
            .filter(|id| uuid(id))
        else {
            return Some("UNSUPPORTED_HISTORY_STATE");
        };
        let Some(child) = edge
            .get("child")
            .and_then(Value::as_str)
            .filter(|id| uuid(id))
        else {
            return Some("UNSUPPORTED_HISTORY_STATE");
        };
        if parent == child
            || !ids.insert(id.to_ascii_lowercase())
            || !children.insert(child.to_ascii_lowercase())
            || !edge
                .get("at")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
            || edge.get("cwd").is_some_and(|value| {
                !value
                    .as_str()
                    .is_some_and(|cwd| !cwd.is_empty() && Path::new(cwd).is_absolute())
            })
            || ["forkPoint", "anchor"].iter().any(|key| {
                edge.get(*key)
                    .is_some_and(|value| !value.as_str().is_some_and(uuid))
            })
        {
            return Some("UNSUPPORTED_HISTORY_STATE");
        }
        refs.insert(parent.to_ascii_lowercase());
        refs.insert(child.to_ascii_lowercase());
    }
    if !record["cliSessionId"]
        .as_str()
        .is_some_and(|id| refs.contains(&id.to_ascii_lowercase()))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    if let Some(states) = states {
        if states.len() > refs.len() {
            return Some("UNSUPPORTED_HISTORY_STATE");
        }
        for (id, state) in states {
            let Some(state) = state.as_object() else {
                return Some("UNSUPPORTED_HISTORY_STATE");
            };
            if !uuid(id)
                || !refs.contains(&id.to_ascii_lowercase())
                || !state.keys().all(|key| {
                    [
                        "model",
                        "preRefusalModel",
                        "refusedUserMessageUuid",
                        "refusalFallbackTurnUuid",
                    ]
                    .contains(&key.as_str())
                })
                || !state
                    .get("model")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
                || ["preRefusalModel"].iter().any(|key| {
                    state
                        .get(*key)
                        .is_some_and(|v| !v.as_str().is_some_and(|s| !s.is_empty()))
                })
                || ["refusedUserMessageUuid", "refusalFallbackTurnUuid"]
                    .iter()
                    .any(|key| {
                        state
                            .get(*key)
                            .is_some_and(|value| !value.as_str().is_some_and(uuid))
                    })
            {
                return Some("UNSUPPORTED_HISTORY_STATE");
            }
        }
    }
    if refs.iter().any(|id| transcript_issue(index, id).is_some()) {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    None
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Baseline {
    version: u32,
    records: BTreeMap<String, Value>,
}

struct Plan {
    preview: Preview,
    source_dir: PathBuf,
    target_dir: PathBuf,
    source: Records,
    target: Records,
    registries: BTreeMap<PathBuf, Option<Blob>>,
    transcripts: Transcripts,
    baseline_path: PathBuf,
    baseline: Option<Blob>,
    next_baseline: Value,
    proposed: BTreeMap<String, Value>,
    catalog_mode: bool,
    inputs: BTreeMap<PathBuf, Records>,
    fast_witness: Option<FastWitness>,
}

fn namespace_paths(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
) -> Result<(PathBuf, PathBuf)> {
    disjoint(&[&roots.records, &roots.pool, &roots.state])?;
    let s = identity_key(source)?;
    let t = identity_key(target)?;
    check(s != t, "SOURCE_EQUALS_TARGET")?;
    Ok((roots.records.join(s), roots.records.join(t)))
}

fn validate_roots(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
) -> Result<(PathBuf, PathBuf)> {
    let (s, t) = namespace_paths(roots, source, target)?;
    Directory::open(&s, false)?;
    Directory::open(&t, false)?;
    Directory::open(&roots.pool, false)?;
    match Directory::open(&roots.state, false) {
        Ok(d) => d.private()?,
        Err(e) if e == "MISSING_PATH" => (),
        Err(e) => return Err(e),
    }
    Ok((s, t))
}

fn build_plan(roots: &Roots, source: &Identity, target: &Identity) -> Result<Plan> {
    build_plan_filtered(roots, source, target, None)
}

fn build_plan_filtered(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    source_only: Option<&str>,
) -> Result<Plan> {
    let (source_dir, target_dir) = validate_roots(roots, source, target)?;
    let src = records(&source_dir)?;
    let dst = records(&target_dir)?;
    check(!src.is_empty(), "SOURCE_SIDEBAR_EMPTY")?;
    if let Some(name) = source_only {
        check(
            record_name(name) && src.contains_key(name) && !dst.contains_key(name),
            "INVALID_SOURCE_ONLY_SELECTION",
        )?;
    }
    let baseline_path = baseline_path(roots, source, target)?;
    let baseline = read_blob(&baseline_path)?;
    let mut next: Baseline = baseline
        .as_ref()
        .map(decode)
        .transpose()?
        .unwrap_or(Baseline {
            version: 1,
            records: BTreeMap::new(),
        });
    let current_baseline = baseline.as_ref().map(decode::<Baseline>).transpose()?;
    check(
        next.version == 1
            && next
                .records
                .iter()
                .all(|(n, v)| record_name(n) && v.is_object()),
        "INVALID_BASELINE",
    )?;
    let mut registries = BTreeMap::new();
    let mut tasks = BTreeSet::new();
    for dir in [&source_dir, &target_dir] {
        let path = dir.join("scheduled-tasks.json");
        let blob = read_blob(&path)?;
        tasks.extend(registry_tasks(blob.as_ref())?);
        registries.insert(path, blob);
    }
    for record in src.values().chain(dst.values()) {
        for key in [
            "spawnedFrom",
            "dispatchParentId",
            "dispatchParentOrigin",
            "notifySessionId",
        ] {
            if let Some(value) = record.value.get(key) {
                relationship_ids(value, &mut tasks);
            }
        }
    }
    let mut related_names = BTreeSet::new();
    if let Some(name) = source_only {
        related_names.insert(name.to_owned());
        if let Some(parent) = src[name].value["forkedFromSessionId"].as_str() {
            related_names.insert(format!("{parent}.json"));
        }
    }
    let mut wanted = BTreeSet::new();
    for (name, r) in src.iter().chain(dst.iter()) {
        if source_only.is_some() && !related_names.contains(name) {
            continue;
        }
        if let Some(id) = r.value["cliSessionId"].as_str().filter(|s| uuid(s)) {
            wanted.insert(id.to_ascii_lowercase());
        }
        wanted.extend(prior_ids(&r.value));
        wanted.extend(rewind_refs(&r.value));
    }
    let index = transcripts(&roots.pool, &wanted)?;
    let mut issues = BTreeMap::<String, String>::new();
    let mut warnings = Vec::new();
    let mut proposed = BTreeMap::new();
    let mut replaced_branches = BTreeSet::new();
    // Include excluded destination-only parents in the closure too.
    let mut excluded = BTreeSet::new();
    for (name, r) in &src {
        if exclusion(&r.value, &tasks, &r.value)
            .or_else(|| rewind_issue(&r.value, &index))
            .is_some()
        {
            excluded.insert(name.clone());
        }
    }
    for (name, r) in &dst {
        let activity = src.get(name).map(|s| &s.value).unwrap_or(&r.value);
        if target_exclusion(&r.value, &tasks, activity)
            .or_else(|| rewind_issue(&r.value, &index))
            .is_some()
        {
            excluded.insert(name.clone());
        }
    }
    for (name, s) in &src {
        if source_only.is_some_and(|selected| selected != name) {
            continue;
        }
        let t = dst.get(name);
        let reason = exclusion(&s.value, &tasks, &s.value)
            .or_else(|| rewind_issue(&s.value, &index))
            .map(str::to_owned)
            .or_else(|| {
                t.and_then(|t| {
                    target_exclusion(&t.value, &tasks, &s.value)
                        .or_else(|| rewind_issue(&t.value, &index))
                })
                .map(|s| format!("TARGET_{s}"))
            });
        if let Some(reason) = reason {
            issues.insert(name.clone(), reason);
            continue;
        }
        validate_candidate(&s.value)?;
        if let Some(t) = t {
            validate_candidate(&t.value)?;
        }
        if let Some(reason) = transcript_issue(&index, s.value["cliSessionId"].as_str().unwrap()) {
            issues.insert(name.clone(), reason.into());
            continue;
        }
        let base = next.records.get(name);
        if t.is_none() && base.is_some() {
            issues.insert(name.clone(), "PREVIOUSLY_SYNCED_TARGET_MISSING".into());
            continue;
        }
        let p = portable(&s.value);
        let rollover = t.is_some_and(|t| source_rollover(&s.value, &t.value));
        let merged = if let Some(t) = t {
            let target_portable = portable(&t.value);
            if rollover {
                merge_selected_history(&p, &target_portable, base)
            } else {
                merge(&p, &target_portable, base)
            }
        } else {
            Some(p)
        };
        let Some(merged) = merged else {
            issues.insert(name.clone(), "DIVERGENT_METADATA".into());
            continue;
        };
        let result = materialize(&merged, &s.value, t.map(|r| &r.value));
        // Quota-marked candidates are eligible only if this particular projection
        // actually retires their quit state. A retained target pointer can prevent
        // same-pointer cleanup even when the source has newer activity.
        if quit_marked(&result)
            || ["pendingFirstStart", "armedWorkAtQuit"]
                .iter()
                .any(|k| result.get(*k).is_some())
        {
            issues.insert(name.clone(), "TARGET_UNFINISHED_SESSION".into());
            excluded.insert(name.clone());
            continue;
        }
        if let Some(reason) = transcript_issue(&index, result["cliSessionId"].as_str().unwrap()) {
            issues.insert(name.clone(), reason.into());
            continue;
        }
        if let Some(reason) = rewind_issue(&result, &index) {
            issues.insert(name.clone(), reason.into());
            excluded.insert(name.clone());
            continue;
        }
        json_bytes(&result)?;
        proposed.insert(name.clone(), result);
        if rollover {
            replaced_branches.insert(name.clone());
        }
    }
    loop {
        let mut removals = Vec::new();
        for (name, value) in &proposed {
            if let Some(parent) = value["forkedFromSessionId"].as_str() {
                let parent = format!("{parent}.json");
                if excluded.contains(&parent)
                    || (src.contains_key(&parent)
                        && !dst.contains_key(&parent)
                        && !proposed.contains_key(&parent))
                {
                    removals.push(name.clone());
                }
            }
        }
        if removals.is_empty() {
            break;
        }
        for name in removals {
            proposed.remove(&name);
            replaced_branches.remove(&name);
            excluded.insert(name.clone());
            issues.insert(name, "EXCLUDED_PARENT".into());
        }
    }
    let skipped_missing = issues
        .keys()
        .filter(|name| !dst.contains_key(*name))
        .count();
    let skipped_stale = issues
        .keys()
        .filter(|name| {
            src.get(*name)
                .zip(dst.get(*name))
                .is_some_and(|(source, target)| {
                    source.value["cliSessionId"] != target.value["cliSessionId"]
                })
        })
        .count();
    for name in &replaced_branches {
        warnings.push(record_issue(name, "SOURCE_BRANCH_SELECTED", &src, &dst));
    }
    let mut preview = Preview {
        fingerprint: String::new(),
        baseline_changed: false,
        created: 0,
        updated: 0,
        unchanged: 0,
        missing: skipped_missing,
        stale: skipped_stale,
        replaced_branches: replaced_branches.len(),
        issues: issues
            .into_iter()
            .map(|(n, reason)| record_issue(&n, &reason, &src, &dst))
            .collect(),
        warnings: Vec::new(),
        quota_pauses_cleared: 0,
        preserved_branches: 0,
    };
    for (name, after) in &proposed {
        if let Some(parent) = after["forkedFromSessionId"].as_str() {
            let parent = format!("{parent}.json");
            if !src.contains_key(&parent) && !dst.contains_key(&parent) {
                warnings.push(record_issue(name, "PREEXISTING_MISSING_PARENT", &src, &dst));
            }
        }
        let mut priors = prior_ids(&src[name].value);
        priors.extend(prior_ids(after));
        if let Some(target) = dst.get(name) {
            priors.extend(prior_ids(&target.value));
        }
        // Only the checked source/proposed active IDs are exempt. An old target
        // pointer replaced by this rollover may now be a missing historical ID.
        for current in [&src[name].value, after] {
            if let Some(id) = current["cliSessionId"].as_str() {
                priors.remove(&id.to_ascii_lowercase());
            }
        }
        let warning_codes: BTreeSet<_> = priors
            .iter()
            .filter_map(|id| transcript_issue(&index, id))
            .collect();
        for code in warning_codes {
            let reason = match code {
                "MISSING_TRANSCRIPT" => "PRIOR_TRANSCRIPT_MISSING",
                "EMPTY_TRANSCRIPT" => "PRIOR_TRANSCRIPT_EMPTY",
                "AMBIGUOUS_TRANSCRIPT" => "PRIOR_TRANSCRIPT_AMBIGUOUS",
                _ => unreachable!(),
            };
            warnings.push(record_issue(name, reason, &src, &dst));
        }
        let source_portable = portable(&src[name].value);
        let after_portable = portable(after);
        // Advance only common groups; retain divergent groups for the reverse run.
        let mut base = next
            .records
            .get(name)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for key in GROUPS.iter().map(|g| g.0).chain(SCALARS.iter().copied()) {
            if source_portable.get(key) == after_portable.get(key) {
                base.remove(key);
                if let Some(value) = source_portable.get(key) {
                    base.insert(key.into(), value.clone());
                }
            }
        }
        next.records.insert(name.clone(), Value::Object(base));
        match dst.get(name) {
            None => preview.created += 1,
            Some(before) if &before.value == after => preview.unchanged += 1,
            Some(before) => {
                preview.updated += 1;
                if stale_quota(&before.value, &src[name].value)
                    && after.get("error").is_none()
                    && before.value["cliSessionId"] == after["cliSessionId"]
                {
                    preview.quota_pauses_cleared += 1;
                }
            }
        }
    }
    preview.warnings = warnings;
    preview.baseline_changed = current_baseline.as_ref() != Some(&next);
    proposed.retain(|n, v| dst.get(n).map(|t| &t.value != v).unwrap_or(true));
    let registry_stamps: BTreeMap<_, _> = registries
        .iter()
        .map(|(p, b)| (p, b.as_ref().map(|b| &b.stamp)))
        .collect();
    let mut fingerprint_inputs = json!({"version":1, "source":identity_key(source)?, "target":identity_key(target)?,
        "sourceDir":source_dir, "targetDir":target_dir, "pool":roots.pool, "state":roots.state,
        "sourceRecords":record_stamps(&src), "targetRecords":record_stamps(&dst), "registries":registry_stamps,
        "baseline":baseline.as_ref().map(|b| &b.stamp), "transcripts":index});
    if let Some(name) = source_only {
        fingerprint_inputs["sourceOnly"] = json!(name);
    }
    preview.fingerprint = digest(&json_bytes(&fingerprint_inputs)?);
    Ok(Plan {
        preview,
        source_dir,
        target_dir,
        source: src,
        target: dst,
        registries,
        transcripts: index,
        baseline_path,
        baseline,
        next_baseline: serde_json::to_value(next).map_err(|_| "INVALID_BASELINE")?,
        proposed,
        catalog_mode: false,
        inputs: BTreeMap::new(),
        fast_witness: None,
    })
}

pub fn preview(roots: &Roots, source: &Identity, target: &Identity) -> Result<Preview> {
    no_pending(roots, None)?;
    Ok(build_plan(roots, source, target)?.preview)
}

pub fn preview_continuity(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
) -> Result<Preview> {
    preview_continuity_with_fields(roots, source, target, accounts, &BTreeSet::new())
}

/// Known adapter field names, independent of a Desktop release number. New
/// optional fields do not change admission; meaningful unknown native state
/// is surfaced as an affected-session issue instead of silently discarded.
pub(super) fn unknown_persisted_fields(projected: &BTreeSet<String>) -> BTreeSet<String> {
    let known: BTreeSet<String> =
        serde_json::from_str(include_str!("supported_persisted_fields.json"))
            .expect("valid adapter field inventory");
    projected.difference(&known).cloned().collect()
}

pub(super) fn preview_continuity_with_fields(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    unknown_fields: &BTreeSet<String>,
) -> Result<Preview> {
    no_pending(roots, None)?;
    Ok(
        continuity::build_plan_with_fields(roots, source, target, accounts, unknown_fields)?
            .preview,
    )
}

/// Content-free read-only timing of the actual per-publication witnesses.
/// Used by the local diagnostic example; never applies a transfer.
pub fn continuity_witness_millis(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
) -> Result<u128> {
    let plan = continuity::build_plan(roots, source, target, accounts)?;
    let expected = record_stamps(&plan.target);
    let baseline = plan.baseline.as_ref().map(|blob| blob.stamp.clone());
    let started = std::time::Instant::now();
    check_plan(
        roots,
        &plan,
        &expected,
        &baseline,
        false,
        Some(&record_metadata(&plan.target)),
    )?;
    Ok(started.elapsed().as_millis())
}

// The journal contains only hashes/identities and validated relative operation
// names. Full preimages live in the caller-supplied recovery directory.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    name: String,
    baseline: bool,
    before: Option<Stamp>,
    after_hash: String,
    write_id: Option<FileId>,
    undo_id: Option<FileId>,
    phase: String,
    temp: String,
    undo_temp: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    summary: RunSummary,
    roots_hash: String,
    backup_id: FileId,
    backups: BTreeMap<String, String>,
    operations: Vec<Operation>,
    #[serde(default)]
    catalog_mode: bool,
}

fn roots_hash(roots: &Roots) -> Result<String> {
    Ok(digest(&json_bytes(&json!([
        roots.records,
        roots.pool,
        roots.state
    ]))?))
}

fn journal_path(roots: &Roots, id: &str) -> PathBuf {
    roots.state.join(id).join("journal.json")
}
fn save(roots: &Roots, journal: &Journal) -> Result<()> {
    atomic_json(&journal_path(roots, &journal.summary.id), journal)
}

fn load(roots: &Roots, id: &str) -> Result<Journal> {
    check(valid_run_id(id), "INVALID_RUN_ID")?;
    let journal: Journal = decode(&required_blob(&journal_path(roots, id))?)?;
    check(
        journal.version == 1
            && journal.summary.id == id
            && journal.roots_hash == roots_hash(roots)?,
        "INVALID_JOURNAL",
    )?;
    check(
        journal.summary.last_error.as_deref().is_none_or(safe_code),
        "INVALID_JOURNAL",
    )?;
    // Completed history outlives the live namespaces and transcript pool. Only
    // operations that use those paths should require the directories to exist.
    namespace_paths(roots, &journal.summary.source, &journal.summary.target)?;
    let mut names = BTreeSet::new();
    let mut baselines = 0;
    for (i, op) in journal.operations.iter().enumerate() {
        check(
            names.insert((op.baseline, op.name.clone())),
            "INVALID_JOURNAL",
        )?;
        if op.baseline {
            baselines += 1;
            check(
                op.name == "baseline.json" && i + 1 == journal.operations.len(),
                "INVALID_JOURNAL",
            )?;
        } else {
            check(record_name(&op.name), "INVALID_JOURNAL")?;
        }
        check(
            [&op.temp, &op.undo_temp].iter().all(|s| {
                s.strip_prefix(".handoff-")
                    .and_then(|s| s.strip_suffix(".tmp"))
                    .map(uuid)
                    .unwrap_or(false)
            }),
            "INVALID_JOURNAL",
        )?;
        check(
            op.after_hash.len() == 64 && op.after_hash.bytes().all(|c| c.is_ascii_hexdigit()),
            "INVALID_JOURNAL",
        )?;
        check(
            [
                "planned",
                "write_pending",
                "written",
                "undo_pending",
                "undone",
            ]
            .contains(&op.phase.as_str()),
            "INVALID_JOURNAL",
        )?;
    }
    check(
        baselines == 1
            && [
                "prepared",
                "applying",
                "applied",
                "rolling_back",
                "rolled_back",
            ]
            .contains(&journal.summary.state.as_str()),
        "INVALID_JOURNAL",
    )?;
    for name in journal.backups.keys() {
        check(
            name == "baseline.json"
                || ["source/", "target/"].iter().any(|p| {
                    name.strip_prefix(*p)
                        .map(|n| record_name(n) || n == "scheduled-tasks.json")
                        .unwrap_or(false)
                }),
            "INVALID_JOURNAL",
        )?;
    }
    if journal.summary.state == "applied" {
        check(
            journal
                .operations
                .iter()
                .all(|op| op.phase == "written" && op.write_id.is_some()),
            "INVALID_JOURNAL",
        )?;
    }
    if journal.summary.state == "rolled_back" {
        check(
            journal.operations.iter().all(|op| op.phase == "undone"),
            "INVALID_JOURNAL",
        )?;
    }
    Ok(journal)
}

fn no_pending(roots: &Roots, except: Option<&str>) -> Result<()> {
    let dir = match Directory::open(&roots.state, false) {
        Ok(d) => d,
        Err(e) if e == "MISSING_PATH" => return Ok(()),
        Err(e) => return Err(e),
    };
    dir.private()?;
    for name in dir.names()?.into_iter().filter(|n| n.starts_with("run-")) {
        if Some(name.as_str()) == except {
            continue;
        }
        let j = load(roots, &name).map_err(|_| "PENDING_RUN")?;
        check(
            ["applied", "rolled_back"].contains(&j.summary.state.as_str()),
            "PENDING_RUN",
        )?;
    }
    Ok(())
}

pub(crate) fn require_no_pending(roots: &Roots) -> Result<()> {
    no_pending(roots, None)
}

/// Account switches must be guarded even when this process has a custom
/// Claude profile configured. A completed run may have been made under the
/// default profile, so this check deliberately does not compare roots hashes.
pub(crate) fn require_no_pending_state(state: &Path) -> Result<()> {
    let dir = match Directory::open(state, false) {
        Ok(dir) => dir,
        Err(code) if code == "MISSING_PATH" => return Ok(()),
        Err(code) => return Err(code),
    };
    dir.private()?;
    for name in dir
        .names()?
        .into_iter()
        .filter(|name| name.starts_with("run-"))
    {
        if !valid_run_id(&name) {
            return Err("PENDING_RUN".into());
        }
        let journal: Journal = required_blob(&state.join(&name).join("journal.json"))
            .and_then(|blob| decode(&blob))
            .map_err(|_| "PENDING_RUN")?;
        check(
            journal.version == 1
                && journal.summary.id == name
                && match journal.summary.state.as_str() {
                    "applied" => journal
                        .operations
                        .iter()
                        .all(|op| op.phase == "written" && op.write_id.is_some()),
                    "rolled_back" => journal.operations.iter().all(|op| op.phase == "undone"),
                    _ => false,
                },
            "PENDING_RUN",
        )?;
    }
    Ok(())
}

pub fn list_runs(roots: &Roots) -> Result<Vec<RunSummary>> {
    disjoint(&[&roots.records, &roots.pool, &roots.state])?;
    let dir = match Directory::open(&roots.state, false) {
        Ok(d) => d,
        Err(e) if e == "MISSING_PATH" => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    dir.private()?;
    let mut runs = Vec::new();
    for name in dir.names()?.into_iter().filter(|n| n.starts_with("run-")) {
        runs.push(load(roots, &name)?.summary);
    }
    runs.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(runs)
}

pub fn record_failure(roots: &Roots, run_id: &str, code: &str) -> Result<()> {
    let safe = if safe_code(code) { code } else { "UNKNOWN" };
    let _lock = Lock::acquire(&roots.state)?;
    let mut journal = load(roots, run_id)?;
    check(
        journal.summary.state != "applied" && journal.summary.state != "rolled_back",
        "INVALID_JOURNAL",
    )?;
    journal.summary.last_error = Some(safe.into());
    save(roots, &journal)
}

fn operation_path(roots: &Roots, j: &Journal, op: &Operation) -> Result<PathBuf> {
    if op.baseline {
        if j.catalog_mode {
            Ok(continuity::state_path(roots))
        } else {
            baseline_path(roots, &j.summary.source, &j.summary.target)
        }
    } else {
        Ok(roots
            .records
            .join(identity_key(&j.summary.target)?)
            .join(&op.name))
    }
}

fn backup_key(op: &Operation) -> String {
    if op.baseline {
        "baseline.json".into()
    } else {
        format!("target/{}", op.name)
    }
}

fn check_plan(
    roots: &Roots,
    plan: &Plan,
    expected: &BTreeMap<String, Stamp>,
    baseline: &Option<Stamp>,
    full_transcript_check: bool,
    target_metadata: Option<&BTreeMap<String, TranscriptMetadata>>,
) -> Result<()> {
    if let Some(witness) = &plan.fast_witness {
        if !full_transcript_check {
            check_fast_witness(roots, plan, witness)?;
            let metadata = target_metadata.ok_or("SNAPSHOT_DRIFT")?;
            check(
                metadata.len() == expected.len()
                    && metadata
                        .iter()
                        .all(|(name, m)| expected.get(name).is_some_and(|s| s.id == m.id)),
                "SNAPSHOT_DRIFT",
            )?;
            check_record_metadata(&plan.target_dir, &witness.target_id, metadata)?;
            for (path, blob) in &plan.registries {
                check(
                    stamp(path)? == blob.as_ref().map(|b| b.stamp.clone()),
                    "REGISTRY_DRIFT",
                )?;
            }
            return check(stamp(&plan.baseline_path)? == *baseline, "BASELINE_DRIFT");
        }
    }
    for (dir, snapshot) in &plan.inputs {
        if dir != &plan.target_dir {
            check(
                record_stamps(&records(dir)?) == record_stamps(snapshot),
                "SNAPSHOT_DRIFT",
            )?;
        }
    }
    check(
        record_stamps(&records(&plan.source_dir)?) == record_stamps(&plan.source),
        "SNAPSHOT_DRIFT",
    )?;
    check(
        record_stamps(&records(&plan.target_dir)?) == *expected,
        "SNAPSHOT_DRIFT",
    )?;
    for (path, blob) in &plan.registries {
        check(
            stamp(path)? == blob.as_ref().map(|b| b.stamp.clone()),
            "REGISTRY_DRIFT",
        )?;
    }
    check(stamp(&plan.baseline_path)? == *baseline, "BASELINE_DRIFT")?;
    let wanted = plan.transcripts.keys().cloned().collect();
    if full_transcript_check {
        check(
            transcripts(&roots.pool, &wanted)? == plan.transcripts,
            "TRANSCRIPT_DRIFT",
        )
    } else {
        check(
            transcripts_metadata(&roots.pool, &wanted)?
                == expected_transcript_metadata(&plan.transcripts),
            "TRANSCRIPT_DRIFT",
        )
    }
}

fn stage(path: &Path, name: &str, bytes: &[u8]) -> Result<FileId> {
    exclusive(&path.with_file_name(name), bytes)
}

fn publish_operation(
    path: &Path,
    temp: &str,
    expected: &Option<Stamp>,
    after: &Stamp,
) -> Result<TranscriptMetadata> {
    let (dir, name) = parent(path)?;
    check(stamp(path)? == *expected, "EXPECTED_IMAGE_MISMATCH")?;
    check(
        stamp(&path.with_file_name(temp))? == Some(after.clone()),
        "STAGING_DRIFT",
    )?;
    dir.publish(temp, &name, expected.is_some())?;
    dir.unlink(temp)?;
    let written = required_blob(path)?;
    check(written.stamp == *after, "POST_IMAGE_MISMATCH")?;
    Ok(written.metadata)
}

pub fn apply(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
) -> Result<RunSummary> {
    apply_observed(
        roots,
        source,
        target,
        expected_fingerprint,
        backup_dir,
        guard,
        None,
        &mut |_| {},
    )
}

pub fn apply_observed(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
) -> Result<RunSummary> {
    apply_observed_filtered(
        roots,
        source,
        target,
        expected_fingerprint,
        backup_dir,
        guard,
        quick_guard,
        on_journal_published,
        None,
    )
}

pub fn apply_continuity_observed(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
) -> Result<RunSummary> {
    apply_planned(
        roots,
        source,
        target,
        expected_fingerprint,
        backup_dir,
        guard,
        quick_guard,
        on_journal_published,
        None,
        Some(accounts),
        None,
        &mut |_, _, _| {},
    )
}

pub fn apply_continuity_with_progress(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<RunSummary> {
    apply_continuity_with_fields_and_progress(
        roots,
        source,
        target,
        accounts,
        &BTreeSet::new(),
        expected_fingerprint,
        backup_dir,
        guard,
        quick_guard,
        on_journal_published,
        progress,
    )
}

pub(super) fn apply_continuity_with_fields_and_progress(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    unknown_fields: &BTreeSet<String>,
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<RunSummary> {
    apply_planned(
        roots,
        source,
        target,
        expected_fingerprint,
        backup_dir,
        guard,
        quick_guard,
        on_journal_published,
        None,
        Some(accounts),
        Some(unknown_fields),
        progress,
    )
}

fn apply_observed_filtered(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
    source_only: Option<&str>,
) -> Result<RunSummary> {
    apply_planned(
        roots,
        source,
        target,
        expected_fingerprint,
        backup_dir,
        guard,
        quick_guard,
        on_journal_published,
        source_only,
        None,
        None,
        &mut |_, _, _| {},
    )
}

fn apply_planned(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    expected_fingerprint: &str,
    backup_dir: &Path,
    guard: &mut dyn FnMut() -> Result<()>,
    mut quick_guard: Option<&mut dyn FnMut() -> Result<()>>,
    on_journal_published: &mut dyn FnMut(&str),
    source_only: Option<&str>,
    accounts: Option<&[Identity]>,
    unknown_fields: Option<&BTreeSet<String>>,
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<RunSummary> {
    validate_roots(roots, source, target)?;
    disjoint(&[&roots.records, &roots.pool, &roots.state, backup_dir])?;
    let _lock = Lock::acquire(&roots.state)?;
    no_pending(roots, None)?;
    guard_call(guard)?;
    let mut plan = match accounts {
        Some(accounts) => continuity::build_plan_with_fields(
            roots,
            source,
            target,
            accounts,
            unknown_fields.unwrap_or(&BTreeSet::new()),
        )?,
        None => build_plan_filtered(roots, source, target, source_only)?,
    };
    check(
        plan.preview.fingerprint == expected_fingerprint,
        "PREVIEW_CHANGED",
    )?;
    if plan.catalog_mode {
        check(
            plan.preview.missing == 0
                && plan.preview.stale == 0
                && plan.preview.replaced_branches == 0,
            "UNRESOLVED_SOURCE_ROWS",
        )?;
        // Preview uses inode/size/time witnesses. Hash once here, then at commit;
        // no repeated multi-gigabyte reads while publishing sidebar records.
        let wanted = plan.transcripts.keys().cloned().collect();
        progress("hashing", 0, plan.transcripts.len());
        let hashed = transcripts(&roots.pool, &wanted)?;
        check(
            expected_transcript_metadata(&hashed)
                == expected_transcript_metadata(&plan.transcripts),
            "TRANSCRIPT_DRIFT",
        )?;
        plan.transcripts = hashed;
        // Full input images are checked once before any publication; per-row
        // checks bind inode/size/mtime/ctime and directory topology. Commit still
        // rereads every source image and hashes every referenced transcript.
        for (dir, rows) in &plan.inputs {
            check(
                record_stamps(&records(dir)?) == record_stamps(rows),
                "SNAPSHOT_DRIFT",
            )?;
        }
    }
    if source_only.is_some() {
        check(
            plan.preview.created == 1
                && plan.preview.updated == 0
                && plan.preview.missing == 0
                && plan.preview.stale == 0
                && plan.preview.replaced_branches == 0
                && plan.preview.issues.is_empty()
                && plan.proposed.len() == 1,
            "CANARY_NOT_CLEAN",
        )?;
    }
    progress("backup", 0, plan.proposed.len());
    let backup = Directory::open(backup_dir, true)?;
    check(backup.names()?.is_empty(), "BACKUP_NOT_EMPTY")?;
    // The caller may supply a future path or an existing empty directory. Ensure
    // it is private before writing any record payload.
    backup.make_private()?;
    backup.private()?;
    let backup_id = file_id(&backup.0)?;
    Directory::open(&backup_dir.join("target"), true)?.private()?;
    let mut backups = BTreeMap::new();
    // Only files that may be overwritten need preimages for conditional rollback.
    for name in plan.proposed.keys() {
        if let Some(record) = plan.target.get(name) {
            let key = format!("target/{name}");
            exclusive(&backup_dir.join(&key), &record.blob.bytes)?;
            backups.insert(key, record.blob.stamp.hash.clone());
        }
    }
    if let Some(blob) = &plan.baseline {
        exclusive(&backup_dir.join("baseline.json"), &blob.bytes)?;
        backups.insert("baseline.json".into(), blob.stamp.hash.clone());
    }
    let mut operations = Vec::new();
    let mut payloads = Vec::new();
    let operation = |name: String, baseline: bool, before: Option<Stamp>, bytes: &[u8]| Operation {
        name,
        baseline,
        before,
        after_hash: digest(bytes),
        write_id: None,
        undo_id: None,
        phase: "planned".into(),
        temp: format!(".handoff-{}.tmp", Uuid::new_v4()),
        undo_temp: format!(".handoff-{}.tmp", Uuid::new_v4()),
    };
    for (name, value) in &plan.proposed {
        let bytes = json_bytes(value)?;
        operations.push(operation(
            name.clone(),
            false,
            plan.target.get(name).map(|r| r.blob.stamp.clone()),
            &bytes,
        ));
        payloads.push(bytes);
    }
    let baseline_bytes = json_bytes(&plan.next_baseline)?;
    // Even an unchanged baseline participates: rollback must reject a later run's
    // replacement baseline before undoing any earlier record writes.
    operations.push(operation(
        "baseline.json".into(),
        true,
        plan.baseline.as_ref().map(|b| b.stamp.clone()),
        &baseline_bytes,
    ));
    payloads.push(baseline_bytes);
    let id = format!("run-{}", Uuid::new_v4());
    let mut journal = Journal {
        version: 1,
        catalog_mode: plan.catalog_mode,
        roots_hash: roots_hash(roots)?,
        backup_id,
        backups,
        operations,
        summary: RunSummary {
            id: id.clone(),
            state: "prepared".into(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "CLOCK_ERROR")?
                .as_millis()
                .try_into()
                .map_err(|_| "CLOCK_ERROR")?,
            created: plan.preview.created,
            updated: plan.preview.updated,
            skipped_missing: plan.preview.missing,
            skipped_stale: plan.preview.stale,
            replaced_branches: plan.preview.replaced_branches,
            source: source.clone(),
            target: target.clone(),
            backup_dir: backup_dir.to_str().ok_or("UNSAFE_PATH")?.into(),
            last_error: None,
        },
    };
    // Publish the complete prepared journal before exposing a run directory.
    // A crash during backup preparation has no metadata side effects or pending run.
    let staging = roots.state.join(format!(".prepare-{}", Uuid::new_v4()));
    Directory::open(&staging, true)?.private()?;
    atomic_json(&staging.join("journal.json"), &journal)?;
    rename_directory(&staging, &roots.state.join(&id), &mut || {
        on_journal_published(&id)
    })?;
    let mut expected = record_stamps(&plan.target);
    let mut target_metadata = record_metadata(&plan.target);
    let mut baseline = plan.baseline.as_ref().map(|b| b.stamp.clone());
    for (index, bytes) in payloads.iter().enumerate() {
        progress("writing", index, payloads.len());
        journal.summary.state = "applying".into();
        save(roots, &journal)?;
        guard_call_quick(guard, &mut quick_guard)?;
        check_plan(
            roots,
            &plan,
            &expected,
            &baseline,
            false,
            Some(&target_metadata),
        )?;
        verify_backups(&journal)?;
        let path = operation_path(roots, &journal, &journal.operations[index])?;
        if journal.operations[index].baseline {
            Directory::open(path.parent().unwrap(), true)?.private()?;
        }
        let owner = stage(&path, &journal.operations[index].temp, bytes)?;
        journal.operations[index].write_id = Some(owner.clone());
        journal.operations[index].phase = "write_pending".into();
        save(roots, &journal)?;
        guard_call_quick(guard, &mut quick_guard)?;
        check_plan(
            roots,
            &plan,
            &expected,
            &baseline,
            false,
            Some(&target_metadata),
        )?;
        verify_backups(&journal)?;
        let op = &journal.operations[index];
        let after = Stamp {
            hash: op.after_hash.clone(),
            id: owner,
        };
        let written_metadata = publish_operation(&path, &op.temp, &op.before, &after)?;
        if op.baseline {
            baseline = Some(after);
        } else {
            expected.insert(op.name.clone(), after);
            target_metadata.insert(op.name.clone(), written_metadata);
        }
        journal.operations[index].phase = "written".into();
        save(roots, &journal)?;
    }
    progress("verifying", payloads.len(), payloads.len());
    guard_call(guard)?;
    check_plan(roots, &plan, &expected, &baseline, true, None)?;
    verify_backups(&journal)?;
    journal.summary.state = "applied".into();
    save(roots, &journal)?;
    Ok(journal.summary)
}

fn rename_directory(from: &Path, to: &Path, on_published: &mut dyn FnMut()) -> Result<()> {
    let (dir, name) = parent(from)?;
    check(from.parent() == to.parent(), "UNSAFE_PATH")?;
    #[cfg(unix)]
    {
        let source = std::ffi::CString::new(name).map_err(|_| "UNSAFE_PATH")?;
        let target = std::ffi::CString::new(
            to.file_name()
                .and_then(|s| s.to_str())
                .ok_or("UNSAFE_PATH")?,
        )
        .map_err(|_| "UNSAFE_PATH")?;
        // A freshly generated UUID names each run; an existing directory is refused.
        match Directory::open(to, false) {
            Err(e) if e == "MISSING_PATH" => (),
            _ => return Err("ALREADY_EXISTS".into()),
        }
        check(
            unsafe {
                libc::renameat(
                    dir.0.as_raw_fd(),
                    source.as_ptr(),
                    dir.0.as_raw_fd(),
                    target.as_ptr(),
                )
            } == 0,
            "PUBLISH_FAILED",
        )?;
        // From this point a durable run path is visible. A later directory
        // sync error must still reach the caller's recovery path.
        on_published();
        dir.sync()
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, name, on_published);
        Err("UNSUPPORTED_PLATFORM".into())
    }
}

fn verify_backups(journal: &Journal) -> Result<BTreeMap<String, Vec<u8>>> {
    let path = Path::new(&journal.summary.backup_dir);
    let dir = Directory::open(path, false).map_err(|_| "BACKUP_UNAVAILABLE")?;
    dir.private()?;
    check(file_id(&dir.0)? == journal.backup_id, "BACKUP_REPLACED")?;
    let mut bytes = BTreeMap::new();
    let mut children: BTreeMap<String, Directory> = BTreeMap::new();
    for (name, hash) in &journal.backups {
        let (opened, leaf) = if name == "baseline.json" {
            (&dir, name.as_str())
        } else {
            // Match load()'s v1 whitelist, including older source/registry
            // preimages. Root mode 0700 protects each anchored child.
            let (prefix, leaf) = name.split_once('/').ok_or("BACKUP_MISMATCH")?;
            check(
                ["source", "target"].contains(&prefix)
                    && (record_name(leaf) || leaf == "scheduled-tasks.json"),
                "BACKUP_MISMATCH",
            )?;
            if !children.contains_key(prefix) {
                children.insert(
                    prefix.into(),
                    dir.child(prefix).map_err(|_| "BACKUP_UNAVAILABLE")?,
                );
            }
            (&children[prefix], leaf)
        };
        let blob = read_blob_from(opened, leaf, &path.join(name))
            .map_err(|_| "BACKUP_UNAVAILABLE")?
            .ok_or("BACKUP_UNAVAILABLE")?;
        check(&blob.stamp.hash == hash, "BACKUP_DRIFT")?;
        bytes.insert(name.clone(), blob.bytes);
    }
    for (name, child) in &children {
        check_backup_child(&dir, name, child)?;
    }
    check(
        file_id(&Directory::open(path, false)?.0)? == journal.backup_id,
        "BACKUP_REPLACED",
    )?;
    for op in &journal.operations {
        if let Some(before) = &op.before {
            check(
                journal.backups.get(&backup_key(op)) == Some(&before.hash),
                "BACKUP_MISMATCH",
            )?;
        }
    }
    Ok(bytes)
}

fn check_backup_child(parent: &Directory, name: &str, opened: &Directory) -> Result<()> {
    check(
        file_id(&parent.child(name).map_err(|_| "BACKUP_REPLACED")?.0)? == file_id(&opened.0)?,
        "BACKUP_REPLACED",
    )
}

// A later, fully rolled-back transfer restores the pair baseline from its
// preimage with a new inode. Follow only journaled before -> undo inode edges;
// equal bytes alone must not make an unrelated replacement look owned.
fn owned_post_image(
    roots: &Roots,
    journal: &Journal,
    op: &Operation,
    current: &Stamp,
) -> Result<bool> {
    if current.hash != op.after_hash {
        return Ok(false);
    }
    let Some(write_id) = &op.write_id else {
        return Ok(false);
    };
    let original = Stamp {
        hash: op.after_hash.clone(),
        id: write_id.clone(),
    };
    if current == &original {
        return Ok(true);
    }
    let path = operation_path(roots, journal, op)?;
    let mut reachable = vec![original];
    let mut edges = Vec::new();
    for run in list_runs(roots)? {
        if run.id == journal.summary.id
            || run.state != "rolled_back"
            || run.created_at < journal.summary.created_at
        {
            continue;
        }
        let later = load(roots, &run.id)?;
        let Some(later_op) = later
            .operations
            .iter()
            .find(|candidate| candidate.baseline == op.baseline && candidate.name == op.name)
        else {
            continue;
        };
        if later_op.phase != "undone" || operation_path(roots, &later, later_op)? != path {
            continue;
        }
        let (Some(before), Some(undo_id)) = (&later_op.before, &later_op.undo_id) else {
            continue;
        };
        let restored = Stamp {
            hash: before.hash.clone(),
            id: undo_id.clone(),
        };
        edges.push((before.clone(), restored, later));
    }
    loop {
        let mut grew = false;
        for (before, restored, later) in &edges {
            if !reachable.contains(before) || reachable.contains(restored) {
                continue;
            }
            verify_backups(later)?;
            if restored == current {
                return Ok(true);
            }
            reachable.push(restored.clone());
            grew = true;
        }
        if !grew {
            break;
        }
    }
    Ok(false)
}

// All operation images, including the baseline, are validated before any undo.
// Inode witnesses cover updates as well as creations and interrupted restorations.
fn rollback_preflight(roots: &Roots, journal: &Journal) -> Result<Vec<bool>> {
    let (_, target) = namespace_paths(roots, &journal.summary.source, &journal.summary.target)?;
    // A missing parent must not look like an absent record preimage, including
    // baseline-only runs. Recheck after every guard before any conditional undo.
    Directory::open(&target, false).map_err(|code| {
        if code == "MISSING_PATH" {
            "ROLLBACK_TARGET_MISSING".into()
        } else {
            code
        }
    })?;
    let mut undo = Vec::new();
    for op in &journal.operations {
        let current = stamp(&operation_path(roots, journal, op)?)?;
        let original = current == op.before;
        let restored = op
            .before
            .as_ref()
            .zip(current.as_ref())
            .map(|(b, c)| b.hash == c.hash && op.undo_id.as_ref() == Some(&c.id))
            .unwrap_or(false);
        let absent_undo = op.before.is_none() && current.is_none();
        let post = current
            .as_ref()
            .map(|c| owned_post_image(roots, journal, op, c))
            .transpose()?
            .unwrap_or(false);
        let valid = match op.phase.as_str() {
            "planned" => original,
            "write_pending" => original || post,
            "written" => post,
            "undo_pending" => post || restored || absent_undo || original,
            "undone" => restored || absent_undo || original,
            _ => false,
        };
        check(valid, "ROLLBACK_DRIFT")?;
        undo.push(post && !original);
    }
    Ok(undo)
}

pub fn rollback(
    roots: &Roots,
    run_id: &str,
    guard: &mut dyn FnMut() -> Result<()>,
) -> Result<RunSummary> {
    check(valid_run_id(run_id), "INVALID_RUN_ID")?;
    disjoint(&[&roots.records, &roots.pool, &roots.state])?;
    let _lock = Lock::acquire(&roots.state)?;
    no_pending(roots, Some(run_id))?;
    let mut journal = load(roots, run_id)?;
    disjoint(&[
        &roots.records,
        &roots.pool,
        &roots.state,
        Path::new(&journal.summary.backup_dir),
    ])?;
    let backups = verify_backups(&journal)?;
    guard_call(guard)?;
    rollback_preflight(roots, &journal)?;
    if journal.summary.state == "rolled_back" {
        return Ok(journal.summary);
    }
    journal.summary.state = "rolling_back".into();
    save(roots, &journal)?;
    for index in (0..journal.operations.len()).rev() {
        guard_call(guard)?;
        verify_backups(&journal)?;
        let undo = rollback_preflight(roots, &journal)?;
        let path = operation_path(roots, &journal, &journal.operations[index])?;
        if undo[index] {
            // If the process died after staging but before journaling its inode,
            // leave that unowned stage alone and use a fresh exclusive name.
            if journal.operations[index].before.is_some() {
                let op = &journal.operations[index];
                if let Some(existing) = stamp(&path.with_file_name(&op.undo_temp))? {
                    if op.undo_id.as_ref() != Some(&existing.id) {
                        journal.operations[index].undo_temp =
                            format!(".handoff-{}.tmp", Uuid::new_v4());
                        journal.operations[index].undo_id = None;
                        save(roots, &journal)?;
                    }
                }
            }
            let op = &journal.operations[index];
            if op.before.is_some() {
                // A crash may leave an undo stage. Reuse only its recorded inode.
                let staged = path.with_file_name(&op.undo_temp);
                let owner = if let Some(existing) = stamp(&staged)? {
                    check(
                        op.undo_id.as_ref() == Some(&existing.id)
                            && Some(existing.hash.as_str())
                                == op.before.as_ref().map(|b| b.hash.as_str()),
                        "STAGING_DRIFT",
                    )?;
                    existing.id
                } else {
                    stage(&path, &op.undo_temp, &backups[&backup_key(op)])?
                };
                journal.operations[index].undo_id = Some(owner);
            }
            journal.operations[index].phase = "undo_pending".into();
            save(roots, &journal)?;
            guard_call(guard)?;
            verify_backups(&journal)?;
            rollback_preflight(roots, &journal)?;
            let op = &journal.operations[index];
            let current = stamp(&path)?.ok_or("ROLLBACK_DRIFT")?;
            check(
                owned_post_image(roots, &journal, op, &current)?,
                "ROLLBACK_DRIFT",
            )?;
            let expected = Some(current);
            if let Some(before) = &op.before {
                let restored = Stamp {
                    hash: before.hash.clone(),
                    id: op.undo_id.clone().ok_or("INVALID_JOURNAL")?,
                };
                publish_operation(&path, &op.undo_temp, &expected, &restored)?;
            } else {
                check(stamp(&path)? == expected, "ROLLBACK_DRIFT")?;
                let (dir, name) = parent(&path)?;
                dir.unlink(&name)?;
            }
        }
        journal.operations[index].phase = "undone".into();
        save(roots, &journal)?;
        cleanup_stages(&path, &journal.operations[index])?;
    }
    guard_call(guard)?;
    rollback_preflight(roots, &journal)?;
    verify_backups(&journal)?;
    journal.summary.state = "rolled_back".into();
    save(roots, &journal)?;
    Ok(journal.summary)
}

fn cleanup_stages(path: &Path, op: &Operation) -> Result<()> {
    for (name, owner) in [(&op.temp, &op.write_id), (&op.undo_temp, &op.undo_id)] {
        if let Some(blob) = read_blob(&path.with_file_name(name))? {
            // Unjournaled staging debris is never treated as an owned write.
            if owner.as_ref() == Some(&blob.stamp.id) {
                let (dir, _) = parent(path)?;
                dir.unlink(name)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
#[path = "engine_tests.rs"]
mod tests;
