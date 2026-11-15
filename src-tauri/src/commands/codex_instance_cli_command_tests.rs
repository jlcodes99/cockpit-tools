use super::{build_launch_command_text, CodexLaunchContext};

#[test]
fn codex_cli_custom_command_inherits_profile_and_working_directory() {
    let sandbox = tempfile::tempdir().unwrap();
    let dir = sandbox.path().join("working ' directory");
    let profile = sandbox.path().join("account ' profile");
    std::fs::create_dir(&dir).unwrap();
    let context = CodexLaunchContext {
        user_data_dir: profile.to_string_lossy().into_owned(),
        working_dir: Some(dir.to_string_lossy().into_owned()),
        extra_args: "--version".into(),
    };
    let command = build_launch_command_text(
        &context,
        "/bin/false",
        None,
        Some("printf '%s\\n' \"$CODEX_HOME\"; pwd"),
    )
    .unwrap();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &command])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (home, cwd) = stdout.split_once('\n').unwrap();
    assert_eq!(home, context.user_data_dir);
    assert_eq!(
        std::path::Path::new(cwd.trim_end()).canonicalize().unwrap(),
        dir.canonicalize().unwrap()
    );
}
