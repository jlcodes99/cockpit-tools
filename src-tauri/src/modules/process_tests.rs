// Process 模块测试：平台路径、Codex 启动参数和进程清理行为。
// 保持测试模块位于原作用域，super 引用和 cfg 条件不变。
#[cfg(test)]
mod legacy_platform_adapter_cleanup_tests {
    use super::{orphaned_legacy_platform_adapter_pid_from_ps_line, utf8_command_output_snippet};

    #[test]
    fn matches_orphaned_legacy_platform_adapter() {
        let line = " 1359     1 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(line, 99999),
            Some(1359)
        );
    }

    #[test]
    fn ignores_non_orphaned_or_current_processes() {
        let line = " 1359 1805 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(line, 99999),
            None
        );

        let current_line = " 1359 1 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(current_line, 1359),
            None
        );
    }

    #[test]
    fn ignores_current_sidecar_and_official_apps() {
        let sidecar =
            " 64680 1805 /Applications/Cockpit Tools.app/Contents/MacOS/cockpit-cliproxy --parent-pid 1805";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(sidecar, 99999),
            None
        );

        let official_codex =
            " 9300 1 /Applications/Codex.app/Contents/Frameworks/Codex Framework.framework/Helpers/browser_crashpad_handler";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(official_codex, 99999),
            None
        );
    }

    #[test]
    fn command_output_snippet_drops_non_utf8_bytes() {
        assert_eq!(utf8_command_output_snippet(&[0xb4, 0xed, 0xce, 0xf3]), None);
    }

    #[test]
    fn command_output_snippet_keeps_utf8_text() {
        assert_eq!(
            utf8_command_output_snippet("No such process\n".as_bytes()).as_deref(),
            Some("No such process")
        );
    }
}

#[cfg(test)]
mod managed_sidecar_port_cleanup_tests {
    use super::{
        command_line_parent_pid, managed_sidecar_command_matches,
        managed_sidecar_parent_allows_cleanup, normalized_process_argument,
    };
    use std::path::Path;

    #[test]
    fn parses_sidecar_parent_pid_forms() {
        assert_eq!(
            command_line_parent_pid("cockpit-cliproxy --parent-pid 1805"),
            Some(1805)
        );
        assert_eq!(
            command_line_parent_pid("cockpit-cliproxy --parent-pid=64680 --config x"),
            Some(64680)
        );
        assert_eq!(command_line_parent_pid("cockpit-cliproxy"), None);
    }

    #[test]
    fn requires_expected_sidecar_binary_and_config_path() {
        let config =
            Path::new("/Users/demo/.antigravity_cockpit/codex_local_access_sidecar/config.json");
        let command = "/Applications/Cockpit Tools.app/Contents/MacOS/cockpit-cliproxy --config /Users/demo/.antigravity_cockpit/codex_local_access_sidecar/config.json --parent-pid 1805";
        assert!(managed_sidecar_command_matches(
            command,
            "cockpit-cliproxy",
            config
        ));
        assert!(!managed_sidecar_command_matches(
            command,
            "cockpit-cliproxy",
            Path::new("/Users/demo/another/config.json")
        ));
        assert!(!managed_sidecar_command_matches(
            "python server.py --config /Users/demo/.antigravity_cockpit/codex_local_access_sidecar/config.json",
            "cockpit-cliproxy",
            config
        ));
    }

    #[test]
    fn normalizes_windows_process_arguments() {
        assert_eq!(
            normalized_process_argument(
                r#""C:\Program Files\Cockpit Tools\cockpit-cliproxy.exe""#
            ),
            "c:/program files/cockpit tools/cockpit-cliproxy.exe"
        );
    }

