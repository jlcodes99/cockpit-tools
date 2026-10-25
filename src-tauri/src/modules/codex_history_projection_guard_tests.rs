use super::*;

fn snapshot(executable: Option<&str>, name: &str, home: HomeEvidence) -> ProcessSnapshot {
    ProcessSnapshot {
        name: name.into(),
        executable: executable.map(str::to_owned),
        command_prefix: Vec::new(),
        home,
        zombie: false,
    }
}

#[test]
fn default_desktop_without_codex_home_is_still_a_writer() {
    for executable in [
        "/Applications/Codex.app/Contents/MacOS/Codex",
        "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
        "C:\\Program Files\\WindowsApps\\OpenAI.Codex\\app\\Codex.exe",
    ] {
        let process = snapshot(
            Some(executable),
            "Codex",
            HomeEvidence::Default("/user/.codex".into()),
        );
        assert_eq!(
            check_snapshots("/user/.codex", &[process], true, false),
            Err(RUNNING.into())
        );
    }
}

#[test]
fn target_managed_home_blocks_but_explicit_other_home_does_not() {
    let target = "/profiles/target";
    let matching = snapshot(
        Some("/runtime/codex"),
        "codex",
        HomeEvidence::Explicit(target.into()),
    );
    assert_eq!(
        check_snapshots(target, &[matching], true, false),
        Err(RUNNING.into())
    );
    let other = snapshot(
        Some("/runtime/codex"),
        "codex",
        HomeEvidence::Explicit("/profiles/other".into()),
    );
    assert!(check_snapshots(target, &[other], true, false).is_ok());
    let default = snapshot(
        Some("/Applications/Codex.app/Contents/MacOS/Codex"),
        "Codex",
        HomeEvidence::Default("/user/.codex".into()),
    );
    assert!(check_snapshots(target, &[default], true, false).is_ok());
}

#[test]
fn orphan_app_server_and_interactive_cli_are_both_writers() {
    for argument in ["app-server", "exec", "resume"] {
        let mut process = snapshot(
            Some("/runtime/codex"),
            "codex",
            HomeEvidence::Explicit("/profile".into()),
        );
        process.command_prefix = vec!["/runtime/codex".into(), argument.into()];
        assert_eq!(
            check_snapshots("/profile", &[process], true, false),
            Err(RUNNING.into())
        );
    }
}

#[test]
fn official_release_binary_names_are_detected_after_resolving_launcher_symlinks() {
    for executable in [
        "/opt/homebrew/Caskroom/codex/1/codex-aarch64-apple-darwin",
        "/opt/codex-x86_64-unknown-linux-musl",
        r"C:\Tools\codex-x86_64-pc-windows-msvc.exe",
    ] {
        let process = snapshot(
            Some(executable),
            executable_name(executable),
            HomeEvidence::Explicit("/profile".into()),
        );
        assert_eq!(
            check_snapshots("/profile", &[process], true, false),
            Err(RUNNING.into())
        );
    }
}

#[test]
fn official_npm_launcher_is_detected_without_matching_node_eval_or_other_scripts() {
    let mut process = snapshot(
        Some("/usr/bin/node"),
        "node",
        HomeEvidence::Explicit("/profile".into()),
    );
    process.command_prefix = vec![
        "node".into(),
        "/opt/node_modules/@openai/codex/bin/codex.js".into(),
    ];
    assert_eq!(
        check_snapshots("/profile", &[process], true, false),
        Err(RUNNING.into())
    );
    for script in [
        "-e",
        "/tmp/codex.js",
        "/opt/node_modules/other-codex/bin/codex.js",
    ] {
        let mut process = snapshot(Some("/usr/bin/node"), "node", HomeEvidence::Unknown);
        process.command_prefix = vec!["node".into(), script.into()];
        assert!(check_snapshots("/profile", &[process], true, false).is_ok());
    }
}

