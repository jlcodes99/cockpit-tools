//! Terminal launch plans shared by Codex command previews and execution.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexTerminalLaunchPlan {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) display_command: String,
    pub(super) terminal_name: String,
}

fn escape_applescript(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Whether Windows Terminal (`wt.exe`) is reachable on `PATH`.
///
/// Win11 ships `wt.exe` under `%LOCALAPPDATA%\Microsoft\WindowsApps` (on PATH by default).
/// Cockpit's `Command::spawn` uses `CreateProcess` directly and bypasses the OS default-terminal
/// redirection, so for `default_terminal = "system"` we probe for `wt.exe` and route through
/// Windows Terminal when available.
///
/// Compiled on all targets so shared helpers (and macOS/Linux CI) type-check; non-Windows always
/// returns false.
fn windows_terminal_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        return windows_terminal_available_on_paths(std::env::var_os("PATH"));
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn windows_terminal_available_on_paths(path: Option<std::ffi::OsString>) -> bool {
    #[cfg(target_os = "windows")]
    {
        let candidates = ["wt.exe", "wt"];
        let paths = path.as_deref();
        return std::env::split_paths(paths.unwrap_or_default())
            .any(|dir| candidates.iter().any(|name| dir.join(name).is_file()));
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn format_terminal_display_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| {
            if arg.is_empty() {
                "\"\"".to_string()
            } else if arg
                .chars()
                .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '&' | '|' | ';'))
            {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn build_windows_codex_terminal_launch_plan(
    command: &str,
    terminal: &str,
) -> CodexTerminalLaunchPlan {
    let available = terminal.trim().eq_ignore_ascii_case("system") && windows_terminal_available();
    build_windows_codex_terminal_launch_plan_with_availability(command, terminal, available)
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn build_windows_codex_terminal_launch_plan_with_availability(
    command: &str,
    terminal: &str,
    windows_terminal_is_available: bool,
) -> CodexTerminalLaunchPlan {
    let normalized = terminal.trim().to_ascii_lowercase();
    // `system` honors OS default: prefer Windows Terminal when installed, else PowerShell.
    let use_windows_terminal =
        (normalized == "system" && windows_terminal_is_available) || normalized == "wt";
    let (program, args, terminal_name) = if normalized == "pwsh" {
        (
            "pwsh",
            vec!["-NoExit", "-Command", command],
            "PowerShell Core",
        )
    } else if normalized == "powershell" {
        (
            "powershell",
            vec!["-NoExit", "-Command", command],
            "PowerShell",
        )
    } else if normalized == "cmd" {
        (
            "cmd",
            vec![
                "/C",
                "start",
                "",
                "powershell",
                "-NoExit",
                "-Command",
                command,
            ],
            "Command Prompt",
        )
    } else if use_windows_terminal {
        (
            "wt",
            vec!["powershell", "-NoExit", "-Command", command],
            "Windows Terminal",
        )
    } else {
        (
            "powershell",
            vec!["-NoExit", "-Command", command],
            "PowerShell",
        )
    };
    let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();

    CodexTerminalLaunchPlan {
        program: program.to_string(),
        display_command: format_terminal_display_command(program, &args),
        args,
        terminal_name: terminal_name.to_string(),
    }
}

fn build_macos_codex_terminal_launch_plan(
    command: &str,
    terminal: &str,
) -> Result<CodexTerminalLaunchPlan, String> {
    let normalized = terminal.trim();
    let is_iterm = normalized.to_ascii_lowercase().contains("iterm");
    let is_ghostty = normalized.eq_ignore_ascii_case("Ghostty");
    let is_terminal_app =
        normalized.is_empty() || normalized == "system" || normalized == "Terminal";
    let (terminal_name, script) = if is_iterm {
        (
            "iTerm2",
            format!(
                "tell application \"iTerm\"
                    activate
                    if not (exists window 1) then
                        create window with default profile
                        tell current session of current window
                            write text \"{}\"
                        end tell
                    else
                        tell current window
                            create tab with default profile
                            tell current session
                                write text \"{}\"
                            end tell
                        end tell
                    end if
                end tell",
                escape_applescript(command),
                escape_applescript(command)
            ),
        )
    } else if is_ghostty {
        // Ghostty executes the surface command with `exec -l`; the launch
        // expression needs a shell to interpret `cd`, `&&` and env assignments.
        let shell_command = format!("/bin/bash -lc '{}'", command.replace('\'', "'\\''"));
        (
            "Ghostty",
            format!(
                "tell application \"Ghostty\"
                    activate
                    set cfg to new surface configuration
                    set command of cfg to \"{}\"
                    new window with configuration cfg
                end tell",
                escape_applescript(&shell_command)
            ),
        )
    } else if is_terminal_app {
        (
            "Terminal.app",
            format!(
                "tell application \"Terminal\"
                    activate
                    do script \"{}\"
                end tell",
                escape_applescript(command)
            ),
        )
    } else {
        return Err(format!(
            "当前终端暂不支持直接执行：{}。请改用 Terminal、iTerm2 或 Ghostty。",
            normalized
        ));
    };

    Ok(CodexTerminalLaunchPlan {
        program: "osascript".to_string(),
        args: vec!["-e".to_string(), script],
        display_command: format!("{} → {}", terminal_name, command),
        terminal_name: terminal_name.to_string(),
    })
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn build_linux_codex_terminal_launch_plan(
    command: &str,
    terminal: &str,
) -> CodexTerminalLaunchPlan {
    let normalized = terminal.trim();
    let use_system_terminal = normalized.is_empty() || normalized.eq_ignore_ascii_case("system");
    let program = if use_system_terminal {
        "x-terminal-emulator"
    } else {
        normalized
    };
    let shell_command = format!("{}; exec bash", command);
    let args = if program.eq_ignore_ascii_case("gnome-terminal") {
        vec!["--", "bash", "-lc", shell_command.as_str()]
    } else {
        vec!["-e", "bash", "-lc", shell_command.as_str()]
    };
    let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
    let terminal_name = if use_system_terminal {
        "系统终端"
    } else {
        program
    };

    CodexTerminalLaunchPlan {
        program: program.to_string(),
        display_command: format_terminal_display_command(program, &args),
        args,
        terminal_name: terminal_name.to_string(),
    }
}

pub(super) fn build_codex_terminal_launch_plan(
    command: &str,
    terminal: &str,
) -> Result<CodexTerminalLaunchPlan, String> {
    #[cfg(target_os = "macos")]
    {
        return build_macos_codex_terminal_launch_plan(command, terminal);
    }

    #[cfg(target_os = "windows")]
    {
        return Ok(build_windows_codex_terminal_launch_plan(command, terminal));
    }

    #[cfg(target_os = "linux")]
    {
        return Ok(build_linux_codex_terminal_launch_plan(command, terminal));
    }

    #[allow(unreachable_code)]
    Err("Codex CLI 终端执行仅支持 macOS、Windows 和 Linux".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_system_terminal_falls_back_to_powershell_when_wt_is_unavailable() {
        let plan =
            build_windows_codex_terminal_launch_plan_with_availability("codex", "system", false);

        assert_eq!(plan.program, "powershell");
        assert_eq!(plan.args, ["-NoExit", "-Command", "codex"]);
        assert_eq!(plan.terminal_name, "PowerShell");
    }

    #[test]
    fn windows_system_terminal_uses_windows_terminal_when_available() {
        let plan =
            build_windows_codex_terminal_launch_plan_with_availability("codex", "system", true);

        assert_eq!(plan.program, "wt");
        assert_eq!(plan.args, ["powershell", "-NoExit", "-Command", "codex"]);
        assert_eq!(plan.terminal_name, "Windows Terminal");

        let explicit_powershell =
            build_windows_codex_terminal_launch_plan_with_availability("codex", "powershell", true);
        assert_eq!(explicit_powershell.program, "powershell");
    }

    #[test]
    fn windows_terminal_launch_plans_match_explicit_user_choice() {
        let powershell = build_windows_codex_terminal_launch_plan("codex", "PowerShell");
        assert_eq!(powershell.program, "powershell");
        assert_eq!(powershell.args, ["-NoExit", "-Command", "codex"]);

        let legacy_powershell = build_windows_codex_terminal_launch_plan("codex", "powershell");
        assert_eq!(legacy_powershell.program, "powershell");
        assert_eq!(legacy_powershell.terminal_name, "PowerShell");

        let pwsh = build_windows_codex_terminal_launch_plan("codex", "pwsh");
        assert_eq!(pwsh.program, "pwsh");
        assert_eq!(pwsh.args, ["-NoExit", "-Command", "codex"]);

        let windows_terminal = build_windows_codex_terminal_launch_plan("codex", "wt");
        assert_eq!(windows_terminal.program, "wt");
        assert_eq!(
            windows_terminal.args,
            ["powershell", "-NoExit", "-Command", "codex"]
        );

        let cmd = build_windows_codex_terminal_launch_plan("codex", "cmd");
        assert_eq!(cmd.program, "cmd");
        assert_eq!(
            cmd.args,
            [
                "/C",
                "start",
                "",
                "powershell",
                "-NoExit",
                "-Command",
                "codex",
            ]
        );
    }

    #[test]
    fn linux_system_terminal_launch_plan_uses_terminal_emulator_fallbacks() {
        let plan = build_linux_codex_terminal_launch_plan("codex --version", "system");

        assert_eq!(plan.program, "x-terminal-emulator");
        assert_eq!(
            plan.args,
            ["-e", "bash", "-lc", "codex --version; exec bash"]
        );
        assert_eq!(plan.terminal_name, "系统终端");
    }

    #[test]
    fn linux_gnome_terminal_launch_plan_uses_gnome_argument_shape() {
        let plan = build_linux_codex_terminal_launch_plan("codex", "gnome-terminal");

        assert_eq!(plan.program, "gnome-terminal");
        assert_eq!(plan.args, ["--", "bash", "-lc", "codex; exec bash"]);
        assert_eq!(plan.terminal_name, "gnome-terminal");
    }

    #[test]
    fn macos_ghostty_launch_plan_uses_ghostty_applescript() {
        let plan = build_macos_codex_terminal_launch_plan("codex --version", "Ghostty")
            .expect("Ghostty should have a macOS launch plan");

        assert_eq!(plan.program, "osascript");
        assert_eq!(plan.terminal_name, "Ghostty");
        assert_eq!(plan.display_command, "Ghostty → codex --version");
        assert_eq!(plan.args.len(), 2);
        assert_eq!(plan.args[0], "-e");
        assert!(plan.args[1].contains("tell application \"Ghostty\""));
        assert!(plan.args[1].contains("new surface configuration"));
        assert!(plan.args[1].contains("set command of cfg to \"/bin/bash -lc 'codex --version'\""));
    }

    #[cfg(unix)]
    fn run_ghostty_launch_command(command: &str) -> std::process::Output {
        let plan = build_macos_codex_terminal_launch_plan(command, "Ghostty").unwrap();
        let literal = plan.args[1]
            .lines()
            .find_map(|line| line.trim().strip_prefix("set command of cfg to "))
            .expect("Ghostty surface command");
        let mut chars = literal
            .strip_prefix('"')
            .unwrap()
            .strip_suffix('"')
            .unwrap()
            .chars();
        let mut surface_command = String::new();
        while let Some(ch) = chars.next() {
            surface_command.push(if ch == '\\' {
                match chars.next().expect("complete AppleScript escape") {
                    'n' => '\n',
                    escaped => escaped,
                }
            } else {
                ch
            });
        }
        // Exercise the generated shell expression without opening a terminal.
        std::process::Command::new("/bin/bash")
            .args(["--noprofile", "--norc", "-c"])
            .arg(format!("exec -l {surface_command}"))
            .output()
            .expect("run Ghostty's shell entry point")
    }

    #[cfg(unix)]
    #[test]
    fn macos_ghostty_runs_working_directory_and_environment_setup() {
        let output = run_ghostty_launch_command(
            r#"cd / && GHOSTTY_LAUNCH_TEST='profile with spaces' /bin/sh -c 'printf "%s|%s" "$PWD" "$GHOSTTY_LAUNCH_TEST"'"#,
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "/|profile with spaces"
        );
    }

    #[cfg(unix)]
    #[test]
    fn macos_ghostty_preserves_quotes_and_literal_shell_characters() {
        let output = run_ghostty_launch_command(
            r#"GHOSTTY_LAUNCH_TEST='用户 / O'"'"'Brien "quoted" $HOME `uname` \n' /usr/bin/printenv GHOSTTY_LAUNCH_TEST"#,
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "用户 / O'Brien \"quoted\" $HOME `uname` \\n\n",
        );
    }

    #[cfg(unix)]
    #[test]
    fn macos_ghostty_does_not_launch_after_failed_directory_change() {
        let output = run_ghostty_launch_command("cd /dev/null/invalid && printf unexpected");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_terminal_probe_detects_wt_exe_on_path() {
        let temp = std::env::temp_dir().join(format!("cockpit-wt-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("wt.exe"), b"placeholder").expect("write wt.exe stub");

        // Build a synthetic PATH-like OsString containing the temp dir. split_paths uses ';' as
        // the separator on Windows. This never touches the real process environment, so it is
        // safe to run alongside other tests.
        let synthetic_path =
            std::env::join_paths(std::iter::once(temp.as_path())).expect("join synthetic path");

        let detected = windows_terminal_available_on_paths(Some(synthetic_path));

        let _ = std::fs::remove_dir_all(&temp);
        assert!(detected, "wt.exe on PATH should be detected");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_terminal_probe_returns_false_when_wt_absent() {
        let temp =
            std::env::temp_dir().join(format!("cockpit-wt-probe-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("create temp dir");

        let synthetic_path =
            std::env::join_paths(std::iter::once(temp.as_path())).expect("join synthetic path");

        let detected = windows_terminal_available_on_paths(Some(synthetic_path));

        let _ = std::fs::remove_dir_all(&temp);
        assert!(
            !detected,
            "wt.exe absent from the controlled PATH should not be detected"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_terminal_probe_returns_false_when_path_unset() {
        assert!(
            !windows_terminal_available_on_paths(None),
            "missing PATH should never report Windows Terminal available"
        );
    }
}
