//! Refuse projection repair while any relevant Codex writer may still be alive.
//! This probe never sends signals and never exposes process arguments or environment values.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

const RUNNING: &str = "CODEX_HISTORY_REPAIR_RUNNING";
const PROCESS_PROBE: &str = "CODEX_HISTORY_REPAIR_PROCESS_PROBE";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExecutableKind {
    Codex,
    Other,
    Uncertain,
}

enum HomeEvidence {
    Explicit(String),
    Default(String),
    Unknown,
}

struct ProcessSnapshot {
    name: String,
    executable: Option<String>,
    // Only argv[0] and the official npm launcher's script argument are needed.
    command_prefix: Vec<String>,
    home: HomeEvidence,
    zombie: bool,
}

/// Call on a blocking worker and recheck immediately before publishing the repair.
/// A process starting after this snapshot cannot be excluded by a read-only probe.
pub(crate) fn ensure_stopped(profile: &Path) -> Result<(), String> {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return Err(PROCESS_PROBE.into());
    }
    let windows = cfg!(target_os = "windows");
    let target = canonical_path_identity(profile, windows).ok_or(PROCESS_PROBE)?;
    let default_home = dirs::home_dir().map(|home| home.join(".codex"));
    let mut system = System::new();
    let refreshed = system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always)
            .with_environ(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    // sysinfo does not return an I/O error. An empty enumeration or an absent
    // self process is not evidence that the machine has no other writers.
    let probe_healthy =
        refreshed > 0 && system.process(Pid::from_u32(std::process::id())).is_some();
    let snapshots = system
        .processes()
        .values()
        .map(|process| {
            let mut snapshot = ProcessSnapshot {
                name: process.name().to_string_lossy().into_owned(),
                executable: process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned()),
                command_prefix: process
                    .cmd()
                    .iter()
                    .take(2)
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                home: HomeEvidence::Unknown,
                zombie: process.status() == ProcessStatus::Zombie,
            };
            if !snapshot.zombie && executable_kind(&snapshot) != ExecutableKind::Other {
                snapshot.home = process_home(
                    process.environ(),
                    process.cwd(),
                    default_home.as_deref(),
                    windows,
                );
            }
            snapshot
        })
        .collect::<Vec<_>>();
    check_snapshots(&target, &snapshots, probe_healthy, windows)
}

fn check_snapshots(
    target: &str,
    snapshots: &[ProcessSnapshot],
    probe_healthy: bool,
    windows: bool,
) -> Result<(), String> {
    if !probe_healthy {
        return Err(PROCESS_PROBE.into());
    }
    let target = normalize_path_identity(target, windows);
    let mut uncertain = false;
    for process in snapshots {
        if process.zombie {
            continue;
        }
        match executable_kind(process) {
            ExecutableKind::Other => continue,
            ExecutableKind::Uncertain => {
                uncertain = true;
                continue;
            }
            ExecutableKind::Codex => {}
        }
        match &process.home {
            HomeEvidence::Explicit(home) | HomeEvidence::Default(home) => {
                if normalize_path_identity(home, windows) == target {
                    return Err(RUNNING.into());
                }
            }
            HomeEvidence::Unknown => uncertain = true,
        }
    }
    if uncertain {
        Err(PROCESS_PROBE.into())
    } else {
        Ok(())
    }
}

fn executable_name(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
}

fn codex_executable_name(value: &str) -> bool {
    matches!(
        executable_name(value).to_ascii_lowercase().as_str(),
        "codex"
            | "codex.exe"
            | "chatgpt"
            | "chatgpt.exe"
            | "codex-aarch64-apple-darwin"
            | "codex-x86_64-apple-darwin"
            | "codex-aarch64-unknown-linux-gnu"
            | "codex-x86_64-unknown-linux-gnu"
            | "codex-aarch64-unknown-linux-musl"
            | "codex-x86_64-unknown-linux-musl"
            | "codex-aarch64-pc-windows-msvc.exe"
            | "codex-x86_64-pc-windows-msvc.exe"
    )
}