    #[test]
    fn cleanup_rejects_live_siblings_and_detached_sidecars() {
        assert!(managed_sidecar_parent_allows_cleanup(
            Some(1805),
            1805,
            true
        ));
        assert!(managed_sidecar_parent_allows_cleanup(
            Some(1804),
            1805,
            false
        ));
        assert!(!managed_sidecar_parent_allows_cleanup(
            Some(1804),
            1805,
            true
        ));
        assert!(!managed_sidecar_parent_allows_cleanup(
            Some(0),
            1805,
            false
        ));
        assert!(!managed_sidecar_parent_allows_cleanup(
            None, 1805, false
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod qoder_macos_process_tests {
    use super::is_qoder_macos_main_process_command_line;

    #[test]
    fn matches_current_and_legacy_qoder_main_processes() {
        assert!(is_qoder_macos_main_process_command_line(
            "/Applications/Qoder IDE.app/Contents/MacOS/Qoder"
        ));
        assert!(is_qoder_macos_main_process_command_line(
            "/Applications/Qoder.app/Contents/MacOS/Qoder"
        ));
        assert!(!is_qoder_macos_main_process_command_line(
            "/Applications/Qoder IDE.app/Contents/Frameworks/Qoder Helper.app/Contents/MacOS/Qoder Helper --type=gpu-process --user-data-dir=/tmp/qoder"
        ));
        assert!(!is_qoder_macos_main_process_command_line(
            "/Applications/Qoder IDE.app/Contents/Frameworks/Electron Framework.framework/Helpers/chrome_crashpad_handler"
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod codex_macos_launch_tests {
    use super::{
        is_codex_direct_app_server_command_line, is_codex_macos_main_process_command_line,
        select_codex_direct_app_server_descendants, CodexProcessTreeEntry,
    };

    #[test]
    fn matches_chatgpt_and_legacy_codex_main_processes() {
        assert!(is_codex_macos_main_process_command_line(
            "/applications/chatgpt.app/contents/macos/chatgpt"
        ));
        assert!(is_codex_macos_main_process_command_line(
            "/applications/codex.app/contents/macos/codex"
        ));
        assert!(!is_codex_macos_main_process_command_line(
            "/applications/chatgpt.app/contents/resources/codex app-server"
        ));
    }

    #[test]
    fn matches_only_bundled_direct_app_server_commands() {
        let executable = "/Applications/ChatGPT.app/Contents/Resources/codex";
        assert!(is_codex_direct_app_server_command_line(
            "/Applications/ChatGPT.app/Contents/Resources/codex app-server --analytics-default-enabled",
            executable,
        ));
        assert!(is_codex_direct_app_server_command_line(
            "\"/Applications/ChatGPT.app/Contents/Resources/codex\" app-server --listen stdio://",
            executable,
        ));
        assert!(!is_codex_direct_app_server_command_line(
            "/Applications/ChatGPT.app/Contents/Resources/codex app-server daemon",
            executable,
        ));
        assert!(!is_codex_direct_app_server_command_line(
            "/opt/homebrew/bin/codex app-server --analytics-default-enabled",
            executable,
        ));
        assert!(!is_codex_direct_app_server_command_line(
            "/Applications/ChatGPT.app/Contents/Resources/codex exec hello",
            executable,
        ));
    }

    #[test]
    fn selects_direct_app_server_only_from_target_main_process_tree() {
        let executable = "/Applications/ChatGPT.app/Contents/Resources/codex";
        let entries = vec![
            CodexProcessTreeEntry {
                pid: 100,
                parent_pid: 1,
                command_line: "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".to_string(),
            },
            CodexProcessTreeEntry {
                pid: 110,
                parent_pid: 100,
                command_line: "helper".to_string(),
            },
            CodexProcessTreeEntry {
                pid: 120,
                parent_pid: 110,
                command_line: format!("{} app-server --analytics-default-enabled", executable),
            },
            CodexProcessTreeEntry {
                pid: 200,
                parent_pid: 1,
                command_line: "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".to_string(),
            },
            CodexProcessTreeEntry {
                pid: 220,
                parent_pid: 200,
                command_line: format!("{} app-server --analytics-default-enabled", executable),
            },
        ];

        assert_eq!(
            select_codex_direct_app_server_descendants(&entries, &[100], executable),
            vec![120]
        );
    }
}

#[cfg(test)]
mod codex_launch_args_tests {
    use super::{
        build_codex_app_launch_args, codex_managed_store_launch_unsafe_error,
        CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX,
    };

    #[test]
    fn keeps_user_launch_args_without_adding_remote_debugging() {
        assert!(build_codex_app_launch_args(&[]).is_empty());
        assert_eq!(
            build_codex_app_launch_args(&[
                " --remote-debugging-port=9333 ".to_string(),
                "".to_string(),
                " --disable-gpu ".to_string(),
            ]),
            vec![
                "--remote-debugging-port=9333".to_string(),
                "--disable-gpu".to_string(),
            ]
        );
    }

    #[test]
    fn managed_store_launch_error_is_machine_readable_and_keeps_causes() {
        let error = codex_managed_store_launch_unsafe_error(
            "denied",
            "fallback failed",
            "launch_path=C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0\\app\\ChatGPT.exe; launch_path_exists=true",
        );
        assert!(error.starts_with(CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX));
        assert!(error.contains("direct_error=denied"));
        assert!(error.contains("powershell_error=fallback failed"));
        // 诊断信息要跟着错误一起回传，用户复制错误即可让支持侧看到实际路径。
        assert!(error.contains("launch_path="));
        assert!(error.contains("launch_path_exists=true"));
    }

    #[test]
    fn managed_store_launch_error_keeps_package_identity_cause_readable() {
        // 三层兜底（直启 / Start-Process / 包身份）都失败时，包身份原因也要留在
        // 错误里，否则用户复制出来的诊断看不到真正卡在哪一层。
        let error = codex_managed_store_launch_unsafe_error(
            "拒绝访问。 (os error 5)",
            "PowerShell 启动 Codex 失败",
            "package_identity_error=包身份启动失败: status=exit code: 1; launch_path_exists=true",
        );
        assert!(error.contains("package_identity_error="));
        assert!(error.contains("launch_path_exists=true"));
    }

    #[test]
    fn managed_store_launch_error_omits_empty_diagnostics() {
        let error = codex_managed_store_launch_unsafe_error("denied", "fallback failed", "  ");
        assert_eq!(
            error,
            "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:direct_error=denied; powershell_error=fallback failed"
        );
    }
}

#[cfg(test)]
mod codex_linux_layout_tests {
    use super::{
        linux_codex_discovery_paths, select_codex_direct_app_server_descendants,
        CodexProcessTreeEntry,
    };
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    #[test]
    fn discovery_includes_official_linux_package_and_path_launchers() {
        let home = Path::new("/home/demo");
        let path = OsString::from("/custom/bin:/usr/bin");
        let candidates = linux_codex_discovery_paths(Some(home), Some(path.as_os_str()));
        assert!(candidates.contains(&PathBuf::from("/usr/bin/chatgpt")));
        assert!(candidates.contains(&PathBuf::from("/usr/lib/chatgpt/ChatGPT")));
        assert!(candidates.contains(&PathBuf::from("/home/demo/.local/bin/chatgpt")));
        assert!(candidates.contains(&PathBuf::from("/custom/bin/chatgpt")));
    }

    #[test]
    fn selects_linux_bundled_app_server_from_target_desktop_tree() {
        let executable = "/usr/lib/chatgpt/resources/codex";
        let entries = vec![
            CodexProcessTreeEntry {
                pid: 100,
                parent_pid: 1,
                command_line: "/usr/lib/chatgpt/ChatGPT".to_string(),
            },
            CodexProcessTreeEntry {
                pid: 120,
                parent_pid: 100,
                command_line: format!("{} app-server --analytics-default-enabled", executable),
            },
            CodexProcessTreeEntry {
                pid: 220,
                parent_pid: 200,
                command_line: format!("{} app-server --analytics-default-enabled", executable),
            },
        ];
        assert_eq!(
            select_codex_direct_app_server_descendants(&entries, &[100], executable),
            vec![120]
        );
    }
}

#[cfg(test)]
mod codex_path_migration_tests {
    use super::{
        is_codex_embedded_backend_executable, score_windows_candidate,
        should_migrate_legacy_codex_launch_path, should_probe_legacy_codex_launch_path,
    };
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn migrates_official_windows_store_codex_path_when_chatgpt_exists() {
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_2.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
            ),
        ));
    }

    #[test]
    fn keeps_legacy_path_when_chatgpt_is_not_detected() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
        ));
    }

    #[test]
    fn does_not_replace_custom_codex_executable() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new(r"D:\Tools\Codex.exe"),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_2.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
            ),
        ));
    }

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

    #[test]
    fn only_probes_official_legacy_codex_paths_for_migration() {
        assert!(should_probe_legacy_codex_launch_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
        )));
        assert!(should_probe_legacy_codex_launch_path(Path::new(
            "/Applications/Codex.app"
        )));
        assert!(!should_probe_legacy_codex_launch_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_2.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
        )));
        assert!(!should_probe_legacy_codex_launch_path(Path::new(
            r"D:\Tools\Codex.exe"
        )));
        assert!(!should_probe_legacy_codex_launch_path(Path::new(
            "/Users/test/Applications/Codex.app"
        )));
    }

    #[test]
    fn scan_rejects_codex_keyword_helper_executables() {
        let exe_names = HashSet::from(["chatgpt.exe".to_string()]);
        let keywords = vec!["chatgpt".to_string()];

        assert!(score_windows_candidate(
            Path::new("C:/Tools/CodexHelper.exe"),
            &exe_names,
            &keywords,
        )
        .is_none());
        assert!(
            score_windows_candidate(Path::new("C:/Tools/ChatGPT.exe"), &exe_names, &keywords,)
                .is_some()
        );
    }

    #[test]
    fn scan_excludes_embedded_resources_backend() {
        assert!(is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
        )));
        assert!(!is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        )));
        assert!(!is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__2p2nqsd0c76g0\app\Codex.exe"
        )));
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{
        running_app_candidate_matches, windows_app_launch_signature,
        windows_trae_candidate_matches_platform,
    };
    use crate::modules::trae_account::TraePlatformKind;
    use std::path::Path;

    #[test]
    fn windows_launch_signatures_cover_provider_apps() {
        for app in [
            "antigravity_ide",
            "cursor",
            "zed",
            "codebuddy",
            "codebuddy_cn",
            "qoder",
            "trae",
            "trae_solo",
            "trae_cn",
            "trae_solo_cn",
            "workbuddy",
            "windsurf",
            "kiro",
            "codex",
            "claude",
            "vscode",
        ] {
            let signature =
                windows_app_launch_signature(app).unwrap_or_else(|| panic!("missing {app}"));
            assert!(
                !signature.exe_names.is_empty(),
                "{app} must define executable names"
            );
            assert!(
                !signature.common_paths.is_empty(),
                "{app} must define common install paths"
            );
            assert!(
                !signature.display_keywords.is_empty(),
                "{app} must define display keywords"
            );
        }
    }

    #[test]
    fn antigravity_ide_signature_uses_ide_executable_only() {
        let signature = windows_app_launch_signature("antigravity_ide")
            .expect("antigravity ide signature must exist");
        assert!(signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Antigravity IDE.exe")));
        assert!(!signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Antigravity.exe")));
    }

    #[test]
    fn codex_signature_accepts_only_chatgpt_gui_executable() {
        let signature = windows_app_launch_signature("codex").expect("codex signature must exist");
        assert!(signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("ChatGPT.exe")));
        assert!(!signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Codex.exe")));
    }

    #[test]
    fn running_codex_match_accepts_main_executable_and_rejects_embedded_backend() {
        let signature = windows_app_launch_signature("codex").expect("codex signature must exist");
        assert!(running_app_candidate_matches(
            "codex",
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
            ),
            signature,
        ));
        assert!(!running_app_candidate_matches(
            "codex",
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
            ),
            signature,
        ));
        assert!(!running_app_candidate_matches(
            "codex",
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\Codex.exe"
            ),
            signature,
        ));
    }

    #[test]
    fn running_claude_match_requires_claude_executable() {
        let signature =
            windows_app_launch_signature("claude").expect("claude signature must exist");
        assert!(running_app_candidate_matches(
            "claude",
            Path::new(r"C:\Program Files\WindowsApps\Claude_1.0.0\app\Claude.exe"),
            signature,
        ));
        assert!(!running_app_candidate_matches(
            "claude",
            Path::new(r"C:\Tools\Electron.exe"),
            signature,
        ));
    }

    #[test]
    fn trae_windows_scan_candidates_match_exact_platform_dirs() {
        let trae = Path::new(r"D:\Users\李杰\AppData\Local\Programs\Trae\Trae.exe");
        let trae_cn = Path::new(r"D:\Users\李杰\AppData\Local\Programs\Trae CN\Trae CN.exe");
        let solo_cn =
            Path::new(r"D:\Users\李杰\AppData\Local\Programs\TRAE SOLO CN\TRAE SOLO CN.exe");

        assert!(windows_trae_candidate_matches_platform(
            trae,
            TraePlatformKind::Trae
        ));
        assert!(!windows_trae_candidate_matches_platform(
            trae,
            TraePlatformKind::TraeCn
        ));
        assert!(windows_trae_candidate_matches_platform(
            trae_cn,
            TraePlatformKind::TraeCn
        ));
        assert!(!windows_trae_candidate_matches_platform(
            trae_cn,
            TraePlatformKind::Trae
        ));
        assert!(windows_trae_candidate_matches_platform(
            solo_cn,
            TraePlatformKind::TraeSoloCn
        ));
        assert!(!windows_trae_candidate_matches_platform(
            solo_cn,
            TraePlatformKind::TraeSolo
        ));
    }
}

