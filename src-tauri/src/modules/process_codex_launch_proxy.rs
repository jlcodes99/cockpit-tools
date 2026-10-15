/// Keep an explicit direct choice distinct from inheriting the client's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexLaunchProxy {
    Inherit,
    Direct,
    Proxy(String),
}

impl CodexLaunchProxy {
    pub fn append_electron_args(&self, args: &mut Vec<String>) {
        match self {
            Self::Inherit => append_global_electron_proxy_args(args),
            Self::Direct => {
                clear_electron_proxy_args(args);
                args.push("--no-proxy-server".into());
            }
            Self::Proxy(url) => append_electron_proxy_args(args, url),
        }
    }

    // Explicit empty overrides also work across LaunchServices and the Windows
    // package helper, whose child can inherit an environment different from ours.
    fn env_overrides(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Inherit => Vec::new(),
            Self::Proxy(url) => account_proxy_env_pairs(url),
            Self::Direct => [
                ("http_proxy", ""),
                ("https_proxy", ""),
                ("all_proxy", ""),
                ("HTTP_PROXY", ""),
                ("HTTPS_PROXY", ""),
                ("ALL_PROXY", ""),
                ("no_proxy", "*"),
                ("NO_PROXY", "*"),
            ]
            .into_iter()
            .map(|(key, value)| (key, value.to_owned()))
            .collect(),
        }
    }
}

#[cfg(test)]
mod codex_direct_launch_tests {
    use super::*;

    fn assert_direct_env(command: &Command) {
        for name in [
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "no_proxy",
            "NO_PROXY",
        ] {
            let expected = if name.eq_ignore_ascii_case("no_proxy") {
                "*"
            } else {
                ""
            };
            assert!(
                command.get_envs().any(|(key, value)| {
                    key.to_string_lossy().eq_ignore_ascii_case(name)
                        && value.and_then(|value| value.to_str()) == Some(expected)
                }),
                "missing direct override: {name}"
            );
        }
    }

    #[test]
    fn direct_launch_replaces_proxy_pac_and_autodetect_without_losing_other_args() {
        let mut args = [
            "--proxy-server",
            "socks5://127.0.0.1:48825",
            "--proxy-pac-url=https://pac.invalid/config",
            "--proxy-pac-url",
            "https://pac.invalid/other",
            "--proxy-bypass-list",
            "*",
            "--proxy-auto-detect",
            "--no-proxy-server",
            "--remote-debugging-port=9222",
            "--user-data-dir=profile",
        ]
        .map(str::to_owned)
        .to_vec();
        CodexLaunchProxy::Direct.append_electron_args(&mut args);
        assert_eq!(
            args,
            [
                "--remote-debugging-port=9222",
                "--user-data-dir=profile",
                "--no-proxy-server"
            ]
        );
        CodexLaunchProxy::Direct.append_electron_args(&mut args);
        assert_eq!(
            args.iter()
                .filter(|arg| *arg == "--no-proxy-server")
                .count(),
            1
        );
    }

    #[test]
    fn direct_launch_overrides_old_environment_in_the_real_command_builder() {
        let mut command = Command::new("never-launched-client");
        command.envs(account_proxy_env_pairs("socks5://127.0.0.1:48825"));
        command.env("CODEX_HOME", "profile");
        apply_effective_proxy_env_to_command(&mut command, &CodexLaunchProxy::Direct);
        assert_direct_env(&command);
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME"
                && value == Some(std::ffi::OsStr::new("profile"))));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchservices_receives_explicit_empty_proxy_overrides_and_full_bypass() {
        let mut command = Command::new("open");
        append_effective_proxy_env_to_open_args(&mut command, &CodexLaunchProxy::Direct);
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for (name, expected) in CodexLaunchProxy::Direct.env_overrides() {
            assert!(args
                .windows(2)
                .any(|pair| pair[0] == "--env" && pair[1] == format!("{name}={expected}")));
        }
        assert!(!args.iter().any(|arg| arg.contains("127.0.0.1:")));
    }

    #[test]
    fn windows_package_payload_keeps_direct_overrides_for_default_and_managed_profiles() {
        use base64::Engine as _;
        for profile in [None, Some("managed-profile")] {
            let mut env: Vec<_> = account_proxy_env_pairs("http://127.0.0.1:7897")
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect();
            env.extend(
                CodexLaunchProxy::Direct
                    .env_overrides()
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value)),
            );
            let mut args = vec!["--proxy-server=http://127.0.0.1:7897".into()];
            CodexLaunchProxy::Direct.append_electron_args(&mut args);
            let request = codex_package_launcher::LaunchRequest {
                family_name: "OpenAI.Codex_fixture".into(),
                app_id: "CodexGui".into(),
                executable: "never-launched-client.exe".into(),
                args,
                env,
                codex_home: profile.map(str::to_owned),
                app_user_data_dir: profile.map(str::to_owned),
                result_path: PathBuf::from("unused-receipt"),
                nonce: "fixture".into(),
                verify_only: false,
            };
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&request).unwrap());
            let decoded: codex_package_launcher::LaunchRequest = serde_json::from_slice(
                &base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap(),
            )
            .unwrap();
            let command = decoded.child_command();
            assert_direct_env(&command);
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [std::ffi::OsStr::new("--no-proxy-server")]
            );
            for name in ["CODEX_HOME", "CODEX_ELECTRON_USER_DATA_PATH"] {
                let value = command.get_envs().find(|(key, _)| *key == name).unwrap().1;
                assert_eq!(value.and_then(|value| value.to_str()), profile);
            }
        }
    }
}
