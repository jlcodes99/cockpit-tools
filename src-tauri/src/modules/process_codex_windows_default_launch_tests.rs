use super::*;
use std::cell::Cell;

fn package() -> CodexRegisteredLaunch {
    CodexRegisteredLaunch {
        family_name: "OpenAI.Codex_pub".into(),
        app_id: "Gui".into(),
        executable: r"C:\WindowsApps\OpenAI.Codex_1_x64__pub\app\ChatGPT.exe".into(),
    }
}

#[test]
fn configured_path_success_never_runs_automatic_discovery() {
    let selected = PathBuf::from(r"D:\Custom\ChatGPT.exe");
    let result = launch_codex_default_with_path_fallback(
        Some(selected.clone()),
        || panic!("A configured client must be attempted before any discovery"),
        |path| {
            assert_eq!(path, selected);
            Ok((123, path.to_path_buf()))
        },
    )
    .unwrap();
    assert_eq!(result, (123, selected));
}

#[test]
fn failed_configured_path_discovers_once_skips_duplicates_and_tries_later_entries() {
    let discovery_count = Cell::new(0);
    let mut attempted = Vec::new();
    let result = launch_codex_default_with_path_fallback(
        Some(PathBuf::from(r"D:\Custom\ChatGPT.exe")),
        || {
            assert_eq!(discovery_count.replace(1), 0);
            [
                "d:/custom/chatgpt.EXE",
                r"E:\Broken\ChatGPT.exe",
                r"E:\Broken\ChatGPT.exe",
                r"E:\Current\ChatGPT.exe",
            ]
            .map(PathBuf::from)
            .to_vec()
        },
        |path| {
            attempted.push(path.to_path_buf());
            if path == Path::new(r"E:\Current\ChatGPT.exe") {
                Ok((456, path.to_path_buf()))
            } else {
                Err(CodexDefaultLaunchFailure::NotStarted("not usable".into()))
            }
        },
    )
    .unwrap();
    assert_eq!(discovery_count.get(), 1);
    assert_eq!(attempted.len(), 3);
    assert_eq!(attempted[0], Path::new(r"D:\Custom\ChatGPT.exe"));
    assert_eq!(result, (456, PathBuf::from(r"E:\Current\ChatGPT.exe")));
}

#[test]
fn missing_configuration_uses_discovered_path_and_preserves_refreshed_launch_target() {
    let result = launch_codex_default_with_path_fallback(
        None,
        || vec![PathBuf::from(r"E:\Old\ChatGPT.exe")],
        |_| Ok((789, PathBuf::from(r"E:\New\ChatGPT.exe"))),
    )
    .unwrap();
    assert_eq!(result, (789, PathBuf::from(r"E:\New\ChatGPT.exe")));
}

#[test]
fn configured_dispatch_or_confirmation_failure_never_discovers_another_entry() {
    for error in ["activation failed", "confirmation timed out"] {
        let result = launch_codex_default_with_path_fallback(
            Some(PathBuf::from(r"D:\Custom\ChatGPT.exe")),
            || panic!("Do not risk launching a second client after dispatch"),
            |_| Err(CodexDefaultLaunchFailure::Dispatched(error.into())),
        );
        assert_eq!(result.unwrap_err(), error);
    }
}

#[test]
fn discovered_dispatch_failure_stops_before_trying_later_candidates() {
    let attempts = Cell::new(0);
    let result = launch_codex_default_with_path_fallback(
        None,
        || vec![PathBuf::from("first"), PathBuf::from("second")],
        |_| {
            attempts.set(attempts.get() + 1);
            Err(CodexDefaultLaunchFailure::Dispatched("uncertain".into()))
        },
    );
    assert_eq!(result.unwrap_err(), "uncertain");
    assert_eq!(attempts.get(), 1);
}

