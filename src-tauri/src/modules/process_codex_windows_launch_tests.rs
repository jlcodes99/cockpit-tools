use super::*;

#[test]
fn default_exe_clears_profile_environment_and_stale_user_data_arguments() {
    let mut command = Command::new("never-executed-chatgpt.exe");
    command.env("CODEX_HOME", "old-home");
    command.env("CODEX_ELECTRON_USER_DATA_PATH", "old-data");
    let args = vec![
        "--user-data-dir=old".into(),
        "--user-data-dir".into(),
        "old split".into(),
        "--remote-debugging-port=9222".into(),
    ];
    prepare_codex_windows_default_command(&mut command, &args);
    for name in ["CODEX_HOME", "CODEX_ELECTRON_USER_DATA_PATH"] {
        assert!(command
            .get_envs()
            .any(|(key, value)| key == name && value.is_none()));
    }
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("--remote-debugging-port=9222")]
    );
}

fn package() -> CodexRegisteredLaunch {
    CodexRegisteredLaunch {
        family_name: "OpenAI.Codex_2p2nqsd0c76g0".into(),
        app_id: "CodexGui".into(),
        executable:
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.924_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
                .into(),
    }
}

#[test]
fn default_registration_probe_requires_exact_app_identity_and_gui() {
    for invalid in ["", "pkg", "!App", "pkg!", "pkg!App!Other"] {
        assert!(
            build_codex_default_registered_launch_probe(invalid).is_err(),
            "{invalid}"
        );
    }
    let script = build_codex_default_registered_launch_probe(" OpenAI.Codex_pub!O'Brien ").unwrap();
    assert!(script.contains("$family = 'OpenAI.Codex_pub'"));
    assert!(script.contains("$appId = 'O''Brien'"));
    assert!(script.contains("$application.Id -ne $appId"));
    assert!(script.contains("-ieq 'ChatGPT.exe'"));
    assert!(script.contains("[IO.Path]::IsPathRooted($relative)"));
    assert!(script.contains("$exe.StartsWith($root + '\\'"));
    assert!(script.contains("$matches.Count -ne 1"));
}

#[cfg(target_os = "windows")]
#[test]
fn default_registration_resolves_selected_gui_and_rejects_runner() {
    // Real PowerShell parsing with fake registration; no desktop is launched.
    let setup = r#"
function Get-AppxPackage {
  [PSCustomObject]@{ Version=[version]'26.924'; PackageFamilyName='OpenAI.Codex_pub'; InstallLocation='C:\Program Files\WindowsApps\OpenAI.Codex_26.924_x64__pub' }
}
function Get-AppxPackageManifest {
  [PSCustomObject]@{ Package=@{ Applications=@{ Application=@(
    @{Id='Runner';Executable='app\resources\codex.exe'},
    @{Id='Gui';Executable='app\ChatGPT.exe'}
  ) } } }
}
function Test-Path { return $true }
"#;
    let probe = build_codex_default_registered_launch_probe("OpenAI.Codex_pub!Gui").unwrap();
    let output = codex_launch_powershell_output(&format!("{setup}\n{probe}")).unwrap();
    let registered = parse_codex_registered_launch(&output).unwrap().unwrap();
    assert_eq!(registered.app_id, "Gui");
    assert!(registered.executable.ends_with(r"app\ChatGPT.exe"));
    for id in ["Runner", "Missing"] {
        let probe =
            build_codex_default_registered_launch_probe(&format!("OpenAI.Codex_pub!{id}")).unwrap();
        assert!(codex_launch_powershell_output(&format!("{setup}\n{probe}")).is_err());
    }
}