#[test]
fn shell_strings_and_similarly_named_tools_are_not_codex_processes() {
    for executable in [
        "/bin/zsh",
        "/usr/bin/rg",
        "/tools/codex-helper-tool",
        "/apps/Codex Helper",
    ] {
        let mut process = snapshot(
            Some(executable),
            executable_name(executable),
            HomeEvidence::Unknown,
        );
        process.command_prefix = vec![
            executable.into(),
            "CODEX_HOME=/profile /runtime/codex app-server".into(),
        ];
        assert!(check_snapshots("/profile", &[process], true, false).is_ok());
    }
    let spoofed_name = snapshot(Some("/bin/sh"), "codex", HomeEvidence::Unknown);
    assert!(check_snapshots("/profile", &[spoofed_name], true, false).is_ok());
}

#[test]
fn unreadable_executable_or_unknown_codex_home_blocks_repair() {
    let unreadable = snapshot(None, "codex", HomeEvidence::Explicit("/other".into()));
    assert_eq!(
        check_snapshots("/profile", &[unreadable], true, false),
        Err(PROCESS_PROBE.into())
    );
    let unknown_home = snapshot(Some("/runtime/codex"), "codex", HomeEvidence::Unknown);
    assert_eq!(
        check_snapshots("/profile", &[unknown_home], true, false),
        Err(PROCESS_PROBE.into())
    );
    let anonymous = snapshot(None, "", HomeEvidence::Unknown);
    assert_eq!(
        check_snapshots("/profile", &[anonymous], true, false),
        Err(PROCESS_PROBE.into())
    );
}

#[test]
fn zombies_are_ignored_but_probe_failure_never_means_stopped() {
    let mut zombie = snapshot(None, "codex", HomeEvidence::Unknown);
    zombie.zombie = true;
    assert!(check_snapshots("/profile", &[zombie], true, false).is_ok());
    assert_eq!(
        check_snapshots("/profile", &[], false, false),
        Err(PROCESS_PROBE.into())
    );
}

#[test]
fn windows_paths_match_case_separators_and_extended_path_prefixes() {
    for home in [
        r"C:\Users\Alice\Profile\",
        "c:/users/alice/profile",
        r"\\?\C:\Users\ALICE\Profile",
    ] {
        let process = snapshot(
            Some(r"C:\App\CODEX.EXE"),
            "CODEX.EXE",
            HomeEvidence::Explicit(home.into()),
        );
        assert_eq!(
            check_snapshots("C:/Users/Alice/Profile", &[process], true, true),
            Err(RUNNING.into())
        );
    }
    assert_eq!(
        normalize_path_identity(r"\\?\UNC\server\share\Profile", true),
        "//server/share/profile"
    );
}

#[test]
fn native_environment_resolves_explicit_default_and_relative_profiles() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    let default = user.join(".codex");
    let managed = temp.path().join("managed");
    std::fs::create_dir_all(&default).unwrap();
    std::fs::create_dir_all(&managed).unwrap();
    let os_home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let environment = vec![OsString::from(format!("{os_home}={}", user.display()))];
    let expected = canonical_path_identity(&default, cfg!(windows)).unwrap();
    assert!(
        matches!(process_home(&environment, None, Some(&default), cfg!(windows)), HomeEvidence::Default(home) if home == expected)
    );
    let relative = vec![OsString::from("CODEX_HOME=managed")];
    let expected = canonical_path_identity(&managed, cfg!(windows)).unwrap();
    assert!(
        matches!(process_home(&relative, Some(temp.path()), Some(&default), cfg!(windows)), HomeEvidence::Explicit(home) if home == expected)
    );
    assert!(matches!(
        process_home(&relative, None, Some(&default), cfg!(windows)),
        HomeEvidence::Unknown
    ));
    assert!(matches!(
        process_home(&[], None, Some(&default), cfg!(windows)),
        HomeEvidence::Unknown
    ));
    assert!(matches!(
        process_home(
            &[OsString::from("CODEX_HOME=")],
            None,
            Some(&default),
            cfg!(windows)
        ),
        HomeEvidence::Unknown
    ));
}

#[cfg(unix)]
#[test]
fn canonical_profile_identity_resolves_symlink_aliases_without_modifying_them() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    let alias = temp.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert_eq!(
        canonical_path_identity(&real, false),
        canonical_path_identity(&alias, false)
    );
    assert!(alias.symlink_metadata().unwrap().file_type().is_symlink());
}
