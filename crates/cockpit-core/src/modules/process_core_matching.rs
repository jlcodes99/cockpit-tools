// cockpit-core Process：Running-process matching, PID resolution and window focus。
// 通过 include! 保持原模块作用域和跨平台调用路径。
#[cfg(test)]
mod codex_path_migration_tests {
    use super::should_migrate_legacy_codex_launch_path;
    use std::path::Path;

    #[test]
    fn migrates_official_macos_codex_path_when_chatgpt_exists() {
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app"),
            Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
        ));
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app/Contents/MacOS/Codex"),
            Path::new("/Applications/ChatGPT.app"),
        ));
    }

    #[test]
    fn keeps_macos_legacy_path_when_chatgpt_is_not_detected() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app"),
            Path::new("/Applications/Codex.app/Contents/MacOS/Codex"),
        ));
    }

    #[test]
    fn does_not_replace_custom_macos_codex_path() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new("/Users/test/Applications/Codex.app"),
            Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
        ));
    }
}

fn detect_opencode_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidate = std::path::PathBuf::from("/Applications/OpenCode.app");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(local_appdata)
                    .join("Programs")
                    .join("OpenCode")
                    .join("OpenCode.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("OpenCode")
                    .join("OpenCode.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "opencode",
            &["OpenCode.exe", "opencode.exe"],
            &["opencode"],
            &["opencode"],
            &["opencode", "open code"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/opencode", "/opt/opencode/opencode"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_antigravity_launch_path() -> Result<std::path::PathBuf, String> {
    let configured_path = config::get_user_config().antigravity_app_path;
    if let Some(custom) = normalize_custom_path(Some(&configured_path)) {
        #[cfg(target_os = "macos")]
        if is_legacy_antigravity_macos_path(&custom) {
            if let Some(detected) = detect_antigravity_exec_path() {
                update_app_path_in_config("antigravity", &detected, &configured_path);
                return Ok(detected);
            }
        }

        if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
            return Ok(exec);
        }
        if let Some(detected) = detect_antigravity_exec_path() {
            update_app_path_in_config("antigravity", &detected, &configured_path);
            return Ok(detected);
        }
        return Err(app_path_missing_error("antigravity"));
    }

    if let Some(detected) = detect_antigravity_exec_path() {
        update_app_path_in_config("antigravity", &detected, &configured_path);
        return Ok(detected);
    }

    Err(app_path_missing_error("antigravity"))
}

pub fn ensure_antigravity_launch_path_configured() -> Result<(), String> {
    resolve_antigravity_launch_path().map(|_| ())
}

pub fn ensure_vscode_launch_path_configured() -> Result<(), String> {
    resolve_vscode_launch_path().map(|_| ())
}

pub fn ensure_codex_launch_path_configured() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if detect_codex_store_app_user_model_id().is_some() {
            return Ok(());
        }
        if resolve_codex_launch_path().is_ok() {
            return Ok(());
        }
        return Err("未检测到 Codex 商店安装，请先在 Microsoft Store 安装 Codex".to_string());
    }

    #[cfg(not(target_os = "windows"))]
    {
        resolve_codex_launch_path().map(|_| ())
    }
}

pub fn ensure_codebuddy_launch_path_configured() -> Result<(), String> {
    resolve_codebuddy_launch_path().map(|_| ())
}

pub fn ensure_codebuddy_cn_launch_path_configured() -> Result<(), String> {
    resolve_codebuddy_cn_launch_path().map(|_| ())
}

pub fn ensure_qoder_launch_path_configured() -> Result<(), String> {
    resolve_qoder_launch_path().map(|_| ())
}

pub fn ensure_trae_launch_path_configured() -> Result<(), String> {
    resolve_trae_launch_path().map(|_| ())
}

pub fn ensure_workbuddy_launch_path_configured() -> Result<(), String> {
    resolve_workbuddy_launch_path().map(|_| ())
}

fn resolve_vscode_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().vscode_app_path)) {
        #[cfg(target_os = "macos")]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }
        return Err(app_path_missing_error("vscode"));
    }

    Err(app_path_missing_error("vscode"))
}

fn resolve_codebuddy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codebuddy_app_path))
    {
        if let Some(exec) = resolve_codebuddy_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("codebuddy"));
    }

    if let Some(detected) = detect_codebuddy_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_codebuddy_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codebuddy"))
}

fn resolve_codebuddy_cn_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) =
        normalize_custom_path(Some(&config::get_user_config().codebuddy_cn_app_path))
    {
        if let Some(exec) = resolve_codebuddy_cn_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("codebuddy_cn"));
    }

    if let Some(detected) = detect_codebuddy_cn_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_codebuddy_cn_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codebuddy_cn"))
}

fn resolve_qoder_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().qoder_app_path)) {
        if let Some(exec) = resolve_qoder_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("qoder"));
    }

    if let Some(detected) = detect_qoder_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_qoder_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("qoder"))
}

pub fn ensure_zed_launch_path_configured() -> Result<(), String> {
    resolve_zed_launch_path().map(|_| ())
}

pub fn resolve_zed_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().zed_app_path)) {
        if let Some(exec) = resolve_zed_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("zed"));
    }

    if let Some(detected) = detect_zed_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_zed_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("zed"))
}

fn resolve_trae_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().trae_app_path)) {
        if let Some(exec) = resolve_trae_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("trae"));
    }

    if let Some(detected) = detect_trae_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_trae_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("trae"))
}

fn resolve_workbuddy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().workbuddy_app_path))
    {
        if let Some(exec) = resolve_workbuddy_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("workbuddy"));
    }

    if let Some(detected) = detect_workbuddy_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_workbuddy_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("workbuddy"))
}

