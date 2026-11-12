use std::path::{Path, PathBuf};

pub(crate) fn user_data_override(args: &[String]) -> Option<&str> {
    let mut args = args.iter().peekable();
    let mut directory = None;
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if arg == "--user-data-dir" {
            if args.peek().is_some_and(|value| !value.starts_with("--")) {
                directory = args.next().map(String::as_str);
            }
        } else if let Some(value) = arg.strip_prefix("--user-data-dir=") {
            directory = Some(value);
        }
    }
    directory.filter(|value| !value.is_empty())
}

pub(crate) fn without_user_data_override(args: Vec<String>) -> Vec<String> {
    let mut source = args.into_iter().peekable();
    let mut remaining = Vec::new();
    while let Some(arg) = source.next() {
        if arg == "--" {
            remaining.push(arg);
            remaining.extend(source);
            break;
        }
        if arg == "--user-data-dir" {
            if source.peek().is_some_and(|value| !value.starts_with("--")) {
                source.next();
            }
        } else if !arg.starts_with("--user-data-dir=") {
            remaining.push(arg);
        }
    }
    remaining
}

pub(crate) fn portable_root(executable: Option<&Path>) -> Option<PathBuf> {
    let configured = std::env::var_os("VSCODE_PORTABLE").filter(|path| !path.is_empty());
    let install_dir = executable.and_then(Path::parent);
    let (product, adjacent) = if cfg!(target_os = "macos") {
        let bundle = install_dir.and_then(Path::parent).and_then(Path::parent);
        (
            bundle.map(|path| path.join("Contents/Resources/app/product.json")),
            bundle
                .and_then(Path::parent)
                .map(|path| path.join("qoder-portable-data")),
        )
    } else {
        (
            install_dir.map(|path| path.join("resources/app/product.json")),
            install_dir.map(|path| path.join("data")),
        )
    };
    let portable = configured.map(PathBuf::from).or(adjacent)?;
    if !portable.is_dir() {
        return None;
    }
    if product
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .is_some_and(|product| product.get("target").is_some())
    {
        return None;
    }
    Some(portable.join("user-data"))
}

pub(crate) fn user_data_dir(args: &[String], executable: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = portable_root(executable) {
        return absolute(path, executable);
    }
    if let Some(path) = std::env::var_os("VSCODE_APPDATA").filter(|path| !path.is_empty()) {
        return absolute(PathBuf::from(path).join("Qoder"), executable);
    }
    if let Some(path) = user_data_override(args) {
        return absolute(PathBuf::from(path), executable);
    }
    let home = || dirs::home_dir().ok_or_else(|| "QODER_DATA_ROOT_UNAVAILABLE".to_string());
    #[cfg(target_os = "windows")]
    let base = match std::env::var_os("APPDATA").filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home()?.join("AppData/Roaming"),
    };
    #[cfg(target_os = "macos")]
    let base = home()?.join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home()?.join(".config"),
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let base = home()?.join(".config");
    Ok(base.join("Qoder"))
}

fn absolute(path: PathBuf, executable: Option<&Path>) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd = if cfg!(target_os = "windows") {
        executable.and_then(Path::parent).map(Path::to_path_buf)
    } else {
        None
    };
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir().map_err(|_| "QODER_DATA_ROOT_UNAVAILABLE".to_string())?,
    };
    Ok(cwd.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_profile_flags_when_launching_the_resolved_directory() {
        let args = [
            "--user-data-dir=old",
            "--wait",
            "--user-data-dir",
            "new path",
            "--",
            "--user-data-dir=filename",
        ]
        .map(str::to_owned);
        assert_eq!(
            without_user_data_override(args.into()),
            ["--wait", "--", "--user-data-dir=filename"]
        );
    }

    #[test]
    fn reads_the_last_profile_argument_before_positional_arguments() {
        let args = [
            "--user-data-dir=old",
            "--user-data-dir",
            "new path",
            "--",
            "--user-data-dir=ignored",
        ]
        .map(str::to_owned);
        assert_eq!(user_data_override(&args), Some("new path"));
    }
}
