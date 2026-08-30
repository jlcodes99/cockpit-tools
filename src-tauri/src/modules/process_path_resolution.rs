// Process 模块：Application path resolution, platform signatures and persisted path guards。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
#[cfg(test)]
mod app_path_config_guard_tests {
    use super::{app_path_matches_snapshot, normalize_windows_user_facing_path};

    #[test]
    fn detected_path_only_replaces_the_snapshot_it_was_detected_for() {
        assert!(app_path_matches_snapshot("", ""));
        assert!(app_path_matches_snapshot(" /old/path ", "/old/path"));
        assert!(!app_path_matches_snapshot("/manual/path", ""));
        assert!(!app_path_matches_snapshot("/new/path", "/old/path"));
    }

    #[test]
    fn strips_windows_extended_path_prefix_for_user_facing_paths() {
        assert_eq!(
            normalize_windows_user_facing_path(r"\\?\C:\Program Files\WindowsApps\ChatGPT.exe"),
            r"C:\Program Files\WindowsApps\ChatGPT.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r"\\?\UNC\server\share\app.exe"),
            r"\\server\share\app.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r"C:\Apps\Codex\ChatGPT.exe"),
            r"C:\Apps\Codex\ChatGPT.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r#"  "\\?\D:\Codex\ChatGPT.exe"  "#),
            r"D:\Codex\ChatGPT.exe"
        );
        assert_eq!(normalize_windows_user_facing_path("   "), "");
    }
}

#[cfg(test)]
mod linux_antigravity_path_tests {
    use super::{
        antigravity_executable_paths_match, antigravity_install_root_from_path,
        first_linux_antigravity_executable, is_linux_antigravity_process_candidate,
        linux_antigravity_discovery_paths, normalize_path_for_compare,
        persisted_linux_antigravity_identity_matches, resolve_linux_antigravity_exec_path,
    };
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    struct PathTestDir(PathBuf);

