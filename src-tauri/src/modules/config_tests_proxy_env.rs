// Each case executes itself in a fresh test process: the production OnceLock
// and process environment must not depend on other tests or leak into them.
struct ProxyEnvTestDirectory(std::path::PathBuf);

impl Drop for ProxyEnvTestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check_proxy_env_in_fresh_process(test_name: &str, scenario: &str) {
    const SCENARIO: &str = "COCKPIT_TEST_PROXY_ENV_SCENARIO";
    if let Ok(child_scenario) = std::env::var(SCENARIO) {
        assert_eq!(child_scenario, scenario);
        check_proxy_env_restore_scenario(scenario);
        std::fs::write(
            std::env::var("COCKPIT_TEST_PROXY_ENV_RESULT").unwrap(),
            scenario,
        )
        .expect("record executed child case");
        return;
    }

    let directory = ProxyEnvTestDirectory(std::env::temp_dir().join(format!(
        "cockpit-proxy-env-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    )));
    std::fs::create_dir(&directory.0).expect("create isolated proxy environment");
    let mut command = crate::modules::test_support::isolated_test_process();
    let status = command
        .env("HOME", &directory.0)
        .env("USERPROFILE", &directory.0)
        .env("CODEX_HOME", directory.0.join("codex-home"))
        .env("COCKPIT_TOOLS_DATA_DIR", directory.0.join("data"))
        .env("COCKPIT_TOOLS_TEST_DATA_DIR", directory.0.join("data"))
        .env("COCKPIT_TEST_DATA_DIR", directory.0.join("data"))
        .env("TMPDIR", &directory.0)
        .env("TMP", &directory.0)
        .env("TEMP", &directory.0)
        .env(SCENARIO, scenario)
        .env(
            "COCKPIT_TEST_PROXY_ENV_RESULT",
            directory.0.join("verified"),
        )
        .args([
            "--exact",
            &format!("modules::config::tests::{test_name}"),
            "--nocapture",
        ])
        .status()
        .expect("run isolated proxy environment case");
    assert!(
        status.success(),
        "fresh-process proxy case failed: {scenario}"
    );
    assert_eq!(
        std::fs::read_to_string(directory.0.join("verified")).unwrap(),
        scenario,
        "the exact child test must execute rather than silently matching no tests"
    );
}

fn check_proxy_env_restore_scenario(scenario: &str) {
    let keys: Vec<_> = super::MANAGED_PROXY_SET_KEYS
        .iter()
        .chain(super::MANAGED_PROXY_NO_PROXY_KEYS.iter())
        .copied()
        .collect();
    // The child has no inherited credentials or proxy values. Only these
    // synthetic values are captured by the real production OnceLock.
    assert!(keys.iter().all(|key| std::env::var(key).is_err()));
    let original: Vec<(&str, Option<&str>)> = match scenario {
        "disabled" | "enabled" => vec![
            ("HTTP_PROXY", Some("http://127.0.0.1:19001")),
            ("http_proxy", Some("http://127.0.0.1:19002")),
            ("HTTPS_PROXY", Some("http://127.0.0.1:19003")),
            ("https_proxy", Some("http://127.0.0.1:19004")),
            ("ALL_PROXY", Some("socks5://127.0.0.1:19005")),
            ("all_proxy", Some("socks5://127.0.0.1:19006")),
            ("NO_PROXY", Some("*")),
            ("no_proxy", Some("*")),
        ],
        "empty-values" => keys
            .iter()
            .enumerate()
            .map(|(index, key)| (*key, (index % 2 == 0).then_some("")))
            .collect(),
        "absent" => keys.iter().map(|key| (*key, None)).collect(),
        _ => panic!("unknown synthetic proxy environment scenario"),
    };
    // Windows aliases differently cased names to one variable. Keep all name
    // assertions, but give each alias the same synthetic startup value.
    #[cfg(windows)]
    let original: Vec<_> = original
        .iter()
        .map(|(key, _)| {
            let value = original
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
                .unwrap()
                .1;
            (*key, value)
        })
        .collect();
    for (key, value) in &original {
        if let Some(value) = value {
            std::env::set_var(key, value);
        }
    }
    let assert_original = || {
        for (key, value) in &original {
            assert_eq!(std::env::var(key).ok().as_deref(), *value, "{key}");
        }
    };

    let disabled = super::UserConfig {
        global_proxy_enabled: false,
        ..super::UserConfig::default()
    };
    let enabled = super::UserConfig {
        global_proxy_enabled: true,
        global_proxy_url: "http://127.0.0.1:19009".into(),
        global_proxy_no_proxy: "example.test".into(),
        ..super::UserConfig::default()
    };
    if scenario == "disabled" || scenario == "absent" {
        super::sync_global_proxy_env(&disabled);
        assert_original();
    }
    // Includes initially enabled and initially empty/absent environments.
    super::sync_global_proxy_env(&enabled);
    for key in super::MANAGED_PROXY_SET_KEYS {
        assert_eq!(std::env::var(key).unwrap(), enabled.global_proxy_url);
    }
    let bypass = crate::modules::codex_protocol::merge_local_no_proxy("example.test");
    for key in super::MANAGED_PROXY_NO_PROXY_KEYS {
        assert_eq!(std::env::var(key).unwrap(), bypass);
    }
    super::sync_global_proxy_env(&disabled);
    assert_original();

    // Later pollution must neither alter the snapshot nor survive restoration.
    for key in &keys {
        std::env::set_var(key, "synthetic-pollution");
    }
    std::env::remove_var("NO_PROXY");
    super::sync_global_proxy_env(&disabled);
    assert_original();
}

#[test]
fn proxy_env_fresh_disabled_preserves_startup_values() {
    check_proxy_env_in_fresh_process(
        "proxy_env_fresh_disabled_preserves_startup_values",
        "disabled",
    );
}

#[test]
fn proxy_env_fresh_enabled_restores_startup_values() {
    check_proxy_env_in_fresh_process("proxy_env_fresh_enabled_restores_startup_values", "enabled");
}

#[test]
fn proxy_env_fresh_absent_environment_remains_absent() {
    check_proxy_env_in_fresh_process(
        "proxy_env_fresh_absent_environment_remains_absent",
        "absent",
    );
}

#[test]
fn proxy_env_fresh_empty_values_remain_distinct_from_absent() {
    check_proxy_env_in_fresh_process(
        "proxy_env_fresh_empty_values_remain_distinct_from_absent",
        "empty-values",
    );
}
