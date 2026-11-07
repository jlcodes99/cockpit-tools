//! Immutable encrypted resource snapshots. Catalog reads only load identifiers.
use crate::modules::{account, atomic_write, secure_account_storage};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const LIMIT: u64 = 128 * 1024 * 1024;
const COUNT: usize = 512;
const ERROR: &str = "PROXY_PROFILE_RESOURCE";

pub(super) struct CancelOnDrop(pub(super) Arc<AtomicBool>);
impl CancelOnDrop {
    pub(super) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("CATALOG_CANCELLED".into())
    } else {
        Ok(())
    }
}

pub(super) fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn root() -> Result<PathBuf, String> {
    // Use the same root as the encryption key, including isolated test roots.
    Ok(account::get_data_dir()
        .map_err(|_| ERROR)?
        .join("codex-proxy-assets"))
}

fn read_bounded(path: &Path, remaining: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| ERROR)?;
    if !file.metadata().map_err(|_| ERROR)?.is_file() {
        return Err(ERROR.into());
    }
    let mut data = Vec::new();
    file.take(remaining + 1)
        .read_to_end(&mut data)
        .map_err(|_| ERROR)?;
    if data.len() as u64 > remaining {
        return Err("CATALOG_LIMIT".into());
    }
    Ok(data)
}

pub(super) fn put(bytes: &[u8]) -> Result<String, String> {
    let id = format!("{:x}", Sha256::digest(bytes));
    let root = root()?;
    fs::create_dir_all(&root).map_err(|_| ERROR)?;
    let target = root.join(format!("{id}.json"));
    if !target.exists() || get(&id).is_err() {
        let encrypted = secure_account_storage::serialize_account_file(
            "codex-proxy-asset",
            &STANDARD.encode(bytes),
        )
        .map_err(|_| ERROR)?;
        atomic_write::write_secret_string_atomic(&target, &encrypted).map_err(|_| ERROR)?;
    }
    Ok(id)
}

pub(super) fn get(id: &str) -> Result<Vec<u8>, String> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ERROR.into());
    }
    let raw = read_bounded(&root()?.join(format!("{id}.json")), LIMIT * 2)?;
    let raw = String::from_utf8(raw).map_err(|_| ERROR)?;
    let encoded: String =
        secure_account_storage::deserialize_encrypted_snapshot(&raw).map_err(|_| ERROR)?;
    let bytes = STANDARD.decode(encoded).map_err(|_| ERROR)?;
    if bytes.len() as u64 > LIMIT || format!("{:x}", Sha256::digest(&bytes)) != id {
        return Err(ERROR.into());
    }
    Ok(bytes)
}

/// Only called for an explicit import/refresh or a private engine directory.
/// Stable hashes preserve fixed account snapshots when a source is refreshed/deleted.
pub(super) fn capture(
    directory: &Path,
    cancelled: &AtomicBool,
) -> Result<BTreeMap<String, String>, String> {
    check_cancelled(cancelled)?;
    let directory = directory.canonicalize().map_err(|_| ERROR)?;
    let mut pending = vec![directory.clone()];
    let mut files = BTreeMap::new();
    let mut bytes_left = LIMIT;
    let mut entries = 0;
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).map_err(|_| ERROR)? {
            check_cancelled(cancelled)?;
            entries += 1;
            if entries > COUNT {
                return Err("CATALOG_LIMIT".into());
            }
            let entry = entry.map_err(|_| ERROR)?;
            let path = entry.path();
            let relative_path = path
                .strip_prefix(&directory)
                .map_err(|_| ERROR)?
                .to_str()
                .ok_or(ERROR)?
                .replace('\\', "/");
            if relative_path == "cache.db" || relative_path == "cache.db.lock" {
                continue;
            }
            let kind = entry.file_type().map_err(|_| ERROR)?;
            // A snapshot must not traverse a link out of the selected resource tree.
            if kind.is_symlink() {
                return Err(ERROR.into());
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() || !relative(&relative_path) {
                return Err(ERROR.into());
            }
            let data = read_bounded(&path, bytes_left)?;
            check_cancelled(cancelled)?;
            bytes_left -= data.len() as u64;
            files.insert(relative_path, put(&data)?);
        }
        std::thread::yield_now();
    }
    Ok(files)
}

pub(super) async fn restore(
    files: BTreeMap<String, String>,
    directory: Arc<crate::modules::codex_proxy_engine::RuntimeDirectory>,
) -> Result<(), String> {
    let cancel = CancelOnDrop::new();
    let cancelled = cancel.0.clone();
    tokio::task::spawn_blocking(move || {
        if files.len() > COUNT {
            return Err("CATALOG_LIMIT".into());
        }
        let mut remaining = LIMIT;
        for (name, id) in files {
            check_cancelled(&cancelled)?;
            if !relative(&name) {
                return Err(ERROR.into());
            }
            let bytes = get(&id)?;
            remaining = remaining
                .checked_sub(bytes.len() as u64)
                .ok_or("CATALOG_LIMIT")?;
            check_cancelled(&cancelled)?;
            let path = directory.path().join(name);
            fs::create_dir_all(path.parent().ok_or(ERROR)?).map_err(|_| ERROR)?;
            atomic_write::write_bytes_atomic(&path, &bytes).map_err(|_| ERROR)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|_| ERROR)?;
            }
            std::thread::yield_now();
        }
        Ok(())
    })
    .await
    .map_err(|_| ERROR)?
}

pub(super) async fn snapshot(
    directory: Arc<crate::modules::codex_proxy_engine::RuntimeDirectory>,
) -> Result<BTreeMap<String, String>, String> {
    let cancel = CancelOnDrop::new();
    let cancelled = cancel.0.clone();
    tokio::task::spawn_blocking(move || capture(directory.path(), &cancelled))
        .await
        .map_err(|_| ERROR)?
}