#[cfg(test)]
mod codex_windows_default_instance_tests {
    use super::{
        filter_codex_windows_default_process_entries, is_codex_windows_default_process_dir,
        next_codex_default_start_candidate,
    };
    use std::collections::HashSet;

    const DEFAULT_APP_DIR: &str = r"C:\Users\me\AppData\Roaming\Codex\web\Codex";
    const MANAGED_APP_DIR: &str =
        r"C:\Users\me\AppData\Roaming\.antigravity_cockpit\instances\codex-app-data\5184";

    #[test]
    fn classifies_default_processes_without_mixing_managed_instances() {
        let default_dirs = HashSet::from([DEFAULT_APP_DIR.to_ascii_lowercase()]);
        assert!(is_codex_windows_default_process_dir(None, &default_dirs));
        assert!(is_codex_windows_default_process_dir(
            Some(DEFAULT_APP_DIR),
            &default_dirs
        ));
        assert!(!is_codex_windows_default_process_dir(
            Some(MANAGED_APP_DIR),
            &default_dirs
        ));

        let entries = vec![(4584, None), (5184, Some(MANAGED_APP_DIR.to_string()))];
        let filtered = filter_codex_windows_default_process_entries(&entries, &default_dirs);
        assert_eq!(filtered, vec![(4584, None)]);
    }

