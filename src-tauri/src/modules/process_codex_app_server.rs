// Bundled app-server ownership and command parsing, shared by shutdown and diagnostics.
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
    // The current ChatGPT bundle nests its CLI in a signed helper app. Keep
    // matching the exact installation, including legacy Codex bundles; never
    // accept an arbitrary executable merely because it is named `codex`.
    let current = expected
        .strip_suffix("/Contents/Resources/codex")
        .map(|root| {
            format!("{root}/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex")
        });
    let args = std::iter::once(expected)
        .chain(current.as_deref())
        .find_map(|executable| {
            let quoted = format!("\"{executable}\"");
            let remainder = command_line
                .strip_prefix(executable)
                .or_else(|| command_line.strip_prefix(&quoted))?;
            // Do not accept a path suffix (codex-other, codex-cli/...) as arguments.
            remainder
                .starts_with(char::is_whitespace)
                .then(|| remainder.trim_start())
        });
    let Some(args) = args else {
        return false;
    };
    let args = split_command_tokens(args);
    let mut index = 0;
    while let Some(arg) = args.get(index).map(String::as_str) {
        match arg {
            "app-server" => {
                return !args[index + 1..].iter().any(|value| value == "daemon");
            }
            "-c" | "--config" | "--enable" | "--disable" => {
                if args
                    .get(index + 1)
                    .is_none_or(|value| value.is_empty() || value.starts_with('-'))
                {
                    return false;
                }
                index += 2;
            }
            _ if arg
                .strip_prefix("--config=")
                .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("--enable=")
                    .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("--disable=")
                    .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("-c")
                    .is_some_and(|value| !value.is_empty()) =>
            {
                index += 1;
            }
            // In particular, `exec ... app-server` is not a server process.
            _ => return false,
        }
    }
    false
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

#[cfg(test)]
mod codex_app_server_ownership_tests {
    use super::*;

    const LEGACY: &str = "/Applications/ChatGPT.app/Contents/Resources/codex";
    const CURRENT: &str =
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";

    #[test]
    fn recognizes_current_bundle_with_global_cli_options() {
        for options in [
            "-c features.code_mode_host=true",
            "--config features.code_mode_host=true --enable feature --disable other",
            "--config=features.code_mode_host=true --enable=feature",
            "-cfeatures.code_mode_host=true",
            "-c 'instructions=keep app-server as text'",
            "",
        ] {
            assert!(
                is_codex_direct_app_server_command_line(
                    &format!("{CURRENT} {options} app-server --analytics-default-enabled"),
                    LEGACY,
                ),
                "options: {options}"
            );
        }
    }

    #[test]
    fn supports_quoted_and_unquoted_installation_paths_with_spaces() {
        let legacy = "/Volumes/Apps Disk/ChatGPT.app/Contents/Resources/codex";
        let current = CURRENT.replace("/Applications", "/Volumes/Apps Disk");
        for executable in [current.clone(), format!("\"{current}\"")] {
            assert!(is_codex_direct_app_server_command_line(
                &format!("{executable} -c features.code_mode_host=true app-server"),
                legacy,
            ));
        }
    }

    #[test]
    fn rejects_other_installations_daemons_and_app_server_in_argument_values() {
        for command in [
            format!("{CURRENT}-other app-server"),
            format!("{CURRENT} exec app-server"),
            format!("{CURRENT} -c app-server"),
            format!("{CURRENT} --config= app-server"),
            format!("{CURRENT} --unknown app-server"),
            format!("{CURRENT} -c features.x=true app-server daemon"),
            format!("{CURRENT} app-server-other"),
            "/tmp/codex app-server".into(),
            CURRENT.replace("/Applications", "/tmp") + " app-server",
        ] {
            assert!(
                !is_codex_direct_app_server_command_line(&command, LEGACY),
                "{command}"
            );
        }
    }

    #[test]
    fn preserves_other_instances_orphans_and_cyclic_process_trees() {
        let entry = |pid, parent_pid, command: &str| CodexProcessTreeEntry {
            pid,
            parent_pid,
            command_line: command.to_owned(),
        };
        let command = format!("{CURRENT} -c features.code_mode_host=true app-server");
        let entries = vec![
            entry(100, 1, "desktop"),
            entry(110, 100, "helper"),
            entry(120, 110, &command),
            entry(200, 1, "another desktop"),
            entry(220, 200, &command),
            entry(300, 1, &command),
            entry(400, 410, &command),
            entry(410, 400, "cycle"),
            entry(500, 100, "/tmp/codex app-server"),
        ];
        assert_eq!(
            select_codex_direct_app_server_descendants(&entries, &[100], LEGACY),
            vec![120]
        );
        assert!(select_codex_direct_app_server_descendants(&entries, &[], LEGACY).is_empty());
    }
}