#[cfg(target_os = "macos")]
fn resolve_codex_launch_path() -> Result<std::path::PathBuf, String> {
    let configured_path = config::get_user_config().codex_app_path;
    if let Some(custom) = normalize_custom_path(Some(&configured_path)) {
        if let Some(migrated) = migrate_legacy_codex_launch_path(&custom) {
            return Ok(migrated);
        }
        if let Some(exec) = resolve_codex_macos_exec_path(&custom) {
            return Ok(exec);
        }
        if let Some(detected) = detect_codex_exec_path() {
            update_app_path_in_config("codex", &detected, &configured_path);
            return Ok(detected);
        }
        return Err(app_path_missing_error("codex"));
    }

    if let Some(detected) = detect_codex_exec_path() {
        update_app_path_in_config("codex", &detected, &configured_path);
        return Ok(detected);
    }

    Err(app_path_missing_error("codex"))
}

#[cfg(not(target_os = "macos"))]
fn resolve_codex_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codex_app_path)) {
        if let Some(exec) = resolve_macos_exec_path(&custom, "Codex") {
            return Ok(exec);
        }
        return Err(app_path_missing_error("codex"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(detected) = detect_codex_exec_path() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codex"))
}

pub fn detect_and_save_app_path(app: &str, force: bool) -> Option<String> {
    let current = config::get_user_config();
    match app {
        "antigravity" => {
            if !force && !current.antigravity_app_path.trim().is_empty() {
                return Some(current.antigravity_app_path);
            }
            if let Some(detected) = detect_antigravity_exec_path() {
                update_app_path_in_config("antigravity", &detected, &current.antigravity_app_path);
                return Some(config::get_user_config().antigravity_app_path);
            }
        }
        "codex" => {
            if !force && !current.codex_app_path.trim().is_empty() {
                #[cfg(target_os = "macos")]
                if migrate_legacy_codex_launch_path(&current.codex_app_path).is_some() {
                    return Some(config::get_user_config().codex_app_path);
                }
                return Some(current.codex_app_path);
            }
            if let Some(detected) = detect_codex_exec_path() {
                update_app_path_in_config("codex", &detected, &current.codex_app_path);
                return Some(config::get_user_config().codex_app_path);
            }
        }
        "zed" => {
            if !force && !current.zed_app_path.trim().is_empty() {
                return Some(current.zed_app_path);
            }
            if let Some(detected) = detect_zed_exec_path() {
                update_app_path_in_config("zed", &detected, &current.zed_app_path);
                return Some(config::get_user_config().zed_app_path);
            }
        }
        "vscode" => {
            if !force && !current.vscode_app_path.trim().is_empty() {
                return Some(current.vscode_app_path);
            }
            if let Some(detected) = detect_vscode_exec_path() {
                update_app_path_in_config("vscode", &detected, &current.vscode_app_path);
                return Some(config::get_user_config().vscode_app_path);
            }
        }
        "codebuddy" => {
            if !force && !current.codebuddy_app_path.trim().is_empty() {
                return Some(current.codebuddy_app_path);
            }
            if let Some(detected) = detect_codebuddy_exec_path() {
                update_app_path_in_config("codebuddy", &detected, &current.codebuddy_app_path);
                return Some(config::get_user_config().codebuddy_app_path);
            }
        }
        "codebuddy_cn" => {
            if !force && !current.codebuddy_cn_app_path.trim().is_empty() {
                return Some(current.codebuddy_cn_app_path);
            }
            if let Some(detected) = detect_codebuddy_cn_exec_path() {
                update_app_path_in_config(
                    "codebuddy_cn",
                    &detected,
                    &current.codebuddy_cn_app_path,
                );
                return Some(config::get_user_config().codebuddy_cn_app_path);
            }
        }
        "qoder" => {
            if !force && !current.qoder_app_path.trim().is_empty() {
                return Some(current.qoder_app_path);
            }
            if let Some(detected) = detect_qoder_exec_path() {
                update_app_path_in_config("qoder", &detected, &current.qoder_app_path);
                return Some(config::get_user_config().qoder_app_path);
            }
        }
        "trae" => {
            if !force && !current.trae_app_path.trim().is_empty() {
                return Some(current.trae_app_path);
            }
            if let Some(detected) = detect_trae_exec_path() {
                update_app_path_in_config("trae", &detected, &current.trae_app_path);
                return Some(config::get_user_config().trae_app_path);
            }
        }
        "opencode" => {
            if !force && !current.opencode_app_path.trim().is_empty() {
                return Some(current.opencode_app_path);
            }
            if let Some(detected) = detect_opencode_exec_path() {
                update_app_path_in_config("opencode", &detected, &current.opencode_app_path);
                return Some(config::get_user_config().opencode_app_path);
            }
        }
        "workbuddy" => {
            if !force && !current.workbuddy_app_path.trim().is_empty() {
                return Some(current.workbuddy_app_path);
            }
            if let Some(detected) = detect_workbuddy_exec_path() {
                update_app_path_in_config("workbuddy", &detected, &current.workbuddy_app_path);
                return Some(config::get_user_config().workbuddy_app_path);
            }
        }
        _ => {}
    }
    None
}

pub fn is_pid_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        // On macOS, use ps to avoid sysinfo TCC dialogs.
        // Treat zombie/defunct process as not running.
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output();
        let output = match output {
            Ok(value) if value.status.success() => value,
            _ => return false,
        };

        let stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stat.is_empty() {
            return false;
        }
        let first = stat.chars().next().unwrap_or_default();
        if first == 'Z' || first == 'z' {
            return false;
        }
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        system.process(Pid::from(pid as usize)).is_some()
    }
}