    #[test]
    fn rejects_start_result_while_old_default_pid_is_still_present() {
        let before = HashSet::from([4584]);
        assert_eq!(
            next_codex_default_start_candidate(&[4584], &before, None, 0),
            (None, 0)
        );
        assert_eq!(
            next_codex_default_start_candidate(&[4584, 18500], &before, None, 0),
            (None, 0)
        );
        assert_eq!(
            next_codex_default_start_candidate(&[18500], &before, None, 0),
            (Some(18500), 1)
        );
    }

    #[test]
    fn requires_the_new_default_pid_to_remain_stable() {
        let before = HashSet::from([4584]);
        let (pid, streak) = next_codex_default_start_candidate(&[18500], &before, Some(18500), 1);
        assert_eq!((pid, streak), (Some(18500), 2));
        let (pid, streak) = next_codex_default_start_candidate(&[18500], &before, Some(18500), 2);
        assert_eq!((pid, streak), (Some(18500), 3));
        let (pid, streak) = next_codex_default_start_candidate(&[], &before, Some(18500), 3);
        assert_eq!((pid, streak), (None, 0));
        let (pid, streak) = next_codex_default_start_candidate(&[18500], &before, None, 0);
        assert_eq!((pid, streak), (Some(18500), 1));
    }
}

#[cfg(test)]
mod windows_codex_exe_match_tests {
    use super::{is_matching_codex_windows_exe, windowsapps_package_family};

