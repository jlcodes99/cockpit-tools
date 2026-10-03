use crate::modules;
use std::path::Path;

#[tauri::command]
pub async fn antigravity_cli_status() -> Result<modules::antigravity_cli::CliStatus, String> {
    modules::antigravity_cli::status().await
}

fn quote_path(value: &str, windows: bool) -> String {
    if windows {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn launch_command(executable: &str, directory: &str, windows: bool) -> String {
    if windows {
        format!(
            "Set-Location -LiteralPath {}; & {}",
            quote_path(directory, true),
            quote_path(executable, true)
        )
    } else {
        format!(
            "cd -- {} && {}",
            quote_path(directory, false),
            quote_path(executable, false)
        )
    }
}

#[tauri::command]
pub async fn antigravity_cli_launch(working_directory: Option<String>) -> Result<(), String> {
    let executable = modules::antigravity_cli::find_executable()
        .ok_or_else(|| "未找到 agy，请先安装 Antigravity CLI 并将其加入 PATH".to_string())?;
    let directory = working_directory
        .filter(|path| !path.trim().is_empty())
        .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned()))
        .ok_or_else(|| "无法定位工作目录".to_string())?;
    if !Path::new(&directory).is_absolute() || !Path::new(&directory).is_dir() {
        return Err("CLI 工作目录必须是已存在的绝对路径".to_string());
    }
    let command = launch_command(&executable.to_string_lossy(), &directory, cfg!(windows));
    let terminal = modules::config::get_user_config().default_terminal;
    let plan =
        super::codex_instance::terminal::build_codex_terminal_launch_plan(&command, &terminal)?;
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new(&plan.program)
                .args(&plan.args)
                .output()
                .map_err(|e| format!("无法打开 CLI 终端 ({}): {e}", plan.terminal_name))?;
            if !output.status.success() {
                return Err("终端未能启动 Antigravity CLI".to_string());
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut child = std::process::Command::new(&plan.program)
                .args(&plan.args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("无法打开 CLI 终端 ({}): {e}", plan.terminal_name))?;
            // Reap terminal processes without blocking the command or leaking zombies.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Ok(())
    })
    .await
    .map_err(|_| "启动 Antigravity CLI 任务失败".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_paths_are_quoted_for_each_shell() {
        assert_eq!(
            launch_command("/tmp/a'gy", "/tmp/$(touch nope)", false),
            "cd -- '/tmp/$(touch nope)' && '/tmp/a'\"'\"'gy'"
        );
        assert_eq!(
            launch_command("C:\\a'gy.exe", "C:\\$workspace", true),
            "Set-Location -LiteralPath 'C:\\$workspace'; & 'C:\\a''gy.exe'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_launch_does_not_evaluate_directory_or_executable_text() {
        let root = std::env::temp_dir().join(format!("agy-shell-test-{}", uuid::Uuid::new_v4()));
        let working = root.join("space ' $(touch INJECTED)");
        std::fs::create_dir_all(&working).unwrap();
        let command = launch_command("/bin/pwd", &working.to_string_lossy(), false);
        let output = std::process::Command::new("bash")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            working.to_str().unwrap()
        );
        assert!(!root.join("INJECTED").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
