use super::*;

#[test]
fn missing_path_allows_noop_close_only_after_positive_absence_confirmation() {
    let error = "APP_PATH_NOT_FOUND:codex".to_string();
    assert_eq!(
        codex_close_path_or_confirmed_idle(Err(error.clone()), || true),
        Ok(None)
    );
    assert_eq!(
        codex_close_path_or_confirmed_idle(Err(error.clone()), || false),
        Err(error)
    );
    assert_eq!(
        codex_close_path_or_confirmed_idle(Err("permission denied".into()), || panic!(
            "Do not hide unrelated errors"
        )),
        Err("permission denied".into())
    );
    assert_eq!(
        codex_close_path_or_confirmed_idle(Ok(PathBuf::from("selected")), || panic!(
            "Known paths use the existing identity-aware close"
        )),
        Ok(Some(PathBuf::from("selected")))
    );
}

#[test]
fn failed_incomplete_or_nonempty_process_probes_never_allow_auth_mutation() {
    let empty = br#"{"completed":true,"count":0}"#;
    assert!(codex_pathless_probe_confirms_idle(true, empty));
    assert!(codex_pathless_probe_confirms_idle(
        true,
        b"\xef\xbb\xbf{\"completed\":true,\"count\":0}\r\n"
    ));
    assert!(!codex_pathless_probe_confirms_idle(false, empty));
    for probe in [
        br#"{"completed":true,"count":1}"#.as_slice(),
        br#"{"completed":false,"count":0}"#,
        br#"{"count":0}"#,
        br#"{"completed":true}"#,
        br#"{"completed":true,"count":-1}"#,
        br#"{"completed":true,"count":"0"}"#,
        b"",
        b"null",
        b"CIM unavailable",
    ] {
        assert!(!codex_pathless_probe_confirms_idle(true, probe));
    }
}

#[test]
fn desktop_path_validation_rejects_cli_and_nonexistent_selections() {
    assert!(validate_codex_app_path("").is_err());
    assert!(validate_codex_app_path("  ").is_err());
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        validate_codex_app_path(&dir.path().join("missing").to_string_lossy()),
        Err("CODEX_APP_PATH_INVALID".into())
    );
    assert!(is_codex_macos_gui_candidate(Path::new(
        "/Applications/ChatGPT.app"
    )));
    assert!(is_codex_macos_gui_candidate(Path::new(
        "/Custom/My Client.app/Contents/MacOS/ChatGPT"
    )));
    assert!(is_codex_macos_gui_candidate(Path::new(
        "/Applications/Codex.app/Contents/MacOS/Codex"
    )));
    assert!(!is_codex_macos_gui_candidate(Path::new(
        "/Applications/ChatGPT.app/Contents/Resources/codex"
    )));
    assert!(!is_codex_macos_gui_candidate(Path::new(
        "/usr/local/bin/codex"
    )));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_desktop_validation_requires_executable_gui_file_in_the_selected_bundle() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("ChatGPT.app");
    let executable = bundle.join("Contents/MacOS/ChatGPT");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"fixture").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(validate_codex_app_path(&bundle.to_string_lossy()).is_err());
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(validate_codex_app_path(&bundle.to_string_lossy()).is_ok());
    assert!(validate_codex_app_path(&executable.to_string_lossy()).is_ok());
    let cli = bundle.join("Contents/Resources/codex");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(&cli, b"cli").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(validate_codex_app_path(&cli.to_string_lossy()).is_err());
}
