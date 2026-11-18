use super::{
    resolve_instance_launch_context, sanitize_codex_config_before_launch, CodexLaunchContext,
};
use crate::modules::{
    codex_account, codex_protocol,
    codex_session_resume::{self as store, ResumeSettings, ResumeTarget},
    codex_wakeup::{self, CodexCliResolvedRuntime},
    logger,
};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

pub(crate) const RUN_ARG: &str = "--internal-codex-cli-resume";

pub(crate) fn run_from_terminal(instance: &str, expected_home: &Path) -> Result<i32, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(run(instance, expected_home))
}

async fn run(instance: &str, expected_home: &Path) -> Result<i32, String> {
    let settings = store::get(instance)?;
    if !settings.enabled {
        return Err("此实例的会话续跑已关闭".into());
    }
    let context = resolve_instance_launch_context(instance)?;
    let home = Path::new(&context.user_data_dir);
    if !same_home(home, expected_home) {
        return Err("Codex 实例目录已变更，请重新打开启动窗口".into());
    }
    let lease = codex_account::try_acquire_profile_mutation_lease(home, "cli-session-resume")?;
    let attempt = start(instance, &context, &settings, false)
        .await?
        .ok_or("此会话已有续跑记录，或实例设置已变更")?;
    drop(lease);
    let exit = attempt.wait(instance).await?;
    Ok(exit.code().unwrap_or(1))
}

/// Switch callers already hold the matching profile mutation lease and have
/// committed the new configuration. Hand the prompt to the child before
/// returning, so no second lease or delayed dispatch is needed.
pub(super) async fn start_after_switch(
    instance: &str,
    expected_home: &Path,
) -> Result<bool, String> {
    let settings = store::get(instance)?;
    if !settings.enabled {
        return Ok(false);
    }
    let context = resolve_instance_launch_context(instance)?;
    if !same_home(Path::new(&context.user_data_dir), expected_home) {
        return Err("Codex 实例目录已变更，请重新启动该实例".into());
    }
    let Some(attempt) = start(instance, &context, &settings, true).await? else {
        return Ok(false);
    };
    let instance = instance.to_owned();
    tauri::async_runtime::spawn(async move {
        match attempt.wait(&instance).await {
            Ok(exit) if exit.success() => logger::log_info(&format!(
                "[Codex CLI Resume] Background command completed: instance={instance}"
            )),
            Ok(exit) => logger::log_warn(&format!(
                "[Codex CLI Resume] Background command failed: instance={instance}, exit={exit}"
            )),
            Err(error) => logger::log_warn(&format!(
                "[Codex CLI Resume] Background command result unavailable: instance={instance}, error={error}"
            )),
        };
    });
    Ok(true)
}

async fn start(
    instance: &str,
    context: &CodexLaunchContext,
    settings: &ResumeSettings,
    background: bool,
) -> Result<Option<ResumeAttempt>, String> {
    settings.validate()?;
    let home = Path::new(&context.user_data_dir);
    let cli = tokio::task::spawn_blocking(codex_wakeup::resolve_cli_runtime)
        .await
        .map_err(|error| error.to_string())??;
    let configured = [PathBuf::from(&cli.binary_path)];
    let node = cli.node_path.as_deref().map(Path::new);
    require_profile_idle(home, &configured, node)
        .map_err(|error| format!("读取会话前：{error}"))?;
    sanitize_codex_config_before_launch(home)?;

    let previous = if settings.uses_previous_desktop_session() {
        store::take_stopped_thread(instance, settings)?
    } else {
        None
    };
    let thread = select_thread(context, &cli, settings, previous).await?;
    require_profile_idle(home, &configured, node)
        .map_err(|error| format!("发送提示前：{error}"))?;
    if background {
        super::ensure_codex_instance_start_not_cancelled(instance)?;
    }

    let launch = Uuid::new_v4().to_string();
    if !store::claim(instance, thread, &launch, settings)? {
        return Ok(None);
    }

    let mut command = cli_command(context, &cli);
    command
        .args(["exec", "resume", "--skip-git-repo-check"])
        .arg(thread.to_string())
        .arg("-")
        .stdin(Stdio::piped());
    if background {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        hide_console(&mut command);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            store::settle(instance, thread, &launch, "notStarted")?;
            return Err(error.to_string());
        }
    };
    let mut input = child.stdin.take().ok_or("续跑进程的标准输入不可用")?;
    let write = input.write_all(settings.prompt.as_bytes()).await;
    drop(input);
    if let Err(error) = write {
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        return Err(error.to_string());
    }
    Ok(Some(ResumeAttempt {
        child,
        thread,
        launch,
    }))
}