#[test]
fn registration_response_requires_gui_identity() {
    assert!(parse_codex_registered_launch("null").unwrap().is_none());
    let registered = parse_codex_registered_launch(r#"{"family_name":"OpenAI.Codex_publisher","app_id":"Gui","executable":"D:\\WindowsApps\\new\\app\\ChatGPT.exe"}"#).unwrap().unwrap();
    assert_eq!(registered.app_id, "Gui");
    for invalid in [
        "",
        "warning: null",
        r#"{"family_name":"","app_id":"App","executable":"ChatGPT.exe"}"#,
        r#"{"family_name":"pkg","app_id":"App","executable":"resources/codex.exe"}"#,
    ] {
        assert!(parse_codex_registered_launch(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn missing_or_other_profile_and_old_pids_cannot_confirm_launch() {
    let target = normalize_path_for_compare(r"C:\managed\profile");
    let entries = vec![
        (10, None),
        (11, Some(r"C:\default".into())),
        (12, Some(r"C:\managed\profile".into())),
    ];
    assert_eq!(
        codex_managed_launch_candidate(&entries, &target, &HashSet::from([12])),
        None
    );
    assert_eq!(
        codex_managed_launch_candidate(&entries, &target, &HashSet::new()),
        Some(12)
    );
    assert_eq!(
        codex_managed_launch_candidate(&[], &target, &HashSet::new()),
        None
    );
}

#[cfg(target_os = "windows")]
#[test]
fn registration_probe_refreshes_old_package_and_selects_gui_not_first_application() {
    // Execute the generated PowerShell with fake Appx registration. No app is launched.
    let setup = r#"
function Get-AppxPackage {
  [PSCustomObject]@{ Name='OpenAI.Codex'; Version=[version]'26.924'; PackageFamilyName='OpenAI.Codex_pub'; InstallLocation='C:\Program Files\WindowsApps\OpenAI.Codex_26.924_x64__pub' }
}
function Get-AppxPackageManifest {
  [PSCustomObject]@{ Package=@{ Applications=@{ Application=@(
    @{Id='Runner';Executable='app\resources\codex.exe'},
    @{Id='Gui';Executable='app\ChatGPT.exe'}
  ) } } }
}
function Test-Path { return $true }
"#;
    let script = format!(
        "{setup}\n{}",
        build_codex_registered_launch_probe(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.900_x64__pub\app\ChatGPT.exe"
        ))
    );
    let output = codex_launch_powershell_output(&script).unwrap();
    let registered = parse_codex_registered_launch(&output).unwrap().unwrap();
    assert_eq!(registered.app_id, "Gui");
    assert!(registered.executable.contains("26.924"));
    let script = format!(
        "{setup}\n{}",
        build_codex_registered_launch_probe(Path::new(r"C:\Tools\ChatGPT.exe"))
    );
    assert!(
        parse_codex_registered_launch(&codex_launch_powershell_output(&script).unwrap())
            .unwrap()
            .is_none()
    );
}

#[cfg(target_os = "windows")]
#[test]
fn package_identity_probe_rejects_unpackaged_process() {
    // cargo's test process is unpackaged. This exercises the real Windows API.
    assert!(verify_codex_process_package(std::process::id(), "OpenAI.Codex_pub").is_err());
}

#[test]
fn registered_store_route_never_attempts_direct_exe() {
    let package = package();
    let path = Path::new(&package.executable);
    assert_eq!(
        codex_managed_launch_route(path, Some(&package)).unwrap(),
        CodexManagedLaunchRoute::PackageIdentity
    );
    assert!(codex_managed_launch_route(path, None).is_err());
    // A registered package may live outside WindowsApps.
    assert_eq!(
        codex_managed_launch_route(Path::new(r"D:\Apps\Codex\ChatGPT.exe"), Some(&package))
            .unwrap(),
        CodexManagedLaunchRoute::PackageIdentity
    );
    assert_eq!(
        codex_managed_launch_route(Path::new(r"C:\Tools\ChatGPT.exe"), None).unwrap(),
        CodexManagedLaunchRoute::DirectExe
    );
}

#[test]
fn package_refresh_keeps_publisher_identity() {
    assert_eq!(
        codex_store_package_family_from_path(Path::new(&package().executable)).as_deref(),
        Some("openai.codex_2p2nqsd0c76g0")
    );
    assert_eq!(
        codex_store_package_family_from_path(Path::new(r"C:\Tools\ChatGPT.exe")),
        None
    );
    assert_eq!(
        codex_store_package_family_from_path(Path::new(
            r"C:\WindowsApps\OpenAI.Codex_1_x64__other\app\ChatGPT.exe"
        ))
        .as_deref(),
        Some("openai.codex_other")
    );
}

#[test]
fn manifest_hint_matches_selected_executable_and_rejects_ambiguity() {
    let path = PathBuf::from(&package().executable);
    let manifest = r#"<Package><Applications>
      <Application Id="Runner" Executable="app\resources\codex.exe"/>
      <Application Id="Gui&amp;One" Executable="app\ChatGPT.exe"/>
    </Applications></Package>"#;
    let hint = codex_package_hint_from_manifest(&path, manifest).unwrap();
    assert_eq!(hint.app_id, "Gui&One");
    assert_eq!(hint.executable, package().executable);
    assert_eq!(hint.family_name, "openai.codex_2p2nqsd0c76g0");
    for invalid in [
        r#"<Package><Application Id="Gui" Executable="app\resources\codex.exe"/></Package>"#,
        r#"<Package><Application Id="Gui" Executable="app\ChatGPT.exe"/><Application Id="Other" Executable="app\ChatGPT.exe"/></Package>"#,
        r#"<Package><Application Id="Gui" Executable="..\app\ChatGPT.exe"/></Package>"#,
        r#"<Package><Application Id="" Executable="app\ChatGPT.exe"/></Package>"#,
        r#"<Package><Application Id="Gui" Executable="app\ChatGPT.exe" invalid /></Package>"#,
    ] {
        assert!(
            codex_package_hint_from_manifest(&path, invalid).is_none(),
            "{invalid}"
        );
    }
    assert!(
        codex_package_hint_from_manifest(Path::new(r"C:\Tools\ChatGPT.exe"), manifest).is_none()
    );
}

#[test]
fn package_validation_failure_refreshes_once_and_uses_updated_executable() {
    let old = package();
    let mut updated = old.clone();
    updated.executable = updated.executable.replace("26.924", "26.999");
    let mut attempted = Vec::new();
    let actual = activate_codex_package_with_refresh(
        old,
        |package| {
            attempted.push(package.executable.clone());
            if attempted.len() == 1 {
                Err("CODEX_PACKAGE_REGISTRATION_CHANGED: changed".into())
            } else {
                Ok(())
            }
        },
        || Ok(updated.clone()),
    )
    .unwrap();
    assert_eq!(attempted.len(), 2);
    assert_eq!(attempted[1], updated.executable);
    assert_eq!(actual.executable, updated.executable);
}

#[test]
fn activation_errors_never_trigger_ambiguous_or_unbounded_retries() {
    for error in [
        "activation timed out",
        "CODEX_PACKAGE_REGISTRATION_CHANGED: changed",
    ] {
        let mut count = 0;
        let result = activate_codex_package_with_refresh(
            package(),
            |_| {
                count += 1;
                Err(error.into())
            },
            || {
                assert!(error.contains("CODEX_PACKAGE_REGISTRATION_CHANGED"));
                Ok(package())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            count,
            if error.contains("CODEX_PACKAGE_REGISTRATION_CHANGED") {
                2
            } else {
                1
            }
        );
    }
    let result = activate_codex_package_with_refresh(
        package(),
        |_| Err("CODEX_PACKAGE_REGISTRATION_CHANGED".into()),
        || Err("registry unavailable".into()),
    );
    assert_eq!(result.unwrap_err(), "registry unavailable");
}

fn request(
    home: Option<&str>,
    data: Option<&Path>,
    args: &[String],
    env: &[(String, String)],
) -> codex_package_launcher::LaunchRequest {
    codex_package_launch_request(
        &package(),
        home,
        data,
        args,
        env,
        std::env::temp_dir().join("unused-codex-test-receipt.json"),
        "test-nonce".into(),
    )
}

#[test]
fn managed_and_default_commands_keep_selected_profiles_and_proxy_environment() {
    let env = vec![
        (
            "HTTPS_PROXY".into(),
            "http://user:secret@127.0.0.1:9123".into(),
        ),
        ("CODEX_HOME".into(), "wrong-home".into()),
        ("CODEX_ELECTRON_USER_DATA_PATH".into(), "wrong-data".into()),
    ];
    let args = vec![
        "--user-data-dir=old".into(),
        "--user-data-dir".into(),
        "old split".into(),
        "--remote-debugging-port=9222".into(),
        "--proxy-server=http://127.0.0.1:9123".into(),
    ];
    for managed in [false, true] {
        let request = request(
            managed.then_some("selected-home"),
            managed.then_some(Path::new(r"C:\O'Brien\资料 space")),
            &args,
            &env,
        );
        let command = request.child_command();
        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            env["CODEX_HOME"].as_deref(),
            managed.then_some("selected-home")
        );
        assert_eq!(
            env["CODEX_ELECTRON_USER_DATA_PATH"].as_deref(),
            managed.then_some(r"C:\O'Brien\资料 space")
        );
        assert_eq!(
            env["HTTPS_PROXY"].as_deref(),
            Some("http://user:secret@127.0.0.1:9123")
        );
        assert!(env[codex_package_launcher::PAYLOAD_ENV].is_none());
        assert_eq!(
            request
                .args
                .iter()
                .filter(|arg| arg.starts_with("--user-data-dir"))
                .count(),
            usize::from(managed)
        );
        assert!(!request.args.iter().any(|arg| arg == "old split"));
        assert!(request
            .args
            .contains(&"--remote-debugging-port=9222".into()));
    }
}

#[test]
fn activation_only_invokes_gui_helper_and_does_not_query_registration() {
    let request = request(
        Some("home"),
        Some(Path::new("data")),
        &[],
        &[("HTTPS_PROXY".into(), "secret-proxy-value".into())],
    );
    let script =
        build_codex_package_activation_script(&request, Path::new(r"C:\O'Brien\Cockpit Tools.exe"))
            .unwrap();
    assert!(script.contains("-PreventBreakaway"));
    assert!(script.contains("-Command 'C:\\O''Brien\\Cockpit Tools.exe'"));
    assert!(script.contains(codex_package_launcher::HELPER_ARG));
    assert!(!script.contains("Get-AppxPackage"));
    assert!(!script.contains("powershell.exe"));
    assert!(!script.contains("secret-proxy-value"));
    assert!(!script.contains("shell:AppsFolder"));
}

#[test]
fn native_validation_rejects_old_installation_and_wrong_manifest_entry() {
    let mut request = request(None, None, &[], &[]);
    let root = Path::new(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.924_x64__2p2nqsd0c76g0");
    let manifest = r#"<Package><Applications><Application Id="CodexGui" Executable="app/ChatGPT.exe"/><Application Id="Runner" Executable="app/resources/codex.exe"/></Applications></Package>"#;
    assert!(codex_package_launcher::matches_package_entry(
        &request,
        "openai.codex_2p2nqsd0c76g0",
        root,
        manifest
    ));
    assert!(!codex_package_launcher::matches_package_entry(
        &request,
        "OpenAI.Codex_other",
        root,
        manifest
    ));
    request.executable = request.executable.replace("26.924", "26.900");
    assert!(!codex_package_launcher::matches_package_entry(
        &request,
        &request.family_name,
        root,
        manifest
    ));
    request.executable = package().executable;
    request.app_id = "Runner".into();
    assert!(!codex_package_launcher::matches_package_entry(
        &request,
        &request.family_name,
        root,
        manifest
    ));
}

#[test]
fn uncertain_activation_proceeds_to_confirmation_without_retry() {
    let mut activated = 0;
    let actual = activate_codex_package_with_refresh(
        package(),
        |_| {
            activated += 1;
            Err("CODEX_ACTIVATION_UNCERTAIN: timeout".into())
        },
        || panic!("Uncertain activation must never rediscover and relaunch"),
    )
    .unwrap();
    assert_eq!(activated, 1);
    assert_eq!(actual.executable, package().executable);
}

#[test]
fn quick_path_validation_rejects_cli_dll_wrong_name_and_broken_files() {
    let directory = std::env::temp_dir().join(format!("codex-pe-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("ChatGPT.exe");
    let mut bytes = vec![0u8; 64 + 94];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
    bytes[64..68].copy_from_slice(b"PE\0\0");
    for magic in [0x10bu16, 0x20b] {
        bytes[88..90].copy_from_slice(&magic.to_le_bytes());
        bytes[156..158].copy_from_slice(&2u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(is_usable_codex_windows_gui_path(&path));
        let wrong = directory.join("codex.exe");
        std::fs::write(&wrong, &bytes).unwrap();
        assert!(!is_usable_codex_windows_gui_path(&wrong));
        std::fs::remove_file(wrong).unwrap();
        bytes[156..158].copy_from_slice(&3u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(!is_usable_codex_windows_gui_path(&path));
    }
    bytes[156..158].copy_from_slice(&2u16.to_le_bytes());
    bytes[86..88].copy_from_slice(&0x2000u16.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    assert!(!is_usable_codex_windows_gui_path(&path));
    std::fs::write(&path, b"not an executable").unwrap();
    assert!(!is_usable_codex_windows_gui_path(&path));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn successful_launch_receipts_round_trip_without_waiting_for_timeout() {
    use codex_package_launcher::{LaunchReply, PendingReceipt, Receipt};
    for reply in [
        LaunchReply::Spawned {
            pid: 123,
            validation_us: 1459,
            helper_has_console: false,
        },
        LaunchReply::Validated {
            validation_us: 1459,
            helper_has_console: false,
        },
    ] {
        let pending = PendingReceipt::new().unwrap();
        std::fs::write(
            &pending.path,
            serde_json::to_vec(&Receipt {
                nonce: pending.nonce.clone(),
                reply,
            })
            .unwrap(),
        )
        .unwrap();
        match pending
            .wait(Duration::ZERO)
            .expect("Successful launch receipt")
        {
            LaunchReply::Spawned {
                pid,
                validation_us,
                helper_has_console,
            } => {
                assert_eq!(pid, 123);
                assert_eq!(validation_us, 1459);
                assert!(!helper_has_console);
            }
            LaunchReply::Validated {
                validation_us,
                helper_has_console,
            } => {
                assert_eq!(validation_us, 1459);
                assert!(!helper_has_console);
            }
            reply => panic!("Unexpected reply: {reply:?}"),
        }
    }
}

#[test]
fn receipts_are_isolated_authenticated_and_removed_after_attempt() {
    use codex_package_launcher::{LaunchReply, PendingReceipt, Receipt};
    let first = PendingReceipt::new().unwrap();
    let second = PendingReceipt::new().unwrap();
    let path = first.path.clone();
    assert_ne!(first.path, second.path);
    std::fs::write(
        &first.path,
        serde_json::to_vec(&Receipt {
            nonce: second.nonce.clone(),
            reply: LaunchReply::EntryStale,
        })
        .unwrap(),
    )
    .unwrap();
    assert!(first.wait(Duration::from_millis(25)).is_none());
    std::fs::write(
        &first.path,
        serde_json::to_vec(&Receipt {
            nonce: first.nonce.clone(),
            reply: LaunchReply::EntryStale,
        })
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        first.wait(Duration::from_millis(25)),
        Some(LaunchReply::EntryStale)
    ));
    assert!(second.wait(Duration::from_millis(25)).is_none());
    drop(first);
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}

#[cfg(target_os = "windows")]
#[test]
fn default_child_receives_proxy_and_clears_inherited_profile_without_console() {
    use std::os::windows::process::CommandExt;
    let proxy = "socks5h://127.0.0.1:53669";
    let mut env = account_proxy_env_pairs(proxy)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<Vec<_>>();
    env.push(("CODEX_HOME".into(), "wrong-home".into()));
    env.push(("CODEX_ELECTRON_USER_DATA_PATH".into(), "wrong-data".into()));
    let script = "$values=@{};foreach($key in @('ALL_PROXY','HTTP_PROXY','HTTPS_PROXY','CODEX_HOME','CODEX_ELECTRON_USER_DATA_PATH','COCKPIT_CODEX_PACKAGE_LAUNCH_PAYLOAD')){$values[$key]=[Environment]::GetEnvironmentVariable($key,'Process')};$values|ConvertTo-Json -Compress";
    let mut request = request(None, None, &[], &env);
    request.executable = PathBuf::from(std::env::var("SystemRoot").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
        .to_string_lossy()
        .into_owned();
    request.args = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        script.into(),
    ];
    let output = crate::modules::process_timeout::output_with_timeout(
        request.child_command().creation_flags(CREATE_NO_WINDOW),
        Duration::from_secs(15),
    )
    .unwrap();
    assert!(output.status.success());
    let env: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for key in ["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY"] {
        assert_eq!(env[key], proxy);
    }
    for key in [
        "CODEX_HOME",
        "CODEX_ELECTRON_USER_DATA_PATH",
        codex_package_launcher::PAYLOAD_ENV,
    ] {
        assert!(env[key].is_null());
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "Command-only local package validation; never starts Codex or a UI"]
fn local_manifest_preparation_matches_real_registration() {
    use codex_package_launcher::{LaunchReply, PendingReceipt};
    let path = PathBuf::from(
        std::env::var("COCKPIT_CODEX_PROBE_EXPECTED_EXE").expect("Explicit GUI path required"),
    );
    let helper = PathBuf::from(
        std::env::var("COCKPIT_CODEX_GUI_HELPER_EXE")
            .expect("Explicit GUI subsystem host helper required"),
    );
    assert!(windows_executable_has_gui_subsystem(&helper));
    assert!(is_usable_codex_windows_gui_path(&path));
    let started = Instant::now();
    let hint = codex_package_hint_from_path(&path).unwrap();
    let manifest_us = started.elapsed().as_micros();
    let registered = query_codex_registered_launch(&path).unwrap().unwrap();
    assert_eq!(hint.app_id, registered.app_id);
    assert!(hint
        .family_name
        .eq_ignore_ascii_case(&registered.family_name));
    for managed in [false, true] {
        let receipt = PendingReceipt::new().unwrap();
        let mut request = codex_package_launch_request(
            &hint,
            managed.then_some("diagnostic-home"),
            managed.then_some(Path::new("diagnostic-data")),
            &[],
            &[],
            receipt.path.clone(),
            receipt.nonce.clone(),
        );
        request.verify_only = true;
        let script = build_codex_package_activation_script(&request, &helper).unwrap();
        let started = Instant::now();
        let output = codex_launch_powershell_output(&script).unwrap();
        match receipt
            .wait(Duration::from_secs(5))
            .expect("Native helper receipt")
        {
            LaunchReply::Validated {
                validation_us,
                helper_has_console,
            } => {
                assert!(
                    !helper_has_console,
                    "Native package helper must not allocate a terminal"
                );
                println!("manifest_us={manifest_us} native_validation_us={validation_us} managed={managed} command_total_ms={} helper_has_console={helper_has_console} {}", started.elapsed().as_millis(), output.replace('\n', " "));
            }
            other => panic!("Unexpected native validation receipt: {other:?}"),
        }
    }
}
