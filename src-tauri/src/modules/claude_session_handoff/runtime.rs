//! macOS lifecycle and bounded login-transition checks. No tokens or raw log
//! text are emitted; only fresh account/org metadata is used after relaunch.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::engine::Identity;
use chrono::{Local, NaiveDateTime, TimeZone};

// Exact-version adapters for the narrow ordinary-local contract; see
// docs/development/claude-desktop-2.16120.0-static-review.md for latest evidence.
const REVIEWED_VERSIONS: &[&str] = &[
    "1.52386.6",
    "2.110.0",
    "2.2553.1",
    "2.2553.13",
    "2.9939.2",
    "2.9939.4",
    "2.16120.0",
    "2.26454.0",
];

pub(super) fn supported_profile() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("MACOS_REQUIRED".into());
    }
    if ["CLAUDE_CONFIG_DIR", "CLAUDE_DESKTOP_USER_DATA_DIR"]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|v| !v.trim().is_empty()))
    {
        return Err("DEFAULT_PROFILE_REQUIRED".into());
    }
    Ok(())
}

pub(super) fn regular_directory(path: &Path) -> Result<(), String> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        let stat = std::fs::symlink_metadata(&cursor).map_err(|_| "SIDEBAR_NOT_INITIALIZED")?;
        if stat.file_type().is_symlink() {
            return Err("SYMLINK_NOT_SUPPORTED".into());
        }
        if !stat.is_dir() {
            return Err("SIDEBAR_NOT_INITIALIZED".into());
        }
    }
    Ok(())
}

pub(super) fn app_bundle() -> Result<PathBuf, String> {
    supported_profile()?;
    let configured = crate::modules::config::get_user_config().claude_app_path;
    let raw = if configured.trim().is_empty() {
        "/Applications/Claude.app"
    } else {
        configured.trim()
    };
    let mut app = PathBuf::from(raw);
    if app.ends_with("Contents/MacOS/Claude") {
        app = app
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("APP_PATH_NOT_FOUND:claude")?
            .to_path_buf();
    }
    if app.extension().is_none_or(|s| s != "app")
        || !app.is_absolute()
        || !app.join("Contents/MacOS/Claude").is_file()
    {
        return Err("APP_PATH_NOT_FOUND:claude".into());
    }
    Ok(app)
}

pub(super) fn desktop_version() -> Result<String, String> {
    let app = app_bundle()?;
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .map_err(|_| "DESKTOP_VERSION_UNAVAILABLE")?;
    if !output.status.success() {
        return Err("DESKTOP_VERSION_UNAVAILABLE".into());
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|_| "DESKTOP_VERSION_UNAVAILABLE")?
        .trim()
        .to_owned();
    if version.is_empty() {
        return Err("DESKTOP_VERSION_UNAVAILABLE".into());
    }
    Ok(version)
}

pub(super) fn check_version(actual: &str, expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|v| v != actual) {
        return Err("DESKTOP_VERSION_CHANGED".into());
    }
    if !REVIEWED_VERSIONS.contains(&actual) {
        return Err("DESKTOP_VERSION_REQUIRES_REVIEW".into());
    }
    Ok(())
}

fn identity_from_log(text: &str, minimum_at: i64) -> Option<Identity> {
    let mut current = None;
    for line in text.lines() {
        let Some(date) = line
            .get(..19)
            .and_then(|date| NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S").ok())
        else {
            continue;
        };
        let Some(at) = Local
            .from_local_datetime(&date)
            .single()
            .map(|date| date.timestamp_millis())
        else {
            continue;
        };
        if at < minimum_at {
            continue;
        }
        let tail = line[19..]
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
            .trim_start();
        let Some((level, body)) = tail
            .strip_prefix('[')
            .and_then(|tail| tail.split_once("] "))
        else {
            continue;
        };
        if !level
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            continue;
        }
        if body.starts_with("[account] Login-state transition")
            || body.starts_with("[LocalSessionManager] Account logged out")
            || body.starts_with("[LocalSessionManager] Org changed")
            || body.starts_with("[LocalSessionManager] Cannot initialize sessions")
            || body.starts_with("[LocalSessionManager] loadSessions failed")
        {
            current = None;
        }
        if body.starts_with("[LocalSessionManager] Initialization succeeded") {
            let field = |key: &str| {
                body.split_once(key)
                    .and_then(|(_, rest)| rest.split([',', ' ', '\t']).next())
                    .and_then(|id| uuid::Uuid::parse_str(id).ok())
                    .map(|id| id.hyphenated().to_string())
            };
            current = field("accountId=")
                .zip(field("orgId="))
                .map(|(account, org)| Identity { account, org });
        }
    }
    current
}

