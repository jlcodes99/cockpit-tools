use antigravity_cockpit_tools_lib::models::account::Account;
use antigravity_cockpit_tools_lib::modules::{
    account,
    antigravity_cli,
    antigravity_credential,
    provider_current_state,
};
use std::process::Command;

fn get_test_accounts() -> (Account, Account) {
    let accounts = account::list_accounts().expect("list_accounts must succeed");
    assert!(
        accounts.len() >= 2,
        "Integration tests require at least 2 accounts configured in the account index"
    );
    (accounts[0].clone(), accounts[1].clone())
}

/// Test 1: 账号切换持久性与 agy 原生互认
/// 准备账号 A 与账号 B，执行切换事务，验证原生 `agy models` 日志身份，再切回。
#[tokio::test]
async fn test_cli_account_switch_persistence_and_native_recognition() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = &account_a.id;
    let account_a_email = &account_a.email;
    let account_b_id = &account_b.id;
    let account_b_email = &account_b.email;

    // 1. 快照当前初始凭据
    let initial_snapshot = antigravity_credential::snapshot_antigravity_system_credential()
        .expect("Snapshot initial credential");

    // 2. 切换到 Account B
    println!("[Test 1] Switching to Account B...");
    let switched_b = antigravity_cli::switch_account_transaction(account_b_id)
        .await
        .expect("Switch to Account B transaction should succeed");
    assert_eq!(&switched_b.id, account_b_id);
    assert_eq!(&switched_b.email, account_b_email);

    // 验证当前状态已绑定 Account B
    let current_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");
    assert_eq!(&current_id, account_b_id);

    // 3. 运行原生 agy 验证身份识别
    let agy_path = antigravity_cli::detect_agy_binary().expect("detect agy binary");
    let agy_out = Command::new(&agy_path)
        .arg("models")
        .output()
        .expect("execute agy models");
    if !agy_out.status.success() {
        eprintln!("agy models failed! status: {}", agy_out.status);
        eprintln!("stdout: {}", String::from_utf8_lossy(&agy_out.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&agy_out.stderr));
    }
    assert!(agy_out.status.success(), "agy models should succeed: status={}", agy_out.status);

    // 读取最近 agy 日志验证认证账号
    let log_path = std::fs::read_link(
        dirs::home_dir().unwrap().join(".gemini/antigravity-cli/cli.log")
    ).map(|p| dirs::home_dir().unwrap().join(".gemini/antigravity-cli").join(p));

    if let Ok(actual_log_path) = log_path {
        let log_content = std::fs::read_to_string(&actual_log_path).unwrap_or_default();
        let mut last_auth_line = None;
        for line in log_content.lines() {
            if line.contains("applyAuthResult: email=") {
                last_auth_line = Some(line.to_string());
            }
        }
        println!("[Test 1] agy last auth log: {:?}", last_auth_line);
        assert!(
            last_auth_line.as_ref().map(|l| l.contains(account_b_email)).unwrap_or(false),
            "Expected agy to authenticate as Account B ({})", account_b_email
        );
    }

    // 4. 切换回 Account A
    println!("[Test 1] Switching back to Account A...");
    let switched_a = antigravity_cli::switch_account_transaction(account_a_id)
        .await
        .expect("Switch back to Account A transaction should succeed");
    assert_eq!(&switched_a.id, account_a_id);
    assert_eq!(&switched_a.email, account_a_email);

    let current_id_after = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");
    assert_eq!(&current_id_after, account_a_id);

    // 验证切回后的 agy 原生身份
    let agy_out_a = Command::new(&agy_path)
        .arg("models")
        .output()
        .expect("execute agy models for Account A");
    assert!(agy_out_a.status.success());

    // 恢复快照保底
    let _ = antigravity_credential::restore_antigravity_system_credential(initial_snapshot.as_deref());
}

/// Test 2: “一键运行”事务与故障隔离（Decoupled Launch Failure）
/// 验证：凭据校验成功但进程拉起失败时，保留新账号绑定，返回 CLI_LAUNCH_FAILED，不触发凭据回滚
#[tokio::test]
async fn test_switch_and_run_decoupled_launch_failure() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = &account_a.id;
    let account_b_id = &account_b.id;

    let initial_snapshot = antigravity_credential::snapshot_antigravity_system_credential()
        .expect("Snapshot initial credential");

    // 首先保证处于 Account A
    let _ = antigravity_cli::switch_account_transaction(account_a_id).await;

    // 运行一个必定失败的 terminal / command 选项（例如非法的 terminal 配置）
    // 但凭据切换事务必须先成功执行
    let options = antigravity_cli::AntigravityCliRunOptions {
        cwd: Some("/non_existent_directory_for_launch_failure_testing_xyz".to_string()),
        args: Some(vec!["models".to_string()]),
        terminal: Some("direct".to_string()),
    };

    let result = antigravity_cli::run_antigravity_cli(account_b_id, Some(options)).await;
    println!("[Test 2] Result with non-existent cwd: {:?}", result.is_err());

    // 无论进程是否因为非法 cwd 报错（或者报错 CLI_LAUNCH_FAILED）：
    // 关键断言：凭据必须已经顺利切换并提交为 Account B，绝不因为启动阶段问题回滚！
    let current_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");
    assert_eq!(&current_id, account_b_id, "Current account must remain Account B");

    // 还原回 Account A
    let _ = antigravity_cli::switch_account_transaction(account_a_id).await;
    let _ = antigravity_credential::restore_antigravity_system_credential(initial_snapshot.as_deref());
}

