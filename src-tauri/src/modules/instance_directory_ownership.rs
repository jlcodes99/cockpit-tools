//! Conservative directory provenance. Legacy/imported paths are never adopted.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = ".cockpit-instance-owner.json";
#[derive(Serialize, Deserialize)]
struct Origin {
    version: u32,
    instance_id: String,
    creation_source: String,
    directory: PathBuf,
}

/// Record only a newly created directory. An existing empty directory is still
/// user-owned, and importing it must never overwrite somebody else's marker.
pub fn record_creation(
    path: &Path,
    instance_id: &str,
    mode: &str,
    existed: bool,
) -> Result<(), String> {
    if existed
        || matches!(
            mode.to_ascii_lowercase().as_str(),
            "existingdir" | "existing_dir"
        )
    {
        return Ok(());
    }
    let normalized = crate::modules::data_paths::without_compatibility_alias(path);
    let path = normalized.as_path();
    let data = crate::modules::account::get_data_dir()?;
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let Some(root) = canonical.parent() else {
        return Ok(());
    };
    let managed = data.join("instances");
    // External/custom locations remain user-owned even when initialized here.
    if root.parent() != managed.canonicalize().ok().as_deref() {
        return Ok(());
    }
    if crate::modules::instance_storage_cleanup::path_has_link_ancestor(path)? {
        return Ok(());
    }
    write_origin(
        path,
        instance_id,
        if mode.eq_ignore_ascii_case("empty") {
            "empty"
        } else {
            "copy"
        },
    )
}

fn write_origin(path: &Path, instance_id: &str, source: &str) -> Result<(), String> {
    let origin = Origin {
        version: 1,
        instance_id: instance_id.to_string(),
        creation_source: source.to_string(),
        directory: path.canonicalize().map_err(|e| e.to_string())?,
    };
    let bytes = serde_json::to_vec(&origin).map_err(|e| e.to_string())?;
    // A copied source marker is replaced only in this newly created directory.
    crate::modules::atomic_write::write_bytes_atomic(&path.join(MARKER), &bytes)
        .map_err(|e| e.to_string())
}

pub fn owner(path: &Path) -> Result<Option<String>, String> {
    let marker = path.join(MARKER);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Ok(None);
    }
    let origin: Origin = match serde_json::from_slice(&fs::read(marker).map_err(|e| e.to_string())?)
    {
        Ok(origin) => origin,
        Err(_) => return Ok(None),
    };
    if origin.version != 1
        || !matches!(origin.creation_source.as_str(), "empty" | "copy")
        || uuid::Uuid::parse_str(&origin.instance_id).is_err()
        || path.canonicalize().ok().as_ref() != Some(&origin.directory)
    {
        return Ok(None);
    }
    Ok(Some(origin.instance_id))
}

#[cfg(test)]
pub(crate) fn test_mark_created(path: &Path) {
    write_origin(path, &uuid::Uuid::new_v4().to_string(), "empty").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_created_owner_is_recognized_and_copying_marker_cannot_adopt_a_path() {
        let base = std::env::temp_dir().join(format!("origin-{}", uuid::Uuid::new_v4()));
        let created = base.join("created");
        let imported = base.join("imported");
        fs::create_dir_all(&created).unwrap();
        fs::create_dir_all(&imported).unwrap();
        assert_eq!(owner(&imported).unwrap(), None);
        let id = uuid::Uuid::new_v4().to_string();
        write_origin(&created, &id, "empty").unwrap();
        assert_eq!(owner(&created).unwrap(), Some(id));
        fs::copy(created.join(MARKER), imported.join(MARKER)).unwrap();
        assert_eq!(owner(&imported).unwrap(), None);
        fs::write(imported.join(MARKER), "{}").unwrap();
        assert_eq!(owner(&imported).unwrap(), None);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn existing_import_does_not_write_a_marker() {
        let base = std::env::temp_dir().join(format!("origin-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        record_creation(
            &base,
            &uuid::Uuid::new_v4().to_string(),
            "existing_dir",
            true,
        )
        .unwrap();
        assert!(!base.join(MARKER).exists());
        fs::remove_dir_all(base).unwrap();
    }
}
