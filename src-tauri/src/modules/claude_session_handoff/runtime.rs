//! macOS lifecycle and bounded login-transition checks. No tokens or raw log
//! text are emitted; only fresh account/org metadata is used after relaunch.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::engine::Identity;
use chrono::{Local, NaiveDateTime, TimeZone};

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
    if browser_owned_native_host(p, all) || bundled_updater(p).is_some() {
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

fn bundled_updater(p: &Process) -> Option<&Path> {
    let path = Path::new(&p.executable);
    let suffix = Path::new("Contents/Frameworks/Squirrel.framework/Resources/ShipIt");
    if !path.ends_with(suffix) {
        return None;
    }
    let mut app = path;
    for _ in suffix.components() {
        app = app.parent()?;
    }
    (app.is_absolute() && app.extension().is_some_and(|extension| extension == "app"))
        .then_some(app)
}

fn updater_for(p: &Process, app: &Path) -> bool {
    bundled_updater(p) == Some(app)
}

fn process_error(code: &str, p: &Process) -> String {
    let role = if main_process(p) {
        "desktop-main"
    } else if cli_process(p) {
        "claude-cli"
    } else if bundled_updater(p).is_some() {
        "desktop-updater"
    } else {
        "desktop-helper"
    };
    let name = Path::new(&p.executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Claude");
    serde_json::json!({"code":code,"processId":p.pid,"processRole":role,"processName":name})
        .to_string()
}

pub(super) fn diagnostic_code(error: &str) -> String {
    serde_json::from_str::<serde_json::Value>(error)
        .ok()
        .and_then(|value| value.get("code")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| error.to_owned())
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
        return Err(process_error("CLAUDE_WRITER_RUNNING", blocker));
    }
    Ok(())
}

pub(super) fn assert_quiet() -> Result<(), String> {
    check_quiet(false)
}

pub(super) fn assert_quiet_with_evidence() -> Result<(), String> {
    check_quiet(true)
}

fn publication_quiet(all: &[Process], app: &Path) -> Result<(), String> {
    if let Some(writer) = all.iter().find(|p| writer(p, all)) {
        return Err(process_error("CLAUDE_WRITER_RUNNING", writer));
    }
    if let Some(updater) = all.iter().find(|p| updater_for(p, app)) {
        return Err(process_error("DESKTOP_CONTRACT_CHANGED", updater));
    }
    Ok(())
}

pub(super) fn assert_publication_quiet(app: &Path) -> Result<(), String> {
    publication_quiet(&processes()?, app)
}

pub(super) fn wait_until_quiet(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match assert_quiet() {
            Ok(()) => return Ok(()),
            Err(code)
                if diagnostic_code(&code) == "CLAUDE_WRITER_RUNNING"
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(code) => return Err(code),
        }
    }
}

fn quit_validated_main(main: &Process, app: &Path) -> Result<(), String> {
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
    Ok(())
}

#[derive(Debug)]
pub(super) struct ShutdownReceipt {
    pub was_running: bool,
    /// This operation observed a selected bundled updater before its first quiet
    /// boundary, and waited until no selected updater remained. Not attribution of
    /// every archive change to that process.
    pub bundled_update_settled: bool,
    pub settled_contract: Option<super::storage_contract::Contract>,
}

#[derive(Debug, PartialEq, Eq)]
enum ShutdownAction {
    Updating,
    Waiting,
    QuitRelaunched(u32),
    Ready,
}

struct ShutdownWait {
    original_main: Option<(u32, i64)>,
    initial_updaters: Vec<(u32, i64)>,
    update_since: Option<Duration>,
    waiting_since: Duration,
    update_settled: bool,
    requit: bool,
}

impl ShutdownWait {
    fn new(all: &[Process], app: &Path) -> Self {
        let initial_updaters: Vec<_> = all
            .iter()
            .filter(|p| updater_for(p, app))
            .map(|p| (p.pid, p.started))
            .collect();
        let update_since = (!initial_updaters.is_empty()).then_some(Duration::ZERO);
        Self {
            original_main: all
                .iter()
                .find(|p| main_process(p))
                .map(|p| (p.pid, p.started)),
            initial_updaters,
            waiting_since: Duration::ZERO,
            update_since,
            update_settled: false,
            requit: false,
        }
    }