#[test]
fn exhausted_candidates_report_original_failures_without_claiming_success() {
    let result = launch_codex_default_with_path_fallback(
        Some(PathBuf::from("configured")),
        || vec![PathBuf::from("detected")],
        |path| {
            Err(CodexDefaultLaunchFailure::NotStarted(format!(
                "rejected {}",
                path.display()
            )))
        },
    );
    let error = result.unwrap_err();
    assert!(error.contains("rejected configured"));
    assert!(error.contains("rejected detected"));
    assert!(
        launch_codex_default_with_path_fallback(None, Vec::new, |_| panic!("No available entry"),)
            .is_err()
    );
}

#[test]
fn missing_paths_keep_the_error_protocol_used_by_the_path_configuration_dialog() {
    let error = launch_codex_default_with_path_fallback(
        Some(PathBuf::from(r"D:\Missing\ChatGPT.exe")),
        Vec::new,
        |_| {
            Err(CodexDefaultLaunchFailure::NotStarted(
                app_path_missing_error("codex"),
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error, "APP_PATH_NOT_FOUND:codex");
}

#[test]
fn stale_registration_can_fall_back_only_when_no_client_was_created() {
    let activated = Cell::new(0);
    let result = activate_codex_default_package_with_refresh(
        package(),
        |_| {
            activated.set(activated.get() + 1);
            Err("CODEX_PACKAGE_REGISTRATION_CHANGED: entry rejected".into())
        },
        || Ok(package()),
    );
    assert!(matches!(
        result,
        Err(CodexDefaultLaunchFailure::NotStarted(_))
    ));
    assert_eq!(activated.get(), 2);

    let result = activate_codex_default_package_with_refresh(
        package(),
        |_| Err("CODEX_PACKAGE_REGISTRATION_CHANGED: entry rejected".into()),
        || Err("registration query failed".into()),
    );
    assert!(
        matches!(result, Err(CodexDefaultLaunchFailure::NotStarted(error)) if error == "registration query failed")
    );
}

#[test]
fn ambiguous_activation_continues_to_confirmation_without_refresh_or_relaunch() {
    let activated = Cell::new(0);
    let result = activate_codex_default_package_with_refresh(
        package(),
        |_| {
            activated.set(activated.get() + 1);
            Err("CODEX_ACTIVATION_UNCERTAIN: no receipt".into())
        },
        || panic!("An uncertain dispatch must only confirm the process"),
    );
    assert!(result.is_ok());
    assert_eq!(activated.get(), 1);
    let result = activate_codex_default_package_with_refresh(
        package(),
        |_| Err("PowerShell failed without a receipt".into()),
        || panic!("Unknown failures must not refresh and relaunch"),
    );
    assert!(matches!(
        result,
        Err(CodexDefaultLaunchFailure::Dispatched(_))
    ));
}

#[test]
fn discovery_reads_actual_gui_id_and_rejects_runner_traversal_and_ambiguity() {
    let manifest = r#"<Package><Applications>
      <Application Id="Runner" Executable="app/resources/codex.exe" />
      <Application Id="DesktopGui" Executable="desktop/ChatGPT.exe" />
    </Applications></Package>"#;
    assert_eq!(
        appx_manifest_gui_executable_from_text(manifest).as_deref(),
        Some("desktop/ChatGPT.exe")
    );
    for invalid in [
        r#"<Application Id="Gui" Executable="../ChatGPT.exe" />"#,
        r#"<Application Id="Gui" Executable="C:/ChatGPT.exe" />"#,
        r#"<Application Id="Gui" Executable="/app/ChatGPT.exe" />"#,
        r#"<Application Id="" Executable="app/ChatGPT.exe" />"#,
        r#"<Application Id="Runner" Executable="app/resources/codex.exe" />"#,
        r#"<Applications><Application Id="One" Executable="one/ChatGPT.exe" /><Application Id="Two" Executable="two/ChatGPT.exe" /></Applications>"#,
    ] {
        assert_eq!(
            appx_manifest_gui_executable_from_text(invalid),
            None,
            "{invalid}"
        );
    }
}