    const OLD_STORE: &str = r"c:\program files\windowsapps\openai.codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\chatgpt.exe";
    const NEW_STORE: &str = r"c:\program files\windowsapps\openai.codex_26.908.4834.0_x64__2p2nqsd0c76g0\app\chatgpt.exe";
    const OTHER_FAMILY: &str = r"c:\program files\windowsapps\openai.chatgpt_1.0.0.0_x64__2p2nqsd0c76g0\app\chatgpt.exe";

    #[test]
    fn windowsapps_family_is_parsed_from_package_directory() {
        assert_eq!(
            windowsapps_package_family(OLD_STORE).as_deref(),
            Some("openai.codex")
        );
        assert_eq!(
            windowsapps_package_family(r"C:/Program Files/WindowsApps/OpenAI.Codex_1.0_x64__abc/app/Codex.exe")
                .as_deref(),
            Some("openai.codex")
        );
        assert!(windowsapps_package_family(r"c:\users\me\codex\codex.exe").is_none());
    }

    #[test]
    fn store_package_matches_across_version_directories() {
        // 商店更新后配置路径与运行中进程只差版本目录，必须仍然认作同一实例，
        // 否则会误判「客户端没启动 / 已关闭」，官方登录更会直接判定失败。
        assert!(is_matching_codex_windows_exe(NEW_STORE, OLD_STORE));
        assert!(is_matching_codex_windows_exe(OLD_STORE, NEW_STORE));
        assert!(is_matching_codex_windows_exe(NEW_STORE, NEW_STORE));
    }

