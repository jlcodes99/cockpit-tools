use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value as JsonValue};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "macos")]
const CODEX_APP_SERVER_EXECUTABLE: &str = "/Applications/Codex.app/Contents/Resources/codex";
const CODEX_APP_SERVER_EXECUTABLE_ENV: &str = "CODEX_APP_SERVER_EXECUTABLE";
const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
struct AppServerLaunchSpec {
    executable: PathBuf,
    node_path: Option<PathBuf>,
}

impl AppServerLaunchSpec {
    fn direct(executable: PathBuf) -> Self {
        Self {
            executable,
            node_path: None,
        }
    }
}

pub fn rebuild_thread_metadata(codex_home: &Path) -> Result<(), String> {
    crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &codex_home.join("config.toml"),
    )?;
    let launch_spec = official_app_server_launch_spec()?;
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] starting rebuild_thread_metadata: executable={}, codex_home={}",
        launch_spec.executable.display(),
        codex_home.display()
    ));
    let mut child = spawn_app_server_command(&launch_spec, codex_home)?;

    let stdout = child
        .stdout
        .take()
        .ok_or("无法读取官方 app-server stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("无法读取官方 app-server stderr")?;
    let mut stdin = child.stdin.take().ok_or("无法写入官方 app-server stdin")?;
    let (sender, receiver) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Official AppServer][stderr] {}",
                line
            ));
        }
    });

    let result = (|| {
        send_request(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "cockpit-tools",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": null,
                },
            }),
        )?;
        wait_for_response(&receiver, 1)?;

        send_request(
            &mut stdin,
            json!({
                "method": "thread/list",
                "id": 2,
                "params": {
                    "cursor": null,
                    "limit": 1,
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                    "modelProviders": null,
                    "sourceKinds": [],
                    "archived": false,
                },
            }),
        )?;
        wait_for_response(&receiver, 2)?;
        Ok::<(), String>(())
    })();

    finish_child(&mut child);
    let _ = reader.join();
    let _ = stderr_reader.join();
    if let Err(error) = &result {
        crate::modules::logger::log_warn(&format!(
            "[Codex Official AppServer] rebuild_thread_metadata failed: codex_home={}, error={}",
            codex_home.display(),
            error
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[Codex Official AppServer] rebuild_thread_metadata completed: codex_home={}",
            codex_home.display()
        ));
    }
    result
}

fn official_app_server_launch_spec() -> Result<AppServerLaunchSpec, String> {
    let mut candidates = Vec::new();
    if let Some(executable) = std::env::var_os(CODEX_APP_SERVER_EXECUTABLE_ENV) {
        if !executable.as_os_str().is_empty() {
            candidates.push(AppServerLaunchSpec::direct(PathBuf::from(executable)));
        }
    }
    if let Some(launch_spec) = configured_app_server_launch_spec() {
        candidates.push(launch_spec);
    }
    if let Some(launch_spec) = cli_runtime_app_server_launch_spec() {
        candidates.push(launch_spec);
    }
    #[cfg(target_os = "macos")]
    candidates.push(AppServerLaunchSpec::direct(PathBuf::from(
        CODEX_APP_SERVER_EXECUTABLE,
    )));

    for candidate in &candidates {
        if candidate.executable.exists() {
            return Ok(candidate.clone());
        }
    }

    let searched_paths = candidates
        .iter()
        .map(|candidate| candidate.executable.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "未找到官方 Codex app-server 可执行文件: {}",
        searched_paths
    ))
}

fn cli_runtime_app_server_launch_spec() -> Option<AppServerLaunchSpec> {
    let runtime = crate::modules::codex_wakeup::resolve_cli_runtime().ok()?;
    Some(AppServerLaunchSpec {
        executable: PathBuf::from(runtime.binary_path),
        node_path: runtime.node_path.map(PathBuf::from),
    })
}

fn configured_app_server_launch_spec() -> Option<AppServerLaunchSpec> {
    let launch_path = crate::modules::process::resolve_codex_launch_path().ok()?;
    derive_app_server_executable_from_launch_path(&launch_path).map(AppServerLaunchSpec::direct)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppServerLaunchPlatform {
    Windows,
    Macos,
    Other,
}

impl AppServerLaunchPlatform {
    fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            _ => Self::Other,
        }
    }
}

fn derive_app_server_executable_from_launch_path(launch_path: &Path) -> Option<PathBuf> {
    derive_app_server_executable_for_platform(launch_path, AppServerLaunchPlatform::current())
}

