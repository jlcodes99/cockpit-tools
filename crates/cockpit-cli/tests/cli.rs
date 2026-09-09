use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn isolated_data_dir() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cockpit-cli-test-{suffix}"));
    fs::create_dir_all(&path).expect("test data directory should be created");
    path
}

fn run_cli(args: &[&str]) -> (String, String, std::process::ExitStatus, PathBuf) {
    let data_dir = isolated_data_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_cockpit-cli"))
        .args(args)
        .env("COCKPIT_TOOLS_DATA_DIR", &data_dir)
        .output()
        .expect("cockpit-cli should start");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    (stdout, stderr, output.status, data_dir)
}

#[test]
fn root_json_output_has_stable_envelope() {
    let (stdout, stderr, status, data_dir) = run_cli(&["--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["data"]["operation"], "help");
    assert_eq!(value["data"]["status"], "ok");
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn quota_json_output_is_machine_readable_without_core_access() {
    let (stdout, stderr, status, data_dir) = run_cli(&["--json", "quota", "CURSOR"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["data"]["provider"], "cursor");
    assert_eq!(value["data"]["operation"], "quota");
    assert_eq!(value["data"]["status"], "not_implemented");
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn copilot_switch_reports_unsupported_status_in_json() {
    let (stdout, stderr, status, data_dir) =
        run_cli(&["switch", "github-copilot", "account-1", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["data"]["provider"], "copilot");
    assert_eq!(value["data"]["operation"], "switch");
    assert_eq!(value["data"]["status"], "unsupported");
    assert!(value["data"].get("access_token").is_none());
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn empty_cursor_account_list_is_isolated_from_user_data() {
    let (stdout, stderr, status, data_dir) = run_cli(&["--json", "list", "cursor"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(value["schema_version"], 1);
    assert!(value["data"]
        .as_array()
        .expect("data should be an array")
        .is_empty());
    assert!(data_dir.join("cursor_accounts.json").exists());
    assert!(!data_dir.join("codex").exists());
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn missing_cursor_account_returns_non_zero_without_secret_output() {
    let (stdout, stderr, status, data_dir) = run_cli(&["switch", "cursor", "missing-account"]);

    assert!(!status.success());
    assert!(!stdout.contains("access_token"));
    assert!(!stderr.contains("access_token"));
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}