#[cfg(not(target_os = "macos"))]
fn extract_user_data_dir(args: &[std::ffi::OsString]) -> Option<String> {
    let tokens: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect();
    let mut index = 0;
    while index < tokens.len() {
        let value = tokens[index].as_str();
        if let Some(rest) = value.strip_prefix("--user-data-dir=") {
            return Some(rest.to_string());
        }
        if value == "--user-data-dir" {
            index += 1;
            if index >= tokens.len() {
                return None;
            }
            let mut parts = Vec::new();
            while index < tokens.len() {
                let part = tokens[index].as_str();
                if part.starts_with("--") {
                    break;
                }
                parts.push(part);
                index += 1;
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
            return None;
        }
        index += 1;
    }
    None
}

fn extract_user_data_dir_from_command_line(command_line: &str) -> Option<String> {
    let tokens = split_command_tokens(command_line);
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix("--user-data-dir=") {
            if !rest.trim().is_empty() {
                return Some(rest.to_string());
            }
        }
        if token == "--user-data-dir" {
            index += 1;
            if index >= tokens.len() {
                return None;
            }
            let mut parts = Vec::new();
            while index < tokens.len() {
                let part = tokens[index].as_str();
                if part.starts_with("--") || is_env_token(part) {
                    break;
                }
                parts.push(part);
                index += 1;
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
            return None;
        }
        index += 1;
    }
    None
}

#[cfg(target_os = "macos")]
fn parse_env_value(raw: &str) -> Option<String> {
    let rest = raw.trim_start();
    if rest.is_empty() {
        return None;
    }
    let value = if rest.starts_with('"') {
        let end = rest[1..].find('"').map(|idx| idx + 1).unwrap_or(rest.len());
        &rest[1..end]
    } else if rest.starts_with('\'') {
        let end = rest[1..]
            .find('\'')
            .map(|idx| idx + 1)
            .unwrap_or(rest.len());
        &rest[1..end]
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        &rest[..end]
    };
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(target_os = "macos")]
fn extract_env_value_from_tokens(tokens: &[String], key: &str) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    let prefix = format!("{}=", key);
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix(&prefix) {
            let mut parts: Vec<&str> = Vec::new();
            if !rest.is_empty() {
                parts.push(rest);
            }
            let mut next = index + 1;
            while next < tokens.len() {
                let value = tokens[next].as_str();
                if value.starts_with("--") || is_env_token(value) {
                    break;
                }
                parts.push(value);
                next += 1;
            }
            if parts.is_empty() {
                return None;
            }
            let joined = parts.join(" ");
            let trimmed = joined.trim();
            if trimmed.is_empty() {
                return None;
            }
            return Some(trimmed.to_string());
        }
        index += 1;
    }
    None
}