struct ResumeAttempt {
    child: tokio::process::Child,
    thread: Uuid,
    launch: String,
}

impl ResumeAttempt {
    async fn wait(mut self, instance: &str) -> Result<ExitStatus, String> {
        let exit = self.child.wait().await.map_err(|error| error.to_string())?;
        // A failing CLI may have submitted the turn. Leave the claim in place.
        if exit.success() {
            store::settle(instance, self.thread, &self.launch, "submitted")?;
        }
        Ok(exit)
    }
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    #[cfg(not(windows))]
    let _ = command;
}

fn cli_command(context: &CodexLaunchContext, cli: &CodexCliResolvedRuntime) -> Command {
    let mut command = if let Some(node) = &cli.node_path {
        let mut command = Command::new(node);
        command.arg(&cli.binary_path);
        command
    } else {
        Command::new(&cli.binary_path)
    };
    let proxy_exceptions = format!(
        "{},{}",
        std::env::var("NO_PROXY").unwrap_or_default(),
        std::env::var("no_proxy").unwrap_or_default()
    );
    let proxy_exceptions = codex_protocol::merge_local_no_proxy(&proxy_exceptions);
    command
        .env("CODEX_HOME", &context.user_data_dir)
        .env("NO_PROXY", &proxy_exceptions)
        .env("no_proxy", proxy_exceptions);
    if let Some(dir) = &context.working_dir {
        command.current_dir(dir);
    }
    command
}

fn same_home(left: &Path, right: &Path) -> bool {
    let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let (left, right) = (canonical(left), canonical(right));
    #[cfg(windows)]
    return left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy());
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn is_cli_executable(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("codex") || name.eq_ignore_ascii_case("codex.exe")
        })
}

struct CliOwner {
    pid: sysinfo::Pid,
    #[cfg(unix)]
    managed_daemon: bool,
}

pub(crate) async fn cli_allows_auto_switch(home: PathBuf) -> bool {
    let profile = home.clone();
    let owners = tokio::task::spawn_blocking(move || configured_cli_owners(&profile)).await;
    let Ok(Ok(owners)) = owners else {
        return false;
    };
    match owners.as_slice() {
        [] => true,
        #[cfg(unix)]
        [CliOwner {
            pid,
            managed_daemon: true,
        }] => crate::modules::codex_cli_daemon::managed_daemon_is_idle(&home, pid.as_u32()).await,
        _ => false,
    }
}

fn configured_cli_owners(home: &Path) -> Result<Vec<CliOwner>, String> {
    let runtime = codex_wakeup::load_runtime_config()?;
    let configured: Vec<_> = runtime
        .codex_cli_path
        .map(PathBuf::from)
        .into_iter()
        .chain(std::env::var_os("CODEX_CLI_PATH").map(PathBuf::from))
        .filter(|path| path.is_file())
        .collect();
    profile_cli_owners(
        home,
        &configured,
        runtime.node_path.as_deref().map(Path::new),
    )
}