    #[test]
    fn package_family_and_file_name_still_have_to_match() {
        // 不同包族：ChatGPT 桌面端与 Codex 是两个应用，不能混为一谈。
        assert!(!is_matching_codex_windows_exe(OTHER_FAMILY, OLD_STORE));
        // 同包族但不是同一个可执行文件（内置 codex.exe 不应当成主进程）。
        assert!(!is_matching_codex_windows_exe(
            r"c:\program files\windowsapps\openai.codex_26.908.4834.0_x64__2p2nqsd0c76g0\app\resources\codex.exe",
            NEW_STORE
        ));
    }

    #[test]
    fn non_store_paths_keep_strict_comparison() {
        let local = r"c:\users\me\appdata\local\programs\codex\codex.exe";
        let other = r"d:\elsewhere\codex\codex.exe";
        assert!(is_matching_codex_windows_exe(local, local));
        assert!(!is_matching_codex_windows_exe(other, local));
        // 一侧是商店路径、另一侧不是时不得放宽。
        assert!(!is_matching_codex_windows_exe(local, NEW_STORE));
        assert!(!is_matching_codex_windows_exe(NEW_STORE, local));
    }

    #[test]
    fn empty_paths_never_match() {
        assert!(!is_matching_codex_windows_exe("", NEW_STORE));
        assert!(!is_matching_codex_windows_exe(NEW_STORE, ""));
        assert!(!is_matching_codex_windows_exe("", ""));
    }
}

#[cfg(test)]
mod codex_package_identity_launch_tests {
    use super::{
        quote_windows_command_argument, windowsapps_install_location_from_launch_path,
        windowsapps_package_family,
    };
    use std::path::Path;

