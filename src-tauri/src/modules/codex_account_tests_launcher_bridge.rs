// #2708：Codex OAuth 切号（OpenaiBuiltin + base_url=None）会无条件移除
// openai_base_url，把第三方 launcher（codex-chatgpt-web）管理的本地 bridge
// 一起删掉。守卫为窄白名单：键带 launcher 管理注释、URL 为 loopback+显式
// 端口+/v1、integration-journal 报 active 且 endpoint/configPath 均匹配；
// 其余情况维持原删除行为，API-key 投影不受影响。

const LAUNCHER_BRIDGE_URL: &str = "http://127.0.0.1:17841/v1";
const LAUNCHER_MANAGED_COMMENT: &str =
    "# Managed by codex-chatgpt-web: Responses use the local bridge; Voice stays on ChatGPT.";

fn launcher_write_config_toml(base_dir: &std::path::Path, base_url: &str) {
    let content = format!(
        "{LAUNCHER_MANAGED_COMMENT}\nopenai_base_url = \"{base_url}\"\nmodel = \"gpt-5.5\"\n"
    );
    fs::write(base_dir.join("config.toml"), content).expect("write config.toml");
}

fn launcher_write_journal(
    home: &std::path::Path,
    active: bool,
    base_url: &str,
    config_path: &std::path::Path,
) {
    let dir = home.join(".codex-chatgpt-web").join("codex");
    fs::create_dir_all(&dir).expect("create launcher journal dir");
    let journal = serde_json::json!({
        "active": active,
        "installed": {
            "openai_base_url": base_url,
            "configPath": config_path.to_string_lossy(),
        }
    });
    fs::write(
        dir.join("integration-journal.json"),
        serde_json::to_string(&journal).expect("serialize journal"),
    )
    .expect("write journal");
}

fn launcher_oauth_account(id: &str) -> CodexAccount {
    let mut oauth = CodexAccount::new(
        id.to_string(),
        "oauth@example.com".to_string(),
        make_codex_tokens(
            "oauth@example.com",
            id,
            "org-launcher-test",
            "acct-launcher-test",
            "refresh.token",
        ),
    );
    oauth.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
    oauth
}

#[test]
fn oauth_switch_preserves_active_launcher_managed_bridge() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-launcher-bridge-preserve");
    let base_dir = make_temp_dir("codex-launcher-bridge-preserve");
    launcher_write_config_toml(&base_dir, LAUNCHER_BRIDGE_URL);
    let canonical_config = base_dir.join("config.toml").canonicalize().expect("canonicalize");
    launcher_write_journal(&_env.home_dir, true, LAUNCHER_BRIDGE_URL, &canonical_config);

    let account = launcher_oauth_account("oauth-launcher-keep");
    write_auth_file_to_dir(&base_dir, &account).expect("write auth bundle");

    let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
    assert!(
        content.contains("Managed by codex-chatgpt-web"),
        "launcher managed comment must survive the switch: {content}"
    );
    assert!(
        content.contains(LAUNCHER_BRIDGE_URL),
        "launcher bridge url must survive the switch: {content}"
    );
    fs::remove_dir_all(&base_dir).ok();
}

#[test]
fn oauth_switch_still_clears_remote_or_credential_bearing_relay() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-launcher-bridge-remote");
    let base_dir = make_temp_dir("codex-launcher-bridge-remote");
    let remote = "https://relay.example.com/v1";
    launcher_write_config_toml(&base_dir, remote);
    let canonical_config = base_dir.join("config.toml").canonicalize().expect("canonicalize");
    launcher_write_journal(&_env.home_dir, true, remote, &canonical_config);

    let account = launcher_oauth_account("oauth-launcher-remote");
    write_auth_file_to_dir(&base_dir, &account).expect("write auth bundle");

    let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
    assert!(
        !content.contains("openai_base_url"),
        "non-loopback relay must still be cleared: {content}"
    );
    fs::remove_dir_all(&base_dir).ok();
}

#[test]
fn oauth_switch_requires_active_and_matching_journal() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-launcher-bridge-inactive");
    let base_dir = make_temp_dir("codex-launcher-bridge-inactive");
    launcher_write_config_toml(&base_dir, LAUNCHER_BRIDGE_URL);
    let canonical_config = base_dir.join("config.toml").canonicalize().expect("canonicalize");
    launcher_write_journal(&_env.home_dir, false, LAUNCHER_BRIDGE_URL, &canonical_config);

    let account = launcher_oauth_account("oauth-launcher-inactive");
    write_auth_file_to_dir(&base_dir, &account).expect("write auth bundle");

    let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
    assert!(
        !content.contains("openai_base_url"),
        "inactive journal must not preserve the bridge: {content}"
    );
    fs::remove_dir_all(&base_dir).ok();
}

#[test]
fn api_key_projection_still_applies_own_endpoint() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-launcher-bridge-apikey");
    let base_dir = make_temp_dir("codex-launcher-bridge-apikey");
    launcher_write_config_toml(&base_dir, LAUNCHER_BRIDGE_URL);
    let canonical_config = base_dir.join("config.toml").canonicalize().expect("canonicalize");
    launcher_write_journal(&_env.home_dir, true, LAUNCHER_BRIDGE_URL, &canonical_config);

    let api_key = CodexAccount::new_api_key(
        "api-key-launcher-test".to_string(),
        "api@example.com".to_string(),
        "sk-test-key".to_string(),
        CodexApiProviderMode::Custom,
        Some("https://relay.example.com/v1".to_string()),
        Some("relay".to_string()),
        Some("Relay".to_string()),
        Vec::new(),
    );
    write_auth_file_to_dir(&base_dir, &api_key).expect("write auth bundle");

    let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
    assert!(
        content.contains("base_url = \"https://relay.example.com/v1\""),
        "api-key projection must apply its own endpoint: {content}"
    );
    fs::remove_dir_all(&base_dir).ok();
}