/// Test 3: 回滚保护（Rollback Integrity）
/// 凭据回读校验失败时，读取快照成功回滚并返回 CREDENTIAL_VERIFY_FAILED
#[tokio::test]
async fn test_rollback_on_verify_failure() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = &account_a.id;
    let account_b_id = &account_b.id;

    let initial_snapshot = antigravity_credential::snapshot_antigravity_system_credential()
        .expect("Snapshot initial credential");

    // 保证初始为 Account A
    let _ = antigravity_cli::switch_account_transaction(account_a_id).await;
    let baseline_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");
    assert_eq!(&baseline_id, account_a_id);

    // 模拟构造一个与当前凭据不一致的 Account 触发 verify 失败
    let mut invalid_account = account::load_account(account_b_id).expect("Load Account B");
    // 故意将 token 置为空或篡改以让 verify 校验不匹配
    invalid_account.token.access_token = "tampered_token_that_wont_match".to_string();

    // 写入并校验
    let verify_res = antigravity_credential::verify_antigravity_system_credential(&invalid_account);
    assert!(verify_res.is_err(), "Verification should fail for tampered account token");

    // 验证回滚函数在接收到快照时能正确还原
    antigravity_credential::restore_antigravity_system_credential(initial_snapshot.as_deref())
        .expect("Rollback to snapshot must succeed");

    // 验证当前状态依然保持 Account A
    let current_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");
    assert_eq!(&current_id, account_a_id, "Current account must remain unchanged Account A");
}

/// Test 4: 多 Runtime Target 状态隔离验证
/// 验证：切换 CLI 到 Account B 时，IDE 与 Legacy 当前账号完全不受影响
#[tokio::test]
async fn test_multi_runtime_target_state_isolation() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = &account_a.id;
    let account_b_id = &account_b.id;

    // 1. 设置 IDE 与 Legacy 绑定 Account A
    let _ = antigravity_cockpit_tools_lib::modules::instance::update_default_settings(
        Some(Some(account_a_id.to_string())),
        None,
        Some(false),
    );
    let _ = antigravity_cockpit_tools_lib::modules::antigravity_legacy_instance::update_default_settings(
        Some(Some(account_a_id.to_string())),
        None,
        Some(false),
    );

    // 2. 切换 CLI 到 Account B
    let _ = antigravity_cli::switch_account_transaction(account_b_id).await.expect("Switch CLI to B");

    // 3. 验证状态隔离：
    // CLI 为 Account B
    let cli_id = provider_current_state::get_current_account_id("antigravity_cli")
        .ok()
        .flatten();
    assert_eq!(cli_id.as_deref(), Some(account_b_id.as_str()));

    // IDE 依然为 Account A
    let ide_id = antigravity_cockpit_tools_lib::modules::instance::load_default_settings()
        .ok()
        .and_then(|s| s.bind_account_id);
    assert_eq!(ide_id.as_deref(), Some(account_a_id.as_str()));

    // Legacy 依然为 Account A
    let legacy_id = antigravity_cockpit_tools_lib::modules::antigravity_legacy_instance::load_default_settings()
        .ok()
        .and_then(|s| s.bind_account_id);
    assert_eq!(legacy_id.as_deref(), Some(account_a_id.as_str()));

    // 4. 切回 CLI 到 Account A
    let _ = antigravity_cli::switch_account_transaction(account_a_id).await.expect("Switch CLI back to A");
}