    #[test]
    fn quotes_arguments_that_contain_spaces() {
        // `--user-data-dir` 指向的实例目录常含空格，必须整体加引号。
        assert_eq!(
            quote_windows_command_argument(
                r"--user-data-dir=C:\Users\some user\.antigravity_cockpit\ud"
            ),
            r#""--user-data-dir=C:\Users\some user\.antigravity_cockpit\ud""#
        );
    }

    #[test]
    fn leaves_simple_arguments_untouched() {
        assert_eq!(
            quote_windows_command_argument("--remote-debugging-port=9333"),
            "--remote-debugging-port=9333"
        );
        assert_eq!(quote_windows_command_argument(""), "\"\"");
    }

    #[test]
    fn escapes_embedded_quotes_and_backslash_runs() {
        // CreateProcess 规则：引号前的反斜杠加倍，引号自身再转义一个。
        assert_eq!(quote_windows_command_argument(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_windows_command_argument(r"a\b"), r"a\b");
        // 以反斜杠结尾时收尾引号必须被保护，否则会把参数引号转义掉。
        assert_eq!(
            quote_windows_command_argument(r"C:\dir with space\"),
            r#""C:\dir with space\\""#
        );
    }

    #[test]
    fn derives_install_location_from_store_launch_path() {
        assert_eq!(
            windowsapps_install_location_from_launch_path(Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.908.9136.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
            ))
            .as_deref(),
            Some(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.908.9136.0_x64__2p2nqsd0c76g0")
        );
        // 非商店路径不能落到包身份分支上。
        assert!(windowsapps_install_location_from_launch_path(Path::new(
            r"C:\Users\me\AppData\Local\Programs\Codex\Codex.exe"
        ))
        .is_none());
    }

    #[test]
    fn install_location_round_trips_to_the_registered_package_family() {
        // 外层脚本按 InstallLocation 反查包，这里确认解析结果仍属同一包族。
        let install_location = windowsapps_install_location_from_launch_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.908.9136.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        ))
        .expect("store path should resolve");
        let fake_exe = format!(r"{}\app\ChatGPT.exe", install_location);
        assert_eq!(
            windowsapps_package_family(&fake_exe).as_deref(),
            Some("openai.codex")
        );
    }
}

#[cfg(test)]
mod windows_launch_fallback_tests {
    use super::{is_windowsapps_launch_path, windows_powershell_executable_candidates};
    use std::path::{Path, PathBuf};

    #[test]
    fn powershell_candidates_prefer_path_then_system32_then_pwsh() {
        let candidates = windows_powershell_executable_candidates(Some(r"D:\Windows"));
        assert_eq!(candidates[0], PathBuf::from("powershell.exe"));
        assert_eq!(
            candidates[1],
            PathBuf::from(r"D:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
        assert_eq!(candidates[2], PathBuf::from("pwsh.exe"));
    }

    #[test]
    fn powershell_candidates_keep_windows_default_root_fallback() {
        let candidates = windows_powershell_executable_candidates(None);
        assert_eq!(
            candidates[1],
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
    }

    #[test]
    fn detects_windowsapps_launch_paths() {
        assert!(is_windowsapps_launch_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
        )));
        assert!(is_windowsapps_launch_path(Path::new(
            r"C:/Program Files/WindowsApps/OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe/app/Codex.exe"
        )));
        assert!(!is_windowsapps_launch_path(Path::new(
            r"C:\Users\me\AppData\Local\Programs\Codex\Codex.exe"
        )));
    }
}

#[cfg(test)]
mod codex_store_gui_exe_tests {
    use super::{appx_manifest_gui_executable_from_text, is_chatgpt_store_gui_exe};
    use std::path::Path;

    /// 真实清单的精简版：`App` 是桌面端入口，`CodexCoreCommandRunner` 是另一个应用。
    const REAL_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Applications>
    <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="ChatGPT" />
    </Application>
    <Application Id="CodexCoreCommandRunner" Executable="app/resources/codex-command-runner.exe" EntryPoint="Windows.FullTrustApplication" />
  </Applications>
</Package>"#;