    fn step(
        &mut self,
        all: &[Process],
        app: &Path,
        elapsed: Duration,
    ) -> Result<ShutdownAction, String> {
        if let Some(updater) = all.iter().find(|p| updater_for(p, app)) {
            if self.initial_updaters.is_empty() {
                // Capture the exact bundled updater while closing the initially
                // observed Desktop, before the first quiet boundary. A closed
                // App operation grants no late-capture window.
                if self.original_main.is_none() || self.update_settled {
                    return Err(process_error("DESKTOP_CONTRACT_CHANGED", updater));
                }
                self.initial_updaters.push((updater.pid, updater.started));
            }
            let update_since = *self.update_since.get_or_insert(elapsed);
            if elapsed.saturating_sub(update_since) >= Duration::from_secs(90) {
                return Err(process_error("DESKTOP_UPDATE_TIMEOUT", updater));
            }
            if let Some(original) = all
                .iter()
                .find(|p| Some((p.pid, p.started)) == self.original_main)
            {
                if elapsed >= Duration::from_secs(15) {
                    return Err(process_error("CLAUDE_WRITER_RUNNING", original));
                }
            }
            return Ok(ShutdownAction::Updating);
        }
        if !self.initial_updaters.is_empty() && !self.update_settled {
            self.update_settled = true;
            self.waiting_since = elapsed;
        }
        if self.update_settled && !self.requit {
            if let Some(main) = all.iter().find(|p| main_process(p)) {
                let relaunched = self.original_main.map_or_else(
                    || {
                        self.initial_updaters
                            .iter()
                            .any(|updater| main.started >= updater.1)
                    },
                    |original| (main.pid, main.started) != original && main.started >= original.1,
                );
                if relaunched {
                    // Squirrel can relaunch the same default App when updating.
                    // One normal quit is allowed, with the same strict profile,
                    // ownership and process-identity checks as the first quit.
                    can_quit(all, app)?;
                    self.requit = true;
                    self.waiting_since = elapsed;
                    return Ok(ShutdownAction::QuitRelaunched(main.pid));
                }
            }
        }
        let Some(blocker) = all.iter().find(|p| writer(p, all)) else {
            return Ok(ShutdownAction::Ready);
        };
        if elapsed.saturating_sub(self.waiting_since) >= Duration::from_secs(15) {
            return Err(process_error("CLAUDE_WRITER_RUNNING", blocker));
        }
        Ok(ShutdownAction::Waiting)
    }
}

pub(super) fn quit_normally(app: &Path) -> Result<bool, String> {
    quit_and_settle(app, false, &mut || {}).map(|receipt| receipt.was_running)
}

