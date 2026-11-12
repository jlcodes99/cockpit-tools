//! OpenCode V2 owns its credential database. Use the installed CLI to update it.
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::modules::{logger, process_timeout};

const MANAGED_PREFIX: &str = "cred_cockpit_codex_";
const CLI_TIMEOUT: Duration = Duration::from_secs(20);

fn append_bundled_cli(candidates: &mut Vec<PathBuf>, app: &Path) {
    if app
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        candidates.push(app.join("Contents/Resources/opencode-cli"));
    } else if let Some(directory) = app.parent() {
        let cli = if cfg!(windows) {
            "opencode-cli.exe"
        } else {
            "opencode-cli"
        };
        candidates.push(directory.join("resources").join(cli));
        candidates.push(directory.join(cli));
    }
}

fn cli_candidates(custom_app_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let custom = custom_app_path.trim().trim_matches('"');
    if !custom.is_empty() {
        append_bundled_cli(&mut candidates, Path::new(custom));
    }
    #[cfg(target_os = "macos")]
    {
        append_bundled_cli(&mut candidates, Path::new("/Applications/OpenCode.app"));
        if let Some(home) = dirs::home_dir() {
            append_bundled_cli(&mut candidates, &home.join("Applications/OpenCode.app"));
        }
    }
    #[cfg(windows)]
    for app in crate::modules::process::windows_opencode_install_candidates() {
        append_bundled_cli(&mut candidates, &app);
    }
    let names: &[&str] = if cfg!(windows) {
        &["opencode.exe", "opencode.cmd"]
    } else {
        &["opencode"]
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths).filter(|dir| dir.is_absolute()) {
            candidates.extend(names.iter().map(|name| directory.join(name)));
        }
    }
    if let Some(home) = dirs::home_dir() {
        candidates.extend(
            names
                .iter()
                .map(|name| home.join(".opencode/bin").join(name)),
        );
    }
    candidates
}

fn major_version(text: &str) -> Option<u32> {
    let version = text.trim().strip_prefix("opencode ").unwrap_or(text.trim());
    let (major, rest) = version.trim_start_matches('v').split_once('.')?;
    if !rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    major.parse().ok()
}

fn has_v2_credentials_database(data_dir: &Path) -> bool {
    let path = std::env::var_os("OPENCODE_DB")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| data_dir.join("opencode.db"));
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|db| {
            db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='credential')",
            [],
            |row| row.get::<_, bool>(0),
        )
        })
        .unwrap_or(false)
}

pub(super) fn detect_cli(
    custom_app_path: &str,
    data_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let mut seen = std::collections::HashSet::new();
    for cli in cli_candidates(custom_app_path) {
        if !seen.insert(cli.clone()) || !cli.is_file() {
            continue;
        }
        let version = if cli.file_stem().is_some_and(|name| name == "opencode-cli") {
            fs::read_to_string(cli.with_file_name("opencode-cli.version"))
                .ok()
                .and_then(|version| major_version(&version))
        } else {
            None
        }
        .or_else(|| {
            let output = process_timeout::output_with_timeout(
                Command::new(&cli).arg("--version"),
                Duration::from_secs(3),
            )
            .ok()?;
            output
                .status
                .success()
                .then(|| major_version(&String::from_utf8_lossy(&output.stdout)))
                .flatten()
        });
        if version.is_some_and(|major| major >= 2) {
            return Ok(Some(cli));
        }
    }
    if has_v2_credentials_database(data_dir) {
        return Err("OpenCode V2 credential database detected, but its CLI was not found".into());
    }
    Ok(None)
}

