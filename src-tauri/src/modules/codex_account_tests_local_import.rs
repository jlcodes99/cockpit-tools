// Codex 账号测试：多开实例「获取本地账号」的 profile 目录来源。
// 测试与生产实现共享 super 作用域，验证本机凭据确实按传入目录读取。

/// 本机导入必须读取传入的 profile 目录，而不是固定读取默认 CODEX_HOME。
#[test]
fn import_from_local_at_reads_credentials_from_the_given_profile_dir() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-local-import-profile-dir-test");

    // 多开实例的凭据只写在实例自己的 profile 目录里。
    let instance_dir = env.home_dir.join("instances").join("codex").join("work");
    std::fs::create_dir_all(&instance_dir).expect("create instance profile dir");
    std::fs::write(
        instance_dir.join("auth.json"),
        serde_json::json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "sk-instance-profile-key",
        })
        .to_string(),
    )
    .expect("write instance auth.json");

    let account =
        super::import_from_local_at(&instance_dir).expect("import account from instance profile");
    assert!(account.is_api_key_auth());
    assert_eq!(
        account.openai_api_key.as_deref(),
        Some("sk-instance-profile-key")
    );

    // 默认 CODEX_HOME 中没有凭据：失败信息应指向默认目录，证明读取来源由参数决定。
    let default_home_error = super::import_from_local_at(&env.codex_home())
        .expect_err("default codex home has no importable credentials");
    assert!(default_home_error.contains(&env.codex_home().display().to_string()));
}
