use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_core::models::codex::{
    CodexAccount, CodexAccountIndex, CodexAccountSummary, CodexAgentIdentity, CodexQuota,
    CodexQuotaErrorInfo, CodexTokens,
};
use cockpit_core::models::{
    DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile, InstanceStore,
};

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
    let result = run_cli_in_data_dir(args, &data_dir);
    (result.0, result.1, result.2, data_dir)
}

fn run_cli_in_data_dir(
    args: &[&str],
    data_dir: &Path,
) -> (String, String, std::process::ExitStatus) {
    let codex_home = data_dir.join("codex-home");
    fs::create_dir_all(&codex_home).expect("Codex home should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_cockpit-cli"))
        .args(args)
        .env("COCKPIT_TOOLS_DATA_DIR", data_dir)
        .env("CODEX_HOME", &codex_home)
        .output()
        .expect("cockpit-cli should start");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    (stdout, stderr, output.status)
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

fn codex_account(id: &str, email: &str) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.to_string(),
        email.to_string(),
        CodexTokens {
            id_token: "id-token-secret".to_string(),
            access_token: "access-token-secret".to_string(),
            refresh_token: Some("refresh-token-secret".to_string()),
        },
    );
    account.openai_api_key = Some("openai-api-key-secret".to_string());
    account.agent_identity = Some(CodexAgentIdentity {
        agent_runtime_id: "runtime-id".to_string(),
        agent_private_key: "agent-private-key-secret".to_string(),
        task_id: None,
        account_id: "account-id".to_string(),
        chatgpt_user_id: "user-id".to_string(),
        email: Some(email.to_string()),
        plan_type: Some("Plus".to_string()),
        chatgpt_account_is_fedramp: false,
    });
    account.plan_type = Some("Plus".to_string());
    account.tags = Some(vec!["work".to_string(), "primary".to_string()]);
    account.quota = Some(CodexQuota {
        hourly_percentage: 32,
        hourly_reset_time: Some(1_700_000_000),
        hourly_window_minutes: Some(300),
        hourly_window_present: Some(true),
        weekly_percentage: 71,
        weekly_reset_time: Some(1_700_500_000),
        weekly_window_minutes: Some(10_080),
        weekly_window_present: Some(true),
        raw_data: Some(serde_json::json!({
            "access_token": "raw-access-token-secret",
            "refresh_token": "raw-refresh-token-secret"
        })),
    });
    account.usage_updated_at = Some(1_700_000_100);
    account
}

fn write_codex_accounts(data_dir: &Path, accounts: &[CodexAccount], current_id: Option<&str>) {
    let accounts_dir = data_dir.join("codex_accounts");
    fs::create_dir_all(&accounts_dir).expect("Codex accounts directory should be created");

    let index = CodexAccountIndex {
        version: "1.0".to_string(),
        detail_schema_version: 2,
        accounts: accounts
            .iter()
            .map(|account| CodexAccountSummary {
                id: account.id.clone(),
                email: account.email.clone(),
                plan_type: account.plan_type.clone(),
                created_at: account.created_at,
                last_used: account.last_used,
            })
            .collect(),
        current_account_id: current_id.map(str::to_string),
    };
    fs::write(
        data_dir.join("codex_accounts.json"),
        serde_json::to_vec_pretty(&index).expect("Codex account index should serialize"),
    )
    .expect("Codex account index should be written");

    for account in accounts {
        fs::write(
            accounts_dir.join(format!("{}.json", account.id)),
            serde_json::to_vec_pretty(account).expect("Codex account should serialize"),
        )
        .expect("Codex account should be written");
    }
}

