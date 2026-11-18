use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const MAX_HEADER: u32 = 32 * 1024 * 1024;
const MAX_SOURCE: u64 = 24 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Assets {
    pub(super) entry_scripts: Vec<String>,
    pub(super) hosts: Vec<String>,
    pub(super) menus: Vec<String>,
    pub(super) model_queries: Vec<String>,
}

struct CachedArchive {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
    assets: Option<Arc<Assets>>,
}

static CACHE: Mutex<Option<CachedArchive>> = Mutex::new(None);

pub(super) fn for_client() -> Option<Arc<Assets>> {
    let configured = crate::modules::config::get_user_config().codex_app_path;
    // Launch preparation already resolves the configured executable. The
    // read-only detector also handles profiles started before it was saved.
    let path = archive_for_path(Path::new(configured.trim())).or_else(|| {
        crate::modules::process::detect_codex_exec_path().and_then(|path| archive_for_path(&path))
    })?;
    let mut cache = CACHE.lock().ok()?;
    let metadata = fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    let len = metadata.len();
    if let Some(previous) = cache.as_ref().filter(|previous| {
        previous.path == path && previous.modified == modified && previous.len == len
    }) {
        return previous.assets.clone();
    }
    let assets = scan(&path).map(Arc::new);
    *cache = Some(CachedArchive {
        path,
        modified,
        len,
        assets: assets.clone(),
    });
    assets
}

fn archive_for_path(path: &Path) -> Option<PathBuf> {
    let path = fs::canonicalize(path).ok()?;
    // macOS app bundles expose Contents/Resources, while Windows and Linux
    // keep a resources directory next to the Electron executable.
    path.ancestors().take(3).find_map(|directory| {
        [
            directory.join("Contents/Resources/app.asar"),
            directory.join("Resources/app.asar"),
            directory.join("resources/app.asar"),
        ]
        .into_iter()
        .find(|archive| archive.is_file())
    })
}

pub(super) fn scan(path: &Path) -> Option<Assets> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut prefix = [0; 16];
    file.read_exact(&mut prefix).ok()?;
    let word = |start| u32::from_le_bytes(prefix[start..start + 4].try_into().unwrap());
    let header_size = word(4);
    let payload_size = word(8);
    let json_size = word(12);
    let data_start = 8 + u64::from(header_size);
    if word(0) != 4
        || header_size > MAX_HEADER
        || payload_size.checked_add(4)? != header_size
        || json_size.checked_add(4)? > payload_size
        || data_start > len
    {
        return None;
    }
    let mut header = vec![0; json_size as usize];
    file.read_exact(&mut header).ok()?;
    let index: Value = serde_json::from_slice(&header).ok()?;
    let webview = index.pointer("/files/webview/files")?;
    let index_html = &webview["index.html"];
    let (start, size) = packed_range(index_html, data_start, len)?;
    if size > MAX_SOURCE {
        return None;
    }
    let mut buffer = vec![0; size as usize];
    file.seek(SeekFrom::Start(start)).ok()?;
    file.read_exact(&mut buffer).ok()?;
    let html = std::str::from_utf8(&buffer).ok()?;
    let script =
        regex::Regex::new(r#"<script\b[^>]*\bsrc=["'](?:\.?/)?assets/([^"'\s]+)["']"#).ok()?;
    let mut assets = Assets {
        entry_scripts: script
            .captures_iter(html)
            .map(|capture| capture[1].to_owned())
            .filter(|name| is_chunk_name(name))
            .collect(),
        ..Assets::default()
    };
    if assets.entry_scripts.is_empty() {
        return None;
    }

    let entries = webview.pointer("/assets/files/async/files")?.as_object()?;
    let mut chunks = Vec::new();
    for (name, entry) in entries {
        if !is_chunk_name(name) || entry["unpacked"].as_bool() == Some(true) {
            continue;
        }
        let (start, size) = packed_range(entry, data_start, len)?;
        if size <= MAX_SOURCE {
            chunks.push((start, size as usize, name));
        }
    }
    chunks.sort_unstable_by_key(|(start, _, _)| *start);
    for (start, size, name) in chunks {
        buffer.resize(size, 0);
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        let Ok(source) = std::str::from_utf8(&buffer) else {
            continue;
        };
        // Cheap candidate discovery only. The renderer verifies the full
        // contracts against its live manifest before any module is replaced.
        if source.contains("async writeServiceTier") {
            assets.hosts.push(name.clone());
        }
        if source.contains("serviceTierAccess:") && source.contains("personalAccessToken") {
            assets.menus.push(name.clone());
        }
        if source.contains("includeUltraReasoningEffort:") && source.contains("teamCatalog:") {
            assets.model_queries.push(name.clone());
        }
    }
    Some(assets)
}

fn packed_range(entry: &Value, data_start: u64, file_len: u64) -> Option<(u64, u64)> {
    let size = entry["size"].as_u64()?;
    let start = data_start.checked_add(entry["offset"].as_str()?.parse().ok()?)?;
    (start.checked_add(size)? <= file_len).then_some((start, size))
}

fn is_chunk_name(name: &str) -> bool {
    name.strip_suffix(".js")
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(id, hash)| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit())
                && !hash.is_empty()
                && hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_candidates_with_arbitrary_chunk_names_and_ignores_non_chunks() {
        let parts = [
            (
                "index.html",
                r#"<script src="/assets/123.deadbeef.js"></script>"#,
            ),
            ("400000.abcd.js", "async writeServiceTier(x){}"),
            (
                "400001.ef12.js",
                "personalAccessToken;serviceTierAccess: true",
            ),
            (
                "400002.4321.js",
                "includeUltraReasoningEffort:true,teamCatalog:null",
            ),
            ("label.abcd.js", "async writeServiceTier(x){}"),
            ("400003.9876.js", "unrelated"),
        ];
        let mut payload = Vec::new();
        let mut entries = serde_json::Map::new();
        for (name, content) in parts {
            entries.insert(
                name.to_owned(),
                json!({
                    "offset": payload.len().to_string(),
                    "size": content.len()
                }),
            );
            payload.extend_from_slice(content.as_bytes());
        }
        let html = entries.remove("index.html").unwrap();
        let header = json!({
            "files": {"webview": {"files": {
                "index.html": html,
                "assets": {"files": {"async": {"files": entries}}}
            }}}
        })
        .to_string();
        let padded = (header.len() + 3) & !3;
        let mut archive = Vec::new();
        for word in [4, padded as u32 + 8, padded as u32 + 4, header.len() as u32] {
            archive.extend_from_slice(&word.to_le_bytes());
        }
        archive.extend_from_slice(header.as_bytes());
        archive.resize(16 + padded, 0);
        archive.extend_from_slice(&payload);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app.asar");
        fs::write(&path, &archive).unwrap();
        let result = scan(&path).unwrap();
        assert_eq!(result.entry_scripts, ["123.deadbeef.js"]);
        assert_eq!(result.hosts, ["400000.abcd.js"]);
        assert_eq!(result.menus, ["400001.ef12.js"]);
        assert_eq!(result.model_queries, ["400002.4321.js"]);
        fs::write(&path, &archive[..archive.len() - 1]).unwrap();
        assert!(scan(&path).is_none());
    }
}