fn split_command_tokens(command_line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in command_line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn is_env_token(token: &str) -> bool {
    let (key, _) = match token.split_once('=') {
        Some(parts) => parts,
        None => return false,
    };
    if key.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let first = match chars.next() {
        Some(value) => value,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(target_os = "macos")]
fn extract_env_value(command_line: &str, key: &str) -> Option<String> {
    let needle = format!("{}=", key);
    let pos = command_line.find(&needle)?;
    let rest = &command_line[pos + needle.len()..];
    parse_env_value(rest)
}

fn normalize_path_for_compare(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(target_os = "windows")]
    fn normalize_windows_extended_path(raw: &str) -> String {
        let mut value = raw.trim().trim_matches('"').replace('/', "\\");
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("\\\\?\\unc\\") {
            let rest = value
                .chars()
                .skip("\\\\?\\UNC\\".chars().count())
                .collect::<String>();
            value = format!("\\\\{}", rest);
        } else if lower.starts_with("\\\\?\\") {
            value = value
                .chars()
                .skip("\\\\?\\".chars().count())
                .collect::<String>();
        }
        value
    }

    #[cfg(target_os = "windows")]
    let normalized_input = normalize_windows_extended_path(trimmed);
    #[cfg(not(target_os = "windows"))]
    let normalized_input = trimmed.to_string();

    let resolved = std::fs::canonicalize(&normalized_input)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(normalized_input);

    #[cfg(target_os = "windows")]
    let resolved = normalize_windows_extended_path(&resolved);

    #[cfg(target_os = "windows")]
    {
        return resolved.to_lowercase();
    }
    #[cfg(not(target_os = "windows"))]
    {
        return resolved;
    }
}

fn is_helper_command_line(cmdline_lower: &str) -> bool {
    cmdline_lower.contains("--type=")
        || cmdline_lower.contains("helper")
        || cmdline_lower.contains("plugin")
        || cmdline_lower.contains("renderer")
        || cmdline_lower.contains("gpu")
        || cmdline_lower.contains("crashpad")
        || cmdline_lower.contains("utility")
        || cmdline_lower.contains("audio")
        || cmdline_lower.contains("sandbox")
        || cmdline_lower.contains("--node-ipc")
        || cmdline_lower.contains("--clientprocessid=")
        || cmdline_lower.contains("\\resources\\app\\extensions\\")
        || cmdline_lower.contains("/resources/app/extensions/")
}

#[cfg(not(target_os = "macos"))]
fn is_antigravity_main_process(
    name: &str,
    exe_path: &str,
    command_line_lower: Option<&str>,
) -> bool {
    let cmdline = command_line_lower.unwrap_or("");
    if cmdline.contains("antigravity tools") || cmdline.contains("antigravity tools.app/contents/")
    {
        return false;
    }
    if !cmdline.is_empty() && is_helper_command_line(cmdline) {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = name;
        return exe_path.contains("antigravity ide.app")
            && !exe_path.contains("antigravity tools.app")
            && !exe_path.contains("crashpad");
    }

    #[cfg(target_os = "windows")]
    {
        return is_windows_antigravity_main_executable(name, exe_path);
    }

    #[cfg(target_os = "linux")]
    {
        return (name.contains("antigravity-ide") || exe_path.contains("/antigravity-ide"))
            && !name.contains("tools")
            && !exe_path.contains("tools");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (name, exe_path);
        false
    }
}

fn collect_running_process_exe_by_pid() -> HashMap<u32, String> {
    let mut map = HashMap::new();

    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        if let Ok(output) = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                    let normalized = normalize_path_for_compare(&exe);
                    if !normalized.is_empty() {
                        map.insert(pid, normalized);
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        for (pid, process) in system.processes() {
            let Some(exe) = process.exe().and_then(|value| value.to_str()) else {
                continue;
            };
            let normalized = normalize_path_for_compare(exe);
            if normalized.is_empty() {
                continue;
            }
            map.insert(pid.as_u32(), normalized);
        }
    }

    map
}

fn filter_entries_by_expected_launch_path(
    app_label: &str,
    entries: Vec<(u32, Option<String>)>,
    expected: Option<String>,
) -> Vec<(u32, Option<String>)> {
    if entries.is_empty() {
        return entries;
    }
    let Some(expected) = expected else {
        return Vec::new();
    };
    let exe_by_pid = collect_running_process_exe_by_pid();
    #[cfg(target_os = "macos")]
    let expected_app_root = normalize_macos_app_root(std::path::Path::new(&expected))
        .map(|root| normalize_path_for_compare(&root))
        .filter(|root| !root.is_empty());
    let mut result = Vec::new();
    let mut missing_exe = 0usize;
    let mut path_mismatch = 0usize;
    #[cfg(target_os = "macos")]
    let mut app_root_match = 0usize;
    for (pid, dir) in entries {
        match exe_by_pid.get(&pid) {
            Some(actual) if actual == &expected => result.push((pid, dir)),
            Some(actual) => {
                #[cfg(not(target_os = "macos"))]
                let _ = actual;
                #[cfg(target_os = "macos")]
                {
                    if let Some(expected_root) = expected_app_root.as_ref() {
                        let actual_root = normalize_macos_app_root(std::path::Path::new(actual))
                            .map(|root| normalize_path_for_compare(&root))
                            .filter(|root| !root.is_empty());
                        if actual_root.as_ref() == Some(expected_root) {
                            app_root_match += 1;
                            result.push((pid, dir));
                            continue;
                        }
                    }
                }
                path_mismatch += 1;
            }
            None => missing_exe += 1,
        }
    }
    if result.is_empty() {
        #[cfg(target_os = "macos")]
        {
            crate::modules::logger::log_warn(&format!(
                "[{} Resolve] 启动路径硬匹配未命中：expected={}, path_mismatch={}, missing_exe={}, app_root_match={}",
                app_label, expected, path_mismatch, missing_exe, app_root_match
            ));
        }
        #[cfg(not(target_os = "macos"))]
        crate::modules::logger::log_warn(&format!(
            "[{} Resolve] 启动路径硬匹配未命中：expected={}, path_mismatch={}, missing_exe={}",
            app_label, expected, path_mismatch, missing_exe
        ));
    } else {
        #[cfg(target_os = "macos")]
        if app_root_match > 0 {
            crate::modules::logger::log_info(&format!(
                "[{} Resolve] 使用 .app 根路径匹配到进程：expected={}, app_root_match={}",
                app_label, expected, app_root_match
            ));
        }
    }
    result
}

fn resolve_expected_antigravity_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_antigravity_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[AG Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[AG Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_vscode_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_vscode_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[VSCode Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[VSCode Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_codebuddy_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codebuddy_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[CodeBuddy Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[CodeBuddy Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_codebuddy_cn_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codebuddy_cn_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[CodeBuddy CN Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[CodeBuddy CN Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_workbuddy_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_workbuddy_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddy Resolve] 启动路径未配置或无效，跳过 PID 匹配：{}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[WorkBuddy Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "macos")]
fn resolve_expected_codex_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codex_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[Codex Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "macos")]
fn collect_antigravity_process_entries_from_ps() -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    let output = match output {
        Ok(value) => value,
        Err(_) => return result,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !lower.contains(ANTIGRAVITY_APP_CONTENTS_MARKER) {
            continue;
        }
        if lower.contains("antigravity tools.app/contents/")
            || lower.contains("crashpad_handler")
            || is_helper_command_line(&lower)
        {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "windows")]
fn windows_antigravity_process_name_for_expected_exe(expected_exe_path: &str) -> String {
    Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.eq_ignore_ascii_case("Antigravity.exe")
                || name.eq_ignore_ascii_case("Antigravity IDE.exe")
        })
        .unwrap_or("Antigravity IDE.exe")
        .to_string()
}

#[cfg(target_os = "windows")]
fn collect_antigravity_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let process_name = windows_antigravity_process_name_for_expected_exe(expected_exe_path);
    let script = build_windows_path_filtered_process_probe_script(&process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[AG Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[AG Probe] PowerShell 进程探测失败: {}",
                    err
                ));
            }
            return result;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[AG Probe] PowerShell 进程探测返回非 0 状态: {}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return result;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !is_antigravity_main_process(&process_name.to_lowercase(), "", Some(&lower)) {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "windows")]
fn resolve_windows_process_exe_for_match(process: &sysinfo::Process) -> (Option<String>, bool) {
    if let Some(exe) = process.exe().and_then(|value| value.to_str()) {
        let normalized = normalize_path_for_compare(exe);
        if !normalized.is_empty() {
            return (Some(normalized), false);
        }
    }
    if let Some(first) = process.cmd().first() {
        let normalized = normalize_path_for_compare(first.to_string_lossy().as_ref());
        if !normalized.is_empty() {
            return (Some(normalized), true);
        }
    }
    (None, false)
}

#[cfg(target_os = "windows")]
fn collect_antigravity_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args = process.cmd();
        let args_lower = args
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        if !is_antigravity_main_process(&name, &exe_path, Some(&args_lower)) {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(args);
                result.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    if result.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[AG Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[AG Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, result.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    result
}

#[cfg(target_os = "linux")]
fn collect_antigravity_process_entries_from_proc() -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(value) => value,
        Err(_) => return result,
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let pid_str = file_name.to_string_lossy();
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        let cmdline = match std::fs::read(&cmdline_path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if cmdline.is_empty() {
            continue;
        }
        let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        let cmd_lower = cmdline_str.to_lowercase();
        let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
            .unwrap_or_default();
        if !cmd_lower.contains("antigravity-ide") && !exe_path.contains("antigravity-ide") {
            continue;
        }
        if cmd_lower.contains("tools") || exe_path.contains("tools") {
            continue;
        }
        if is_helper_command_line(&cmd_lower) {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(&cmdline_str);
        result.push((pid, dir));
    }
    result
}

pub fn collect_antigravity_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_antigravity_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "macos")]
    {
        let entries = collect_antigravity_process_entries_macos();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
        }
        let entries = collect_antigravity_process_entries_from_ps();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
        }
        // macOS 下避免回退到 sysinfo，防止触发 TCC「其他 App 数据」授权弹窗
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_antigravity_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        if strict_process_detect_enabled() {
            crate::modules::logger::log_warn(
                "[AG Probe] strict mode enabled and PowerShell returned empty; skip sysinfo fallback",
            );
            return Vec::new();
        }
        crate::modules::logger::log_warn(
            "[AG Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_antigravity_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "linux")]
    {
        let entries = collect_antigravity_process_entries_from_proc();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
        }
        return Vec::new();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

fn pick_preferred_pid(mut pids: Vec<u32>) -> Option<u32> {
    if pids.is_empty() {
        return None;
    }
    pids.sort();
    pids.dedup();
    pids.first().copied()
}

fn normalize_non_empty_path_for_compare(value: &str) -> Option<String> {
    let normalized = normalize_path_for_compare(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn build_user_data_dir_match_target(
    requested_user_data_dir: Option<&str>,
    default_user_data_dir: Option<String>,
    fallback_to_default_when_missing: bool,
) -> Option<(String, bool)> {
    let requested_target =
        requested_user_data_dir.and_then(|value| normalize_non_empty_path_for_compare(value));
    let default_target =
        default_user_data_dir.and_then(|value| normalize_non_empty_path_for_compare(&value));
    let target = requested_target.or_else(|| {
        if fallback_to_default_when_missing {
            default_target.clone()
        } else {
            None
        }
    })?;
    let allow_none_for_target = default_target
        .as_ref()
        .map(|value| value == &target)
        .unwrap_or(false);
    Some((target, allow_none_for_target))
}

fn collect_matching_pids_by_user_data_dir(
    entries: &[(u32, Option<String>)],
    target_dir: &str,
    allow_none_for_target: bool,
) -> Vec<u32> {
    let mut matches = Vec::new();
    for (pid, dir) in entries {
        match dir.as_ref() {
            Some(value) => {
                let normalized = normalize_path_for_compare(value);
                if !normalized.is_empty() && normalized == target_dir {
                    matches.push(*pid);
                }
            }
            None => {
                if allow_none_for_target {
                    matches.push(*pid);
                }
            }
        }
    }
    matches
}

fn resolve_pid_from_entries_by_user_data_dir(
    last_pid: Option<u32>,
    target_dir: &str,
    allow_none_for_target: bool,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    if target_dir.is_empty() {
        return None;
    }

    let matches =
        collect_matching_pids_by_user_data_dir(entries, target_dir, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[PID Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(target_dir, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

fn get_default_antigravity_user_data_dir() -> Option<String> {
    crate::modules::instance::get_default_user_data_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty())
}

fn resolve_antigravity_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_antigravity_user_data_dir(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_vscode_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_vscode_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_codebuddy_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_codebuddy_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_codebuddy_cn_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_codebuddy_cn_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_workbuddy_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_workbuddy_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

#[cfg(target_os = "windows")]
fn get_default_codex_windows_app_user_data_dir() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        std::path::PathBuf::from(appdata)
            .join("Codex")
            .to_string_lossy()
            .to_string(),
    )
}

#[cfg(target_os = "windows")]
fn get_managed_codex_windows_app_user_data_dir(codex_home: &str) -> Option<String> {
    let trimmed = codex_home.trim();
    if trimmed.is_empty() {
        return None;
    }
    crate::modules::codex_instance::get_windows_app_user_data_dir(Path::new(trimmed))
        .ok()
        .map(|value| value.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn resolve_codex_windows_target_and_fallback(codex_home: Option<&str>) -> Option<(String, bool)> {
    match codex_home {
        Some(home) => build_user_data_dir_match_target(
            get_managed_codex_windows_app_user_data_dir(home).as_deref(),
            get_default_codex_windows_app_user_data_dir(),
            false,
        ),
        None => build_user_data_dir_match_target(
            None,
            get_default_codex_windows_app_user_data_dir(),
            true,
        ),
    }
}

#[cfg(target_os = "macos")]
fn collect_qoder_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let pid = match pid_str.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let lower = cmdline.to_lowercase();
            let is_qoder = lower.contains("qoder.app/contents/macos/");
            if !is_qoder {
                continue;
            }
            if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            entries.push((pid, dir));
        }
    }
    entries
}

#[cfg(target_os = "macos")]
fn collect_trae_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let pid = match pid_str.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let lower = cmdline.to_lowercase();
            let is_trae = lower.contains("trae.app/contents/macos/");
            if !is_trae {
                continue;
            }
            if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            entries.push((pid, dir));
        }
    }
    entries
}

#[cfg(target_os = "macos")]
fn resolve_qoder_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let default_user_data_dir = crate::modules::qoder_instance::get_default_qoder_user_data_dir()
        .ok()
        .map(|value| value.to_string_lossy().to_string());
    let (target, allow_none_for_target) = build_user_data_dir_match_target(
        user_data_dir,
        default_user_data_dir,
        !strict_process_detect_enabled(),
    )?;
    let entries = collect_qoder_process_entries_macos();
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, &entries)
}