    #[test]
    fn picks_the_app_application_executable() {
        assert_eq!(
            appx_manifest_gui_executable_from_text(REAL_MANIFEST).as_deref(),
            Some("app/ChatGPT.exe")
        );
    }

    #[test]
    fn ignores_other_applications_and_missing_executable() {
        // Id 顺序对调也要命中 App，而不是第一个 Application 节点。
        let other_first = r#"<Application Id="CodexCoreCommandRunner" Executable="app/resources/codex-command-runner.exe" /><Application Id="App" Executable="app/ChatGPT.exe" />"#;
        assert_eq!(
            appx_manifest_gui_executable_from_text(other_first).as_deref(),
            Some("app/ChatGPT.exe")
        );
        assert_eq!(
            appx_manifest_gui_executable_from_text(r#"<Application Id="App" />"#),
            None
        );
        assert_eq!(appx_manifest_gui_executable_from_text("<Package />"), None);
        assert_eq!(appx_manifest_gui_executable_from_text(""), None);
    }

    #[test]
    fn only_accepts_the_chatgpt_gui_binary() {
        assert!(is_chatgpt_store_gui_exe(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
        )));
        assert!(is_chatgpt_store_gui_exe(Path::new(
            r"E:\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\chatgpt.EXE"
        )));
        // 同包目录里的另外两个 exe 都不是桌面端入口。
        assert!(!is_chatgpt_store_gui_exe(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
        )));
        assert!(!is_chatgpt_store_gui_exe(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\resources\codex.exe"
        )));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolves_gui_exe_from_manifest_and_falls_back_to_convention() {
        use super::codex_store_gui_exe_in;

        let root = std::env::temp_dir().join(format!(
            "cockpit-tools-codex-store-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);

        // 1) 清单声明 app/ChatGPT.exe 且文件存在 → 采信清单
        let with_manifest = root.join("with-manifest");
        std::fs::create_dir_all(with_manifest.join("app")).expect("create app dir");
        std::fs::write(with_manifest.join("AppxManifest.xml"), REAL_MANIFEST)
            .expect("write manifest");
        std::fs::write(with_manifest.join("app").join("ChatGPT.exe"), b"stub")
            .expect("write exe");
        assert_eq!(
            codex_store_gui_exe_in(&with_manifest),
            Some(with_manifest.join("app").join("ChatGPT.exe"))
        );

        // 2) 读不到清单 → 退回约定路径 app\ChatGPT.exe
        let no_manifest = root.join("no-manifest");
        std::fs::create_dir_all(no_manifest.join("app")).expect("create app dir");
        std::fs::write(no_manifest.join("app").join("ChatGPT.exe"), b"stub").expect("write exe");
        assert_eq!(
            codex_store_gui_exe_in(&no_manifest),
            Some(no_manifest.join("app").join("ChatGPT.exe"))
        );

        // 3) 只有 Codex.exe → 不再猜其它 exe
        let only_codex = root.join("only-codex");
        std::fs::create_dir_all(only_codex.join("app")).expect("create app dir");
        std::fs::write(only_codex.join("app").join("Codex.exe"), b"stub").expect("write exe");
        assert_eq!(codex_store_gui_exe_in(&only_codex), None);

        // 4) 清单指向非 ChatGPT.exe → 不接受
        let manifest_other = root.join("manifest-other");
        std::fs::create_dir_all(manifest_other.join("app")).expect("create app dir");
        std::fs::write(
            manifest_other.join("AppxManifest.xml"),
            br#"<Application Id="App" Executable="app/Codex.exe" />"#,
        )
        .expect("write manifest");
        std::fs::write(manifest_other.join("app").join("Codex.exe"), b"stub").expect("write exe");
        assert_eq!(codex_store_gui_exe_in(&manifest_other), None);

        let _ = std::fs::remove_dir_all(&root);
    }
}