fn official_node_launcher(name: &str, command_prefix: &[String]) -> bool {
    if !matches!(name.to_ascii_lowercase().as_str(), "node" | "node.exe") {
        return false;
    }
    command_prefix.get(1).is_some_and(|script| {
        script
            .replace('\\', "/")
            .to_ascii_lowercase()
            .ends_with("/node_modules/@openai/codex/bin/codex.js")
    })
}

fn executable_kind(process: &ProcessSnapshot) -> ExecutableKind {
    if let Some(executable) = process.executable.as_deref().filter(|exe| !exe.is_empty()) {
        if codex_executable_name(executable)
            || official_node_launcher(executable_name(executable), &process.command_prefix)
        {
            ExecutableKind::Codex
        } else {
            // Never match a shell's -c string or a random argument mentioning Codex.
            ExecutableKind::Other
        }
    } else if codex_executable_name(&process.name)
        || official_node_launcher(executable_name(&process.name), &process.command_prefix)
        || process
            .command_prefix
            .first()
            .is_some_and(|arg| codex_executable_name(arg))
        || process.name.is_empty()
    {
        // The name/argv hint cannot establish the identity when exe is unreadable.
        ExecutableKind::Uncertain
    } else {
        ExecutableKind::Other
    }
}

fn process_home(
    environment: &[OsString],
    cwd: Option<&Path>,
    default_home: Option<&Path>,
    windows: bool,
) -> HomeEvidence {
    // sysinfo uses an empty list both for an empty environment and read failure.
    // In either case absence of CODEX_HOME cannot prove this is the default instance.
    if environment.is_empty() {
        return HomeEvidence::Unknown;
    }
    let mut code_home: Option<&str> = None;
    let mut user_home: Option<&str> = None;
    for entry in environment {
        let Some(entry) = entry.to_str() else {
            return HomeEvidence::Unknown;
        };
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        let matches = |expected: &str| {
            if windows {
                key.eq_ignore_ascii_case(expected)
            } else {
                key == expected
            }
        };
        if matches("CODEX_HOME") {
            if code_home.is_some_and(|previous| previous != value) {
                return HomeEvidence::Unknown;
            }
            code_home = Some(value);
        } else if matches(if windows { "USERPROFILE" } else { "HOME" }) {
            if user_home.is_some_and(|previous| previous != value) {
                return HomeEvidence::Unknown;
            }
            user_home = Some(value);
        }
    }
    if let Some(home) = code_home {
        return resolve_home_identity(home, cwd, windows)
            .map(HomeEvidence::Explicit)
            .unwrap_or(HomeEvidence::Unknown);
    }
    let standard = user_home
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".codex"))
        .or_else(|| default_home.map(Path::to_path_buf));
    standard
        .as_deref()
        .and_then(|home| canonical_path_identity(home, windows))
        .map(HomeEvidence::Default)
        .unwrap_or(HomeEvidence::Unknown)
}

fn resolve_home_identity(home: &str, cwd: Option<&Path>, windows: bool) -> Option<String> {
    if home.is_empty() {
        return None;
    }
    let home = Path::new(home);
    let path = if home.is_absolute() {
        home.to_path_buf()
    } else {
        cwd?.join(home)
    };
    canonical_path_identity(&path, windows)
}

fn canonical_path_identity(path: &Path, windows: bool) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    Some(normalize_path_identity(canonical.to_str()?, windows))
}

fn normalize_path_identity(path: &str, windows: bool) -> String {
    if !windows {
        return if path == "/" {
            path.to_owned()
        } else {
            path.trim_end_matches('/').to_owned()
        };
    }
    let normalized = path.replace('\\', "/").to_lowercase();
    let normalized = if let Some(unc) = normalized.strip_prefix("//?/unc/") {
        format!("//{unc}")
    } else {
        normalized
            .strip_prefix("//?/")
            .unwrap_or(&normalized)
            .to_owned()
    };
    normalized.trim_end_matches('/').to_owned()
}

#[cfg(test)]
#[path = "codex_history_projection_guard_tests.rs"]
mod tests;
