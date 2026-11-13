use std::path::{Path, PathBuf};

pub(crate) fn vscode_launch_path_is_insiders(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "visual studio code - insiders.app"
                | "microsoft vs code insiders"
                | "code - insiders.exe"
                | "code-insiders.exe"
                | "code-insiders"
        )
    })
}

pub fn vscode_data_root_candidates() -> Result<Vec<PathBuf>, String> {
    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        let appdata = PathBuf::from(appdata);
        return Ok(vec![appdata.join("Code"), appdata.join("Code - Insiders")]);
    }

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        let base = home.join("Library").join("Application Support");
        return Ok(vec![base.join("Code"), base.join("Code - Insiders")]);
    }

    #[cfg(target_os = "linux")]
    {
        let base = if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
            let trimmed = xdg_config_home.trim();
            if trimmed.is_empty() {
                dirs::home_dir()
                    .ok_or("无法获取用户主目录".to_string())?
                    .join(".config")
            } else {
                PathBuf::from(trimmed)
            }
        } else {
            dirs::home_dir()
                .ok_or("无法获取用户主目录".to_string())?
                .join(".config")
        };
        return Ok(vec![base.join("Code"), base.join("Code - Insiders")]);
    }

    #[allow(unreachable_code)]
    Err("GitHub Copilot 仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn resolve_preferred_vscode_data_root() -> Result<PathBuf, String> {
    let candidates = vscode_data_root_candidates()?;
    let configured = crate::modules::config::get_user_config().vscode_app_path;
    if !configured.trim().is_empty() {
        let index = usize::from(vscode_launch_path_is_insiders(&configured));
        return candidates
            .into_iter()
            .nth(index)
            .ok_or_else(|| "未找到 VS Code 候选目录".to_string());
    }
    for root in &candidates {
        if root.exists() {
            return Ok(root.clone());
        }
    }
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| "未找到 VS Code 候选目录".to_string())
}

pub fn resolve_vscode_data_root_for_state_db() -> Result<PathBuf, String> {
    let candidates = vscode_data_root_candidates()?;
    for root in &candidates {
        if vscode_state_db_path(root).exists() || vscode_shared_storage_db_exists(root) {
            return Ok(root.clone());
        }
    }
    for root in &candidates {
        if root.exists() {
            return Ok(root.clone());
        }
    }
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| "未找到 VS Code 候选目录".to_string())
}

pub fn resolve_vscode_data_root(user_data_dir: Option<&str>) -> Result<PathBuf, String> {
    if let Some(raw) = user_data_dir {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    resolve_vscode_data_root_for_state_db()
}

pub fn vscode_state_db_path(data_root: &Path) -> PathBuf {
    data_root
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

#[cfg(target_os = "windows")]
pub fn vscode_local_state_path(data_root: &Path) -> PathBuf {
    data_root.join("Local State")
}

#[cfg(target_os = "windows")]
pub fn vscode_shared_storage_db_path(data_root: &Path) -> Option<PathBuf> {
    let folder_name = data_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(|name| {
            if name.eq_ignore_ascii_case("Code - Insiders") {
                ".vscode-shared-insiders"
            } else {
                ".vscode-shared"
            }
        })?;

    Some(
        dirs::home_dir()?
            .join(folder_name)
            .join("sharedStorage")
            .join("state.vscdb"),
    )
}

pub fn vscode_shared_storage_db_exists(data_root: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        vscode_shared_storage_db_path(data_root)
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = data_root;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::vscode_launch_path_is_insiders;

    #[test]
    fn vscode_launch_path_selects_the_matching_editor_edition() {
        for path in [
            r"C:\Programs\Microsoft VS Code Insiders\Code - Insiders.exe",
            "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Electron",
            "/usr/bin/code-insiders",
        ] {
            assert!(vscode_launch_path_is_insiders(path), "{path}");
        }
        for path in [
            r"C:\Programs\Microsoft VS Code\Code.exe",
            "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
            "/tmp/project-insiders/code",
        ] {
            assert!(!vscode_launch_path_is_insiders(path), "{path}");
        }
    }
}