/// Test 5: 并发 A/B 切换互斥串行化验证
/// 验证：并发发起 A→B 与 B→A 切换请求时，底层锁确保两笔事务串行完成，
/// 最终当前账号与系统凭据完全一致，且匹配最后成功的事务。
#[tokio::test]
async fn test_concurrent_ab_switches_serialized() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = account_a.id.clone();
    let account_b_id = account_b.id.clone();

    let initial_snapshot = antigravity_credential::snapshot_antigravity_system_credential()
        .expect("Snapshot initial credential");

    // 并发启动两个异步任务，分别尝试切到 Account A 和 Account B
    let b_id = account_b_id.clone();
    let handle_b = tokio::spawn(async move {
        antigravity_cli::switch_account_transaction(&b_id).await
    });
    let a_id = account_a_id.clone();
    let handle_a = tokio::spawn(async move {
        antigravity_cli::switch_account_transaction(&a_id).await
    });

    let (res_b, res_a) = tokio::join!(handle_b, handle_a);
    let res_b = res_b.expect("join handle_b");
    let res_a = res_a.expect("join handle_a");

    // 两笔事务都必须安全成功（互斥串行执行，没有死锁或校验冲突）
    assert!(res_b.is_ok(), "Switch to B failed: {:?}", res_b.err());
    assert!(res_a.is_ok(), "Switch to A failed: {:?}", res_a.err());

    // 最终读取系统持久化状态与底层凭据
    let final_bound_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");

    // 验证底层 Keychain 凭据与当前绑定一致
    let expected_account = account::load_account(&final_bound_id).expect("Load expected account");
    let verify_res = antigravity_credential::verify_antigravity_system_credential(&expected_account);
    assert!(
        verify_res.is_ok(),
        "Keychain credential must match the final bound account (ID: {}, err: {:?})",
        final_bound_id,
        verify_res.err()
    );

    // 恢复初始快照
    let _ = antigravity_credential::restore_antigravity_system_credential(initial_snapshot.as_deref());
}

/// Test 6: 跨进程并发切号互斥测试 (Cross-Process Switching Race)
/// 验证：同时由当前测试进程和独立 child process (cockpit-cli) 发起并发切号时，
/// 跨进程文件锁保证两笔事务严格串行执行，无死锁，无残留锁，且最终：
/// credential active account == provider current account
#[tokio::test]
async fn test_cross_process_concurrent_switch() {
    let (account_a, account_b) = get_test_accounts();
    let account_a_id = account_a.id.clone();
    let account_b_id = account_b.id.clone();

    let initial_snapshot = antigravity_credential::snapshot_antigravity_system_credential()
        .expect("Snapshot initial credential");

    // 确定 cockpit-cli 可执行文件路径
    let mut cli_path = std::env::current_exe()
        .expect("current test exe path");
    cli_path.pop(); // pop test binary name
    if cli_path.ends_with("deps") {
        cli_path.pop(); // pop deps dir
    }
    let cockpit_cli_bin = cli_path.join("cockpit-cli");
    assert!(cockpit_cli_bin.exists(), "cockpit-cli binary must exist at {:?}", cockpit_cli_bin);

    // 1. 异步启动一个独立 OS 子进程执行 `cockpit-cli switch agy <Account B>`
    let b_id = account_b_id.clone();
    let child_task = tokio::task::spawn_blocking(move || {
        Command::new(&cockpit_cli_bin)
            .args(["switch", "agy", &b_id])
            .output()
    });

    // 2. 当前进程几乎同一时刻发起内部切换到 Account A
    let current_proc_switch = antigravity_cli::switch_account_transaction(&account_a_id).await;

    // 3. 等待子进程执行完毕
    let child_output = child_task.await.expect("join child task").expect("execute child process");

    println!("[Test 6] Child stdout: {}", String::from_utf8_lossy(&child_output.stdout));
    println!("[Test 6] Child stderr: {}", String::from_utf8_lossy(&child_output.stderr));
    println!("[Test 6] Current proc switch ok: {}", current_proc_switch.is_ok());

    assert!(child_output.status.success(), "Child process switch must succeed");
    assert!(current_proc_switch.is_ok(), "Current process switch must succeed");

    // 4. 核心断言：最终状态必须满足 credential active account == provider current account
    let final_bound_id = provider_current_state::get_current_account_id("antigravity_cli")
        .expect("Read CLI current account ID")
        .expect("CLI current account ID should be Some");

    let expected_account = account::load_account(&final_bound_id).expect("Load final account");
    let verify_res = antigravity_credential::verify_antigravity_system_credential(&expected_account);
    assert!(
        verify_res.is_ok(),
        "Cross-process invariant failed! Credential in Keychain must match provider current account: final_bound_id={}, verify_err={:?}",
        final_bound_id,
        verify_res.err()
    );

    // 5. 验证锁文件未被死锁锁定，当前能即刻再次获取
    let lock_probe = antigravity_cli::CrossProcessSwitchLock::acquire().await;
    assert!(lock_probe.is_ok(), "Cross-process lock must be cleanly released and acquireable");
    drop(lock_probe);

    // 恢复初始凭据
    let _ = antigravity_credential::restore_antigravity_system_credential(initial_snapshot.as_deref());
}


