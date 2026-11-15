use std::path::{Path, PathBuf};

use crate::modules::{config, process};

pub(crate) const CLAUDE_CLI_MISSING: &str = "CLI_PATH_NOT_FOUND:claude_cli";

pub(crate) fn executable_file(path: &Path) -> bool {
    path.is_absolute()
        && path.metadata().is_ok_and(|metadata| {
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(windows)]
            {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ["exe", "cmd", "bat", "com"]
                            .iter()
                            .any(|name| ext.eq_ignore_ascii_case(name))
                    })
            }
            #[cfg(not(any(unix, windows)))]
            false
        })
}

pub(crate) fn detect_claude_cli() -> Option<PathBuf> {
    let mut dirs = crate::modules::codex_wakeup::collect_runtime_search_dirs();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
    }
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    dirs.into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|file| executable_file(file))
}

pub(crate) fn resolve_claude_cli() -> Result<PathBuf, String> {
    let configured = config::get_user_config().claude_cli_path;
    (if configured.is_empty() {
        detect_claude_cli()
    } else {
        let path = PathBuf::from(configured);
        executable_file(&path).then_some(path)
    })
    .ok_or_else(|| CLAUDE_CLI_MISSING.to_string())
}

#[tauri::command]
pub(crate) async fn inspect_cli_launch_path(
    app: String,
    detect: bool,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let config = config::get_user_config();
        let path = match (app.as_str(), detect) {
            ("claude_cli", true) => detect_claude_cli(),
            ("antigravity_cli", true) => super::account::cli_launch::detect_cli(),
            ("codebuddy_cli", true) => crate::modules::codebuddy_cli::detect_cli(),
            ("claude_cli", false) => Some(config.claude_cli_path.into()),
            ("antigravity_cli", false) => Some(config.antigravity_cli_path.into()),
            ("codebuddy_cli", false) => Some(config.codebuddy_cli_path.into()),
            _ => return Err("UNKNOWN_CLI_TARGET".into()),
        };
        Ok(path
            .map(|path| process::normalize_windows_user_facing_path(&path.to_string_lossy()))
            .filter(|path| !path.is_empty()))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn save_cli_launch_path(app: String, path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = process::normalize_windows_user_facing_path(path.trim());
        if !path.is_empty() && !executable_file(Path::new(&path)) {
            return Err(format!("CLI_PATH_NOT_FOUND:{app}"));
        }
        config::patch_user_config(move |config| {
            match app.as_str() {
                "claude_cli" => config.claude_cli_path = path,
                "antigravity_cli" => config.antigravity_cli_path = path,
                "codebuddy_cli" => config.codebuddy_cli_path = path,
                _ => return Err("UNKNOWN_CLI_TARGET".into()),
            }
            Ok(())
        })?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}