fn derive_app_server_executable_for_platform(
    launch_path: &Path,
    platform: AppServerLaunchPlatform,
) -> Option<PathBuf> {
    match platform {
        AppServerLaunchPlatform::Windows => {
            let parent = launch_path.parent()?;
            let file_name = launch_path.file_name().and_then(|value| value.to_str())?;
            let file_name_is_codex = file_name.eq_ignore_ascii_case("codex.exe");
            let parent_name = parent
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            if file_name_is_codex
                && matches!(parent_name.as_deref(), Some("resources") | Some("bin"))
            {
                return Some(launch_path.to_path_buf());
            }
            let bundled_app_server = parent.join("resources").join("codex.exe");
            if bundled_app_server.exists() || file_name == "Codex.exe" {
                return Some(bundled_app_server);
            }
            if file_name_is_codex {
                return Some(launch_path.to_path_buf());
            }
            None
        }
        AppServerLaunchPlatform::Macos => {
            let parent = launch_path.parent()?;
            let already_resource_binary = launch_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value == "codex")
                .unwrap_or(false)
                && parent
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("Resources"))
                    .unwrap_or(false);
            if already_resource_binary {
                return Some(launch_path.to_path_buf());
            }
            if let Some(app_root) = macos_app_root_from_path(launch_path) {
                return Some(app_root.join("Contents").join("Resources").join("codex"));
            }
            Some(launch_path.to_path_buf())
        }
        AppServerLaunchPlatform::Other => Some(launch_path.to_path_buf()),
    }
}

fn macos_app_root_from_path(path: &Path) -> Option<PathBuf> {
    let path_text = path.to_string_lossy();
    let app_index = path_text.find(".app")?;
    Some(PathBuf::from(&path_text[..app_index + 4]))
}