pub(super) fn quit_and_settle(
    app: &Path,
    bind_updated_storage: bool,
    updating: &mut dyn FnMut(),
) -> Result<ShutdownReceipt, String> {
    let all = processes()?;
    let running = can_quit(&all, app)?;
    let mut waiting = ShutdownWait::new(&all, app);
    if let Some(main) = all.iter().find(|p| main_process(p)) {
        quit_validated_main(main, app)?;
    }
    let started = Instant::now();
    let mut update_reported = false;
    loop {
        let observed = processes()?;
        let result = waiting.step(&observed, app, started.elapsed());
        let failure = match result {
            Ok(ShutdownAction::Ready) => {
                let settled_contract = if waiting.update_settled && bind_updated_storage {
                    let binding = (|| {
                        publication_quiet(&observed, app)?;
                        let contract = super::storage_contract::inspect(app)?;
                        assert_publication_quiet(app)?;
                        contract.assert_unchanged()?;
                        Ok::<_, String>(contract)
                    })();
                    match binding {
                        Ok(contract) => Some(contract),
                        Err(code) => {
                            if running {
                                let _ = reopen_if_closed(app);
                            }
                            return Err(code);
                        }
                    }
                } else {
                    None
                };
                return Ok(ShutdownReceipt {
                    was_running: running,
                    bundled_update_settled: waiting.update_settled,
                    settled_contract,
                });
            }
            Ok(ShutdownAction::Updating) => {
                if !update_reported {
                    updating();
                    update_reported = true;
                }
                None
            }
            Ok(ShutdownAction::QuitRelaunched(pid)) => quit_validated_main(
                observed
                    .iter()
                    .find(|p| p.pid == pid)
                    .ok_or("PROCESS_INVENTORY_UNAVAILABLE")?,
                app,
            )
            .err(),
            Ok(ShutdownAction::Waiting) => None,
            Err(code) => Some(code),
        };
        if let Some(code) = failure {
            tracing::warn!(
                code = diagnostic_code(&code),
                diagnostics = code,
                "Claude handoff lifecycle stopped before publication"
            );
            // Reopening during Squirrel installation interrupted the updater in
            // the real failure. Leave it undisturbed until it finishes.
            if running && !observed.iter().any(|p| updater_for(p, app)) {
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
    let observed = processes()?;
    if observed.iter().any(main_process) || observed.iter().any(|p| updater_for(p, app)) {
        return Ok(false);
    }
    reopen(app)?;
    Ok(true)
}

pub(super) fn reopen(app: &Path) -> Result<(), String> {
    if let Some(updater) = processes()?.iter().find(|p| updater_for(p, app)) {
        return Err(process_error("DESKTOP_UPDATE_TIMEOUT", updater));
    }
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
    fn quit_triggered_late_updater_is_captured_before_first_quiet_boundary() {
        let app = Path::new("/Applications/Claude.app");
        let main = process(100, 1, 10, "/Applications/Claude.app/Contents/MacOS/Claude");
        let updater = process(
            200,
            1,
            20,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        let mut wait = ShutdownWait::new(&[main.clone()], app);
        assert_eq!(
            wait.step(&[main], app, Duration::from_secs(1)),
            Ok(ShutdownAction::Waiting)
        );
        assert_eq!(
            wait.step(&[updater.clone()], app, Duration::from_secs(2)),
            Ok(ShutdownAction::Updating)
        );
        assert_eq!(
            wait.step(&[updater], app, Duration::from_secs(40)),
            Ok(ShutdownAction::Updating)
        );
        assert_eq!(
            wait.step(&[], app, Duration::from_secs(41)),
            Ok(ShutdownAction::Ready)
        );
        assert!(wait.update_settled);
    }

    #[test]
    fn configured_bundle_name_and_post_settlement_updater_use_distinct_guards() {
        let app = Path::new("/Applications/Claude Preview.app");
        let updater = process(200, 1, 10, "/Applications/Claude Preview.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt");
        assert!(updater_for(&updater, app));
        assert!(!updater_for(
            &updater,
            Path::new("/Applications/Claude.app")
        ));
        assert!(!writer(&updater, &[updater.clone()]));
        assert_eq!(
            diagnostic_code(&publication_quiet(&[updater], app).unwrap_err()),
            "DESKTOP_CONTRACT_CHANGED"
        );
        assert!(publication_quiet(&[], app).is_ok());
    }

    #[test]
    fn updater_is_not_a_sidebar_writer_but_still_requires_settlement() {
        let app = Path::new("/Applications/Claude.app");
        let main = process(100, 1, 10, "/Applications/Claude.app/Contents/MacOS/Claude");
        let updater = process(
            29011,
            1,
            9,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        assert!(!writer(&updater, &[updater.clone()]));
        let mut wait = ShutdownWait::new(&[main, updater.clone()], app);
        for elapsed in [1, 15, 38] {
            assert_eq!(
                wait.step(&[updater.clone()], app, Duration::from_secs(elapsed)),
                Ok(ShutdownAction::Updating)
            );
            assert!(!wait.update_settled);
        }
        assert_eq!(
            wait.step(&[], app, Duration::from_secs(39)),
            Ok(ShutdownAction::Ready)
        );
        assert!(wait.update_settled);
        for executable in [
            "/Applications/Claude.app/Contents/Helpers/ShipIt",
            "/Applications/Claude.app/Contents/Frameworks/Unknown.framework/Resources/ShipIt",
            "/Applications/Claude.app/Contents/Helpers/unknown-code-worker",
        ] {
            let unknown = process(200, 1, 9, executable);
            assert!(writer(&unknown, &[unknown.clone()]));
        }
    }

    #[test]
    fn updater_timeout_is_specific_and_never_marks_shutdown_complete() {
        let app = Path::new("/Applications/Claude.app");
        let updater = process(
            29011,
            1,
            9,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        let mut wait = ShutdownWait::new(&[updater.clone()], app);
        let error = wait
            .step(&[updater], app, Duration::from_secs(90))
            .unwrap_err();
        let details: serde_json::Value = serde_json::from_str(&error).unwrap();
        assert_eq!(details["code"], "DESKTOP_UPDATE_TIMEOUT");
        assert_eq!(details["processId"], 29011);
        assert_eq!(details["processRole"], "desktop-updater");
        assert_eq!(details["processName"], "ShipIt");
        assert!(!wait.update_settled);
    }

    #[test]
    fn updater_relaunch_gets_one_validated_normal_quit_and_other_restarts_stay_blocked() {
        let app = Path::new("/Applications/Claude.app");
        let main = process(100, 1, 10, "/Applications/Claude.app/Contents/MacOS/Claude");
        let updater = process(
            200,
            1,
            9,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        let relaunched = process(300, 1, 20, "/Applications/Claude.app/Contents/MacOS/Claude");
        let mut wait = ShutdownWait::new(&[main.clone(), updater.clone()], app);
        assert_eq!(
            wait.step(&[updater], app, Duration::from_secs(38)),
            Ok(ShutdownAction::Updating)
        );
        assert_eq!(
            wait.step(&[relaunched.clone()], app, Duration::from_secs(39)),
            Ok(ShutdownAction::QuitRelaunched(300))
        );
        assert_eq!(
            wait.step(&[], app, Duration::from_secs(40)),
            Ok(ShutdownAction::Ready)
        );
        let twice = process(400, 1, 30, &relaunched.executable);
        let error = wait
            .step(&[twice], app, Duration::from_secs(54))
            .unwrap_err();
        assert_eq!(diagnostic_code(&error), "CLAUDE_WRITER_RUNNING");
        let mut ordinary = ShutdownWait::new(&[main], app);
        let error = ordinary
            .step(&[relaunched], app, Duration::from_secs(15))
            .unwrap_err();
        assert_eq!(diagnostic_code(&error), "CLAUDE_WRITER_RUNNING");
        let mut unexpected = ShutdownWait::new(&[], app);
        let updater = process(
            500,
            1,
            31,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        assert_eq!(
            diagnostic_code(
                &unexpected
                    .step(&[updater], app, Duration::from_secs(1))
                    .unwrap_err()
            ),
            "DESKTOP_CONTRACT_CHANGED"
        );
    }

    #[test]
    fn lingering_real_writer_has_safe_diagnostics_and_remains_blocked_after_update() {
        let app = Path::new("/Applications/Claude.app");
        let updater = process(
            200,
            1,
            9,
            "/Applications/Claude.app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt",
        );
        let helper = process(300, 1, 20, "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper");
        let mut wait = ShutdownWait::new(&[updater], app);
        assert_eq!(
            wait.step(&[helper.clone()], app, Duration::from_secs(39)),
            Ok(ShutdownAction::Waiting)
        );
        let error = wait
            .step(&[helper], app, Duration::from_secs(54))
            .unwrap_err();
        assert_eq!(diagnostic_code(&error), "CLAUDE_WRITER_RUNNING");
        let details: serde_json::Value = serde_json::from_str(&error).unwrap();
        assert_eq!(details["processName"], "Claude Helper");
        assert!(!error.contains("/Applications/"));
        assert_eq!(
            diagnostic_code("CLAUDE_WRITER_RUNNING"),
            "CLAUDE_WRITER_RUNNING"
        );
    }

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