#[test]
fn empty_codex_account_list_is_read_only_and_machine_readable() {
    let data_dir = isolated_data_dir();
    let (stdout, stderr, status) = run_cli_in_data_dir(&["--json", "list", "codex"], &data_dir);
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
    assert!(!data_dir.join("codex_accounts.json").exists());
    assert!(!data_dir.join("codex_accounts").exists());
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn codex_account_output_is_safe_and_projects_status_and_quota() {
    let data_dir = isolated_data_dir();
    let ready = codex_account("ready", "ready@example.com");
    let mut reauth = codex_account("reauth", "reauth@example.com");
    reauth.requires_reauth = true;
    let mut exhausted = codex_account("exhausted", "exhausted@example.com");
    exhausted
        .quota
        .as_mut()
        .expect("fixture should have quota")
        .hourly_percentage = 0;
    let mut quota_error = codex_account("quota-error", "quota-error@example.com");
    quota_error.quota = None;
    quota_error.quota_error = Some(CodexQuotaErrorInfo {
        code: Some("OFFLINE".to_string()),
        message: "cached error".to_string(),
        timestamp: 1_700_000_200,
    });
    let mut unknown = codex_account("unknown", "unknown@example.com");
    unknown.quota = None;
    write_codex_accounts(
        &data_dir,
        &[ready, reauth, exhausted, quota_error, unknown],
        Some("ready"),
    );

    let (stdout, stderr, status) = run_cli_in_data_dir(&["--json", "list", "codex"], &data_dir);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");
    let accounts = value["data"].as_array().expect("data should be an array");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(accounts.len(), 5);
    assert_eq!(accounts[0]["status"], "ready");
    assert_eq!(accounts[0]["quota"]["hourly_percentage"], 32);
    assert_eq!(accounts[0]["quota"]["weekly_window_minutes"], 10_080);
    assert_eq!(accounts[1]["status"], "reauth_required");
    assert_eq!(accounts[2]["status"], "quota_exhausted");
    assert_eq!(accounts[3]["status"], "quota_error");
    assert_eq!(accounts[4]["status"], "unknown");
    for secret_name in [
        "openai_api_key",
        "access_token",
        "refresh_token",
        "agent_private_key",
        "raw_data",
    ] {
        assert!(
            !stdout.contains(secret_name),
            "Codex output must not expose {secret_name}"
        );
    }
    for secret in [
        "openai-api-key-secret",
        "access-token-secret",
        "refresh-token-secret",
        "agent-private-key-secret",
        "raw-access-token-secret",
    ] {
        assert!(
            !stdout.contains(secret),
            "Codex output must not expose {secret}"
        );
    }
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn codex_current_and_quota_use_cached_data_without_auth_mutation() {
    let data_dir = isolated_data_dir();
    let account = codex_account("current", "current@example.com");
    write_codex_accounts(&data_dir, std::slice::from_ref(&account), Some("current"));
    let index_before = fs::read(data_dir.join("codex_accounts.json"))
        .expect("Codex account index should be readable");
    let account_before = fs::read(data_dir.join("codex_accounts/current.json"))
        .expect("Codex account should be readable");

    let (current_stdout, current_stderr, current_status) =
        run_cli_in_data_dir(&["--json", "current", "codex"], &data_dir);
    let current: serde_json::Value =
        serde_json::from_str(&current_stdout).expect("current output should be JSON");
    assert!(current_status.success());
    assert!(current_stderr.is_empty());
    assert_eq!(current["data"]["id"], "current");
    assert_eq!(current["data"]["status"], "ready");
    assert!(current["data"].get("access_token").is_none());

    let (quota_stdout, quota_stderr, quota_status) =
        run_cli_in_data_dir(&["--json", "quota", "codex"], &data_dir);
    let quota: serde_json::Value =
        serde_json::from_str(&quota_stdout).expect("quota output should be JSON");
    assert!(quota_status.success());
    assert!(quota_stderr.is_empty());
    assert_eq!(quota["data"][0]["quota"]["hourly_percentage"], 32);
    assert_eq!(quota["data"][0]["quota"]["weekly_percentage"], 71);
    assert!(!quota_stdout.contains("raw-access-token-secret"));

    assert_eq!(
        index_before,
        fs::read(data_dir.join("codex_accounts.json")).unwrap()
    );
    assert_eq!(
        account_before,
        fs::read(data_dir.join("codex_accounts/current.json")).unwrap()
    );

    let (selected_stdout, selected_stderr, selected_status) = run_cli_in_data_dir(
        &["--json", "quota", "codex", "CURRENT@EXAMPLE.COM"],
        &data_dir,
    );
    let selected: serde_json::Value =
        serde_json::from_str(&selected_stdout).expect("selected quota output should be JSON");
    assert!(selected_status.success());
    assert!(selected_stderr.is_empty());
    assert_eq!(selected["data"].as_array().unwrap().len(), 1);
    assert_eq!(selected["data"][0]["id"], "current");

    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn codex_instances_output_contains_safe_persisted_metadata_and_default_instance() {
    let data_dir = isolated_data_dir();
    let instance_dir = data_dir.join("instances").join("work");
    fs::create_dir_all(&instance_dir).expect("instance directory should be created");
    let store = InstanceStore {
        instances: vec![InstanceProfile {
            id: "instance-1".to_string(),
            name: "work".to_string(),
            user_data_dir: instance_dir.to_string_lossy().into_owned(),
            working_dir: Some("C:\\projects\\work".to_string()),
            extra_args: "--secret-arg should-not-be-exported".to_string(),
            bind_account_id: Some("account-1".to_string()),
            launch_mode: InstanceLaunchMode::Cli,
            created_at: 1_700_000_000,
            last_launched_at: Some(1_700_000_100),
            last_pid: Some(4242),
        }],
        default_settings: DefaultInstanceSettings {
            bind_account_id: None,
            extra_args: "--default-secret-arg".to_string(),
            launch_mode: InstanceLaunchMode::App,
            follow_local_account: true,
            last_pid: Some(3131),
        },
    };
    fs::write(
        data_dir.join("codex_instances.json"),
        serde_json::to_vec_pretty(&store).expect("instance store should serialize"),
    )
    .expect("instance store should be written");

    let (stdout, stderr, status) =
        run_cli_in_data_dir(&["--json", "instances", "codex"], &data_dir);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout should be JSON");
    let instances = value["data"].as_array().expect("data should be an array");

    assert!(status.success());
    assert!(
        stderr.is_empty(),
        "JSON mode should not write diagnostics: {stderr}"
    );
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0]["name"], "work");
    assert_eq!(instances[0]["launch_mode"], "cli");
    assert_eq!(instances[0]["initialized"], true);
    assert_eq!(instances[0]["last_pid"], 4242);
    assert_eq!(instances[0]["is_default"], false);
    assert_eq!(instances[1]["id"], "__default__");
    assert_eq!(instances[1]["is_default"], true);
    assert_eq!(instances[1]["follow_local_account"], true);
    assert_eq!(instances[1]["last_pid"], 3131);
    assert!(instances[0].get("extra_args").is_none());
    assert!(!stdout.contains("secret-arg"));
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn malformed_codex_index_fails_without_repairing_or_exposing_secrets() {
    let data_dir = isolated_data_dir();
    let index_path = data_dir.join("codex_accounts.json");
    let malformed = "{\"access_token\":\"must-not-leak\"";
    fs::write(&index_path, malformed).expect("malformed index should be written");

    let (stdout, stderr, status) = run_cli_in_data_dir(&["--json", "list", "codex"], &data_dir);

    assert!(!status.success());
    assert!(stdout.is_empty());
    assert!(!stderr.contains("must-not-leak"));
    assert_eq!(fs::read_to_string(index_path).unwrap(), malformed);
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}

#[test]
fn malformed_codex_instance_store_fails_without_repairing_or_exposing_secrets() {
    let data_dir = isolated_data_dir();
    let store_path = data_dir.join("codex_instances.json");
    let malformed = "{\"access_token\":\"must-not-leak\"";
    fs::write(&store_path, malformed).expect("malformed instance store should be written");

    let (stdout, stderr, status) =
        run_cli_in_data_dir(&["--json", "instances", "codex"], &data_dir);

    assert!(!status.success());
    assert!(stdout.is_empty());
    assert!(!stderr.contains("must-not-leak"));
    assert_eq!(fs::read_to_string(&store_path).unwrap(), malformed);
    assert!(!data_dir.join("codex_instances.json.invalid-json").exists());
    fs::remove_dir_all(data_dir).expect("test data directory should be removable");
}