    impl PathTestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "cockpit-tools-linux-antigravity-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create path test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for PathTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create executable parent");
        }
        std::fs::write(path, b"launcher").expect("write executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("mark test file executable");
        }
    }

    #[test]
    fn install_root_resolves_root_antigravity_executable() {
        let install = PathTestDir::new("root-layout");
        let executable = install.path().join("antigravity-ide");
        write_executable(&executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(install.path()).expect("resolve root layout"),
            executable
        );
    }

    #[test]
    fn install_root_resolves_bin_antigravity_launcher() {
        let install = PathTestDir::new("bin-layout");
        let executable = install.path().join("bin").join("antigravity-ide");
        write_executable(&executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(install.path()).expect("resolve bin layout"),
            executable
        );
    }

    #[test]
    fn install_root_prefers_root_launcher_when_both_layouts_exist() {
        let install = PathTestDir::new("both-layouts-space-含");
        let root_executable = install.path().join("antigravity-ide");
        let bin_executable = install.path().join("bin").join("antigravity-ide");
        write_executable(&root_executable);
        write_executable(&bin_executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(install.path()).expect("resolve root launcher"),
            root_executable
        );
    }

    #[test]
    fn bin_directory_resolves_to_install_root() {
        let install = PathTestDir::new("bin-directory");
        let bin = install.path().join("bin");
        write_executable(&bin.join("antigravity-ide"));

        assert_eq!(
            antigravity_install_root_from_path(&bin).expect("resolve bin directory root"),
            install.path().to_path_buf()
        );
    }

    #[test]
    fn configured_executable_file_is_preserved() {
        let install = PathTestDir::new("custom-file");
        let executable = install.path().join("custom-antigravity-launcher");
        write_executable(&executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(&executable).expect("keep configured executable"),
            executable
        );
    }

    #[test]
    fn directory_without_supported_executable_is_rejected() {
        let install = PathTestDir::new("empty-layout");

        let error = resolve_linux_antigravity_exec_path(install.path())
            .expect_err("reject directory without a launcher");

        assert!(error.contains("antigravity-ide"));
        assert!(error.contains("bin"));
    }

    #[test]
    fn missing_path_is_rejected_without_falling_back_to_current_directory() {
        let install = PathTestDir::new("missing-path");
        let missing = install.path().join("does-not-exist");

        let error = resolve_linux_antigravity_exec_path(&missing)
            .expect_err("reject missing launcher path");

        assert!(error.contains("does not exist"));
    }

    #[test]
    fn discovery_skips_invalid_candidate_before_valid_executable() {
        let install = PathTestDir::new("candidate-fallback");
        let invalid = install.path().join("invalid-directory");
        let executable = install.path().join("valid launcher");
        std::fs::create_dir_all(&invalid).expect("create invalid candidate directory");
        write_executable(&executable);

        assert_eq!(
            first_linux_antigravity_executable([invalid, executable.clone()]),
            Some(executable)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_linux_launcher_is_rejected() {
        let install = PathTestDir::new("non-executable");
        let executable = install.path().join("antigravity-ide");
        std::fs::write(&executable, b"launcher").expect("write non-executable launcher");

        let error = resolve_linux_antigravity_exec_path(&executable)
            .expect_err("reject launcher without an executable bit");

        assert!(error.contains("not executable"));
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_root_launcher_falls_back_to_executable_bin_launcher() {
        use std::os::unix::fs::PermissionsExt;

        let install = PathTestDir::new("non-executable-root-fallback");
        let root_executable = install.path().join("antigravity-ide");
        let bin_executable = install.path().join("bin").join("antigravity-ide");
        std::fs::write(&root_executable, b"launcher").expect("write root launcher");
        std::fs::set_permissions(&root_executable, std::fs::Permissions::from_mode(0o644))
            .expect("remove root launcher execute bits");
        write_executable(&bin_executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(install.path())
                .expect("fall back to executable bin launcher"),
            bin_executable
        );
    }

    #[test]
    fn discovery_includes_user_local_share_install_root() {
        let home = Path::new("/home/cockpit-test");

        let candidates = linux_antigravity_discovery_paths(Some(home), None);

        assert!(candidates.contains(&home.join(".local/share/antigravity-ide")));
    }

    #[test]
    fn discovery_appends_path_entries_once_and_skips_empty_entries() {
        let first = PathTestDir::new("path-first");
        let second = PathTestDir::new("path-second");
        let path_env = std::env::join_paths([
            first.path().as_os_str(),
            second.path().as_os_str(),
            first.path().as_os_str(),
            Path::new("").as_os_str(),
        ])
        .expect("build test PATH");

        let candidates = linux_antigravity_discovery_paths(
            Some(Path::new("/home/cockpit-test")),
            Some(path_env.as_os_str()),
        );
        let first_candidate = first.path().join("antigravity-ide");
        let second_candidate = second.path().join("antigravity-ide");
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| **candidate == first_candidate)
                .count(),
            1
        );
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| **candidate == second_candidate)
                .count(),
            1
        );
        assert!(candidates
            .iter()
            .all(|candidate| candidate != Path::new("antigravity-ide")));
        assert!(
            candidates
                .iter()
                .position(|candidate| *candidate == first_candidate)
                .expect("first PATH candidate")
                < candidates
                    .iter()
                    .position(|candidate| *candidate == second_candidate)
                    .expect("second PATH candidate")
        );
    }

    #[test]
    fn legacy_linux_layout_uses_the_shared_antigravity_resolver() {
        let install = PathTestDir::new("legacy-layout");
        let executable = install.path().join("bin").join("antigravity-ide");
        write_executable(&executable);

        assert_eq!(
            resolve_linux_antigravity_exec_path(install.path())
                .expect("resolve legacy Linux layout"),
            executable
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_symlink_to_executable_is_preserved() {
        let install = PathTestDir::new("relative-link-space-含");
        let target = install.path().join("real launcher");
        let link = install.path().join("launcher-link");
        write_executable(&target);
        std::os::unix::fs::symlink(target.file_name().expect("target name"), &link)
            .expect("create relative executable symlink");

        assert_eq!(
            resolve_linux_antigravity_exec_path(&link).expect("resolve symlink launcher"),
            link
        );
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_is_rejected() {
        let install = PathTestDir::new("broken-link");
        let link = install.path().join("broken-launcher");
        std::os::unix::fs::symlink("missing-launcher", &link)
            .expect("create broken launcher symlink");

        let error =
            resolve_linux_antigravity_exec_path(&link).expect_err("reject broken launcher symlink");

        assert!(error.contains("does not exist"));
    }

    #[test]
    fn root_and_bin_launchers_match_only_within_the_same_install_root() {
        assert!(antigravity_executable_paths_match(
            "/opt/Antigravity Install/bin/antigravity-ide",
            "/opt/Antigravity Install/antigravity-ide",
        ));
        assert!(antigravity_executable_paths_match(
            "/opt/Antigravity Install/antigravity-ide",
            "/opt/Antigravity Install/bin/antigravity-ide",
        ));
        assert!(!antigravity_executable_paths_match(
            "/opt/Antigravity Install/bin/antigravity-ide",
            "/opt/Other Install/antigravity-ide",
        ));
        assert!(!antigravity_executable_paths_match(
            "/opt/Antigravity Install/bin/antigravity-ide",
            "/opt/Antigravity Install/electron",
        ));
    }

    #[test]
    fn persisted_external_runtime_requires_a_managed_user_data_dir() {
        let allowed = HashSet::from([normalize_path_for_compare("/work/profiles/managed")]);

        assert!(persisted_linux_antigravity_identity_matches(
            "/work/install/bin/antigravity-ide",
            "/work/runtime/real-electron",
            Some("/work/profiles/managed"),
            &allowed,
            false,
        ));
        assert!(!persisted_linux_antigravity_identity_matches(
            "/work/install/bin/antigravity-ide",
            "/work/runtime/real-electron",
            None,
            &allowed,
            false,
        ));
        assert!(!persisted_linux_antigravity_identity_matches(
            "/work/install/bin/antigravity-ide",
            "/work/runtime/real-electron",
            Some("/work/profiles/unrelated"),
            &allowed,
            false,
        ));
        assert!(persisted_linux_antigravity_identity_matches(
            "/work/install/bin/antigravity-ide",
            "/work/install/antigravity-ide",
            None,
            &allowed,
            false,
        ));
        assert!(persisted_linux_antigravity_identity_matches(
            "/work/install/bin/antigravity-ide",
            "/work/runtime/real-electron",
            None,
            &allowed,
            true,
        ));
    }

    #[test]
    fn exact_configured_executable_wins_over_ordinary_path_words() {
        for word in [
            "tools",
            "audio",
            "plugin-build",
            "renderer-cache",
            "gpu-work",
            "utility-apps",
            "sandbox-data",
        ] {
            let path = format!("/opt/{word}/Antigravity.AppImage");
            assert!(is_linux_antigravity_process_candidate(
                &format!("{path} --reuse-window"),
                &path.to_ascii_lowercase(),
                true,
            ));
        }
    }

    #[test]
    fn structured_chromium_helper_arguments_are_still_rejected() {
        for args in [
            "--type=renderer",
            "--type utility",
            "--utility-sub-type=network.mojom.NetworkService",
            "--node-ipc",
            "--clientProcessId=42",
        ] {
            assert!(
                !is_linux_antigravity_process_candidate(
                    &format!("/opt/Custom/Antigravity.AppImage {args}"),
                    "/opt/custom/antigravity.appimage",
                    true,
                ),
                "helper argument was admitted: {args}"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod canonical_path_comparison_tests {
    use super::normalize_path_for_compare;

    #[test]
    fn canonicalized_symlink_and_target_paths_compare_equal() {
        let root =
            std::env::temp_dir().join(format!("cockpit-tools-path-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create comparison test directory");
        let target = root.join("antigravity-ide");
        let link = root.join("antigravity-link");
        std::fs::write(&target, b"launcher").expect("write comparison target");
        std::os::unix::fs::symlink(&target, &link).expect("create comparison symlink");

        assert_eq!(
            normalize_path_for_compare(&target.to_string_lossy()),
            normalize_path_for_compare(&link.to_string_lossy())
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(target_os = "macos")]
fn is_legacy_antigravity_macos_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.contains("/applications/antigravity.app")
        || lower.ends_with("/antigravity.app")
        || lower.contains("/antigravity.app/contents/")
}

#[cfg(target_os = "macos")]
fn resolve_macos_app_root_from_config(app: &str) -> Option<String> {
    let current = config::get_user_config();
    let raw = match app {
        "antigravity" => current.antigravity_app_path,
        "codex" => current.codex_app_path,
        "zed" => current.zed_app_path,
        "vscode" => current.vscode_app_path,
        "codebuddy" => current.codebuddy_app_path,
        "codebuddy_cn" => current.codebuddy_cn_app_path,
        "zcode" => current.zcode_app_path,
        _ => String::new(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    let app_root = normalize_macos_app_root(path)?;
    if std::path::Path::new(&app_root).exists() {
        return Some(app_root);
    }
    None
}

/// 从已解析的可执行文件路径中提取 .app 根路径
#[cfg(target_os = "macos")]
fn resolve_macos_app_root_from_launch_path(launch_path: &std::path::Path) -> Option<String> {
    let app_root = normalize_macos_app_root(launch_path)?;
    if std::path::Path::new(&app_root).exists() {
        Some(app_root)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn spawn_open_app_with_options(
    app_root: &str,
    args: &[String],
    force_new_instance: bool,
) -> Result<u32, String> {
    spawn_open_app_with_options_and_env(app_root, args, force_new_instance, &[])
}

#[cfg(target_os = "macos")]
fn spawn_open_app_with_options_and_env(
    app_root: &str,
    args: &[String],
    force_new_instance: bool,
    env_pairs: &[(&str, &str)],
) -> Result<u32, String> {
    let mut cmd = Command::new("open");
    sanitize_macos_gui_launch_env(&mut cmd);
    append_managed_proxy_env_to_open_args(&mut cmd);
    for (key, value) in env_pairs {
        cmd.arg("--env").arg(format!("{}={}", key, value));
    }
    if force_new_instance {
        cmd.arg("-n");
    }
    cmd.arg("-a").arg(app_root);
    if !args.is_empty() {
        cmd.arg("--args");
        for arg in args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
    }
    let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动失败: {}", e))?;
    Ok(child.id())
}

fn find_antigravity_process_exe() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let output = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let _pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let lower = cmdline.to_lowercase();
            if !lower.contains(ANTIGRAVITY_APP_CONTENTS_MARKER) {
                continue;
            }
            if lower.contains("antigravity tools.app/contents/") {
                continue;
            }
            if lower.contains("--type=") || lower.contains("crashpad_handler") {
                continue;
            }
            if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                return Some(std::path::PathBuf::from(exe));
            }
        }
        return None;
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

        let current_pid = std::process::id();

        for (pid, process) in system.processes() {
            let pid_u32 = pid.as_u32();
            if pid_u32 == current_pid {
                continue;
            }

            let name = process.name().to_string_lossy().to_lowercase();
            let exe_path = process
                .exe()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_lowercase();

            let args = process.cmd();
            let args_str = args
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            let is_helper = args_str.contains("--type=")
                || name.contains("helper")
                || name.contains("plugin")
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox")
                || exe_path.contains("crashpad");

            #[cfg(target_os = "windows")]
            let is_antigravity = is_windows_antigravity_ide_main_executable(&name, &exe_path);
            #[cfg(target_os = "linux")]
            let is_antigravity = (name.contains("antigravity-ide")
                || exe_path.contains("/antigravity-ide"))
                && !name.contains("tools")
                && !exe_path.contains("tools");

            if is_antigravity && !is_helper {
                if let Some(exe) = process.exe() {
                    return Some(exe.to_path_buf());
                }
            }
        }

        None
    }
}

fn find_vscode_process_exe() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let output = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let _pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let lower = cmdline.to_lowercase();
            if !lower.contains("visual studio code.app/contents/macos/") {
                continue;
            }
            if lower.contains("--type=") || lower.contains("crashpad_handler") {
                continue;
            }
            if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                return Some(std::path::PathBuf::from(exe));
            }
        }
        return None;
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

        let current_pid = std::process::id();

        for (pid, process) in system.processes() {
            let pid_u32 = pid.as_u32();
            if pid_u32 == current_pid {
                continue;
            }

            let name = process.name().to_string_lossy().to_lowercase();
            let exe_path = process
                .exe()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_lowercase();

            let args = process.cmd();
            let args_str = args
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            let is_helper = args_str.contains("--type=")
                || name.contains("helper")
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox");

            #[cfg(target_os = "windows")]
            let is_vscode = (name == "code.exe" || exe_path.ends_with("\\code.exe")) && !is_helper;
            #[cfg(target_os = "linux")]
            let is_vscode = (name == "code" || exe_path.ends_with("/code")) && !is_helper;

            if is_vscode {
                if let Some(exe) = process.exe() {
                    return Some(exe.to_path_buf());
                }
            }
        }

        None
    }
}

#[cfg(target_os = "macos")]
fn find_codex_process_exe() -> Option<std::path::PathBuf> {
    // Use ps to avoid sysinfo TCC dialogs on macOS
    let output = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
        let _pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let lower = cmdline.to_lowercase();
        if !is_codex_macos_main_process_command_line(&lower) {
            continue;
        }
        if lower.contains("--type=") || lower.contains("crashpad_handler") {
            continue;
        }
        if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
            return Some(std::path::PathBuf::from(exe));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn is_codex_macos_main_process_command_line(lower_cmdline: &str) -> bool {
    lower_cmdline.contains("chatgpt.app/contents/macos/chatgpt")
        || lower_cmdline.contains("codex.app/contents/macos/codex")
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexProcessTreeEntry {
    pid: u32,
    parent_pid: u32,
    command_line: String,
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
fn is_codex_direct_app_server_command_line(
    command_line: &str,
    expected_resource_executable: &str,
) -> bool {
    let command_line = command_line.trim();
    let expected = expected_resource_executable.trim();
    if command_line.is_empty() || expected.is_empty() {
        return false;
    }

    let remainder = if let Some(remainder) = command_line.strip_prefix(expected) {
        remainder
    } else {
        let quoted = format!("\"{}\"", expected);
        let Some(remainder) = command_line.strip_prefix(&quoted) else {
            return false;
        };
        remainder
    };
    let args = remainder.trim_start();
    let Some(after_app_server) = args.strip_prefix("app-server") else {
        return false;
    };
    if !after_app_server.is_empty() && !after_app_server.starts_with(char::is_whitespace) {
        return false;
    }
    !after_app_server.trim_start().starts_with("daemon")
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
fn select_codex_direct_app_server_descendants(
    entries: &[CodexProcessTreeEntry],
    root_pids: &[u32],
    expected_resource_executable: &str,
) -> Vec<u32> {
    let roots: HashSet<u32> = root_pids.iter().copied().filter(|pid| *pid != 0).collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let parents: HashMap<u32, u32> = entries
        .iter()
        .map(|entry| (entry.pid, entry.parent_pid))
        .collect();
    let mut selected = Vec::new();

    for entry in entries {
        if !is_codex_direct_app_server_command_line(
            &entry.command_line,
            expected_resource_executable,
        ) {
            continue;
        }
        let mut current = entry.parent_pid;
        let mut visited = HashSet::new();
        while current != 0 && visited.insert(current) {
            if roots.contains(&current) {
                selected.push(entry.pid);
                break;
            }
            let Some(parent) = parents.get(&current) else {
                break;
            };
            current = *parent;
        }
    }

    selected.sort();
    selected.dedup();
    selected
}

#[cfg(target_os = "macos")]
fn resolve_codex_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "ChatGPT")
        .or_else(|| resolve_macos_exec_path(path_str, "Codex"))
}

#[cfg(target_os = "windows")]
fn is_windows_antigravity_main_executable(name: &str, exe_path: &str) -> bool {
    (name == "antigravity ide.exe"
        || name == "antigravity.exe"
        || name == "antigravity-ide.exe"
        || exe_path.ends_with("\\antigravity ide.exe")
        || exe_path.ends_with("\\antigravity.exe")
        || exe_path.ends_with("\\antigravity-ide.exe"))
        && !exe_path.contains("crashpad")
}

#[cfg(target_os = "windows")]
fn is_windows_antigravity_ide_main_executable(name: &str, exe_path: &str) -> bool {
    (name == "antigravity ide.exe"
        || name == "antigravity-ide.exe"
        || exe_path.ends_with("\\antigravity ide.exe")
        || exe_path.ends_with("\\antigravity-ide.exe"))
        && !exe_path.contains("crashpad")
}

#[cfg(target_os = "windows")]
fn resolve_windows_antigravity_ide_custom_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if path.is_file() {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with("\\antigravity ide.exe") || lower.ends_with("\\antigravity-ide.exe") {
            return Some(path);
        }
        return None;
    }

    if path.is_dir() {
        for exe_name in ["Antigravity IDE.exe", "antigravity-ide.exe"] {
            let candidate = path.join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(any(target_os = "linux", test))]
fn linux_antigravity_discovery_paths(
    home: Option<&Path>,
    path_env: Option<&std::ffi::OsStr>,
) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut push_candidate = |candidate: PathBuf| {
        if seen.insert(candidate.clone()) {
            candidates.push(candidate);
        }
    };

    for candidate in [
        "/usr/bin/antigravity-ide",
        "/usr/local/bin/antigravity-ide",
        "/opt/antigravity-ide",
        "/usr/share/antigravity-ide",
    ] {
        push_candidate(PathBuf::from(candidate));
    }
    if let Some(home) = home {
        push_candidate(home.join(".local/bin/antigravity-ide"));
        push_candidate(home.join(".local/share/antigravity-ide"));
    }
    if let Some(path_env) = path_env {
        for directory in std::env::split_paths(path_env) {
            if directory.as_os_str().is_empty() {
                continue;
            }
            push_candidate(directory.join("antigravity-ide"));
        }
    }
    candidates
}

#[cfg(any(target_os = "linux", test))]
fn is_linux_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let Ok(path_c) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        return unsafe {
            libc::faccessat(
                libc::AT_FDCWD,
                path_c.as_ptr(),
                libc::X_OK,
                libc::AT_EACCESS,
            ) == 0
        };
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode() & 0o111 != 0;
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(any(target_os = "linux", test))]
fn resolve_linux_antigravity_exec_path(path: &Path) -> Result<std::path::PathBuf, String> {
    if path.is_file() {
        if is_linux_executable_file(path) {
            return Ok(path.to_path_buf());
        }
        return Err(format!(
            "Linux Antigravity executable is not executable: {}",
            path.display()
        ));
    }

    if path.is_dir() {
        for relative in ["antigravity-ide", "bin/antigravity-ide"] {
            let candidate = path.join(relative);
            if is_linux_executable_file(&candidate) {
                return Ok(candidate);
            }
        }
        return Err(format!(
            "Linux Antigravity install directory does not contain an executable antigravity-ide or bin/antigravity-ide: {}",
            path.display()
        ));
    }

    Err(format!(
        "Linux Antigravity path does not exist: {}",
        path.display()
    ))
}

#[cfg(any(target_os = "linux", test))]
fn first_linux_antigravity_executable(
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    candidates
        .into_iter()
        .find_map(|candidate| resolve_linux_antigravity_exec_path(&candidate).ok())
}

#[cfg(any(target_os = "linux", test))]
fn linux_antigravity_install_root_for_match(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_string_lossy();
    if !file_name.eq_ignore_ascii_case("antigravity-ide") {
        return None;
    }
    let parent = path.parent()?;
    if parent
        .file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"))
        .unwrap_or(false)
    {
        return parent.parent().map(Path::to_path_buf);
    }
    Some(parent.to_path_buf())
}

#[cfg(any(target_os = "linux", test))]
fn antigravity_executable_paths_match(expected: &str, actual: &str) -> bool {
    let expected = normalize_path_for_compare(expected);
    let actual = normalize_path_for_compare(actual);
    if expected.is_empty() || actual.is_empty() {
        return false;
    }
    if expected == actual {
        return true;
    }

    let Some(expected_root) = linux_antigravity_install_root_for_match(Path::new(&expected)) else {
        return false;
    };
    let Some(actual_root) = linux_antigravity_install_root_for_match(Path::new(&actual)) else {
        return false;
    };
    normalize_path_for_compare(&expected_root.to_string_lossy())
        == normalize_path_for_compare(&actual_root.to_string_lossy())
}

#[cfg(any(target_os = "linux", test))]
fn persisted_linux_antigravity_identity_matches(
    expected_launch: &str,
    actual_executable: &str,
    user_data_dir: Option<&str>,
    allowed_user_data_dirs: &HashSet<String>,
    allow_missing_user_data_dir: bool,
) -> bool {
    if antigravity_executable_paths_match(expected_launch, actual_executable) {
        return true;
    }
    match user_data_dir {
        Some(value) => {
            let normalized = normalize_path_for_compare(value);
            !normalized.is_empty() && allowed_user_data_dirs.contains(&normalized)
        }
        None => allow_missing_user_data_dir,
    }
}

fn antigravity_install_root_from_executable(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let is_linux_launcher = path
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("antigravity-ide")
        })
        .unwrap_or(false);
    let is_bin_directory = parent
        .file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"))
        .unwrap_or(false);
    if is_linux_launcher && is_bin_directory {
        return parent.parent().map(Path::to_path_buf);
    }
    Some(parent.to_path_buf())
}

pub(crate) fn antigravity_install_root_from_path(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return antigravity_install_root_from_executable(path);
    }
    if path.is_dir() {
        #[cfg(any(target_os = "linux", test))]
        {
            let is_bin_directory = path
                .file_name()
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"))
                .unwrap_or(false);
            if is_bin_directory && path.join("antigravity-ide").is_file() {
                return path.parent().map(Path::to_path_buf);
            }
        }
        return Some(path.to_path_buf());
    }
    None
}

pub fn detect_antigravity_exec_path() -> Option<std::path::PathBuf> {
    if let Some(path) = find_antigravity_process_exe() {
        return Some(path);
    }

    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(ANTIGRAVITY_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let base = std::path::PathBuf::from(&local_appdata)
                .join("Programs")
                .join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            let base = std::path::PathBuf::from(&program_files).join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            let base = std::path::PathBuf::from(&program_files_x86).join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "antigravity",
            &["Antigravity IDE.exe", "antigravity-ide.exe"],
            &["antigravity-ide"],
            &["antigravity-ide", "antigravity ide"],
            &["antigravity ide", "antigravity-ide"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir();
        if let Some(executable) = first_linux_antigravity_executable(
            linux_antigravity_discovery_paths(home.as_deref(), std::env::var_os("PATH").as_deref()),
        ) {
            return Some(executable);
        }
    }

    None
}

pub fn detect_antigravity_legacy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        for candidate in [
            ANTIGRAVITY_LEGACY_APP_PATH,
            "/Applications/Antigravity.app/Contents/MacOS/Electron",
        ] {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let base = std::path::PathBuf::from(local_appdata)
                .join("Programs")
                .join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            let base = std::path::PathBuf::from(program_files).join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            let base = std::path::PathBuf::from(program_files_x86).join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "antigravity",
            &["Antigravity.exe", "antigravity.exe", "Electron.exe"],
            &["antigravity"],
            &["antigravity"],
            &["antigravity"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/antigravity",
            "/opt/antigravity/antigravity",
            "/usr/share/antigravity/antigravity",
        ]
        .map(PathBuf::from);
        if let Some(executable) = first_linux_antigravity_executable(candidates) {
            return Some(executable);
        }
    }

    None
}

fn detect_vscode_exec_path() -> Option<std::path::PathBuf> {
    if let Some(path) = find_vscode_process_exe() {
        return Some(path);
    }

    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(VSCODE_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("VSCode")
                    .join("Code.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            candidates.push(
                std::path::PathBuf::from(program_files_x86)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "vscode",
            &["Code.exe", "Code - Insiders.exe"],
            &["code", "code-insiders"],
            &["vscode", "vscode-insiders"],
            &["visual studio code", "vs code", "vscode"],
        ) {
            return Some(path);
        }
        if let Some(path) = detect_vscode_exec_path_by_registry() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/code",
            "/snap/bin/code",
            "/var/lib/flatpak/exports/bin/com.visualstudio.code",
            "/usr/local/bin/code",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let user_local = home.join(".local/bin/code");
            if user_local.exists() {
                return Some(user_local);
            }
        }
    }

    None
}

fn detect_codebuddy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/CodeBuddy.app/Contents/MacOS/CodeBuddy",
            "/Applications/CodeBuddy.app/Contents/MacOS/Electron",
            "/Applications/CodeBuddy.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy")
                    .join("CodeBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("CodeBuddy")
                    .join("CodeBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/codebuddy",
            "/usr/local/bin/codebuddy",
            "/opt/codebuddy/codebuddy",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_codebuddy_cn_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/CodeBuddy CN.app/Contents/MacOS/CodeBuddy CN",
            "/Applications/CodeBuddy CN.app/Contents/MacOS/CodeBuddy",
            "/Applications/CodeBuddy CN.app/Contents/MacOS/Electron",
            "/Applications/CodeBuddy CN.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy CN")
                    .join("CodeBuddy CN.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy CN")
                    .join("CodeBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(&program_files)
                    .join("CodeBuddy CN")
                    .join("CodeBuddy CN.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("CodeBuddy CN")
                    .join("CodeBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/codebuddy-cn",
            "/usr/local/bin/codebuddy-cn",
            "/opt/codebuddy-cn/codebuddy-cn",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_qoder_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Qoder.app/Contents/MacOS/Qoder",
            "/Applications/Qoder.app/Contents/MacOS/Electron",
            "/Applications/Qoder.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Qoder")
                    .join("Qoder.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Qoder")
                    .join("Qoder.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/qoder", "/usr/local/bin/qoder", "/opt/qoder/qoder"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_zcode_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut candidates = vec![std::path::PathBuf::from(
            "/Applications/ZCode.app/Contents/MacOS/ZCode",
        )];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/ZCode.app/Contents/MacOS/ZCode"));
        }
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates
                .push(std::path::PathBuf::from(&local_appdata).join("Programs/ZCode/ZCode.exe"));
            candidates.push(std::path::PathBuf::from(local_appdata).join("ZCode/ZCode.exe"));
        }
        for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(root) = std::env::var(variable) {
                candidates.push(std::path::PathBuf::from(root).join("ZCode/ZCode.exe"));
            }
        }
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(path);
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "ZCode",
            &["ZCode.exe"],
            &["zcode"],
            &["zcode"],
            &["zcode", "z.ai"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/bin/zcode",
            "/usr/local/bin/zcode",
            "/opt/ZCode/zcode",
            "/opt/zcode/zcode",
            "/snap/bin/zcode",
        ] {
            let path = std::path::PathBuf::from(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".local/bin/zcode");
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_zed_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Zed.app/Contents/MacOS/zed",
            "/Applications/Zed.app",
            "/usr/local/bin/zed",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Zed")
                    .join("Zed.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Zed")
                    .join("Zed.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/zed", "/usr/local/bin/zed", "/opt/zed/zed"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_trae_exec_path() -> Option<std::path::PathBuf> {
    detect_trae_exec_path_for_platform(crate::modules::trae_account::TraePlatformKind::Trae)
}

fn detect_trae_exec_path_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app_root = format!("/Applications/{}", platform.macos_app_name());
        let candidates = [
            format!("{}/Contents/MacOS/Trae", app_root),
            format!("{}/Contents/MacOS/Electron", app_root),
            app_root,
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        let app_dir = platform.app_support_dir_name();
        let exe_names: &[&str] = match platform {
            crate::modules::trae_account::TraePlatformKind::Trae => &["Trae.exe"],
            crate::modules::trae_account::TraePlatformKind::TraeSolo => {
                &["TRAE SOLO.exe", "Trae.exe", "Electron.exe"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeCn => {
                &["Trae CN.exe", "Trae.exe", "Electron.exe"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
                &["TRAE SOLO CN.exe", "Trae.exe", "Electron.exe"]
            }
        };
        for base_path in crate::modules::trae_account::windows_trae_install_base_paths(platform) {
            if base_path.is_file() {
                candidates.push(base_path);
                continue;
            }
            for exe_name in exe_names {
                candidates.push(base_path.join(exe_name));
            }
        }
        let current = config::get_user_config();
        let configured_scan_roots = trae_configured_app_scan_roots(&current, platform);
        if !configured_scan_roots.trim().is_empty() {
            for root in
                expand_windows_scan_roots(parse_windows_scan_roots(Some(configured_scan_roots)))
            {
                for exe_name in exe_names {
                    if root
                        .file_name()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case(app_dir))
                        .unwrap_or(false)
                    {
                        candidates.push(root.join(exe_name));
                    }
                    candidates.push(root.join(app_dir).join(exe_name));
                }
            }
        }
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            for exe_name in exe_names {
                candidates.push(
                    std::path::PathBuf::from(&local_appdata)
                        .join("Programs")
                        .join(app_dir)
                        .join(exe_name),
                );
            }
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            for exe_name in exe_names {
                candidates.push(
                    std::path::PathBuf::from(&program_files)
                        .join(app_dir)
                        .join(exe_name),
                );
            }
        }
        for candidate in candidates {
            if candidate.exists() && windows_trae_candidate_matches_platform(&candidate, platform) {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates: &[&str] = match platform {
            crate::modules::trae_account::TraePlatformKind::Trae => {
                &["/usr/bin/trae", "/usr/local/bin/trae", "/opt/trae/trae"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeSolo => &[
                "/usr/bin/trae-solo",
                "/usr/local/bin/trae-solo",
                "/opt/trae-solo/trae-solo",
            ],
            crate::modules::trae_account::TraePlatformKind::TraeCn => &[
                "/usr/bin/trae-cn",
                "/usr/local/bin/trae-cn",
                "/opt/trae-cn/trae-cn",
            ],
            crate::modules::trae_account::TraePlatformKind::TraeSoloCn => &[
                "/usr/bin/trae-solo-cn",
                "/usr/local/bin/trae-solo-cn",
                "/opt/trae-solo-cn/trae-solo-cn",
            ],
        };
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_workbuddy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/WorkBuddy.app/Contents/MacOS/WorkBuddy",
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
            "/Applications/WorkBuddy.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("WorkBuddy")
                    .join("WorkBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("WorkBuddy")
                    .join("WorkBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/workbuddy",
            "/usr/local/bin/workbuddy",
            "/opt/workbuddy/workbuddy",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn resolve_codebuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["CodeBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("codebuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_codebuddy_cn_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["CodeBuddy CN", "CodeBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("codebuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_qoder_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["Qoder", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("qoder") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_zed_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["zed", "Zed"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name == "zed" || file_name.contains("zed") {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_trae_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["Trae", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("trae") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_workbuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["WorkBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("workbuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn resolve_qoder_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "Qoder")
}

#[cfg(not(target_os = "macos"))]
fn resolve_zed_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "zed")
}

#[cfg(not(target_os = "macos"))]
fn resolve_trae_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "Trae")
}

#[cfg(not(target_os = "macos"))]
fn resolve_workbuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "WorkBuddy")
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_cn_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "CodeBuddy CN")
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "CodeBuddy")
}

#[cfg(target_os = "windows")]
fn compare_windows_store_version(left: &[u32], right: &[u32]) -> std::cmp::Ordering {
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let left_part = *left.get(idx).unwrap_or(&0);
        let right_part = *right.get(idx).unwrap_or(&0);
        match left_part.cmp(&right_part) {
            std::cmp::Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(target_os = "windows")]
fn parse_codex_store_version_from_dir_name(dir_name: &str) -> Option<Vec<u32>> {
    let lower = dir_name.to_ascii_lowercase();
    let prefix = [
        "openai.chatgpt_",
        "openai.chatgpt-desktop_",
        "openai.codex_",
    ]
    .iter()
    .find(|prefix| lower.starts_with(**prefix))?;
    let suffix = dir_name.get(prefix.len()..)?;
    let version_part = suffix.split('_').next()?.trim();
    if version_part.is_empty() {
        return None;
    }
    let mut version: Vec<u32> = Vec::new();
    for part in version_part.split('.') {
        if part.is_empty() {
            return None;
        }
        version.push(part.parse::<u32>().ok()?);
    }
    if version.is_empty() {
        return None;
    }
    Some(version)
}

#[cfg(target_os = "windows")]
fn codex_store_package_priority(dir_name: &str) -> u8 {
    let lower = dir_name.to_ascii_lowercase();
    if lower.starts_with("openai.chatgpt_") || lower.starts_with("openai.chatgpt-desktop_") {
        2
    } else if lower.starts_with("openai.codex_") {
        1
    } else {
        0
    }
}

#[cfg(target_os = "windows")]
fn find_codex_windows_app_main_exe(app_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    for exe_name in ["ChatGPT.exe", "Codex.exe"] {
        let candidate = app_dir.join(exe_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_exec_path_by_windowsapps_scan() -> Option<std::path::PathBuf> {
    let mut best: Option<(u8, Vec<u32>, std::path::PathBuf)> = None;

    for drive_root in windows_fixed_drive_roots() {
        let drive_letter = drive_root.to_string_lossy().chars().next().unwrap_or('C');
        let windows_apps_root = if drive_letter == 'C' {
            drive_root.join("Program Files").join("WindowsApps")
        } else {
            drive_root.join("WindowsApps")
        };
        let root_path = windows_apps_root;
        if !root_path.exists() {
            continue;
        }

        let entries = match std::fs::read_dir(&root_path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !file_type.is_dir() {
                continue;
            }

            let dir_name = entry.file_name();
            let dir_name = dir_name.to_string_lossy();
            let Some(version) = parse_codex_store_version_from_dir_name(&dir_name) else {
                continue;
            };
            let package_priority = codex_store_package_priority(&dir_name);

            let candidate = match find_codex_windows_app_main_exe(&entry.path().join("app")) {
                Some(path) => path,
                None => continue,
            };

            let replace = match &best {
                None => true,
                Some((best_priority, best_version, _)) => {
                    package_priority > *best_priority
                        || (package_priority == *best_priority
                            && compare_windows_store_version(&version, best_version).is_gt())
                }
            };
            if replace {
                best = Some((package_priority, version, candidate));
            }
        }
    }

    if let Some((_, _, path)) = best {
        crate::modules::logger::log_info(&format!(
            "[Path Detect] codex windowsapps scan hit: {}",
            path.to_string_lossy()
        ));
        return Some(path);
    }

    None
}

#[cfg(target_os = "windows")]
fn detect_codex_exec_path_by_appx_install_location() -> Option<std::path::PathBuf> {
    let script = r#"$names = @('OpenAI.ChatGPT', 'OpenAI.ChatGPT-Desktop', 'OpenAI.Codex')
$pkg = $names |
  ForEach-Object { Get-AppxPackage -Name $_ -ErrorAction SilentlyContinue } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*' -or $_.PackageFamilyName -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
}
if ($pkg -and -not [string]::IsNullOrWhiteSpace($pkg.InstallLocation)) {
  Write-Output ([string]$pkg.InstallLocation.Trim())
}"#;

    let output =
        powershell_output_with_timeout(&["-Command", script], WINDOWS_PROCESS_PROBE_TIMEOUT)
            .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let install_location = line.trim().trim_matches('"');
        if install_location.is_empty() {
            continue;
        }
        let Some(candidate) = find_codex_windows_app_main_exe(
            &std::path::PathBuf::from(install_location).join("app"),
        ) else {
            continue;
        };
        if candidate.exists() {
            crate::modules::logger::log_info(&format!(
                "[Path Detect] codex appx install hit: {}",
                candidate.to_string_lossy()
            ));
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id_by_startapps() -> Option<String> {
    let script = r#"$entry = Get-StartApps |
  Where-Object {
    $_.AppID -like 'OpenAI.ChatGPT*' -or
    $_.AppID -like 'OpenAI.Codex_*' -or
    $_.Name -like 'ChatGPT*' -or
    $_.Name -like 'Codex*'
  } |
  Sort-Object @{ Expression = { if ($_.AppID -like 'OpenAI.ChatGPT*' -or $_.Name -like 'ChatGPT*') { 0 } else { 1 } } }, Name |
  Select-Object -First 1
if ($entry -and -not [string]::IsNullOrWhiteSpace($entry.AppID)) {
  Write-Output ([string]$entry.AppID.Trim())
}"#;

    let output = powershell_output(&["-Command", script]).ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let app_user_model_id = line.trim().trim_matches('"');
        if !app_user_model_id.is_empty() {
            return Some(app_user_model_id.to_string());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id_by_appx_fallback() -> Option<String> {
    let script = r#"$names = @('OpenAI.ChatGPT', 'OpenAI.ChatGPT-Desktop', 'OpenAI.Codex')
$pkg = $names |
  ForEach-Object { Get-AppxPackage -Name $_ -ErrorAction SilentlyContinue } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*' -or $_.PackageFamilyName -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
}
if ($pkg -and -not [string]::IsNullOrWhiteSpace($pkg.PackageFamilyName)) {
  Write-Output ([string]($pkg.PackageFamilyName.Trim() + '!App'))
}"#;

    let output = powershell_output(&["-Command", script]).ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let app_user_model_id = line.trim().trim_matches('"');
        if !app_user_model_id.is_empty() {
            return Some(app_user_model_id.to_string());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id_uncached() -> Option<String> {
    if let Some(app_user_model_id) = detect_codex_store_app_user_model_id_by_startapps() {
        crate::modules::logger::log_info(&format!(
            "[Codex Store] StartApps 命中 AppUserModelId: {}",
            app_user_model_id
        ));
        return Some(app_user_model_id);
    }
    if let Some(app_user_model_id) = detect_codex_store_app_user_model_id_by_appx_fallback() {
        crate::modules::logger::log_info(&format!(
            "[Codex Store] Appx fallback 命中 AppUserModelId: {}",
            app_user_model_id
        ));
        return Some(app_user_model_id);
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id() -> Option<String> {
    if let Some(app_user_model_id) = CODEX_STORE_APP_USER_MODEL_ID_CACHE.get() {
        return Some(app_user_model_id.clone());
    }

    let detected = detect_codex_store_app_user_model_id_uncached();
    if let Some(ref app_user_model_id) = detected {
        let _ = CODEX_STORE_APP_USER_MODEL_ID_CACHE.set(app_user_model_id.clone());
    }
    detected
}

#[cfg(target_os = "windows")]
fn powershell_argument_list_clause(values: &[String]) -> String {
    let arguments = values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("'{}'", escape_powershell_single_quoted(value)))
        .collect::<Vec<_>>();
    if arguments.is_empty() {
        return String::new();
    }
    format!(" -ArgumentList @({})", arguments.join(", "))
}

#[cfg(target_os = "windows")]
fn launch_codex_via_store_app_user_model_id(
    app_user_model_id: &str,
    codex_home: Option<&str>,
    app_user_data_dir: Option<&str>,
    extra_args: &[String],
) -> Result<(), String> {
    let app_user_model_id = app_user_model_id.trim();
    if app_user_model_id.is_empty() {
        return Err("Codex AppUserModelId 为空".to_string());
    }

    let escaped = escape_powershell_single_quoted(app_user_model_id);
    let mut env_pairs = managed_proxy_env_pairs();
    if let Some(codex_home) = codex_home.map(str::trim).filter(|value| !value.is_empty()) {
        env_pairs.push(("CODEX_HOME", codex_home.to_string()));
    }
    if let Some(app_user_data_dir) = app_user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        env_pairs.push((
            "CODEX_ELECTRON_USER_DATA_PATH",
            app_user_data_dir.to_string(),
        ));
    }
    let env_lines = env_pairs
        .into_iter()
        .map(|(key, value)| format!("$env:{}='{}'", key, escape_powershell_single_quoted(&value)))
        .collect::<Vec<_>>()
        .join("\n");
    let argument_list = powershell_argument_list_clause(extra_args);
    let script = format!(
        r#"{env_lines}
$appId='{escaped}';
$target='shell:AppsFolder\' + $appId
Start-Process -FilePath $target{argument_list} -ErrorAction Stop | Out-Null"#
    );

    let output = powershell_output(&["-Command", &script])
        .map_err(|e| format!("系统入口启动调用失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_head = stderr.trim().chars().take(400).collect::<String>();
        return Err(format!(
            "系统入口启动失败: status={}, stderr={}",
            output.status,
            if stderr_head.is_empty() {
                "<empty>".to_string()
            } else {
                stderr_head
            }
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_codex_via_powershell_exec_path(
    launch_path: &std::path::Path,
    codex_home: &str,
    app_user_data_dir: &std::path::Path,
    extra_args: &[String],
) -> Result<(), String> {
    let launch_path = launch_path.to_string_lossy();
    let launch_path = launch_path.trim();
    if launch_path.is_empty() {
        return Err("Codex 启动路径为空".to_string());
    }

    let mut env_pairs = managed_proxy_env_pairs();
    env_pairs.push(("CODEX_HOME", codex_home.to_string()));
    env_pairs.push((
        "CODEX_ELECTRON_USER_DATA_PATH",
        app_user_data_dir.to_string_lossy().to_string(),
    ));
    let env_lines = env_pairs
        .into_iter()
        .map(|(key, value)| format!("$env:{}='{}'", key, escape_powershell_single_quoted(&value)))
        .collect::<Vec<_>>()
        .join("\n");
    let argument_list = powershell_argument_list_clause(extra_args);
    let script = format!(
        r#"{env_lines}
$exe='{exe}';
Start-Process -FilePath $exe{argument_list} -ErrorAction Stop | Out-Null"#,
        exe = escape_powershell_single_quoted(launch_path),
    );

    let output = powershell_output(&["-Command", &script])
        .map_err(|e| format!("PowerShell 启动 Codex 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_head = stderr.trim().chars().take(400).collect::<String>();
        return Err(format!(
            "PowerShell 启动 Codex 失败: status={}, stderr={}",
            output.status,
            if stderr_head.is_empty() {
                "<empty>".to_string()
            } else {
                stderr_head
            }
        ));
    }
    Ok(())
}

const CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX: &str = "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:";

fn codex_managed_store_launch_unsafe_error(direct_error: &str, powershell_error: &str) -> String {
    format!(
        "{}direct_error={}; powershell_error={}",
        CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX, direct_error, powershell_error
    )
}

pub(crate) fn detect_codex_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = find_codex_process_exe() {
            return Some(path);
        }
        let path = std::path::PathBuf::from(CODEX_CHATGPT_APP_PATH);
        if path.exists() {
            return Some(path);
        }
        let path = std::path::PathBuf::from(CODEX_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = detect_codex_exec_path_by_windowsapps_scan() {
            return Some(path);
        }
        if let Some(path) = detect_codex_exec_path_by_appx_install_location() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in linux_codex_discovery_paths(
            dirs::home_dir().as_deref(),
            std::env::var_os("PATH").as_deref(),
        ) {
            if is_linux_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(any(target_os = "linux", test))]
fn linux_codex_discovery_paths(
    home: Option<&Path>,
    path_env: Option<&std::ffi::OsStr>,
) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut push_candidate = |candidate: PathBuf| {
        if seen.insert(candidate.clone()) {
            candidates.push(candidate);
        }
    };

    for candidate in [
        "/usr/bin/chatgpt",
        "/usr/local/bin/chatgpt",
        "/usr/lib/chatgpt/ChatGPT",
        "/opt/chatgpt/ChatGPT",
    ] {
        push_candidate(PathBuf::from(candidate));
    }
    if let Some(home) = home {
        push_candidate(home.join(".local/bin/chatgpt"));
    }
    if let Some(path_env) = path_env {
        for directory in std::env::split_paths(path_env) {
            if !directory.as_os_str().is_empty() {
                push_candidate(directory.join("chatgpt"));
            }
        }
    }
    candidates
}

fn detect_and_save_codex_launch_path() -> Option<std::path::PathBuf> {
    let expected_current = config::get_user_config().codex_app_path;
    let detected = detect_codex_exec_path()?;
    update_app_path_in_config("codex", &detected, &expected_current);
    Some(detected)
}

#[cfg(any(test, target_os = "windows"))]
fn normalized_windows_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(any(test, target_os = "windows"))]
fn is_legacy_codex_store_launch_path(path: &Path) -> bool {
    let normalized = normalized_windows_path_text(path);
    normalized.ends_with("\\codex.exe") && normalized.contains("\\windowsapps\\openai.codex_")
}

#[cfg(any(test, target_os = "windows"))]
fn is_chatgpt_windows_launch_path(path: &Path) -> bool {
    normalized_windows_path_text(path).ends_with("\\chatgpt.exe")
}

#[cfg(any(test, target_os = "macos"))]
fn normalized_macos_codex_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(any(test, target_os = "macos"))]
fn is_official_legacy_codex_macos_path(path: &Path) -> bool {
    matches!(
        normalized_macos_codex_path_text(path).as_str(),
        "/applications/codex.app" | "/applications/codex.app/contents/macos/codex"
    )
}

#[cfg(any(test, target_os = "macos"))]
fn is_official_chatgpt_macos_path(path: &Path) -> bool {
    matches!(
        normalized_macos_codex_path_text(path).as_str(),
        "/applications/chatgpt.app" | "/applications/chatgpt.app/contents/macos/chatgpt"
    )
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn should_migrate_legacy_codex_launch_path(current: &Path, detected: &Path) -> bool {
    let mut should_migrate = false;

    #[cfg(any(test, target_os = "windows"))]
    {
        should_migrate |=
            is_legacy_codex_store_launch_path(current) && is_chatgpt_windows_launch_path(detected);
    }

    #[cfg(any(test, target_os = "macos"))]
    {
        should_migrate |= is_official_legacy_codex_macos_path(current)
            && is_official_chatgpt_macos_path(detected);
    }

    should_migrate
}

/// 只有旧官方 Codex 安装路径才需要继续执行 ChatGPT 路径迁移探测。
///
/// `detect_codex_exec_path` 在 Windows 上可能调用 PowerShell/Get-AppxPackage；
/// 已迁移的 ChatGPT 路径和用户自定义路径不需要反复承担这项开销。
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn should_probe_legacy_codex_launch_path(current: &Path) -> bool {
    let mut should_probe = false;

    #[cfg(any(test, target_os = "windows"))]
    {
        should_probe |= is_legacy_codex_store_launch_path(current);
    }

    #[cfg(any(test, target_os = "macos"))]
    {
        should_probe |= is_official_legacy_codex_macos_path(current);
    }

    should_probe
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn migrate_legacy_codex_launch_path(custom_path: &str) -> Option<std::path::PathBuf> {
    let current_path = std::path::PathBuf::from(custom_path);
    if !should_probe_legacy_codex_launch_path(&current_path) {
        return None;
    }
    let detected = detect_codex_exec_path()?;
    if !should_migrate_legacy_codex_launch_path(&current_path, &detected) {
        return None;
    }

    update_app_path_in_config("codex", &detected, custom_path);
    crate::modules::logger::log_info(&format!(
        "[Path Detect] migrated legacy Codex launch path to ChatGPT: old={} new={}",
        current_path.to_string_lossy(),
        detected.to_string_lossy()
    ));
    Some(detected)
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

        #[cfg(target_os = "windows")]
        {
            if let Some(exec) = resolve_windows_antigravity_ide_custom_path(&custom) {
                return Ok(exec);
            }
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let custom_path = Path::new(&custom);
            if custom_path.exists() {
                return resolve_linux_antigravity_exec_path(custom_path);
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
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

fn resolve_antigravity_legacy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) =
        normalize_custom_path(Some(&config::get_user_config().antigravity_app_path))
    {
        #[cfg(target_os = "macos")]
        if is_legacy_antigravity_macos_path(&custom) {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Antigravity")
                .or_else(|| resolve_macos_exec_path(&custom, "Electron"))
            {
                return Ok(exec);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let custom_path = Path::new(&custom);
            if custom_path.exists() {
                return resolve_linux_antigravity_exec_path(custom_path);
            }
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "linux")))]
        {
            let custom_path = std::path::PathBuf::from(&custom);
            let lower = custom.to_ascii_lowercase();
            if lower.contains("antigravity") && !lower.contains("antigravity ide") {
                if custom_path.is_file() {
                    return Ok(custom_path);
                }
                for exe_name in ["Antigravity.exe", "antigravity.exe", "Electron.exe"] {
                    let candidate = custom_path.join(exe_name);
                    if candidate.exists() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    if let Some(detected) = detect_antigravity_legacy_exec_path() {
        return Ok(detected);
    }

    Err(app_path_missing_error("antigravity"))
}

pub fn ensure_antigravity_legacy_launch_path_configured() -> Result<(), String> {
    resolve_antigravity_legacy_launch_path().map(|_| ())
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
        if resolve_codex_launch_path().is_ok() {
            return Ok(());
        }
        if detect_and_save_app_path("codex", true).is_some() {
            return Ok(());
        }
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

pub fn resolve_zcode_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().zcode_app_path)) {
        if let Some(exec) = resolve_macos_exec_path(&custom, "ZCode") {
            return Ok(exec);
        }
        return Err(app_path_missing_error("zcode"));
    }

    if let Some(detected) = detect_zcode_exec_path() {
        update_app_path_in_config("zcode", &detected, "");
        let detected = detected.to_string_lossy();
        if let Some(exec) = resolve_macos_exec_path(&detected, "ZCode") {
            return Ok(exec);
        }
    }

    Err(app_path_missing_error("zcode"))
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
    resolve_trae_launch_path_for_platform(crate::modules::trae_account::TraePlatformKind::Trae)
}

fn trae_configured_app_path(
    current: &config::UserConfig,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> &str {
    match platform {
        crate::modules::trae_account::TraePlatformKind::Trae => current.trae_app_path.as_str(),
        crate::modules::trae_account::TraePlatformKind::TraeSolo => {
            current.trae_solo_app_path.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeCn => current.trae_cn_app_path.as_str(),
        crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
            current.trae_solo_cn_app_path.as_str()
        }
    }
}

#[cfg(target_os = "windows")]
fn trae_configured_app_scan_roots(
    current: &config::UserConfig,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> &str {
    match platform {
        crate::modules::trae_account::TraePlatformKind::Trae => {
            current.trae_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeSolo => {
            current.trae_solo_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeCn => {
            current.trae_cn_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
            current.trae_solo_cn_app_scan_roots.as_str()
        }
    }
}

fn resolve_trae_launch_path_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Result<std::path::PathBuf, String> {
    let current = config::get_user_config();
    if let Some(custom) = normalize_custom_path(Some(trae_configured_app_path(&current, platform)))
    {
        if let Some(exec) = resolve_trae_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error(platform.provider_key()));
    }

    if let Some(detected) = detect_trae_exec_path_for_platform(platform) {
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

    Err(app_path_missing_error(platform.provider_key()))
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
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codex_app_path)) {
        if let Some(migrated) = migrate_legacy_codex_launch_path(&custom) {
            return Ok(migrated);
        }
        if let Some(exec) = resolve_codex_macos_exec_path(&custom) {
            return Ok(exec);
        }
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
        return Err(app_path_missing_error("codex"));
    }

    if let Some(detected) = detect_and_save_codex_launch_path() {
        return Ok(detected);
    }

    Err(app_path_missing_error("codex"))
}

#[cfg(not(target_os = "macos"))]
fn resolve_codex_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codex_app_path)) {
        #[cfg(target_os = "windows")]
        if let Some(migrated) = migrate_legacy_codex_launch_path(&custom) {
            return Ok(migrated);
        }
        if let Some(exec) = resolve_macos_exec_path(&custom, "Codex") {
            return Ok(exec);
        }
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
        return Err(app_path_missing_error("codex"));
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codex"))
}

pub fn detect_and_save_app_path(app: &str, force: bool) -> Option<String> {
    detect_and_save_app_path_raw(app, force).map(|path| normalize_windows_user_facing_path(&path))
}

fn detect_and_save_app_path_raw(app: &str, force: bool) -> Option<String> {
    let current = config::get_user_config();
    match app {
        "antigravity" | "antigravity_ide" => {
            if !force && !current.antigravity_app_path.trim().is_empty() {
                return Some(current.antigravity_app_path);
            }
            if let Some(detected) = detect_antigravity_exec_path() {
                update_app_path_in_config("antigravity", &detected, &current.antigravity_app_path);
                return Some(config::get_user_config().antigravity_app_path);
            }
        }
        "antigravity_legacy" => {
            if !force && !current.antigravity_app_path.trim().is_empty() {
                return Some(current.antigravity_app_path);
            }
            if let Some(detected) = detect_antigravity_legacy_exec_path() {
                update_app_path_in_config("antigravity", &detected, &current.antigravity_app_path);
                return Some(config::get_user_config().antigravity_app_path);
            }
        }
        "codex" => {
            if !force && !current.codex_app_path.trim().is_empty() {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
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
        "zcode" => {
            if !force && !current.zcode_app_path.trim().is_empty() {
                return Some(current.zcode_app_path);
            }
            if let Some(detected) = detect_zcode_exec_path() {
                update_app_path_in_config("zcode", &detected, &current.zcode_app_path);
                return Some(config::get_user_config().zcode_app_path);
            }
        }
        "trae" | "trae_solo" | "trae_cn" | "trae_solo_cn" => {
            if let Ok(platform) = crate::modules::trae_account::TraePlatformKind::parse(Some(app)) {
                if !force {
                    let configured = trae_configured_app_path(&current, platform);
                    if !configured.trim().is_empty() {
                        return Some(configured.to_string());
                    }
                }
                if let Some(detected) = detect_trae_exec_path_for_platform(platform) {
                    update_app_path_in_config(
                        app,
                        &detected,
                        trae_configured_app_path(&current, platform),
                    );
                    let refreshed = config::get_user_config();
                    return Some(trae_configured_app_path(&refreshed, platform).to_string());
                }
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
    let normalized_input = normalize_windows_user_facing_path(trimmed).replace('/', "\\");
    #[cfg(not(target_os = "windows"))]
    let normalized_input = trimmed.to_string();

    let resolved = std::fs::canonicalize(&normalized_input)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(normalized_input);

    #[cfg(target_os = "windows")]
    let resolved = normalize_windows_user_facing_path(&resolved);

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

#[cfg(any(target_os = "linux", test))]
fn is_linux_chromium_helper_tokens(tokens: &[String]) -> bool {
    for token in tokens {
        let token = token.to_ascii_lowercase();
        if token.starts_with("--type=")
            || token.starts_with("--utility-sub-type=")
            || token.starts_with("--clientprocessid=")
            || token.starts_with("--crashpad-handler")
        {
            return true;
        }
        if matches!(
            token.as_str(),
            "--type" | "--utility-sub-type" | "--clientprocessid" | "--node-ipc"
        ) {
            return true;
        }
    }
    false
}

#[cfg(any(target_os = "linux", test))]
fn linux_proc_cmdline_args(raw: &[u8]) -> Vec<String> {
    raw.split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn linux_antigravity_launcher_signature_from_tokens(
    tokens: &[String],
    exe_path_lower: &str,
) -> bool {
    fn token_has_launcher_component(token: &str) -> bool {
        token
            .split(['/', '\\'])
            .filter(|component| !component.is_empty())
            .any(|component| {
                component
                    .to_ascii_lowercase()
                    .replace(' ', "-")
                    .replace('_', "-")
                    == "antigravity-ide"
            })
    }

    let path_has_signature = Path::new(exe_path_lower)
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("antigravity-ide")
        })
        .unwrap_or(false);
    if path_has_signature {
        return true;
    }

    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if is_env_token(token) {
            index += 1;
            continue;
        }
        if token == "--app" || token == "--application" {
            if let Some(value) = tokens.get(index + 1) {
                if token_has_launcher_component(value) {
                    return true;
                }
            }
            index += 2;
            continue;
        }
        let candidate = if let Some(value) = token
            .strip_prefix("--app=")
            .or_else(|| token.strip_prefix("--application="))
        {
            Some(value)
        } else if token.starts_with("--user-data-dir")
            || token.starts_with("--profile")
            || token.starts_with("--extensions-dir")
        {
            None
        } else if token.starts_with("--") {
            None
        } else {
            Some(token.as_str())
        };
        if candidate.is_some_and(token_has_launcher_component) {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(any(target_os = "linux", test))]
fn linux_antigravity_external_runtime_matches_expected_launch(
    tokens: &[String],
    expected_launch: &str,
) -> bool {
    let expected_launch = normalize_path_for_compare(expected_launch);
    if expected_launch.is_empty() {
        return false;
    }
    let expected_root = linux_antigravity_install_root_for_match(Path::new(&expected_launch))
        .or_else(|| Path::new(&expected_launch).parent().map(Path::to_path_buf));
    let Some(expected_root) = expected_root else {
        return false;
    };
    let expected_root = normalize_path_for_compare(&expected_root.to_string_lossy());
    if expected_root.is_empty() {
        return false;
    }

    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let token_lower = token.to_ascii_lowercase();
        let app_path = if token_lower == "--app" || token_lower == "--application" {
            index += 1;
            tokens.get(index).map(String::as_str)
        } else {
            token
                .strip_prefix("--app=")
                .or_else(|| token.strip_prefix("--application="))
        };
        if let Some(app_path) = app_path {
            let app_path = Path::new(app_path);
            for ancestor in app_path.ancestors() {
                let is_resources_dir = ancestor
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("resources"));
                if !is_resources_dir {
                    continue;
                }
                if let Some(app_root) = ancestor.parent() {
                    let app_root = normalize_path_for_compare(&app_root.to_string_lossy());
                    if app_root == expected_root {
                        return true;
                    }
                }
                break;
            }
        }
        index += 1;
    }
    false
}

#[cfg(any(target_os = "linux", test))]
fn is_linux_antigravity_process_candidate_from_tokens(
    tokens: &[String],
    exe_path_lower: &str,
    expected_executable_match: bool,
) -> bool {
    if !expected_executable_match
        && !linux_antigravity_launcher_signature_from_tokens(tokens, exe_path_lower)
    {
        return false;
    }
    !is_linux_chromium_helper_tokens(tokens)
}

#[cfg(any(target_os = "linux", test))]
fn extract_user_data_dir_from_tokens(tokens: &[String]) -> Option<String> {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix("--user-data-dir=") {
            if !rest.trim().is_empty() {
                return Some(rest.to_string());
            }
        }
        if token == "--user-data-dir" {
            if let Some(value) = tokens.get(index + 1) {
                if !value.trim().is_empty() {
                    return Some(value.to_string());
                }
            }
            return None;
        }
        index += 1;
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn is_linux_antigravity_process_candidate(
    cmdline_lower: &str,
    exe_path_lower: &str,
    expected_executable_match: bool,
) -> bool {
    let tokens = split_command_tokens(cmdline_lower);
    is_linux_antigravity_process_candidate_from_tokens(
        &tokens,
        exe_path_lower,
        expected_executable_match,
    )
}
