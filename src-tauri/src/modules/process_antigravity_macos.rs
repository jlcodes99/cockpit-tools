// Scope both discovery passes to the account running Cockpit. The same app
// bundle may be running in more than one macOS login session.
fn collect_antigravity_process_entries_from_ps() -> Vec<(u32, Option<String>)> {
    collect_macos_antigravity_entries(false)
}

fn collect_antigravity_legacy_process_entries_from_ps() -> Vec<(u32, Option<String>)> {
    collect_macos_antigravity_entries(true)
}

fn collect_macos_antigravity_entries(legacy: bool) -> Vec<(u32, Option<String>)> {
    let Ok(output) = Command::new("ps")
        .args(["-axww", "-o", "uid=,pid=,command="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let lines = String::from_utf8_lossy(&output.stdout);
    let owner = unsafe { libc::geteuid() };
    let mut entries = parse_macos_antigravity_entries(&lines, owner, legacy);
    if legacy {
        // ps flattens argv, so --user-data-dir= paths with spaces would be truncated.
        let pids: Vec<_> = entries
            .iter()
            .filter(|(_, dir)| dir.is_some())
            .map(|(pid, _)| sysinfo::Pid::from_u32(*pid))
            .collect();
        if !pids.is_empty() {
            let mut system = sysinfo::System::new();
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&pids),
                true,
                sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
            );
            entries.retain_mut(|(pid, dir)| {
                if dir.is_none() {
                    return true;
                }
                let Some(process) = system.process(sysinfo::Pid::from_u32(*pid)) else {
                    return false;
                };
                let Some(path) = macos_profiles_from_argv(process.cmd()).last() else {
                    return false;
                };
                *dir = Some(path.to_owned());
                true
            });
        }
    }
    entries
}

fn parse_macos_antigravity_entries(
    lines: &str,
    owner: u32,
    legacy: bool,
) -> Vec<(u32, Option<String>)> {
    let marker = if legacy {
        ANTIGRAVITY_LEGACY_APP_CONTENTS_MARKER
    } else {
        ANTIGRAVITY_APP_CONTENTS_MARKER
    };
    lines
        .lines()
        .filter_map(|line| {
            let (pid, command) =
                owned_macos_process_command(line, owner)?.split_once(char::is_whitespace)?;
            let command = command.trim_start();
            let lower = command.to_lowercase();
            let main_marker = if legacy { "antigravity.app/contents/macos/" } else { marker };
            if !lower.contains(main_marker)
                || (legacy && lower.contains(ANTIGRAVITY_APP_CONTENTS_MARKER))
                || lower.contains("antigravity tools.app/contents/")
                || lower.contains("crashpad_handler")
                || is_helper_command_line(&lower)
            {
                return None;
            }
            Some((
                pid.parse::<u32>().ok()?,
                extract_user_data_dir_from_command_line(command),
            ))
        })
        .collect()
}

fn collect_antigravity_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let owner = unsafe { libc::geteuid() };
    let Ok(output) = Command::new("pgrep")
        .args(["-u", &owner.to_string(), "-f", ANTIGRAVITY_APP_PATH])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut pids: Vec<u32> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    pids.sort_unstable();
    pids.dedup();

    let mut entries = Vec::new();
    for pid in pids {
        let Ok(output) = Command::new("ps")
            .args(["-Eww", "-p", &pid.to_string(), "-o", "uid=,command="])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            // Recheck ownership if the PID changed hands after pgrep.
            let Some(command) = owned_macos_process_command(line, owner) else {
                continue;
            };
            if command.to_lowercase().contains(ANTIGRAVITY_APP_EXEC_MARKER) {
                entries.push((pid, extract_user_data_dir_from_command_line(command)));
            }
        }
    }
    entries
}

#[cfg(test)]
mod antigravity_macos_owner_tests {
    use super::*;

    #[test]
    fn antigravity_instance_profile_path_preserves_spaces_and_uses_effective_argument() {
        let argv = [
            "/Applications/Antigravity.app/Contents/MacOS/Antigravity",
            "--user-data-dir=/tmp/earlier profile",
            "--user-data-dir=/tmp/managed profile",
        ].map(std::ffi::OsString::from);
        assert_eq!(
            macos_profiles_from_argv(&argv).last(),
            Some("/tmp/managed profile"),
        );
    }

    #[test]
    fn macos_antigravity_ignores_other_users_for_both_client_bundles() {
        let lines = concat!(
            " 701 10 /Applications/Antigravity.app/Contents/MacOS/Antigravity\n",
            " 705 11 /Applications/Antigravity.app/Contents/MacOS/Antigravity\n",
            " 705 12 /Applications/Antigravity.app/Contents/Resources/bin/language_server --standalone\n",
            " 701 20 /Applications/Antigravity IDE.app/Contents/MacOS/Antigravity IDE\n",
            " 705 21 /Applications/Antigravity IDE.app/Contents/MacOS/Antigravity IDE --user-data-dir=/tmp/local\n",
        );
        assert_eq!(
            parse_macos_antigravity_entries(lines, 705, true),
            [(11, None)]
        );
        assert_eq!(
            parse_macos_antigravity_entries(lines, 705, false),
            [(21, Some("/tmp/local".into()))],
        );
        assert!(owned_macos_process_command("701 reused-pid", 705).is_none());
    }
}
