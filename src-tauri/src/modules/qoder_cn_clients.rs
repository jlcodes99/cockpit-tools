use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{config, process, qoder_native};
use crate::commands::{account::cli_launch, cli_launch_path::executable_file};

static SWITCH: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Client {
    Desktop,
    Ide,
    Cli,
}

impl Client {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "desktop" => Ok(Self::Desktop),
            "ide" => Ok(Self::Ide),
            "cli" => Ok(Self::Cli),
            _ => Err("QODER_CLIENT_INVALID".into()),
        }
    }
}

pub(crate) fn is_cli_path(path: &Path, cn: bool) -> bool {
    executable_file(path)
        && path.file_stem().is_some_and(|name| {
            let aliases = if cn { ["qoderclicn", "qodercn"] } else { ["qodercli", "qoder"] };
            aliases.iter().any(|alias| name.eq_ignore_ascii_case(alias))
        })
}

pub(crate) fn detect_cli(cn: bool) -> Option<PathBuf> {
    find_named_cli(cn, false)
}

fn find_named_cli(cn: bool, direct: bool) -> Option<PathBuf> {
    let mut roots = super::codex_wakeup::collect_runtime_search_dirs();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/bin"));
        let install = home.join(if cn { ".qoder-cn" } else { ".qoder" });
        roots.extend([install.join("entry"), install.join("bin")]);
    }
    let names: &[&str] = match (cn, cfg!(windows)) {
        (true, true) => &["qoderclicn.exe", "qoderclicn.cmd"],
        (true, false) => &["qoderclicn", "qodercn"],
        (false, true) => &["qodercli.exe", "qodercli.cmd"],
        (false, false) => &["qodercli", "qoder"],
    };
    roots
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .find(|path| executable_file(path) && (!direct || supports_direct_launch(path)))
}

fn supports_direct_launch(path: &Path) -> bool {
    !cfg!(windows) || path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

pub(crate) fn resolve_t3_cli(cn: bool) -> Result<PathBuf, String> {
    if !configured_cli(&config::get_user_config(), cn).trim().is_empty() {
        let path = resolve_cli(cn)?;
        if supports_direct_launch(&path) {
            return Ok(path);
        }
        // ACP starts the executable directly. Prefer the installed executable
        // when the configured CLI path is a Windows command wrapper.
    }
    find_named_cli(cn, true).ok_or_else(|| missing_cli(cn).into())
}

pub(crate) fn configured_cli(config: &config::UserConfig, cn: bool) -> &str {
    if cn { &config.qoder_cn_cli_path } else { &config.qoder_cli_path }
}

fn missing_cli(cn: bool) -> &'static str {
    if cn { "CLI_PATH_NOT_FOUND:qoder_cn_cli" } else { "CLI_PATH_NOT_FOUND:qoder_cli" }
}