fn build_app_server_command(launch_spec: &AppServerLaunchSpec, codex_home: &Path) -> Command {
    let mut command = if let Some(node_path) = launch_spec.node_path.as_ref() {
        let mut command = Command::new(node_path);
        command.arg(&launch_spec.executable);
        command
    } else {
        Command::new(&launch_spec.executable)
    };
    crate::modules::process::apply_managed_proxy_env_to_command(&mut command);
    command
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

fn spawn_app_server_command(
    launch_spec: &AppServerLaunchSpec,
    codex_home: &Path,
) -> Result<Child, String> {
    let mut command = build_app_server_command(launch_spec, codex_home);
    match command.spawn() {
        Ok(child) => Ok(child),
        Err(error) => {
            #[cfg(target_os = "windows")]
            {
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && launch_spec.node_path.is_none()
                    && launch_spec
                        .executable
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains("\\windowsapps\\")
                {
                    if let Some(local_copy) = local_appdata_codex_cli_executable() {
                        let mut local_copy_command = build_app_server_command(
                            &AppServerLaunchSpec::direct(local_copy.clone()),
                            codex_home,
                        );
                        if let Ok(child) = local_copy_command.spawn() {
                            crate::modules::logger::log_warn(&format!(
                                "[Codex Official AppServer] WindowsApps direct launch denied, fallback to LOCALAPPDATA copy succeeded: original={} fallback={}",
                                launch_spec.executable.display(),
                                local_copy.display()
                            ));
                            return Ok(child);
                        }
                    }

                    let staged_executable =
                        stage_windows_app_server_executable(&launch_spec.executable)?;
                    let mut staged_command = build_app_server_command(
                        &AppServerLaunchSpec::direct(staged_executable.clone()),
                        codex_home,
                    );
                    return staged_command.spawn().map_err(|staged_error| {
                        format!(
                            "启动官方 Codex app-server 失败 ({} / CODEX_HOME={}): {}; LOCALAPPDATA 副本与 Windows 临时副本重试均失败，最后一次重试 ({}): {}",
                            launch_spec.executable.display(),
                            codex_home.display(),
                            error,
                            staged_executable.display(),
                            staged_error
                        )
                    });
                }
            }

            Err(format!(
                "启动官方 Codex app-server 失败 ({} / CODEX_HOME={}): {}",
                launch_spec.executable.display(),
                codex_home.display(),
                error
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn local_appdata_codex_cli_executable() -> Option<PathBuf> {
    let local_appdata = std::env::var_os("LOCALAPPDATA")?;
    let path = PathBuf::from(local_appdata)
        .join("OpenAI")
        .join("Codex")
        .join("bin")
        .join("codex.exe");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn stage_windows_app_server_executable(executable: &Path) -> Result<PathBuf, String> {
    let temp_dir = std::env::temp_dir().join("cockpit-tools-codex-appserver");
    std::fs::create_dir_all(&temp_dir).map_err(|error| {
        format!(
            "创建 Codex app-server 临时目录失败 ({}): {}",
            temp_dir.display(),
            error
        )
    })?;

    let file_name = executable.file_name().ok_or_else(|| {
        format!(
            "无法确定官方 Codex app-server 文件名: {}",
            executable.display()
        )
    })?;
    let staged_path = temp_dir.join(file_name);
    std::fs::copy(executable, &staged_path).map_err(|error| {
        format!(
            "复制官方 Codex app-server 到临时目录失败 ({} -> {}): {}",
            executable.display(),
            staged_path.display(),
            error
        )
    })?;
    Ok(staged_path)
}

fn send_request(stdin: &mut impl Write, request: JsonValue) -> Result<(), String> {
    let line = serde_json::to_string(&request)
        .map_err(|error| format!("序列化官方 app-server 请求失败: {}", error))?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("写入官方 app-server 请求失败: {}", error))
}

fn wait_for_response(receiver: &mpsc::Receiver<String>, request_id: i64) -> Result<(), String> {
    loop {
        let line = receiver
            .recv_timeout(APP_SERVER_RESPONSE_TIMEOUT)
            .map_err(|_| format!("等待官方 app-server 响应超时 (id={})", request_id))?;
        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        if value.get("id").and_then(JsonValue::as_i64) != Some(request_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            crate::modules::logger::log_warn(&format!(
                "[Codex Official AppServer] response error: id={}, error={}",
                request_id, error
            ));
            return Err(format!(
                "官方 app-server 返回错误 (id={}): {}",
                request_id, error
            ));
        }
        if value.get("result").is_some() {
            return Ok(());
        }
        return Err(format!(
            "官方 app-server 响应缺少 result (id={}): {}",
            request_id, value
        ));
    }
}

fn finish_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::{derive_app_server_executable_for_platform, AppServerLaunchPlatform};
    use std::path::Path;

    #[test]
    fn derives_windows_app_server_from_gui_launch_path() {
        let root_dir = make_temp_dir("codex-appserver-windows-gui-test");
        let app_dir = root_dir.join("app");
        let resources_dir = app_dir.join("resources");
        std::fs::create_dir_all(&resources_dir).expect("create resources dir");
        let launch_path = app_dir.join("Codex.exe");
        let app_server_path = resources_dir.join("codex.exe");
        std::fs::write(&launch_path, b"gui").expect("write gui executable");
        std::fs::write(&app_server_path, b"app-server").expect("write app-server executable");
        let derived = derive_app_server_executable_for_platform(
            &launch_path,
            AppServerLaunchPlatform::Windows,
        )
        .expect("expected Windows app-server path");
        assert_eq!(derived, app_server_path);
        std::fs::remove_dir_all(root_dir).expect("cleanup temp dir");
    }

    #[test]
    fn keeps_windows_local_appdata_cli_path_unchanged() {
        let launch_path = Path::new("C:")
            .join("Users")
            .join("Example")
            .join("AppData")
            .join("Local")
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe");
        let derived = derive_app_server_executable_for_platform(
            &launch_path,
            AppServerLaunchPlatform::Windows,
        )
        .expect("expected Windows app-server path");
        assert_eq!(derived, launch_path);
    }

    #[test]
    fn keeps_windows_resource_launch_path_unchanged() {
        let launch_path = Path::new("C:")
            .join("Codex")
            .join("App")
            .join("resources")
            .join("codex.exe");
        let derived = derive_app_server_executable_for_platform(
            &launch_path,
            AppServerLaunchPlatform::Windows,
        )
        .expect("expected Windows app-server path");
        assert_eq!(derived, launch_path);
    }

    #[test]
    fn keeps_windows_standalone_cli_launch_path_unchanged() {
        let launch_path = Path::new("C:")
            .join("Tools")
            .join("CodexCli")
            .join("codex.exe");
        let derived = derive_app_server_executable_for_platform(
            &launch_path,
            AppServerLaunchPlatform::Windows,
        )
        .expect("expected Windows app-server path");
        assert_eq!(derived, launch_path);
    }

    #[test]
    fn derives_macos_app_server_from_app_gui_launch_path() {
        let launch_path = Path::new("/Applications/Codex.app/Contents/MacOS/Codex");
        let derived =
            derive_app_server_executable_for_platform(launch_path, AppServerLaunchPlatform::Macos)
                .expect("expected macOS app-server path");
        assert_eq!(
            derived,
            Path::new("/Applications/Codex.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn keeps_macos_resource_launch_path_unchanged() {
        let launch_path = Path::new("/Applications/Codex.app/Contents/Resources/codex");
        let derived =
            derive_app_server_executable_for_platform(launch_path, AppServerLaunchPlatform::Macos)
                .expect("expected macOS app-server path");
        assert_eq!(derived, launch_path);
    }

    #[test]
    fn keeps_macos_standalone_cli_launch_path_unchanged() {
        let launch_path = Path::new("/usr/local/bin/codex");
        let derived =
            derive_app_server_executable_for_platform(launch_path, AppServerLaunchPlatform::Macos)
                .expect("expected macOS CLI path");
        assert_eq!(derived, launch_path);
    }

    #[test]
    fn keeps_linux_standalone_cli_launch_path_unchanged() {
        let launch_path = Path::new("/usr/local/bin/codex");
        let derived =
            derive_app_server_executable_for_platform(launch_path, AppServerLaunchPlatform::Other)
                .expect("expected Linux CLI path");
        assert_eq!(derived, launch_path);
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            std::fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        std::fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }
}