fn run(cli: &Path, data_dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Output, String> {
    let mut command = Command::new(cli);
    command.current_dir(data_dir).args(args);
    let result = if let Some(input) = input {
        process_timeout::output_with_input_timeout(&mut command, input, CLI_TIMEOUT)
    } else {
        process_timeout::output_with_timeout(&mut command, CLI_TIMEOUT)
    };
    let action = args[1];
    let output = result.map_err(|error| format!("OpenCode auth {action}: {error}"))?;
    if !output.status.success() {
        // Command output can contain credential values or upstream response bodies.
        return Err(format!("OpenCode auth {action} failed ({})", output.status));
    }
    Ok(output)
}

fn sync_lease(data_dir: &Path) -> Result<std::fs::File, String> {
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data_dir.join(".cockpit-codex-auth.lock"))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + CLI_TIMEOUT;
    loop {
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("OpenCode credential update lock: {error}")),
        }
    }
}

#[derive(Deserialize)]
struct Integration {
    id: String,
    connections: Vec<Connection>,
}

#[derive(Deserialize)]
struct Connection {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
}

fn stored_openai_ids(stdout: &[u8]) -> Result<Vec<String>, String> {
    let integrations: Vec<Integration> = serde_json::from_slice(stdout)
        .map_err(|_| "OpenCode auth list did not return integration data".to_string())?;
    Ok(integrations
        .into_iter()
        .find(|integration| integration.id == "openai")
        .into_iter()
        .flat_map(|integration| integration.connections)
        .filter(|connection| connection.kind == "credential")
        .filter_map(|connection| connection.id)
        .collect())
}

fn managed_id(id: &str) -> bool {
    id.strip_prefix(MANAGED_PREFIX)
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn import_entry(account_id: &str, legacy: Value) -> Result<(String, Vec<u8>), String> {
    let refresh = legacy["refresh"]
        .as_str()
        .ok_or("Missing Codex refresh token")?;
    let mut hash = Sha256::new();
    hash.update((account_id.len() as u64).to_le_bytes());
    hash.update(account_id.as_bytes());
    hash.update(refresh.as_bytes());
    // Reusing the ID for an unchanged grant also preserves a newer token that
    // OpenCode has already rotated in place.
    let id = format!("{MANAGED_PREFIX}{:x}", hash.finalize());
    let mut value = json!({
        "type":"oauth",
        "methodID":"chatgpt-browser",
        "refresh":refresh,
        "access":legacy["access"],
        "expires":legacy["expires"],
    });
    if let Some(workspace) = legacy["accountId"].as_str() {
        value["metadata"] = json!({"accountID":workspace});
    }
    let input = serde_json::to_vec(&json!([{
        "id":id,
        "integrationID":"openai",
        "label":"Cockpit Tools",
        "active":false,
        "value":value,
    }]))
    .map_err(|_| "Could not encode OpenCode credential".to_string())?;
    Ok((id, input))
}

pub(super) fn sync(
    cli: &Path,
    data_dir: &Path,
    account_id: &str,
    legacy: Value,
    is_current: impl Fn() -> bool,
) -> Result<bool, String> {
    let (id, input) = import_entry(account_id, legacy)?;
    let _lock = sync_lease(data_dir)?;
    if !is_current() {
        return Ok(false);
    }
    run(cli, data_dir, &["auth", "import"], Some(&input))?;
    if !is_current() {
        return Ok(false);
    }
    run(cli, data_dir, &["auth", "switch", "openai", &id], None)?;
    // Activate before pruning. A failed import or activation must leave the
    // previous login usable, and manually saved credentials belong to OpenCode.
    match run(cli, data_dir, &["auth", "list", "--format", "json"], None)
        .and_then(|output| stored_openai_ids(&output.stdout))
    {
        Ok(stored) if stored.first() == Some(&id) => {
            for old in stored.iter().filter(|old| **old != id && managed_id(old)) {
                if let Err(error) = run(cli, data_dir, &["auth", "logout", "openai", old], None) {
                    logger::log_warn(&format!(
                        "[OpenCode] Could not remove old Cockpit credential: {error}"
                    ));
                }
            }
        }
        Ok(_) => return Err("OpenCode did not report the selected credential as active".into()),
        Err(error) => return Err(error),
    }
    Ok(true)
}

#[cfg(test)]
#[path = "opencode_v2_auth_tests.rs"]
mod tests;