fn resolve_cli(cn: bool) -> Result<PathBuf, String> {
    let config = config::get_user_config();
    let path = configured_cli(&config, cn).trim();
    (if path.is_empty() {
        detect_cli(cn)
    } else {
        let path = PathBuf::from(path);
        is_cli_path(&path, cn).then_some(path)
    })
    .ok_or_else(|| missing_cli(cn).into())
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_executable(path: &Path, client: Client) -> Option<PathBuf> {
    let bundle_id = match client {
        Client::Desktop => "com.qodercn.app",
        Client::Ide => "com.aliyun.lingma.ide",
        Client::Cli => return None,
    };
    mac_bundle_executable(path, bundle_id)
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_global_desktop_executable(path: &Path) -> Option<PathBuf> {
    mac_bundle_executable(path, "com.qoder.app")
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_global_ide_executable(path: &Path) -> Option<PathBuf> {
    mac_bundle_executable(path, "com.qoder.ide")
}

#[cfg(target_os = "macos")]
fn mac_bundle_executable(path: &Path, bundle_id: &str) -> Option<PathBuf> {
    let bundle = path.ancestors().find(|ancestor| {
        ancestor
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })?;
    let manifest = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok()?;
    let fields = manifest.as_dictionary()?;
    if fields.get("CFBundleIdentifier")?.as_string()? != bundle_id {
        return None;
    }
    let name = fields.get("CFBundleExecutable")?.as_string()?;
    let executable = bundle.join("Contents/MacOS").join(name);
    ((path == bundle || path == executable) && executable_file(&executable)).then_some(executable)
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_main_process(path: &Path, client: Client) -> bool {
    mac_executable(path, client).is_some_and(|binary| path == binary)
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_current_user_process(process: &sysinfo::Process) -> bool {
    process
        .user_id()
        .is_some_and(|uid| **uid == unsafe { libc::geteuid() })
}

pub(crate) fn resolve_ide(configured: Option<&str>) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = configured.filter(|path| !path.trim().is_empty()) {
            return mac_executable(Path::new(path.trim()), Client::Ide);
        }
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Applications"));
        }
        roots
            .into_iter()
            .find_map(|root| mac_executable(&root.join("Qoder CN IDE.app"), Client::Ide))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = configured;
        None
    }
}

fn prepare_cli(
    cn: bool,
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
) -> Result<impl FnOnce() -> Result<(), String>, String> {
    let binary = resolve_cli(cn)?;
    let dir = working_dir
        .or_else(dirs::home_dir)
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or("QODER_CN_CLI_INVALID_DIRECTORY")?;
    let terminal = terminal.unwrap_or_else(|| config::get_user_config().default_terminal);
    let command = cli_launch::build_command(&binary, &dir, None);
    let plan = cli_launch::build_codex_terminal_launch_plan(&command, &terminal)?;
    Ok(move || cli_launch::open_terminal(plan, &terminal))
}

pub(crate) fn open_client(
    cn: bool,
    client: Client,
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
    instance_id: Option<&str>,
) -> Result<(), String> {
    let _guard = SWITCH.try_lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    match (cn, client) {
        (_, Client::Cli) => prepare_cli(cn, working_dir, terminal)?(),
        (false, Client::Ide) => crate::commands::qoder_instance::initialize_qoder_ide(instance_id),
        (false, Client::Desktop) => open_global_desktop(),
        (true, Client::Desktop) => start_desktop(&process::resolve_qoder_cn_launch_path()?, false),
        (true, Client::Ide) => {
            #[cfg(target_os = "macos")]
            return start_macos(
                &resolve_ide(Some(&config::get_user_config().qoder_cn_ide_path))
                    .ok_or("APP_PATH_NOT_FOUND:qoder_cn_ide")?,
                false,
            );
            #[cfg(not(target_os = "macos"))]
            Err("QODER_CN_IDE_UNSUPPORTED".into())
        }
    }
}

pub(crate) fn resolve_global_desktop(configured: Option<&str>) -> Option<PathBuf> {
    let configured = configured.map(str::trim).filter(|path| !path.is_empty());
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = configured {
            return mac_global_desktop_executable(Path::new(path));
        }
        return std::iter::once(PathBuf::from("/Applications"))
            .chain(dirs::home_dir().map(|home| home.join("Applications")))
            .find_map(|root| mac_global_desktop_executable(&root.join("Qoder.app")))
            .or_else(|| {
                let system = sysinfo::System::new_all();
                qoder_native::mac_global_desktop_pids(&system)
                    .into_iter()
                    .find_map(|pid| {
                        system
                            .process(sysinfo::Pid::from_u32(pid))?
                            .exe()
                            .map(Path::to_path_buf)
                    })
            });
    }
    #[cfg(windows)]
    {
        if let Some(path) = configured {
            let path = PathBuf::from(path);
            return (executable_file(&path)
                && !process::is_qoder_ide_executable(&path)
                && path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Qoder.exe")))
            .then_some(path);
        }
        return std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Programs"))
            .into_iter()
            .chain(
                ["PROGRAMFILES", "PROGRAMFILES(X86)"]
                    .into_iter()
                    .filter_map(std::env::var_os)
                    .map(PathBuf::from),
            )
            .map(|root| root.join("Qoder").join("Qoder.exe"))
            .find(|path| executable_file(path) && !process::is_qoder_ide_executable(path));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = configured;
        None
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn global_desktop_launch_path() -> Result<PathBuf, String> {
    resolve_global_desktop(Some(&config::get_user_config().qoder_desktop_app_path))
        .ok_or_else(|| "APP_PATH_NOT_FOUND:qoder_desktop".into())
}

fn open_global_desktop() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return start_macos(&global_desktop_launch_path()?, false);
    #[cfg(windows)]
    return process::start_qoder_desktop(&global_desktop_launch_path()?, "qoder_desktop")
        .map(|_| ());
    #[cfg(not(any(windows, target_os = "macos")))]
    Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into())
}

pub(crate) fn switch_global_native(
    account_id: &str,
    client: qoder_native::NativeClient,
    launch: bool,
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
) -> Result<bool, String> {
    let _guard = SWITCH.try_lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    if !launch {
        qoder_native::switch(qoder_native::NativeRegion::Global, account_id, client)?;
        return Ok(false);
    }
    if client == qoder_native::NativeClient::Cli {
        return switch_cli(false, account_id, true, working_dir, terminal);
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    return switch_and_launch_global_desktop(account_id);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn switch_and_launch_global_desktop(account_id: &str) -> Result<bool, String> {
    let executable = global_desktop_launch_path()?;
    let region = qoder_native::NativeRegion::Global;
    let running = || {
        let system = sysinfo::System::new_all();
        #[cfg(target_os = "macos")]
        return qoder_native::mac_global_desktop_pids(&system);
        #[cfg(windows)]
        return qoder_native::windows_global_desktop_pids(&system, &executable);
    };
    let _initialization = super::client_initialization::prepare(
        "qoder", "desktop", || qoder_native::desktop_ready(region),
        || if running().is_empty() { open_global_desktop() } else { Ok(()) },
    )?;
    switch_and_launch(
        &running(),
        || {
            qoder_native::switch(
                qoder_native::NativeRegion::Global,
                account_id,
                qoder_native::NativeClient::Desktop,
            )
        },
        || {
            #[cfg(target_os = "macos")]
            start_macos(&executable, true)?;
            #[cfg(windows)]
            process::start_qoder_desktop(&executable, "qoder_desktop")?;
            confirm_running(|| Ok(!running().is_empty()))
        },
        "QODER_DESKTOP_RELAUNCH_FAILED",
    )
}

fn start_desktop(executable: &Path, _new_instance: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return start_macos(
        &mac_executable(executable, Client::Desktop).ok_or("APP_PATH_NOT_FOUND:qoder_cn")?,
        _new_instance,
    );
    #[cfg(windows)]
    return process::start_qoder_desktop(executable, "qoder_cn").map(|_| ());
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = executable;
        Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into())
    }
}

pub(crate) fn switch(
    account_id: &str,
    client: Client,
    launch: bool,
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
) -> Result<bool, String> {
    let _guard = SWITCH.try_lock().map_err(|_| "QODER_ACCOUNT_BUSY")?;
    match client {
        Client::Cli => switch_cli(true, account_id, launch, working_dir, terminal),
        Client::Desktop => switch_desktop(account_id, launch),
        Client::Ide => switch_ide(account_id, launch),
    }
}

fn switch_cli(
    cn: bool,
    account_id: &str,
    launch: bool,
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
) -> Result<bool, String> {
    let start = launch.then(|| prepare_cli(cn, working_dir, terminal)).transpose()?;
    let region = if cn { qoder_native::NativeRegion::Cn } else { qoder_native::NativeRegion::Global };
    qoder_native::switch(region, account_id, qoder_native::NativeClient::Cli)?;
    if let Some(start) = start {
        start().map_err(|error| format!("QODER_SWITCH_LAUNCH_FAILED: {error}"))?;
    }
    Ok(launch)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn switch_desktop(account_id: &str, launch: bool) -> Result<bool, String> {
    let write = || {
        qoder_native::switch(
            qoder_native::NativeRegion::Cn,
            account_id,
            qoder_native::NativeClient::Desktop,
        )
    };
    if !launch {
        write()?;
        return Ok(false);
    }
    let executable = process::resolve_qoder_cn_launch_path()?;
    #[cfg(target_os = "macos")]
    let executable =
        mac_executable(&executable, Client::Desktop).ok_or("APP_PATH_NOT_FOUND:qoder_cn")?;
    let pids = || qoder_native::qoder_cn_desktop_pids(&sysinfo::System::new_all());
    let _initialization = super::client_initialization::prepare(
        "qoder_cn", "desktop", || qoder_native::desktop_ready(qoder_native::NativeRegion::Cn),
        || if pids().is_empty() { start_desktop(&executable, false) } else { Ok(()) },
    )?;
    switch_and_launch(
        &pids(),
        write,
        || {
            start_desktop(&executable, true)?;
            confirm_running(|| Ok(!pids().is_empty()))
        },
        "QODER_CN_RELAUNCH_FAILED",
    )
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn switch_desktop(_: &str, _: bool) -> Result<bool, String> {
    Err("QODER_NATIVE_DESKTOP_UNSUPPORTED".into())
}

#[cfg(target_os = "macos")]
fn switch_ide(account_id: &str, launch: bool) -> Result<bool, String> {
    let pids = || {
        let system = sysinfo::System::new_all();
        if system.processes().is_empty() {
            return Err("QODER_NATIVE_PROCESS_PROBE_FAILED");
        }
        let default_root = dirs::config_dir()
            .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
            .join("QoderCN");
        Ok(system
            .processes()
            .values()
            .filter(|process| {
                mac_current_user_process(process)
                    && process
                        .exe()
                        .is_some_and(|path| mac_main_process(path, Client::Ide))
                    && process::macos_profiles_from_argv(process.cmd())
                        .last()
                        .is_none_or(|root| Path::new(root) == default_root)
            })
            .map(|process| process.pid().as_u32())
            .collect::<Vec<_>>())
    };
    let write = || {
        if !pids()?.is_empty() {
            return Err("QODER_CLIENT_RUNNING: ide".into());
        }
        qoder_native::switch_cn_ide(account_id)
    };
    if !launch {
        write()?;
        return Ok(false);
    }
    let executable = resolve_ide(Some(&config::get_user_config().qoder_cn_ide_path))
        .ok_or("APP_PATH_NOT_FOUND:qoder_cn_ide")?;
    let profile = dirs::config_dir().ok_or("QODER_DATA_ROOT_UNAVAILABLE")?.join("QoderCN");
    let _initialization = super::client_initialization::prepare(
        "qoder_cn", "ide",
        || Ok(super::client_initialization::state_db_ready(&profile)?
            && super::vscode_inject::client_secret_storage_ready("qoder_cn_ide", &profile)?),
        || if pids()?.is_empty() { start_macos(&executable, false) } else { Ok(()) },
    )?;
    switch_and_launch(
        &pids()?,
        write,
        || {
            start_macos(&executable, true)?;
            confirm_running(|| Ok(!pids()?.is_empty()))
        },
        "QODER_CN_RELAUNCH_FAILED",
    )
}

#[cfg(not(target_os = "macos"))]
fn switch_ide(_: &str, _: bool) -> Result<bool, String> {
    Err("QODER_CN_IDE_UNSUPPORTED".into())
}

#[cfg(any(windows, target_os = "macos"))]
fn confirm_running(running: impl Fn() -> Result<bool, String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if running()? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("QODER_LAUNCH_UNCONFIRMED".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn switch_and_launch(
    running: &[u32],
    write: impl FnOnce() -> Result<(), String>,
    start: impl Fn() -> Result<(), String>,
    relaunch_error: &str,
) -> Result<bool, String> {
    process::close_qoder_cn_client(running, 20)?;
    if let Err(error) = write() {
        if !running.is_empty() {
            if let Err(restart_error) = start() {
                return Err(format!("{error} ({restart_error})"));
            }
        }
        return Err(error);
    }
    start().map_err(|error| format!("{relaunch_error}: {error}"))?;
    Ok(true)
}

#[cfg(target_os = "macos")]
fn start_macos(executable: &Path, new_instance: bool) -> Result<(), String> {
    let bundle = executable
        .ancestors()
        .nth(3)
        .ok_or("QODER_CN_APP_INVALID")?;
    let mut command = std::process::Command::new("/usr/bin/open");
    if new_instance {
        command.arg("-n");
    }
    command.arg("-a").arg(bundle);
    let output = super::process_timeout::output_with_timeout(
        &mut command,
        std::time::Duration::from_secs(10),
    )
    .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn same_named_qoder_desktop_and_ide_executables_match_only_their_bundle() {
        let root = tempfile::tempdir().unwrap();
        for (name, identifier, executable, cn_target) in [
            (
                "Qoder CN.app",
                "com.qodercn.app",
                "Qoder CN",
                Some(Client::Desktop),
            ),
            (
                "Qoder CN IDE.app",
                "com.aliyun.lingma.ide",
                "Qoder CN",
                Some(Client::Ide),
            ),
            ("Qoder IDE.app", "com.qoder.ide", "Qoder", None),
            ("Qoder.app", "com.qoder.app", "Qoder", None),
        ] {
            let bundle = root.path().join(name);
            let binary = bundle.join("Contents/MacOS").join(executable);
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(&binary, b"").unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut manifest = plist::Dictionary::new();
            manifest.insert("CFBundleIdentifier".into(), identifier.into());
            manifest.insert("CFBundleExecutable".into(), executable.into());
            plist::Value::Dictionary(manifest)
                .to_file_xml(bundle.join("Contents/Info.plist"))
                .unwrap();
            for client in [Client::Desktop, Client::Ide] {
                assert_eq!(mac_main_process(&binary, client), cn_target == Some(client));
                assert_eq!(
                    mac_executable(&bundle, client).as_deref(),
                    (cn_target == Some(client)).then_some(binary.as_path()),
                );
            }
            let expected_global = (identifier == "com.qoder.app").then_some(binary.as_path());
            assert_eq!(
                mac_global_desktop_executable(&bundle).as_deref(),
                expected_global
            );
            assert_eq!(
                mac_global_desktop_executable(&binary).as_deref(),
                expected_global
            );
            let expected_ide = (identifier == "com.qoder.ide").then_some(binary.as_path());
            assert_eq!(mac_global_ide_executable(&bundle).as_deref(), expected_ide);
            assert_eq!(mac_global_ide_executable(&binary).as_deref(), expected_ide);
        }
    }
}
