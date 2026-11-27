use std::path::{Path, PathBuf};
use std::time::Duration;

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

fn configured_cli_path(app: &str, config: &config::UserConfig) -> Option<String> {
    let path = match app {
        "claude_cli" => &config.claude_cli_path,
        "antigravity_cli" => &config.antigravity_cli_path,
        "codebuddy_cli" => &config.codebuddy_cli_path,
        "qoder_cli" => &config.qoder_cli_path,
        "qoder_cn_cli" => &config.qoder_cn_cli_path,
        _ => return None,
    };
    Some(process::normalize_windows_user_facing_path(path))
}

fn detect_cli_path(app: &str) -> Option<PathBuf> {
    match app {
        "claude_cli" => detect_claude_cli(),
        "antigravity_cli" => super::account::cli_launch::detect_cli(),
        "codebuddy_cli" => crate::modules::codebuddy_cli::detect_cli(),
        "qoder_cli" | "qoder_cn_cli" => crate::modules::qoder_cn_clients::detect_cli(app == "qoder_cn_cli"),
        _ => None,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaunchPathStatus {
    configured_path: Option<String>,
    resolved_path: Option<String>,
    uses_configured_path: bool,
}

#[tauri::command]
pub(crate) async fn get_platform_launch_path(app: String) -> Result<LaunchPathStatus, String> {
    run_launch_path_probe(move || {
        if let Some(path) = configured_cli_path(&app, &config::get_user_config()) {
            let configured_path = (!path.is_empty()).then_some(path);
            let uses_configured_path = configured_path
                .as_deref()
                .is_some_and(|path| valid_cli_path(&app, Path::new(path)));
            let resolved_path = if uses_configured_path {
                configured_path.clone()
            } else if configured_path.is_none() {
                detect_cli_path(&app).map(path_for_display)
            } else {
                None
            };
            return Ok(LaunchPathStatus {
                configured_path,
                resolved_path,
                uses_configured_path,
            });
        }
        if matches!(app.as_str(), "qoder" | "qoder_desktop" | "qoder_cn" | "qoder_cn_ide") {
            let config = config::get_user_config();
            let configured = match app.as_str() {
                "qoder" => &config.qoder_app_path,
                "qoder_desktop" => &config.qoder_desktop_app_path,
                "qoder_cn" => &config.qoder_cn_app_path,
                _ => &config.qoder_cn_ide_path,
            };
            let configured_path = (!configured.trim().is_empty()).then(|| configured.clone());
            let (resolved, uses_configured_path) = if let Some(path) = &configured_path {
                let resolved = qoder_app_candidate(&app, Some(path));
                let valid = resolved.is_some();
                (resolved, valid)
            } else {
                (qoder_app_candidate(&app, None), false)
            };
            return Ok(LaunchPathStatus {
                configured_path,
                resolved_path: resolved.map(path_for_display),
                uses_configured_path,
            });
        }
        let path = process::inspect_preview_desktop_path(&app).ok_or("UNKNOWN_LAUNCH_TARGET")?;
        Ok(LaunchPathStatus {
            configured_path: path.configured_path,
            resolved_path: path.resolved_path.map(path_for_display),
            uses_configured_path: path.uses_configured_path,
        })
    })
    .await
}

fn qoder_app_candidate(app: &str, path: Option<&str>) -> Option<PathBuf> {
    if app == "qoder" {
        return process::qoder_ide_launch_candidate(path);
    }
    if app == "qoder_desktop" {
        return crate::modules::qoder_cn_clients::resolve_global_desktop(path);
    }
    if app == "qoder_cn_ide" {
        return crate::modules::qoder_cn_clients::resolve_ide(path);
    }
    #[cfg(target_os = "macos")]
    if let Some(path) = path {
        use crate::modules::qoder_cn_clients::{mac_executable, Client};
        return mac_executable(Path::new(path), Client::Desktop);
    }
    path.map(PathBuf::from)
        .filter(|path| executable_file(path))
        .or_else(|| (path.is_none()).then(process::detect_qoder_cn_exec_path).flatten())
}

fn valid_cli_path(app: &str, path: &Path) -> bool {
    if matches!(app, "qoder_cli" | "qoder_cn_cli") {
        return crate::modules::qoder_cn_clients::is_cli_path(path, app == "qoder_cn_cli");
    }
    executable_file(path)
}

fn path_for_display(path: PathBuf) -> String {
    process::normalize_windows_user_facing_path(&path.to_string_lossy())
}

fn matches_desktop_scan_target(app: &str, path: &str) -> bool {
    let normalized = path.to_ascii_lowercase().replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or_default();
    let cn_install = normalized
        .split('/')
        .any(|part| matches!(part, "codebuddy cn" | "codebuddy-cn" | "codebuddy_cn"));
    match app {
        "codebuddy" => name == "codebuddy.exe" && !cn_install,
        "codebuddy_cn" => name == "codebuddy cn.exe" || (name == "codebuddy.exe" && cn_install),
        "antigravity" => matches!(name, "antigravity ide.exe" | "antigravity-ide.exe"),
        "antigravity_legacy" => {
            name == "antigravity.exe"
                || (name == "electron.exe" && normalized.contains("/antigravity/"))
        }
        _ => false,
    }
}

#[tauri::command]
pub(crate) async fn scan_platform_launch_paths(app: String) -> Result<Vec<String>, String> {
    run_launch_path_probe(move || {
        if configured_cli_path(&app, &config::get_user_config()).is_some() {
            return Ok(detect_cli_path(&app)
                .map(path_for_display)
                .into_iter()
                .collect());
        }
        if matches!(app.as_str(), "qoder" | "qoder_desktop" | "qoder_cn" | "qoder_cn_ide") {
            return Ok(qoder_app_candidate(&app, None).map(path_for_display).into_iter().collect());
        }
        if !matches!(
            app.as_str(),
            "antigravity" | "antigravity_legacy" | "codebuddy" | "codebuddy_cn"
        ) {
            return Err("UNKNOWN_LAUNCH_TARGET".into());
        }
        let mut paths: Vec<String> = process::detect_preview_desktop_path(&app)
            .map(path_for_display)
            .into_iter()
            .collect();
        if cfg!(target_os = "windows") {
            match process::scan_app_launch_targets(&app, None) {
                Ok(candidates) => {
                    for candidate in candidates {
                        if candidate.target_type == "exe"
                            && matches_desktop_scan_target(&app, &candidate.target)
                            && !paths.contains(&candidate.target)
                        {
                            paths.push(candidate.target);
                        }
                    }
                }
                Err(error) if paths.is_empty() => return Err(error),
                Err(_) => {}
            }
        }
        Ok(paths)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::matches_desktop_scan_target;

    #[test]
    fn desktop_scan_distinguishes_client_variants_and_ignores_cli_processes() {
        let international = r"C:\Apps\CodeBuddy\CodeBuddy.exe";
        let domestic = r"C:\Apps\CodeBuddy CN\CodeBuddy.exe";
        assert!(matches_desktop_scan_target("codebuddy", international));
        assert!(!matches_desktop_scan_target("codebuddy_cn", international));
        assert!(matches_desktop_scan_target("codebuddy_cn", domestic));
        assert!(!matches_desktop_scan_target("codebuddy", domestic));
        assert!(!matches_desktop_scan_target(
            "antigravity_legacy",
            r"C:\Apps\Antigravity\agy.exe"
        ));
    }
}

async fn run_launch_path_probe<T: Send + 'static>(
    probe: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let worker = tokio::task::spawn_blocking(probe);
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .map_err(|_| "LAUNCH_PATH_CHECK_TIMEOUT".to_string())?
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn inspect_cli_launch_path(
    app: String,
    detect: bool,
) -> Result<Option<String>, String> {
    run_launch_path_probe(move || {
        let configured =
            configured_cli_path(&app, &config::get_user_config()).ok_or("UNKNOWN_CLI_TARGET")?;
        let path = if detect {
            detect_cli_path(&app).map(path_for_display)
        } else {
            Some(configured)
        };
        Ok(path.filter(|path| !path.is_empty()))
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_cli_launch_path(app: String, path: String) -> Result<(), String> {
    let platform = app.clone();
    tokio::task::spawn_blocking(move || {
        let path = process::normalize_windows_user_facing_path(path.trim());
        if !path.is_empty() && !valid_cli_path(&app, Path::new(&path)) {
            return Err(format!("CLI_PATH_NOT_FOUND:{app}"));
        }
        config::patch_user_config(move |config| {
            match app.as_str() {
                "claude_cli" => config.claude_cli_path = path,
                "antigravity_cli" => config.antigravity_cli_path = path,
                "codebuddy_cli" => config.codebuddy_cli_path = path,
                "qoder_cli" => config.qoder_cli_path = path,
                "qoder_cn_cli" => config.qoder_cn_cli_path = path,
                _ => return Err("UNKNOWN_CLI_TARGET".into()),
            }
            Ok(())
        })?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())??;
    crate::modules::t3code_provider_bindings::launch_paths::sync_saved_cli_path(&platform);
    Ok(())
}