#[cfg(target_os = "macos")]
fn resolve_trae_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let default_user_data_dir = crate::modules::trae_instance::get_default_trae_user_data_dir()
        .ok()
        .map(|value| value.to_string_lossy().to_string());
    let (target, allow_none_for_target) = build_user_data_dir_match_target(
        user_data_dir,
        default_user_data_dir,
        !strict_process_detect_enabled(),
    )?;
    let entries = collect_trae_process_entries_macos();
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, &entries)
}

pub fn resolve_antigravity_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_antigravity_target_and_fallback(user_data_dir)?;
    let matches = collect_matching_pids_by_user_data_dir(entries, &target, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[AG Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(&target, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

pub fn resolve_antigravity_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_antigravity_process_entries();
    resolve_antigravity_pid_from_entries(last_pid, user_data_dir, &entries)
}

#[cfg(target_os = "macos")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true",
        pid
    );
    crate::modules::logger::log_info(&format!("[Focus] macOS osascript start pid={}", pid));
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("调用 osascript 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!("[Focus] macOS osascript success pid={}", pid));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "窗口聚焦失败，请检查系统辅助功能权限: {}",
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let command = format!(
        r#"$targetPid={pid};$h=[IntPtr]::Zero;for($i=0;$i -lt 20;$i++){{$p=Get-Process -Id $targetPid -ErrorAction Stop;$h=$p.MainWindowHandle;if ($h -ne 0) {{ break }};Start-Sleep -Milliseconds 150}};if ($h -eq 0) {{ throw 'MAIN_WINDOW_HANDLE_EMPTY' }};Add-Type @' 
using System; 
using System.Runtime.InteropServices; 
public class Win32 {{ 
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd); 
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow); 
}} 
'@;[Win32]::ShowWindowAsync($h, 9) | Out-Null;[Win32]::SetForegroundWindow($h) | Out-Null;"#
    );
    crate::modules::logger::log_info(&format!("[Focus] Windows PowerShell start pid={}", pid));
    let output = powershell_output(&["-NoProfile", "-Command", &command])
        .map_err(|e| format!("调用 PowerShell 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!(
            "[Focus] Windows PowerShell success pid={}",
            pid
        ));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("窗口聚焦失败: {}", stderr.trim()))
}