fn identity_log_file() -> Result<std::fs::File, String> {
    let logs = dirs::home_dir()
        .ok_or("HOME_UNAVAILABLE")?
        .join("Library/Logs/Claude");
    regular_directory(&logs)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(logs.join("main.log"))
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?;
    if !file
        .metadata()
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?
        .is_file()
    {
        return Err("DESKTOP_IDENTITY_UNAVAILABLE".into());
    }
    Ok(file)
}

#[cfg(unix)]
fn log_generation(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn log_generation(_metadata: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

pub(super) fn current_identity() -> Result<Option<Identity>, String> {
    let all = processes()?;
    let mut main = all.iter().filter(|process| main_process(process));
    let Some(process) = main.next() else {
        return Ok(None);
    };
    if main.next().is_some() {
        return Err("DEFAULT_PROFILE_REQUIRED".into());
    }
    let mut file = identity_log_file()?;
    let metadata = file
        .metadata()
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?;
    if !metadata.is_file() {
        return Err("DESKTOP_IDENTITY_UNAVAILABLE".into());
    }
    // Read only a bounded tail locally, never expose log text or parse tokens.
    let offset = metadata.len().saturating_sub(1024 * 1024);
    let minimum = process.started + 1000;
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?;
    let text = String::from_utf8_lossy(&bytes);
    let identity = identity_from_log(&text, minimum);
    let after = identity_log_file()?
        .metadata()
        .map_err(|_| "DESKTOP_IDENTITY_UNAVAILABLE")?;
    if log_generation(&after) != log_generation(&metadata) || after.len() != metadata.len() {
        return Ok(None);
    }
    // Do not accept a log belonging to an instance that exited during the read.
    if !processes()?.iter().any(|next| {
        next.pid == process.pid && next.started == process.started && main_process(next)
    }) {
        return Ok(None);
    }
    Ok(identity)
}

pub(super) fn wait_for_launch(app: &Path) -> Result<(), String> {
    let executable = app.join("Contents/MacOS/Claude");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if processes()?
            .iter()
            .any(|process| Path::new(&process.executable) == executable)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("DESKTOP_START_FAILED".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[derive(Clone, Debug)]
struct Process {
    pid: u32,
    parent: u32,
    started: i64,
    executable: String,
    interpreted_cli: bool,
}

fn parse_processes(text: &str) -> Result<Vec<Process>, String> {
    text.lines()
        .filter(|s| !s.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 8 {
                return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
            }
            let date =
                NaiveDateTime::parse_from_str(&parts[2..7].join(" "), "%a %b %e %H:%M:%S %Y")
                    .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
            let started = Local
                .from_local_datetime(&date)
                .single()
                .ok_or("PROCESS_INVENTORY_UNAVAILABLE")?
                .timestamp_millis();
            Ok(Process {
                pid: parts[0]
                    .parse()
                    .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?,
                parent: parts[1]
                    .parse()
                    .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?,
                started,
                executable: parts[7..].join(" "),
                interpreted_cli: false,
            })
        })
        .collect()
}

fn processes() -> Result<Vec<Process>, String> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,lstart=,comm="])
        .env("LC_ALL", "C")
        .output()
        .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
    if !output.status.success() {
        return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
    }
    let mut entries = parse_processes(
        &String::from_utf8(output.stdout).map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?,
    )?;
    // Older CLI installations execute through node/bun. Inspect only interpreter
    // command lines locally; never include arguments in logs, errors or IPC.
    let interpreters: Vec<_> = entries
        .iter()
        .filter(|p| {
            matches!(
                Path::new(&p.executable)
                    .file_name()
                    .and_then(|s| s.to_str()),
                Some("node" | "bun" | "npx")
            )
        })
        .map(|p| p.pid.to_string())
        .collect();
    if !interpreters.is_empty() {
        let output = Command::new("/bin/ps")
            .args(["-p", &interpreters.join(","), "-o", "pid=,args="])
            .output()
            .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
        // All candidates may have exited between the two snapshots (ps status 1).
        if !output.status.success() && output.status.code() != Some(1) {
            return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((pid, args)) = line.trim().split_once(char::is_whitespace) {
                if interpreted_claude(args) {
                    if let Some(p) = entries.iter_mut().find(|p| p.pid.to_string() == pid) {
                        p.interpreted_cli = true;
                    }
                }
            }
        }
    }
    Ok(entries)
}

fn interpreted_claude(args: &str) -> bool {
    args.contains("/@anthropic-ai/claude-code/")
        || args.contains("/.bin/claude")
        || args.contains("/claude-code/cli.js")
}

fn main_process(p: &Process) -> bool {
    p.executable.ends_with(".app/Contents/MacOS/Claude")
}

fn cli_process(p: &Process) -> bool {
    if main_process(p) {
        return false;
    }
    let name = Path::new(&p.executable)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    p.interpreted_cli
        || name == "claude"
        || name == "claude-code"
        || name.starts_with("claude-") && name[7..].starts_with(|c: char| c.is_ascii_digit())
        || p.executable.contains("/.local/share/claude/versions/")
        || p.executable.contains("/@anthropic-ai/claude-code/")
}

fn browser_owned_native_host(p: &Process, all: &[Process]) -> bool {
    p.executable
        .ends_with("/Claude.app/Contents/Helpers/chrome-native-host")
        && all.iter().any(|parent| {
            parent.pid == p.parent
                && parent.started <= p.started
                && parent
                    .executable
                    .ends_with("/Google Chrome.app/Contents/MacOS/Google Chrome")
        })
}

fn writer(p: &Process, all: &[Process]) -> bool {
    // Chrome's native-messaging bridge is launched and owned by Chrome. It can
    // survive the Desktop main process, but it is not a Code sidebar writer.
    // Keep an unknown or reparented host blocked rather than ignoring every
    // process inside Claude.app by basename alone.
    if browser_owned_native_host(p, all) {
        return false;
    }
    main_process(p) || cli_process(p) || {
        let lower = p.executable.to_lowercase();
        (lower.contains("/claude.app/contents/")
            || lower.contains("/contents/frameworks/claude helper")
            || lower.ends_with("/contents/helpers/disclaimer"))
            && !lower.contains("chrome_crashpad_handler")
    }
}

fn owned(worker: &Process, main: &Process, all: &[Process]) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut current = worker;
    loop {
        if current.pid == main.pid {
            return true;
        }
        if !seen.insert(current.pid) {
            return false;
        }
        let parents: Vec<_> = all.iter().filter(|p| p.pid == current.parent).collect();
        if parents.len() != 1 || parents[0].started > current.started {
            return false;
        }
        current = parents[0];
    }
}

fn can_quit(all: &[Process], app: &Path) -> Result<bool, String> {
    if all
        .iter()
        .map(|p| p.pid)
        .collect::<std::collections::HashSet<_>>()
        .len()
        != all.len()
    {
        return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
    }
    let mains: Vec<_> = all.iter().filter(|p| main_process(p)).collect();
    if mains.len() > 1
        || mains
            .first()
            .is_some_and(|p| Path::new(&p.executable) != app.join("Contents/MacOS/Claude"))
    {
        return Err("DEFAULT_PROFILE_REQUIRED".into());
    }
    let main = mains.first();
    if all
        .iter()
        .filter(|p| cli_process(p))
        .any(|worker| main.is_none_or(|main| !owned(worker, main, all)))
    {
        return Err("INDEPENDENT_CLI_RUNNING".into());
    }
    Ok(main.is_some())
}

#[cfg(target_os = "macos")]
fn request_normal_quit(main: &Process, app: &Path) -> Result<(), String> {
    use objc2_app_kit::NSRunningApplication;

    let running = NSRunningApplication::runningApplicationWithProcessIdentifier(main.pid as i32)
        .ok_or("DESKTOP_QUIT_FAILED")?;
    let expected_executable = app.join("Contents/MacOS/Claude");
    let bundle_matches = running
        .bundleIdentifier()
        .is_some_and(|id| id.to_string() == "com.anthropic.claudefordesktop");
    let executable_matches = running
        .executableURL()
        .and_then(|url| url.path())
        .is_some_and(|path| Path::new(&path.to_string()) == expected_executable);
    if !bundle_matches || !executable_matches {
        return Err("DEFAULT_PROFILE_REQUIRED".into());
    }
    // AppKit sends a normal quit request to this exact process. Never force-quit
    // Claude or silently target another instance by bundle identifier alone.
    if !running.terminate() && assert_quiet().is_err() {
        return Err("DESKTOP_QUIT_FAILED".into());
    }
    Ok(())
}

fn check_quiet(log_blocker: bool) -> Result<(), String> {
    let all = processes()?;
    if let Some(blocker) = all.iter().find(|p| writer(p, &all)) {
        if log_blocker {
            let kind = if main_process(blocker) {
                "desktop-main"
            } else if cli_process(blocker) {
                "claude-cli"
            } else {
                "desktop-helper"
            };
            tracing::warn!(
                pid = blocker.pid,
                kind,
                "Claude handoff blocked by active process"
            );
        }
        return Err("CLAUDE_WRITER_RUNNING".into());
    }
    Ok(())
}

pub(super) fn assert_quiet() -> Result<(), String> {
    check_quiet(false)
}

pub(super) fn assert_quiet_with_evidence() -> Result<(), String> {
    check_quiet(true)
}

pub(super) fn wait_until_quiet(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match assert_quiet() {
            Ok(()) => return Ok(()),
            Err(code) if code == "CLAUDE_WRITER_RUNNING" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(code) => return Err(code),
        }
    }
}