fn require_profile_idle(
    home: &Path,
    configured: &[PathBuf],
    node: Option<&Path>,
) -> Result<(), String> {
    if let Some(owner) = profile_cli_owners(home, configured, node)?.first() {
        return Err(format!(
            "此实例已有运行中的 Codex CLI（PID {}），请在原终端继续",
            owner.pid
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn is_managed_daemon(args: &[std::ffi::OsString]) -> bool {
    args.get(1).is_some_and(|arg| arg == "app-server")
        && args
            .windows(2)
            .any(|pair| pair[0] == "--listen" && pair[1] == "unix://")
        && args.iter().any(|arg| arg == "--managed-daemon")
}

fn profile_cli_owners(
    home: &Path,
    configured: &[PathBuf],
    node: Option<&Path>,
) -> Result<Vec<CliOwner>, String> {
    let mut system = System::new();
    let identity = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_user(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, identity);
    let current = sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid))
        .ok_or("无法核对运行中的 Codex CLI")?;
    let current_user = current.user_id().cloned();
    let canonical: Vec<_> = configured
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    let is_configured_cli = |path: &Path| {
        configured
            .iter()
            .chain(&canonical)
            .any(|candidate| candidate == path)
    };
    let is_cli = |candidate: &sysinfo::Process| {
        if matches!(
            candidate.status(),
            ProcessStatus::Dead | ProcessStatus::Zombie
        ) || matches!((current_user.as_ref(), candidate.user_id()), (Some(ours), Some(theirs)) if ours != theirs)
        {
            return false;
        }
        let executable = candidate
            .exe()
            .unwrap_or_else(|| Path::new(candidate.name()));
        let node_process = node.is_some_and(|node| executable == node)
            || executable
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| {
                    name.eq_ignore_ascii_case("node") || name.eq_ignore_ascii_case("node.exe")
                });
        is_cli_executable(executable)
            || is_configured_cli(executable)
            || node_process
                && candidate
                    .cmd()
                    .iter()
                    .skip(1)
                    .any(|arg| is_configured_cli(Path::new(arg)))
    };
    let pids: Vec<_> = system
        .processes()
        .values()
        .filter(|process| is_cli(process))
        .map(sysinfo::Process::pid)
        .collect();
    if pids.is_empty() {
        return Ok(Vec::new());
    }
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        identity
            .with_cwd(UpdateKind::Always)
            .with_environ(UpdateKind::Always),
    );
    let default_home = dirs::home_dir().map(|path| path.join(".codex"));
    let mut owners = Vec::new();
    for candidate in pids.into_iter().filter_map(|pid| system.process(pid)) {
        if !is_cli(candidate) {
            continue;
        }
        let environment = candidate.environ();
        if environment.is_empty() {
            return Err("无法确认一个运行中的 Codex CLI 属于哪个实例".into());
        }
        let value = |key: &str| {
            environment.iter().find_map(|entry| {
                let entry = entry.to_string_lossy();
                let (name, value) = entry.split_once('=')?;
                (name == key || cfg!(windows) && name.eq_ignore_ascii_case(key))
                    .then(|| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
        };
        #[cfg(windows)]
        let home_variable = "USERPROFILE";
        #[cfg(not(windows))]
        let home_variable = "HOME";
        let owner = value("CODEX_HOME")
            .or_else(|| value(home_variable).map(|root| root.join(".codex")))
            .or_else(|| default_home.clone())
            .ok_or("无法确认 Codex 默认目录")?;
        let owner = if owner.is_absolute() {
            owner
        } else {
            candidate
                .cwd()
                .ok_or("无法确认 Codex 相对实例目录")?
                .join(owner)
        };
        if same_home(home, &owner) {
            owners.push(CliOwner {
                pid: candidate.pid(),
                #[cfg(unix)]
                managed_daemon: is_managed_daemon(candidate.cmd()),
            });
        }
    }
    Ok(owners)
}

async fn select_thread(
    context: &CodexLaunchContext,
    cli: &CodexCliResolvedRuntime,
    settings: &ResumeSettings,
    previous: Option<Uuid>,
) -> Result<Uuid, String> {
    let mut command = cli_command(context, cli);
    hide_console(&mut command);
    let mut child = command
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut input = child
        .stdin
        .take()
        .ok_or("Codex app-server 未提供标准输入")?;
    let mut output =
        BufReader::new(child.stdout.take().ok_or("Codex app-server 未提供返回流")?).lines();
    let selected =
        tokio::time::timeout(Duration::from_secs(7), async {
            let mut rpc =
                |id, method, params| json!({"id": id, "method": method, "params": params});
            exchange(&mut input, &mut output, rpc(1, "initialize", json!({
            "clientInfo": {"name":"cockpit-cli-resume","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }))).await?;
            input
                .write_all(b"{\"method\":\"initialized\"}\n")
                .await
                .map_err(|error| error.to_string())?;
            let requested = match settings.target {
                ResumeTarget::Specific => settings.thread_id,
                ResumeTarget::PreviousDesktopSession => previous,
                ResumeTarget::RecentActivity | ResumeTarget::LastCreated => {
                    let sort_key = if settings.target == ResumeTarget::LastCreated {
                        "created_at"
                    } else {
                        "updated_at"
                    };
                    let list = exchange(&mut input, &mut output, rpc(2, "thread/list", json!({
                    "archived":false,"currentAccountOnly":false,"limit":1,"sortKey":sort_key,
                    "sourceKinds":["cli","vscode","exec","appServer","unknown"]
                }))).await?;
                    list.pointer("/data/0/id")
                        .and_then(Value::as_str)
                        .and_then(|id| Uuid::parse_str(id).ok())
                }
            }
            .ok_or("找不到要续跑的会话")?;
            let record = exchange(
                &mut input,
                &mut output,
                rpc(
                    3,
                    "thread/read",
                    json!({
                        "threadId": requested, "includeTurns": false
                    }),
                ),
            )
            .await?;
            if record
                .pointer("/thread/id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
                != Some(requested)
            {
                return Err("当前实例无法读到指定的会话".to_owned());
            }
            // This is an isolated reader. notLoaded only identifies an on-disk
            // thread; the independent process check establishes whether it may be in use.
            Ok(requested)
        })
        .await
        .map_err(|_| "读取会话超时".to_owned());
    child.kill().await.map_err(|error| error.to_string())?;
    selected?
}

async fn exchange(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    message: Value,
) -> Result<Value, String> {
    let id = message["id"].clone();
    let mut line = message.to_string();
    line.push('\n');
    input
        .write_all(line.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    while let Some(raw) = output
        .next_line()
        .await
        .map_err(|error| error.to_string())?
    {
        let Ok(message) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if message.get("id") != Some(&id) {
            continue;
        }
        if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| "Codex 返回了无效的会话结果".into());
    }
    Err("Codex 在返回会话之前退出".into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct FixtureProcess(std::process::Child);

    impl Drop for FixtureProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn cli_owners_defer_only_the_matching_profile_until_the_process_exits() {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("target profile");
        let other_home = sandbox.path().join("other profile");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&other_home).unwrap();
        let executable = sandbox.path().join("custom-codex-runtime");
        std::fs::copy("/bin/sleep", &executable).unwrap();
        let start = |profile: &Path| {
            FixtureProcess(
                std::process::Command::new(&executable)
                    .arg("30")
                    .current_dir(sandbox.path())
                    .env("CODEX_HOME", profile)
                    .spawn()
                    .unwrap(),
            )
        };

        let _other = start(&other_home);
        require_profile_idle(&home, std::slice::from_ref(&executable), None).unwrap();
        let same = start(Path::new("target profile"));
        let error =
            require_profile_idle(&home, std::slice::from_ref(&executable), None).unwrap_err();
        assert!(error.contains(&format!("PID {}", same.0.id())), "{error}");
        drop(same);
        require_profile_idle(&home, std::slice::from_ref(&executable), None).unwrap();
    }
}
