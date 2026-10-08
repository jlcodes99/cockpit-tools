// A read-only, bounded desktop snapshot for restoring proxy listeners. Kept
// separate from the interactive launch probes so startup cannot inherit their
// unbounded ps calls or macOS sysinfo command-line/TCC behavior.
pub(crate) type CodexProxyProcessSnapshot = (u32, Option<String>, Vec<std::ffi::OsString>);

/// Match an already collected process context without probing the PID again.
/// In particular, `None` is the official default desktop context, not the path
/// of a managed profile that happens to be called "default".
pub(crate) fn codex_proxy_profile_matches(profile: Option<&str>, target: Option<&str>) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Some((target, allow_none)) = resolve_codex_windows_target_and_fallback(target) else {
            return false;
        };
        return !collect_matching_pids_by_user_data_dir(
            &[(0, profile.map(str::to_string))],
            &target,
            allow_none,
        )
        .is_empty();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let target = target.and_then(normalize_non_empty_path_for_compare);
        let profile = profile.and_then(normalize_non_empty_path_for_compare);
        profile == target
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn collect_codex_proxy_process_snapshot(
) -> Result<Vec<CodexProxyProcessSnapshot>, String> {
    let output = crate::modules::process_timeout::output_with_timeout(
        Command::new("ps").args(["-axEww", "-o", "pid=,command="]),
        Duration::from_secs(5),
    )
    .map_err(|error| format!("Codex proxy process snapshot failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Codex proxy process snapshot failed: {}",
            output.status
        ));
    }
    let records: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_codex_proxy_macos_snapshot_line)
        .collect();
    // Follow the same configured executable / app-root matching as the normal
    // collector. If no usable path is configured, discover from this snapshot
    // instead of launching another ps or writing a startup-path preference.
    let configured = config::get_user_config().codex_app_path;
    let expected = normalize_custom_path(Some(&configured))
        .and_then(|path| resolve_codex_macos_exec_path(&path))
        .map(|path| normalize_path_for_compare(&path.to_string_lossy()))
        .or_else(|| {
            records
                .first()
                .map(|(_, _, args)| normalize_path_for_compare(&args[0].to_string_lossy()))
        });
    let Some(expected) = expected else {
        return Ok(Vec::new());
    };
    let expected_root = normalize_macos_app_root(std::path::Path::new(&expected))
        .map(|root| normalize_path_for_compare(&root));
    Ok(records
        .into_iter()
        .filter(|(_, _, args)| {
            let actual = normalize_path_for_compare(&args[0].to_string_lossy());
            actual == expected
                || expected_root.as_ref().is_some_and(|expected_root| {
                    normalize_macos_app_root(std::path::Path::new(&actual))
                        .map(|root| normalize_path_for_compare(&root))
                        .as_ref()
                        == Some(expected_root)
                })
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn parse_codex_proxy_macos_snapshot_line(line: &str) -> Option<CodexProxyProcessSnapshot> {
    let (pid, command) = line.trim().split_once(char::is_whitespace)?;
    let pid = pid.parse::<u32>().ok()?;
    let command = command.trim_start();
    let executable = extract_macos_exe_from_cmdline(command)?;
    let lower_executable = executable.to_ascii_lowercase();
    if executable.contains(" --")
        || !(lower_executable.ends_with("/chatgpt.app/contents/macos/chatgpt")
            || lower_executable.ends_with("/codex.app/contents/macos/codex"))
    {
        return None;
    }
    let tail = command.strip_prefix(&executable)?;
    let tokens = split_command_tokens(tail);
    let env_start = tokens
        .iter()
        .position(|token| is_env_token(token))
        .unwrap_or(tokens.len());
    let (args, env) = tokens.split_at(env_start);
    if is_helper_command_line(&args.join(" ").to_ascii_lowercase()) {
        return None;
    }
    let profile = extract_env_value_from_tokens(env, "CODEX_HOME");
    let args = std::iter::once(std::ffi::OsString::from(executable))
        .chain(args.iter().map(std::ffi::OsString::from))
        .collect();
    Some((pid, profile, args))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn collect_codex_proxy_process_snapshot(
) -> Result<Vec<CodexProxyProcessSnapshot>, String> {
    // Windows's official collector already bounds its PowerShell probe and
    // hides its console; it also associates managed child user-data-dir values
    // with the main process. Linux uses native process metadata only.
    let entries = collect_codex_process_entries();
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let pids: Vec<_> = entries.iter().map(|(pid, _)| Pid::from_u32(*pid)).collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    Ok(entries
        .into_iter()
        .filter_map(|(pid, profile)| {
            let process = system.process(Pid::from_u32(pid))?;
            Some((pid, profile, process.cmd().to_vec()))
        })
        .collect())
}

#[cfg(all(test, target_os = "macos"))]
mod codex_proxy_snapshot_tests {
    use super::*;

    #[test]
    fn default_desktop_without_codex_home_is_kept() {
        let entry = parse_codex_proxy_macos_snapshot_line(
            " 123 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT --proxy-server=socks5://127.0.0.1:41000 PATH=/usr/bin HOME=/Users/test",
        ).unwrap();
        assert_eq!(entry.0, 123);
        assert_eq!(entry.1, None);
        assert_eq!(entry.2.len(), 2);
    }

    #[test]
    fn managed_profile_with_spaces_is_preserved() {
        let entry = parse_codex_proxy_macos_snapshot_line(
            "456 /Volumes/Apps Disk/Codex.app/Contents/MacOS/Codex --proxy-server=http://localhost:41000 CODEX_HOME=/Users/test/My Profile PATH=/usr/bin",
        ).unwrap();
        assert_eq!(entry.1.as_deref(), Some("/Users/test/My Profile"));
        assert_eq!(entry.2.len(), 2);
        assert_eq!(
            entry.2[0],
            "/Volumes/Apps Disk/Codex.app/Contents/MacOS/Codex"
        );
    }

    #[test]
    fn rejects_helpers_and_does_not_parse_environment_as_flags() {
        assert!(parse_codex_proxy_macos_snapshot_line(
            "1 /Applications/Codex.app/Contents/MacOS/Codex --type=renderer CODEX_HOME=/profile"
        )
        .is_none());
        assert!(parse_codex_proxy_macos_snapshot_line(
            "1 /usr/bin/tool --title=/Applications/Codex.app/Contents/MacOS/Codex"
        )
        .is_none());
        let entry = parse_codex_proxy_macos_snapshot_line(
            "1 /Applications/Codex.app/Contents/MacOS/Codex SOME_VAR=--proxy-server=http://localhost:41000"
        ).unwrap();
        assert_eq!(entry.2.len(), 1);
    }
}

#[cfg(test)]
mod codex_proxy_profile_tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn default_context_never_matches_a_managed_profile() {
        assert!(codex_proxy_profile_matches(None, None));
        assert!(codex_proxy_profile_matches(Some("  "), None));
        assert!(!codex_proxy_profile_matches(
            Some("/profiles/default"),
            None
        ));
        assert!(!codex_proxy_profile_matches(
            None,
            Some("/profiles/default")
        ));
        assert!(codex_proxy_profile_matches(
            Some(" /profiles/managed "),
            Some("/profiles/managed")
        ));
        assert!(!codex_proxy_profile_matches(
            Some("/profiles/other"),
            Some("/profiles/managed")
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_default_matches_missing_or_default_app_data_only() {
        // Keep the machine's real APPDATA intact: process matching itself must
        // not write it or turn a default desktop into a managed context.
        let Some(default_dir) = get_default_codex_windows_app_user_data_dir() else {
            return;
        };
        assert!(codex_proxy_profile_matches(None, None));
        assert!(codex_proxy_profile_matches(Some(&default_dir), None));
        assert!(!codex_proxy_profile_matches(
            Some(r"C:\profiles\other"),
            None
        ));
        assert!(!codex_proxy_profile_matches(
            None,
            Some(r"C:\profiles\managed")
        ));
        let home = r"C:\profiles\managed";
        let managed = get_managed_codex_windows_app_user_data_dir(home).unwrap();
        assert!(codex_proxy_profile_matches(Some(&managed), Some(home)));
        assert!(!codex_proxy_profile_matches(Some(&default_dir), Some(home)));
    }
}