#[cfg(target_os = "windows")]
pub fn focus_current_process_main_window() -> Result<(), String> {
    focus_window_by_pid(std::process::id())
}

#[cfg(target_os = "linux")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    if let Ok(output) = Command::new("wmctrl").arg("-lp").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let mut parts = line.split_whitespace();
                let win_id = parts.next();
                let _desktop = parts.next();
                let pid_str = parts.next();
                if let (Some(win_id), Some(pid_str)) = (win_id, pid_str) {
                    if pid_str == pid.to_string() {
                        let focus = Command::new("wmctrl").args(["-ia", win_id]).output();
                        if let Ok(focus) = focus {
                            if focus.status.success() {
                                crate::modules::logger::log_info(&format!(
                                    "[Focus] Linux wmctrl success pid={}",
                                    pid
                                ));
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    crate::modules::logger::log_info(&format!(
        "[Focus] Linux wmctrl not available or failed, trying xdotool pid={}",
        pid
    ));
    let output = Command::new("xdotool")
        .args(["search", "--pid", &pid.to_string(), "windowactivate"])
        .output()
        .map_err(|e| format!("调用 xdotool 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!("[Focus] Linux xdotool success pid={}", pid));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("窗口聚焦失败: {}", stderr.trim()))
}

pub fn focus_antigravity_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_antigravity_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity IDE resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity IDE focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

#[cfg(target_os = "macos")]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let target = codex_home
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty());

    let mut matches = Vec::new();
    for (pid, home) in entries {
        match (&target, home.as_ref()) {
            (Some(target_home), Some(home)) => {
                let normalized = normalize_path_for_compare(home);
                if !normalized.is_empty() && &normalized == target_home {
                    matches.push(*pid);
                }
            }
            (None, None) => {
                matches.push(*pid);
            }
            (None, Some(home)) => {
                let normalized = normalize_path_for_compare(home);
                if normalized.is_empty() {
                    matches.push(*pid);
                }
            }
            _ => {}
        }
    }

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                target
                    .as_deref()
                    .map(|value| summarize_text_for_process_log(value, 96))
                    .unwrap_or_else(|| "-".to_string()),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

#[cfg(target_os = "windows")]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codex_windows_target_and_fallback(codex_home)?;
    let matches = collect_matching_pids_by_user_data_dir(entries, &target, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) && !matches.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(&target, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    _codex_home: Option<&str>,
    _entries: &[(u32, Option<String>)],
) -> Option<u32> {
    last_pid.filter(|pid| is_pid_running(*pid))
}

#[cfg(target_os = "macos")]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(target_os = "windows")]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resolve_codex_pid(last_pid: Option<u32>, _codex_home: Option<&str>) -> Option<u32> {
    last_pid.filter(|pid| is_pid_running(*pid))
}

pub fn focus_codex_instance(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_codex_pid(last_pid, codex_home)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Codex resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Codex focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

#[cfg(target_os = "windows")]
fn collect_vscode_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let script = build_windows_path_filtered_process_probe_script("Code.exe", expected_exe_path);
    let output = powershell_output(&["-Command", &script]);
    let output = match output {
        Ok(value) => value,
        Err(_) => return entries,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_vscode_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");
        let is_vscode = name == "code.exe" || exe_path.ends_with("\\code.exe");
        if !is_vscode
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[VSCode Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[VSCode Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

pub fn collect_vscode_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_vscode_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_vscode_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[VSCode Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_vscode_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                if !lower.contains("visual studio code.app/contents/macos/") {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("code") && !exe_path.contains("/code") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_vscode_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_vscode_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_vscode_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_vscode_process_entries();
    resolve_vscode_pid_from_entries(last_pid, user_data_dir, &entries)
}

fn get_default_vscode_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "windows")]
fn collect_codebuddy_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let process_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("CodeBuddy.exe");
    let script = build_windows_path_filtered_process_probe_script(process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[CodeBuddy Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[CodeBuddy Probe] PowerShell 进程探测失败: {}",
                    err
                ));
            }
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Probe] PowerShell 进程探测返回非 0 状态: {}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return entries;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_codebuddy_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let expected_file_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codebuddy.exe")
        .to_ascii_lowercase();

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        let is_codebuddy = name == expected_file_name
            || exe_path.ends_with(&format!("\\{}", expected_file_name))
            || name == "codebuddy.exe"
            || exe_path.ends_with("\\codebuddy.exe")
            || exe_path.contains("\\codebuddy\\");
        if !is_codebuddy
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[CodeBuddy Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

#[cfg(target_os = "windows")]
fn collect_workbuddy_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let process_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("WorkBuddy.exe");
    let script = build_windows_path_filtered_process_probe_script(process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[WorkBuddy Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy Probe] PowerShell 进程探测失败：{}",
                    err
                ));
            }
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Probe] PowerShell 进程探测返回非 0 状态：{}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return entries;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_workbuddy_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let expected_file_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workbuddy.exe")
        .to_ascii_lowercase();

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        let is_workbuddy = name == expected_file_name
            || exe_path.ends_with(&format!("\\{}", expected_file_name))
            || name == "workbuddy.exe"
            || exe_path.ends_with("\\workbuddy.exe")
            || exe_path.contains("\\workbuddy\\");
        if !is_workbuddy
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

pub fn collect_codebuddy_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codebuddy_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_codebuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[CodeBuddy Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_codebuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                if !lower.contains("codebuddy.app/contents/macos/") {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("codebuddy") && !exe_path.contains("/codebuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn collect_codebuddy_cn_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codebuddy_cn_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_codebuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[CodeBuddy CN Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_codebuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                let is_codebuddy = lower.contains("codebuddy cn.app/contents/macos/")
                    || lower.contains("codebuddy.app/contents/macos/");
                if !is_codebuddy {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("codebuddy") && !exe_path.contains("/codebuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_codebuddy_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codebuddy_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_codebuddy_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_codebuddy_process_entries();
    resolve_codebuddy_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn resolve_codebuddy_cn_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codebuddy_cn_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_codebuddy_cn_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_codebuddy_cn_process_entries();
    resolve_codebuddy_cn_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn collect_workbuddy_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_workbuddy_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_workbuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[WorkBuddy Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_workbuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                let is_workbuddy = lower.contains("workbuddy.app/contents/macos/");
                if !is_workbuddy {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("workbuddy") && !exe_path.contains("/workbuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_workbuddy_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_workbuddy_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_workbuddy_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_workbuddy_process_entries();
    resolve_workbuddy_pid_from_entries(last_pid, user_data_dir, &entries)
}

fn get_default_codebuddy_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

fn get_default_codebuddy_cn_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

fn get_default_workbuddy_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("WorkBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("WorkBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("WorkBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

pub fn focus_vscode_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_vscode_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] VS Code resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] VS Code focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

pub fn focus_process_pid(pid: u32) -> Result<u32, String> {
    if pid == 0 || !is_pid_running(pid) {
        return Err("实例未运行，无法定位窗口".to_string());
    }
    focus_window_by_pid(pid)?;
    Ok(pid)
}

#[cfg(target_os = "macos")]
fn collect_antigravity_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let mut pids = Vec::new();
    if let Ok(output) = Command::new("pgrep")
        .args(["-f", ANTIGRAVITY_APP_PATH])
        .output()
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    pids.push(pid);
                }
            }
        }
    }

    if pids.is_empty() {
        return Vec::new();
    }

    pids.sort();
    pids.dedup();

    let mut result = Vec::new();
    for pid in pids {
        let output = Command::new("ps")
            .args(["-Eww", "-p", &pid.to_string(), "-o", "command="])
            .output();
        let output = match output {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let cmdline = line.trim();
            if cmdline.is_empty() {
                continue;
            }
            if !cmdline.to_lowercase().contains(ANTIGRAVITY_APP_EXEC_MARKER) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            result.push((pid, dir));
        }
    }

    result
}
pub fn parse_extra_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in raw.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

fn collect_remaining_pids(entries: &[(u32, Option<String>)]) -> Vec<u32> {
    let mut pids: Vec<u32> = entries.iter().map(|(pid, _)| *pid).collect();
    pids.sort();
    pids.dedup();
    pids
}

fn resolve_entry_user_data_dir_for_matching(
    dir: Option<&String>,
    default_dir: Option<&str>,
) -> Option<String> {
    dir.and_then(|value| normalize_non_empty_path_for_compare(value))
        .or_else(|| default_dir.and_then(normalize_non_empty_path_for_compare))
}

fn entry_matches_target_dirs(
    dir: Option<&String>,
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> bool {
    resolve_entry_user_data_dir_for_matching(dir, default_dir)
        .map(|value| target_dirs.contains(&value))
        .unwrap_or(false)
}

fn select_main_pids_by_target_dirs(
    entries: &[(u32, Option<String>)],
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> Vec<u32> {
    entries
        .iter()
        .filter_map(|(pid, dir)| {
            entry_matches_target_dirs(dir.as_ref(), target_dirs, default_dir).then_some(*pid)
        })
        .collect()
}

fn filter_entries_by_target_dirs(
    entries: Vec<(u32, Option<String>)>,
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> Vec<(u32, Option<String>)> {
    entries
        .into_iter()
        .filter(|(_, dir)| entry_matches_target_dirs(dir.as_ref(), target_dirs, default_dir))
        .collect()
}

fn close_managed_instances_common<CollectEntries, SelectMainPids, CollectRemainingEntries>(
    log_prefix: &str,
    start_message: &str,
    empty_targets_message: &str,
    not_running_message: &str,
    process_display_name: &str,
    failure_message: &str,
    user_data_dirs: &[String],
    timeout_secs: u64,
    collect_entries: CollectEntries,
    select_main_pids: SelectMainPids,
    collect_remaining_entries: CollectRemainingEntries,
    graceful_close: Option<fn(u32)>,
    graceful_wait_secs: Option<u64>,
    detail_logger: Option<fn(&[u32])>,
) -> Result<(), String>
where
    CollectEntries: Fn() -> Vec<(u32, Option<String>)>,
    SelectMainPids: Fn(&[(u32, Option<String>)], &HashSet<String>) -> Vec<u32>,
    CollectRemainingEntries: Fn(&HashSet<String>) -> Vec<(u32, Option<String>)>,
{
    crate::modules::logger::log_info(start_message);

    let target_dirs: HashSet<String> = user_data_dirs
        .iter()
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty())
        .collect();
    if target_dirs.is_empty() {
        crate::modules::logger::log_info(empty_targets_message);
        return Ok(());
    }
    crate::modules::logger::log_info(&format!(
        "[{}] target_dirs={}, timeout_secs={}",
        log_prefix,
        summarize_target_dirs_for_log(&target_dirs),
        timeout_secs
    ));

    let entries = collect_entries();
    crate::modules::logger::log_info(&format!(
        "[{}] collected_entries={}",
        log_prefix,
        summarize_process_entries_for_log(&entries)
    ));

    let mut pids = select_main_pids(&entries, &target_dirs);
    pids.sort();
    pids.dedup();
    if pids.is_empty() {
        crate::modules::logger::log_info(not_running_message);
        return Ok(());
    }
    crate::modules::logger::log_info(&format!(
        "[{}] matched_main_pids={}",
        log_prefix,
        summarize_pid_list_for_log(&pids)
    ));

    crate::modules::logger::log_info(&format!(
        "准备关闭 {} 个{}主进程...",
        pids.len(),
        process_display_name
    ));

    if let Some(graceful_close_fn) = graceful_close {
        for pid in &pids {
            graceful_close_fn(*pid);
        }
        if let Some(wait_secs) = graceful_wait_secs {
            if wait_pids_exit(&pids, wait_secs) {
                crate::modules::logger::log_info(&format!(
                    "[{}] graceful close finished, targets={}",
                    log_prefix,
                    summarize_pid_list_for_log(&pids)
                ));
                return Ok(());
            }
        }
    }

    if let Err(err) = close_pids(&pids, timeout_secs) {
        crate::modules::logger::log_warn(&format!(
            "[{}] close_pids returned error: {}",
            log_prefix, err
        ));
    }

    let mut remaining_entries = collect_remaining_entries(&target_dirs);
    if !remaining_entries.is_empty() {
        let remaining_pids = collect_remaining_pids(&remaining_entries);
        crate::modules::logger::log_warn(&format!(
            "[{}] first remaining pids after close={}",
            log_prefix,
            summarize_pid_list_for_log(&remaining_pids)
        ));
        if let Some(detail_logger_fn) = detail_logger {
            detail_logger_fn(&remaining_pids);
        }
        if !remaining_pids.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[{}] retry force close for remaining pids={}",
                log_prefix,
                summarize_pid_list_for_log(&remaining_pids)
            ));
            if let Err(err) = close_pids(&remaining_pids, 6) {
                crate::modules::logger::log_warn(&format!(
                    "[{}] retry close_pids returned error: {}",
                    log_prefix, err
                ));
            }
            remaining_entries = collect_remaining_entries(&target_dirs);
        }
    }

    if !remaining_entries.is_empty() {
        let remaining_pids = collect_remaining_pids(&remaining_entries);
        if let Some(detail_logger_fn) = detail_logger {
            detail_logger_fn(&remaining_pids);
        }
        crate::modules::logger::log_error(&format!(
            "[{}] still_running_entries={}",
            log_prefix,
            summarize_process_entries_for_log(&remaining_entries)
        ));
        return Err(format!(
            "{} ({})",
            failure_message,
            summarize_pid_list_for_log(&remaining_pids)
        ));
    }

    Ok(())
}

