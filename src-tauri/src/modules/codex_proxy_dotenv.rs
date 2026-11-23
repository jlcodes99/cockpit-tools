use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::process::CodexLaunchProxy;
use super::{atomic_write, logger, process};

const BEGIN: &str = "# >>> Cockpit Tools Codex proxy v1 ";
const END: &str = "# <<< Cockpit Tools Codex proxy v1";
const UNAVAILABLE: &str = "CODEX_PROXY_DOTENV_UNAVAILABLE";
const CHANGED: &str = "CODEX_PROXY_DOTENV_CHANGED";
const STOP_FAILED: &str = "CODEX_PROXY_DOTENV_STOP_FAILED";

pub(crate) fn launch(
    home: &Path,
    proxy: &CodexLaunchProxy,
    spawn: impl FnOnce() -> Result<u32, String>,
) -> Result<u32, String> {
    launch_with_stop(home, proxy, spawn, |pid| process::close_pid(pid, 5))
}

fn launch_with_stop(
    home: &Path,
    proxy: &CodexLaunchProxy,
    spawn: impl FnOnce() -> Result<u32, String>,
    stop: impl FnOnce(u32) -> Result<(), String>,
) -> Result<u32, String> {
    let Some(projection) = Projection::prepare(home, proxy)? else {
        return spawn();
    };
    projection.write()?;
    projection.verify()?;
    let pid = spawn()?;
    if let Err(error) = projection.verify() {
        if stop(pid).is_err() {
            logger::log_warn(&format!(
                "[Codex Start] Failed to stop process {pid} after .env changed"
            ));
            return Err(STOP_FAILED.into());
        }
        return Err(error);
    }
    Ok(pid)
}

struct Projection {
    path: PathBuf,
    before: Option<String>,
    after: Option<String>,
}

impl Projection {
    fn prepare(home: &Path, proxy: &CodexLaunchProxy) -> Result<Option<Self>, String> {
        let path = home.join(".env");
        let inherit = matches!(proxy, CodexLaunchProxy::Inherit);
        let before = read_document(&path, inherit)?;
        let original = match before.as_deref() {
            Some(document) => remove_block(document)?,
            None => None,
        };
        let after = if inherit {
            if original.as_deref() == before.as_deref() {
                return Ok(None);
            }
            original
        } else {
            Some(append_block(original.as_deref(), proxy)?)
        };
        Ok(Some(Self {
            path,
            before,
            after,
        }))
    }

    fn write(&self) -> Result<(), String> {
        if self.before != self.after
            && !atomic_write::replace_private_string_if_matches(
                &self.path,
                self.before.as_deref(),
                self.after.as_deref(),
            )
            .map_err(|_| UNAVAILABLE)?
        {
            return Err(CHANGED.into());
        }
        Ok(())
    }

    fn verify(&self) -> Result<(), String> {
        if read_document(&self.path, false).map_err(|_| CHANGED)? != self.after {
            return Err(CHANGED.into());
        }
        Ok(())
    }
}

fn read_document(path: &Path, inherit: bool) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) if inherit => return Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        _ => return Err(UNAVAILABLE.into()),
    }
    let content = fs::read(path).map_err(|_| UNAVAILABLE)?;
    match String::from_utf8(content) {
        Ok(content) => Ok(Some(content)),
        Err(error)
            if inherit
                && !error
                    .as_bytes()
                    .windows(BEGIN.len())
                    .any(|window| window == BEGIN.as_bytes()) =>
        {
            Ok(None)
        }
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

fn append_block(original: Option<&str>, proxy: &CodexLaunchProxy) -> Result<String, String> {
    let base = original.unwrap_or_default();
    let eol = if base.contains("\r\n") { "\r\n" } else { "\n" };
    let (origin, separator) = match original {
        None => ("absent", ""),
        Some(text) if text.is_empty() || text.ends_with('\n') => ("keep", ""),
        Some(_) if eol == "\r\n" => ("crlf", eol),
        Some(_) => ("lf", eol),
    };
    let expected: HashMap<_, _> = proxy
        .env_overrides()
        .into_iter()
        .map(|(name, value)| (name.to_ascii_uppercase(), value))
        .collect();
    let mut body = String::new();
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
        let value = expected.get(name).ok_or(UNAVAILABLE)?;
        body.push_str(&format!("{name}={value}{eol}"));
    }
    // Codex's arg0 loader sets each parsed key in order. Appending is enough
    // to take precedence on Windows and keeps the original bytes in place.
    let document = format!(
        "{base}{separator}{BEGIN}{origin} {}{eol}{body}{END}{eol}",
        body_hash(&body),
    );
    let parsed: HashMap<_, _> = dotenvy::from_read_iter(document.as_bytes())
        .flatten()
        .map(|(name, value)| (name.to_ascii_uppercase(), value))
        .filter(|(name, _)| expected.contains_key(name))
        .collect();
    if parsed != expected {
        return Err(UNAVAILABLE.into());
    }
    Ok(document)
}

fn remove_block(document: &str) -> Result<Option<String>, String> {
    let begin = marker_offset(document, BEGIN)?;
    let end = marker_offset(document, END)?;
    let (begin, end) = match (begin, end) {
        (None, None) => return Ok(Some(document.to_owned())),
        (Some(begin), Some(end)) if begin < end => (begin, end),
        _ => return Err(UNAVAILABLE.into()),
    };
    let (header, body) = document[begin + BEGIN.len()..end]
        .split_once('\n')
        .ok_or(UNAVAILABLE)?;
    let (origin, checksum) = header
        .trim_end_matches('\r')
        .split_once(' ')
        .ok_or(UNAVAILABLE)?;
    if body_hash(body) != checksum {
        return Err(UNAVAILABLE.into());
    }
    let tail = document[end..]
        .split_once('\n')
        .map_or("", |(_, tail)| tail);
    let prefix = &document[..begin];
    let separator = match origin {
        "absent" if prefix.is_empty() && tail.is_empty() => return Ok(None),
        "absent" | "keep" => "",
        "lf" => "\n",
        "crlf" => "\r\n",
        _ => return Err(UNAVAILABLE.into()),
    };
    // Keep the line break when someone has appended their own settings.
    let prefix = if tail.is_empty() {
        prefix.strip_suffix(separator).unwrap_or(prefix)
    } else {
        prefix
    };
    Ok(Some(format!("{prefix}{tail}")))
}

fn marker_offset(document: &str, marker: &str) -> Result<Option<usize>, String> {
    let mut matches = document.match_indices(marker).filter(|(offset, _)| {
        (*offset == 0 || document.as_bytes()[offset - 1] == b'\n')
            && (marker == BEGIN
                || matches!(&document[offset + marker.len()..], "")
                || document[offset + marker.len()..].starts_with(['\n', '\r']))
    });
    let first = matches.next();
    if matches.next().is_some() {
        return Err(UNAVAILABLE.into());
    }
    Ok(first.map(|(offset, _)| offset))
}

fn body_hash(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.replace("\r\n", "\n")))
}

#[cfg(test)]
#[path = "codex_proxy_dotenv_tests.rs"]
mod tests;