pub(super) fn quit_normally(app: &Path) -> Result<bool, String> {
    let all = processes()?;
    let running = can_quit(&all, app)?;
    if running {
        let main = all.iter().find(|p| main_process(p)).unwrap();
        // Reject custom profiles without printing the command line or reading their stores.
        let args = Command::new("/bin/ps")
            .args(["-p", &main.pid.to_string(), "-o", "args="])
            .output()
            .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
        if !args.status.success() {
            return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
        }
        let args = String::from_utf8_lossy(&args.stdout);
        if args.contains("--user-data-dir") || args.contains("--profile-directory") {
            return Err("DEFAULT_PROFILE_REQUIRED".into());
        }
        #[cfg(target_os = "macos")]
        request_normal_quit(main, app)?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let failure = match assert_quiet() {
            Ok(()) => return Ok(running),
            Err(code) if code == "CLAUDE_WRITER_RUNNING" => {
                if Instant::now() >= deadline {
                    Some(code)
                } else {
                    None
                }
            }
            Err(code) => Some(code),
        };
        if let Some(code) = failure {
            // No handoff write has started. Restore the Desktop session when a
            // post-quit safety check refuses to continue.
            if running {
                let _ = reopen_if_closed(app);
            }
            return Err(code);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub(super) fn reopen_if_closed(app: &Path) -> Result<bool, String> {
    // `open -a` can merely activate a still-exiting instance. Only launch a
    // replacement after the Desktop main process is actually absent.
    if processes()?.iter().any(main_process) {
        return Ok(false);
    }
    reopen(app)?;
    Ok(true)
}

pub(super) fn reopen(app: &Path) -> Result<(), String> {
    let status = Command::new("/usr/bin/open")
        .arg("-a")
        .arg(app)
        .status()
        .map_err(|_| "DESKTOP_REOPEN_FAILED")?;
    if !status.success() {
        return Err("DESKTOP_REOPEN_FAILED".into());
    }
    let expected = app.join("Contents/MacOS/Claude");
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if processes()?
            .iter()
            .any(|p| Path::new(&p.executable) == expected)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("DESKTOP_REOPEN_FAILED".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_identity_requires_fresh_success_and_rejects_logout_or_failed_reinit() {
        let started = Local
            .with_ymd_and_hms(2026, 1, 2, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let success = "2026-01-02 10:00:01 [info] [LocalSessionManager] Initialization succeeded — accountId=00000000-0000-4000-8000-000000000001, orgId=00000000-0000-4000-8000-000000000002, existingSessions=0";
        let parsed = identity_from_log(success, started).unwrap();
        assert_eq!(parsed.account, "00000000-0000-4000-8000-000000000001");
        assert!(identity_from_log(success, started + 10_000).is_none());
        for event in [
            "Account logged out",
            "Org changed",
            "Cannot initialize sessions",
            "loadSessions failed",
        ] {
            assert!(identity_from_log(
                &format!("{success}\n2026-01-02 10:00:02 [info] [LocalSessionManager] {event}"),
                started
            )
            .is_none());
        }
        assert!(identity_from_log(
            &success.replace(
                "accountId=00000000-0000-4000-8000-000000000001",
                "accountId=null"
            ),
            started
        )
        .is_none());
        assert!(identity_from_log(
            &success.replace(
                "[LocalSessionManager]",
                "[Query] copied text: [LocalSessionManager]"
            ),
            started
        )
        .is_none());
    }

    fn process(pid: u32, parent: u32, started: i64, executable: &str) -> Process {
        Process {
            pid,
            parent,
            started,
            executable: executable.into(),
            interpreted_cli: false,
        }
    }
    #[test]
    fn lifecycle_distinguishes_desktop_workers_from_terminal_cli() {
        let app = Path::new("/Applications/Claude.app");
        let main = process(100, 1, 10, "/Applications/Claude.app/Contents/MacOS/Claude");
        let child = process(101, 100, 11, "/test/.local/share/claude/versions/2.1.1");
        assert_eq!(can_quit(&[main.clone(), child.clone()], app), Ok(true));
        let terminal = process(200, 1, 12, "claude");
        assert_eq!(
            can_quit(&[main.clone(), child.clone(), terminal], app),
            Err("INDEPENDENT_CLI_RUNNING".into())
        );
        assert!(can_quit(&[child], app).is_err());
        let recycled = process(101, 100, 9, "claude");
        assert!(can_quit(&[main, recycled], app).is_err());
    }
    #[test]
    fn unknown_and_mid_operation_versions_fail_closed() {
        assert!(check_version("2.110.0", Some("2.110.0")).is_ok());
        assert!(check_version("2.2553.1", Some("2.2553.1")).is_ok());
        assert_eq!(check_version("2.2553.13", None), Ok(()));
        assert_eq!(check_version("2.2553.13", Some("2.2553.13")), Ok(()));
        assert_eq!(check_version("2.9939.2", Some("2.9939.2")), Ok(()));
        assert_eq!(check_version("2.9939.4", Some("2.9939.4")), Ok(()));
        assert_eq!(check_version("2.16120.0", Some("2.16120.0")), Ok(()));
        assert_eq!(
            check_version("2.9939.5", None),
            Err("DESKTOP_VERSION_REQUIRES_REVIEW".into())
        );
        assert_eq!(
            check_version("2.2553.14", None),
            Err("DESKTOP_VERSION_REQUIRES_REVIEW".into())
        );
        assert_eq!(
            check_version("2.2553.13", Some("2.2553.1")),
            Err("DESKTOP_VERSION_CHANGED".into())
        );
        assert!(check_version("2.2553.2", None).is_err());
        assert!(check_version("2.111.0", None).is_err());
        assert!(check_version("2.110.0", Some("1.52386.6")).is_err());
    }
    #[test]
    fn process_names_preserve_spaces_and_reject_bad_inventory() {
        let p = parse_processes("100 1 Sat Sep 19 10:20:30 2026 /Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper").unwrap();
        assert_eq!(p.len(), 1);
        assert!(writer(&p[0], &p));
        assert!(parse_processes("missing start time claude").is_err());
        assert!(interpreted_claude(
            "node /test/node_modules/@anthropic-ai/claude-code/cli.js"
        ));
        assert!(!interpreted_claude(
            "node /test/cockpit-tools/scripts/dev.cjs"
        ));
    }

    #[test]
    fn chrome_owned_native_host_does_not_block_desktop_handoff() {
        let chrome = process(
            100,
            1,
            10,
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        );
        let host = process(
            101,
            100,
            11,
            "/Applications/Claude.app/Contents/Helpers/chrome-native-host",
        );
        assert!(!writer(&host, &[chrome.clone(), host.clone()]));
        assert!(writer(&host, &[host.clone()]));
        let recycled_chrome = process(100, 1, 12, &chrome.executable);
        assert!(writer(&host, &[recycled_chrome, host.clone()]));
        let desktop = process(102, 1, 12, "/Applications/Claude.app/Contents/MacOS/Claude");
        assert!(writer(
            &desktop,
            &[chrome.clone(), host.clone(), desktop.clone()]
        ));
        let helper = process(
            103,
            102,
            13,
            "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper",
        );
        assert!(writer(&helper, &[chrome, host, desktop, helper.clone()]));
    }
}
