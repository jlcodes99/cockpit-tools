// Claude 账号模块：Desktop authentication export, session validation and login helpers。
// 通过 include! 保持原 modules::claude_account 作用域和私有调用关系。
#[cfg(test)]
mod electron_runtime_tests {
    use super::{electron_runtime_download_url, test_electron_runtime_asset_for};

    #[test]
    fn electron_runtime_assets_are_pinned_with_sha256() {
        for (os, arch, platform_key) in [
            ("macos", "aarch64", "darwin-arm64"),
            ("macos", "x86_64", "darwin-x64"),
            ("windows", "x86_64", "win32-x64"),
            ("windows", "aarch64", "win32-arm64"),
            ("linux", "x86_64", "linux-x64"),
            ("linux", "aarch64", "linux-arm64"),
        ] {
            let asset = test_electron_runtime_asset_for(os, arch).expect("asset should exist");
            assert_eq!(asset.platform_key, platform_key);
            assert!(asset.file_name.starts_with("electron-v42.4.0-"));
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.sha256.chars().all(|ch| ch.is_ascii_hexdigit()));
            assert!(electron_runtime_download_url(&asset).contains(asset.file_name));
        }
    }
}

#[cfg(target_os = "windows")]
fn electron_cli_path_arg(path: &Path) -> String {
    let value = path.display().to_string();
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{}", rest))
        .or_else(|| value.strip_prefix(r"\\?\").map(|rest| rest.to_string()))
        .unwrap_or(value)
}

#[cfg(not(target_os = "windows"))]
fn electron_cli_path_arg(path: &Path) -> String {
    path.display().to_string()
}

fn launch_platform_desktop_auth_helper(
    user_data_dir: &Path,
    status_file: &Path,
    export_file: &Path,
    mode: &str,
) -> Result<u32, String> {
    launch_platform_desktop_auth_helper_with_args(
        user_data_dir,
        status_file,
        export_file,
        mode,
        &[],
    )
}

fn launch_platform_desktop_auth_helper_with_args(
    user_data_dir: &Path,
    status_file: &Path,
    export_file: &Path,
    mode: &str,
    extra_args: &[(&str, &Path)],
) -> Result<u32, String> {
    launch_platform_desktop_auth_helper_with_args_and_progress(
        user_data_dir,
        status_file,
        export_file,
        mode,
        extra_args,
        None,
    )
}

fn launch_platform_desktop_auth_helper_with_progress(
    user_data_dir: &Path,
    status_file: &Path,
    export_file: &Path,
    mode: &str,
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<u32, String> {
    launch_platform_desktop_auth_helper_with_args_and_progress(
        user_data_dir,
        status_file,
        export_file,
        mode,
        &[],
        progress,
    )
}

fn launch_platform_desktop_auth_helper_with_args_and_progress(
    user_data_dir: &Path,
    status_file: &Path,
    export_file: &Path,
    mode: &str,
    extra_args: &[(&str, &Path)],
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<u32, String> {
    emit_desktop_login_progress(progress, "resolve-runtime", Some(2.0), None, None);
    let helper_script = find_desktop_auth_helper_script()?;
    let electron = find_electron_executable_for_desktop_auth(progress)?;
    let helper_script_arg = electron_cli_path_arg(&helper_script);
    let stdout_log = user_data_dir.join("claude_desktop_auth_helper.stdout.log");
    let stderr_log = user_data_dir.join("claude_desktop_auth_helper.stderr.log");
    let mut command = std::process::Command::new(electron);
    logger::log_info(&format!(
        "[Claude Auth] 启动 helper: script={}, mode={}, user_data_dir={}, status_file={}, export_file={}",
        helper_script_arg,
        mode,
        user_data_dir.display(),
        status_file.display(),
        export_file.display()
    ));
    command
        .arg(&helper_script_arg)
        .arg("--user-data-dir")
        .arg(user_data_dir)
        .arg("--status-file")
        .arg(status_file)
        .arg("--export-file")
        .arg(export_file)
        .arg("--mode")
        .arg(mode)
        .arg("--probe-timeout-ms")
        .arg("15000")
        .env("ELECTRON_DISABLE_SECURITY_WARNINGS", "true")
        .stdin(std::process::Stdio::null())
        .stdout(
            fs::File::create(&stdout_log)
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null()),
        )
        .stderr(
            fs::File::create(&stderr_log)
                .map(std::process::Stdio::from)
                .unwrap_or_else(|_| std::process::Stdio::null()),
        );
    for (name, path) in extra_args {
        command.arg(name).arg(path);
    }
    emit_desktop_login_progress(progress, "launch", Some(92.0), None, None);
    command
        .arg("--url")
        .arg(if mode == "cookie_probe" || mode == "verify" {
            "https://claude.ai/settings/usage"
        } else {
            "https://claude.ai/"
        });
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 Claude 授权窗口失败: {}", e))?;
    let child_id = child.id();
    std::thread::sleep(Duration::from_millis(300));
    if let Some(status) = child
        .try_wait()
        .map_err(|e| format!("检查 Claude 授权窗口进程失败: {}", e))?
    {
        let stderr = fs::read_to_string(&stderr_log).unwrap_or_default();
        let stdout = fs::read_to_string(&stdout_log).unwrap_or_default();
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "Claude 授权窗口启动后立即退出: {}{}",
            status,
            if detail.is_empty() {
                "".to_string()
            } else {
                format!("；日志: {}", detail)
            }
        ));
    }
    logger::log_info(&format!(
        "[Claude Auth] helper 已启动: pid={}, stdout={}, stderr={}",
        child_id,
        stdout_log.display(),
        stderr_log.display()
    ));
    emit_desktop_login_progress(progress, "ready", Some(100.0), None, None);
    Ok(child_id)
}

#[cfg(target_os = "macos")]
fn terminate_desktop_auth_helper(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

#[cfg(target_os = "windows")]
fn terminate_desktop_auth_helper(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn terminate_desktop_auth_helper(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

#[cfg(target_os = "macos")]
fn is_claude_desktop_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "Claude"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn collect_claude_desktop_main_pids() -> Vec<u32> {
    let output = std::process::Command::new("pgrep")
        .args(["-x", "Claude"])
        .output();
    let mut pids = output
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| {
            stdout
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .filter(|pid| *pid != 0)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(target_os = "macos")]
fn force_kill_claude_desktop_main_processes() {
    let pids = collect_claude_desktop_main_pids();
    if pids.is_empty() {
        return;
    }
    logger::log_warn(&format!(
        "[Claude] force killing main processes before profile write: {}",
        crate::modules::process::summarize_pid_list_for_log(&pids)
    ));
    for pid in pids {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}

#[cfg(target_os = "windows")]
fn is_claude_desktop_running() -> bool {
    crate::modules::claude_instance::resolve_claude_pid(None, None).is_some()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn is_claude_desktop_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "claude"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn quit_claude_desktop_for_profile_write() -> Result<(), String> {
    if !is_claude_desktop_running() {
        return Ok(());
    }
    logger::log_info("[Claude] closing Claude before profile write");
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "tell application id \"{}\" to quit",
                CLAUDE_DESKTOP_BUNDLE_ID_MACOS
            ),
        ])
        .output();
    for _ in 0..40 {
        if !is_claude_desktop_running() {
            std::thread::sleep(std::time::Duration::from_millis(300));
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    if let Ok(target_dir) = get_default_claude_desktop_user_data_dir() {
        let target_dir = target_dir.to_string_lossy().to_string();
        if let Err(error) = crate::modules::claude_instance::close_claude(&[target_dir], 8) {
            logger::log_warn(&format!(
                "[Claude] managed close before profile write failed: {}",
                error
            ));
        }
    }
    for _ in 0..24 {
        if !is_claude_desktop_running() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    force_kill_claude_desktop_main_processes();
    for _ in 0..20 {
        if !is_claude_desktop_running() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    Err("Claude 仍在运行，无法安全写入登录态。请先退出 Claude 后重试。".to_string())
}

#[cfg(target_os = "windows")]
fn quit_claude_desktop_for_profile_write() -> Result<(), String> {
    let target_dir = get_default_claude_desktop_user_data_dir()?
        .to_string_lossy()
        .to_string();
    logger::log_info("[Claude] closing configured Claude Desktop before profile write");
    crate::modules::claude_instance::close_claude(&[target_dir], 8)?;
    if is_claude_desktop_running() {
        return Err("Claude is still running, cannot safely write login state. Please quit Claude and retry.".to_string());
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn quit_claude_desktop_for_profile_write() -> Result<(), String> {
    if !is_claude_desktop_running() {
        return Ok(());
    }
    Err("Claude 仍在运行，无法安全写入登录态。请先退出 Claude 后重试。".to_string())
}

#[cfg(target_os = "macos")]
fn launch_default_claude_desktop() -> Result<(), String> {
    std::process::Command::new("open")
        .args(["-b", CLAUDE_DESKTOP_BUNDLE_ID_MACOS])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch Claude Desktop: {}", e))
}

#[cfg(target_os = "windows")]
fn launch_default_claude_desktop() -> Result<(), String> {
    let pid = crate::modules::claude_instance::start_claude_default_with_args_with_new_window(
        &[],
        false,
    )?;
    logger::log_info(&format!(
        "[Claude] launched configured Claude Desktop pid={}",
        pid
    ));
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn launch_default_claude_desktop() -> Result<(), String> {
    Err("APP_PATH_NOT_FOUND:claude".to_string())
}

fn import_desktop_profile_snapshot(
    source_dir: &Path,
    account_name: Option<&str>,
    source: &str,
) -> Result<ClaudeAccount, String> {
    ensure_desktop_profile_logged_in(source_dir)?;
    let label = desktop_account_display_name(account_name);
    let id = build_desktop_account_id(&label);
    let snapshot_dir = get_desktop_profiles_dir()?.join(&id);
    let metadata = desktop_profile_metadata(source_dir, source)?;
    remove_path_if_exists(&snapshot_dir)?;
    copy_desktop_profile_snapshot(source_dir, &snapshot_dir)?;
    if !desktop_profile_has_valid_cookies(&snapshot_dir) {
        let _ = remove_path_if_exists(&snapshot_dir);
        return Err(format!(
            "Claude profile 快照保存失败，未找到有效登录态: {}",
            snapshot_dir.display()
        ));
    }

    let now = now_ts_ms();
    let desktop_profile = desktop_profile_metadata_json(&metadata, &snapshot_dir, now);
    let mut account = ClaudeAccount {
        id: id.clone(),
        email: label,
        auth_mode: ClaudeAuthMode::DesktopOAuth,
        account_uuid: None,
        organization_uuid: metadata.last_active_org.clone(),
        organization_name: None,
        plan_type: None,
        avatar_url: None,
        profile_updated_at: None,
        quota: None,
        quota_error: None,
        usage_updated_at: None,
        status: None,
        status_reason: None,
        api_key: None,
        api_base_url: None,
        api_provider_id: None,
        api_provider_name: None,
        api_provider_source_tag: None,
        api_provider_website: None,
        api_provider_api_key_url: None,
        api_key_field: None,
        api_model_catalog: None,
        api_extra_env: None,
        desktop_gateway_auth_scheme: None,
        desktop_gateway_credential_kind: None,
        desktop_gateway_config_id: None,
        desktop_gateway_profile_dir: None,
        desktop_gateway_models: None,
        desktop_gateway_connection_mode: None,
        desktop_gateway_upstream_models: None,
        desktop_gateway_model_mappings: None,
        desktop_profile_dir: Some(snapshot_dir.to_string_lossy().to_string()),
        desktop_profile_imported_at: Some(now),
        claude_credentials_raw: Some(json!({
            "authMode": "desktop_oauth",
            "profileSnapshot": true,
            "source": metadata.source,
        })),
        claude_config_raw: Some(json!({
            "desktopProfile": desktop_profile
        })),
        claude_usage_raw: None,
        tags: None,
        account_note: None,
        created_at: now,
        last_used: now,
    };
    let local_profile_applied = apply_desktop_local_profile(&mut account, &snapshot_dir);
    let mut profile_error = None;
    let web_profile =
        metadata
            .web_profile
            .clone()
            .or_else(|| match probe_desktop_web_profile(&snapshot_dir) {
                Ok(profile) => Some(profile),
                Err(error) => {
                    logger::log_warn(&format!(
                        "[Claude] 导入后自动刷新账号资料失败，已保留本地快照: {}",
                        error
                    ));
                    profile_error = Some(format!("Claude 资料刷新失败: {}", error));
                    None
                }
            });
    if let Some(web_profile) = web_profile.as_ref() {
        if apply_desktop_web_profile(&mut account, web_profile) {
            account.status_reason = None;
        } else {
            account.status_reason =
                if local_profile_applied || desktop_account_has_real_profile_data(&account) {
                    None
                } else {
                    desktop_web_profile_error_message(web_profile)
                        .or_else(|| Some("Claude 资料接口未返回邮箱、头像或套餐字段。".to_string()))
                };
        }
    } else if profile_error.is_some()
        && !local_profile_applied
        && !desktop_account_has_real_profile_data(&account)
    {
        account.status_reason = profile_error;
    }
    save_account_and_index(account)
}

pub fn import_cli_from_local() -> Result<ClaudeAccount, String> {
    let config_dir = get_default_claude_code_config_dir()?;
    let credentials_raw = read_claude_code_credentials(&config_dir);
    if credentials_oauth(&credentials_raw).is_none() {
        return Err(
            "未找到本机 Claude Code 登录信息，请先在 Claude Code 完成 OAuth 登录。".to_string(),
        );
    }

    let config_path = get_claude_code_global_config_path(&config_dir)?;
    let config_raw = read_config_file(&config_path)?
        .ok_or_else(|| format!("未找到本机 Claude Code 配置文件: {}", config_path.display()))?;
    if config_oauth_account(&config_raw).is_none() {
        return Err(
            "本机 Claude Code 配置缺少 oauthAccount，请先在 Claude Code 完成登录。".to_string(),
        );
    }

    upsert_account_from_snapshots(credentials_raw, config_raw)
}

pub fn sync_cli_account_from_config_dir_if_same(
    account_id: &str,
    config_dir: &Path,
) -> Result<Option<ClaudeAccount>, String> {
    let existing = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if existing.auth_mode == ClaudeAuthMode::DesktopOAuth {
        return Ok(None);
    }

    let credentials_raw = read_claude_code_credentials(config_dir);
    if credentials_oauth(&credentials_raw).is_none() {
        return Ok(None);
    }
    let config_path = get_claude_code_global_config_path(config_dir)?;
    let Some(config_raw) = read_config_file(&config_path)? else {
        return Ok(None);
    };
    if config_oauth_account(&config_raw).is_none() {
        return Ok(None);
    }

    let incoming =
        derive_account_from_snapshots(credentials_raw, config_raw, Some(existing.clone()))?;
    if !cli_accounts_same_identity(&existing, &incoming) {
        logger::log_warn(&format!(
            "[Claude CLI] 跳过实例登录态同步：绑定账号与实例目录账号不一致，bind_id={}, config_dir={}",
            account_id,
            config_dir.display()
        ));
        return Ok(None);
    }

    if incoming.claude_credentials_raw == existing.claude_credentials_raw
        && incoming.claude_config_raw == existing.claude_config_raw
    {
        return Ok(Some(existing));
    }

    logger::log_info(&format!(
        "[Claude CLI] 同步实例目录登录态到账号快照: account_id={}, config_dir={}",
        account_id,
        config_dir.display()
    ));
    save_account_and_index(incoming).map(Some)
}

pub fn start_desktop_login(
    app: Option<AppHandle>,
    progress_id: Option<String>,
) -> Result<ClaudeDesktopLoginStartResponse, String> {
    let progress = app
        .zip(progress_id.and_then(|value| normalize_non_empty(Some(&value))))
        .map(|(app, progress_id)| ClaudeDesktopLoginProgressContext { app, progress_id });
    emit_desktop_login_progress(progress.as_ref(), "start", Some(0.0), None, None);
    let _ = cancel_desktop_login(None);
    let login_id = generate_random_url_token(18);
    let user_data_dir = get_desktop_login_root_dir()?.join(&login_id);
    let status_file = user_data_dir.join(CLAUDE_DESKTOP_AUTH_STATUS_FILE);
    let export_file = user_data_dir.join(CLAUDE_DESKTOP_AUTH_EXPORT_FILE);
    remove_path_if_exists(&user_data_dir)?;
    fs::create_dir_all(&user_data_dir)
        .map_err(|e| format!("创建 Claude 登录 profile 失败: {}", e))?;
    emit_desktop_login_progress(progress.as_ref(), "profile", Some(1.0), None, None);
    let helper_pid = launch_platform_desktop_auth_helper_with_progress(
        &user_data_dir,
        &status_file,
        &export_file,
        "auth",
        progress.as_ref(),
    )?;
    let pending = PendingClaudeDesktopLoginState {
        login_id,
        user_data_dir,
        status_file,
        export_file,
        helper_pid: Some(helper_pid),
        expires_at: now_ts() + CLAUDE_DESKTOP_LOGIN_TIMEOUT_SECONDS,
        cancelled: false,
    };
    set_pending_desktop_login(Some(pending.clone()));
    Ok(to_desktop_login_start_response(&pending))
}

pub fn complete_desktop_login(
    login_id: &str,
    account_name: Option<&str>,
) -> Result<ClaudeAccount, String> {
    let pending = get_pending_desktop_login_for(login_id)?;
    let export = wait_for_desktop_auth_export_logged_in(&pending.user_data_dir)?;
    terminate_desktop_auth_helper(pending.helper_pid);
    rewrite_desktop_cookies_with_exported_plaintext(&pending.user_data_dir, &export)?;
    let account =
        import_desktop_profile_snapshot(&pending.user_data_dir, account_name, "platform_login")?;
    clear_pending_desktop_login_if_matches(login_id);
    let _ = remove_path_if_exists(&pending.user_data_dir);
    Ok(account)
}

pub fn cancel_desktop_login(login_id: Option<&str>) -> Result<(), String> {
    if let Some(login_id) = login_id.and_then(|value| normalize_non_empty(Some(value))) {
        hydrate_pending_desktop_login_if_missing();
        let state = CLAUDE_PENDING_DESKTOP_LOGIN
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .filter(|state| state.login_id == login_id);
        clear_pending_desktop_login_if_matches(&login_id);
        if let Some(state) = state {
            terminate_desktop_auth_helper(state.helper_pid);
            let _ = remove_path_if_exists(&state.user_data_dir);
        }
        return Ok(());
    }
    hydrate_pending_desktop_login_if_missing();
    if let Some(state) = CLAUDE_PENDING_DESKTOP_LOGIN
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    {
        terminate_desktop_auth_helper(state.helper_pid);
        let _ = remove_path_if_exists(&state.user_data_dir);
    }
    set_pending_desktop_login(None);
    Ok(())
}

pub fn open_desktop_verification_window(account_id: &str) -> Result<(), String> {
    let account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if account.auth_mode != ClaudeAuthMode::DesktopOAuth {
        return Err("只有 Claude 登录账号需要打开验证窗口。".to_string());
    }
    let profile_dir = account
        .desktop_profile_dir
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(PathBuf::from)
        .ok_or_else(|| "Claude 账号缺少 profile 快照".to_string())?;
    ensure_desktop_profile_logged_in(&profile_dir)?;
    let status_file = profile_dir.join("claude_desktop_verification_status.json");
    let export_file = desktop_auth_export_path(&profile_dir);
    let _ = remove_path_if_exists(&status_file);
    let helper_pid = launch_platform_desktop_auth_helper_with_args(
        &profile_dir,
        &status_file,
        &export_file,
        "verify",
        &[],
    )?;
    logger::log_info(&format!(
        "[Claude Auth] verification helper 已启动: account_id={}, pid={}",
        account_id, helper_pid
    ));
    Ok(())
}

fn parse_import_item(value: &Value) -> Result<ClaudeAccount, String> {
    if is_desktop_oauth_json_import(value) {
        return Err(
            "Claude 普通登录态依赖本机 profile 快照，不支持 JSON 导入，请重新登录 Desktop 或改用 Claude Gateway。"
                .to_string(),
        );
    }

    if value
        .get("auth_mode")
        .or_else(|| value.get("authMode"))
        .and_then(|item| item.as_str())
        .map(|mode| {
            mode.eq_ignore_ascii_case("desktop_gateway")
                || mode.eq_ignore_ascii_case("desktopGateway")
        })
        .unwrap_or(false)
    {
        if let Some(api_key) = value
            .get("api_key")
            .or_else(|| value.get("apiKey"))
            .or_else(|| value.get("gatewayApiKey"))
            .and_then(|item| item.as_str())
        {
            let provider_value = value
                .get("apiProvider")
                .or_else(|| value.get("api_provider"))
                .or_else(|| {
                    value
                        .get("claude_credentials_raw")
                        .and_then(|item| item.get("apiProvider"))
                })
                .or_else(|| {
                    value
                        .get("claudeCredentialsRaw")
                        .and_then(|item| item.get("apiProvider"))
                });
            let account_name = value
                .get("email")
                .or_else(|| value.get("accountName"))
                .or_else(|| value.get("name"))
                .and_then(|item| item.as_str());
            let api_model_catalog = value
                .get("api_model_catalog")
                .or_else(|| value.get("apiModelCatalog"))
                .or_else(|| provider_value.and_then(|provider| provider.get("modelCatalog")))
                .and_then(|item| item.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                });
            let desktop_gateway_models = value
                .get("desktop_gateway_models")
                .or_else(|| value.get("desktopGatewayModels"))
                .or_else(|| value.get("inferenceModels"))
                .or_else(|| provider_value.and_then(|provider| provider.get("manualModels")))
                .and_then(|item| item.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            item.as_str().map(ToString::to_string).or_else(|| {
                                item.get("name")
                                    .or_else(|| item.get("id"))
                                    .and_then(|value| value.as_str())
                                    .map(ToString::to_string)
                            })
                        })
                        .collect::<Vec<_>>()
                });
            let api_extra_env = value
                .get("api_extra_env")
                .or_else(|| value.get("apiExtraEnv"))
                .or_else(|| provider_value.and_then(|provider| provider.get("extraEnv")))
                .and_then(|item| item.as_object())
                .map(|object| {
                    object
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_string()))
                        })
                        .collect::<BTreeMap<_, _>>()
                });
            let desktop_gateway_connection_mode = value
                .get("desktop_gateway_connection_mode")
                .or_else(|| value.get("desktopGatewayConnectionMode"))
                .or_else(|| value.get("gatewayConnectionMode"))
                .or_else(|| provider_value.and_then(|provider| provider.get("connectionMode")))
                .and_then(|item| item.as_str());
            let desktop_gateway_upstream_models = value
                .get("desktop_gateway_upstream_models")
                .or_else(|| value.get("desktopGatewayUpstreamModels"))
                .or_else(|| value.get("gatewayUpstreamModels"))
                .or_else(|| provider_value.and_then(|provider| provider.get("upstreamModels")))
                .and_then(|item| item.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                });
            let desktop_gateway_model_mappings = value
                .get("desktop_gateway_model_mappings")
                .or_else(|| value.get("desktopGatewayModelMappings"))
                .or_else(|| value.get("gatewayModelMappings"))
                .or_else(|| provider_value.and_then(|provider| provider.get("modelMappings")))
                .or_else(|| value.get("inferenceModels"))
                .and_then(|item| item.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            Some(ClaudeDesktopGatewayModelMapping {
                                desktop_model: item
                                    .get("desktop_model")
                                    .or_else(|| item.get("desktopModel"))
                                    .or_else(|| item.get("name"))
                                    .and_then(Value::as_str)?
                                    .to_string(),
                                upstream_model: item
                                    .get("upstream_model")
                                    .or_else(|| item.get("upstreamModel"))
                                    .or_else(|| item.get("target"))
                                    .or_else(|| item.get("name"))
                                    .or_else(|| item.get("id"))
                                    .and_then(Value::as_str)?
                                    .to_string(),
                                label_override: item
                                    .get("label_override")
                                    .or_else(|| item.get("labelOverride"))
                                    .and_then(Value::as_str)
                                    .and_then(|value| normalize_non_empty(Some(value))),
                                supports_1m: item
                                    .get("supports_1m")
                                    .or_else(|| item.get("supports1m"))
                                    .and_then(Value::as_bool)
                                    .filter(|value| *value),
                            })
                        })
                        .collect::<Vec<_>>()
                });
            return import_desktop_gateway(
                api_key,
                account_name,
                ClaudeApiKeyProviderConfig {
                    api_base_url: value
                        .get("api_base_url")
                        .or_else(|| value.get("apiBaseUrl"))
                        .or_else(|| value.get("inferenceGatewayBaseUrl"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("baseUrl")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_id: value
                        .get("api_provider_id")
                        .or_else(|| value.get("apiProviderId"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("id")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_name: value
                        .get("api_provider_name")
                        .or_else(|| value.get("apiProviderName"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("name")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_source_tag: value
                        .get("api_provider_source_tag")
                        .or_else(|| value.get("apiProviderSourceTag"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("sourceTag")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_website: value
                        .get("api_provider_website")
                        .or_else(|| value.get("apiProviderWebsite"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("website")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_api_key_url: value
                        .get("api_provider_api_key_url")
                        .or_else(|| value.get("apiProviderApiKeyUrl"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("apiKeyUrl")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_key_field: None,
                    api_model_catalog,
                    api_extra_env,
                },
                value
                    .get("desktop_gateway_auth_scheme")
                    .or_else(|| value.get("desktopGatewayAuthScheme"))
                    .or_else(|| value.get("inferenceGatewayAuthScheme"))
                    .or_else(|| provider_value.and_then(|provider| provider.get("authScheme")))
                    .and_then(|item| item.as_str()),
                desktop_gateway_models,
                desktop_gateway_connection_mode,
                desktop_gateway_upstream_models,
                desktop_gateway_model_mappings,
            );
        }
    }

    if value
        .get("auth_mode")
        .or_else(|| value.get("authMode"))
        .and_then(|item| item.as_str())
        .map(|mode| mode.eq_ignore_ascii_case("api_key") || mode.eq_ignore_ascii_case("apikey"))
        .unwrap_or(false)
    {
        if let Some(api_key) = value
            .get("api_key")
            .or_else(|| value.get("apiKey"))
            .or_else(|| value.get("anthropicApiKey"))
            .and_then(|item| item.as_str())
        {
            let account_name = value
                .get("email")
                .or_else(|| value.get("accountName"))
                .or_else(|| value.get("name"))
                .and_then(|item| item.as_str());
            let provider_value = value
                .get("apiProvider")
                .or_else(|| value.get("api_provider"))
                .or_else(|| {
                    value
                        .get("claude_credentials_raw")
                        .and_then(|item| item.get("apiProvider"))
                });
            let api_model_catalog = value
                .get("api_model_catalog")
                .or_else(|| value.get("apiModelCatalog"))
                .or_else(|| provider_value.and_then(|provider| provider.get("modelCatalog")))
                .and_then(|item| item.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                });
            let api_extra_env = value
                .get("api_extra_env")
                .or_else(|| value.get("apiExtraEnv"))
                .or_else(|| provider_value.and_then(|provider| provider.get("extraEnv")))
                .and_then(|item| item.as_object())
                .map(|object| {
                    object
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_string()))
                        })
                        .collect::<BTreeMap<_, _>>()
                });
            return import_api_key(
                api_key,
                account_name,
                ClaudeApiKeyProviderConfig {
                    api_base_url: value
                        .get("api_base_url")
                        .or_else(|| value.get("apiBaseUrl"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("baseUrl")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_id: value
                        .get("api_provider_id")
                        .or_else(|| value.get("apiProviderId"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("id")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_name: value
                        .get("api_provider_name")
                        .or_else(|| value.get("apiProviderName"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("name")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_source_tag: value
                        .get("api_provider_source_tag")
                        .or_else(|| value.get("apiProviderSourceTag"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("sourceTag")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_website: value
                        .get("api_provider_website")
                        .or_else(|| value.get("apiProviderWebsite"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("website")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_provider_api_key_url: value
                        .get("api_provider_api_key_url")
                        .or_else(|| value.get("apiProviderApiKeyUrl"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("apiKeyUrl")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_key_field: value
                        .get("api_key_field")
                        .or_else(|| value.get("apiKeyField"))
                        .or_else(|| provider_value.and_then(|provider| provider.get("keyField")))
                        .and_then(|item| item.as_str())
                        .map(ToString::to_string),
                    api_model_catalog,
                    api_extra_env,
                },
            );
        }
    }

    if let Some(id) = value.get("id").and_then(|item| item.as_str()) {
        if value.get("claude_credentials_raw").is_some()
            || value.get("claudeCredentialsRaw").is_some()
        {
            let mut account: ClaudeAccount = serde_json::from_value(value.clone())
                .map_err(|e| format!("解析 Claude 账号失败: {}", e))?;
            if account.id.trim().is_empty() {
                account.id = id.to_string();
            }
            return save_account_and_index(account);
        }
    }

    let credentials_raw = value
        .get("claude_credentials_raw")
        .or_else(|| value.get("claudeCredentialsRaw"))
        .or_else(|| value.get("credentials"))
        .cloned()
        .unwrap_or_else(|| {
            if value.get("claudeAiOauth").is_some() {
                value.clone()
            } else {
                Value::Null
            }
        });
    let config_raw = value
        .get("claude_config_raw")
        .or_else(|| value.get("claudeConfigRaw"))
        .or_else(|| value.get("config"))
        .cloned()
        .unwrap_or_else(|| {
            if value.get("oauthAccount").is_some() {
                value.clone()
            } else {
                Value::Null
            }
        });
    upsert_account_from_snapshots(credentials_raw, config_raw)
}

fn is_desktop_oauth_mode_value(mode: &str) -> bool {
    mode.eq_ignore_ascii_case("desktop_oauth")
        || mode.eq_ignore_ascii_case("desktop_o_auth")
        || mode.eq_ignore_ascii_case("desktopOAuth")
}

fn is_desktop_oauth_json_import(value: &Value) -> bool {
    if value
        .get("auth_mode")
        .or_else(|| value.get("authMode"))
        .and_then(|item| item.as_str())
        .map(is_desktop_oauth_mode_value)
        .unwrap_or(false)
    {
        return true;
    }

    if value.get("desktop_profile_dir").is_some()
        || value.get("desktopProfileDir").is_some()
        || value.get("desktop_profile_imported_at").is_some()
        || value.get("desktopProfileImportedAt").is_some()
    {
        return true;
    }

    if value
        .get("claude_credentials_raw")
        .or_else(|| value.get("claudeCredentialsRaw"))
        .and_then(|item| item.get("authMode"))
        .and_then(|item| item.as_str())
        .map(is_desktop_oauth_mode_value)
        .unwrap_or(false)
    {
        return true;
    }

    value
        .get("claude_config_raw")
        .or_else(|| value.get("claudeConfigRaw"))
        .and_then(|item| item.get("desktopProfile"))
        .is_some()
}

pub fn import_from_json(json_content: &str) -> Result<Vec<ClaudeAccount>, String> {
    let value: Value =
        serde_json::from_str(json_content).map_err(|e| format!("解析 JSON 失败: {}", e))?;
    if let Some(arr) = value.as_array() {
        return arr.iter().map(parse_import_item).collect();
    }
    if let Some(arr) = value.get("accounts").and_then(|item| item.as_array()) {
        return arr.iter().map(parse_import_item).collect();
    }
    Ok(vec![parse_import_item(&value)?])
}

pub fn start_oauth_login() -> Result<ClaudeOAuthStartResponse, String> {
    let login_id = generate_random_url_token(18);
    let state = generate_random_url_token(32);
    let code_verifier = generate_random_url_token(32);
    let code_challenge = generate_pkce_challenge(&code_verifier);
    let auth_url = build_oauth_authorize_url(&state, &code_challenge)?;
    let pending = PendingClaudeOAuthState {
        login_id,
        state,
        code_verifier,
        auth_url,
        expires_at: now_ts() + CLAUDE_OAUTH_TIMEOUT_SECONDS,
        cancelled: false,
    };
    set_pending_oauth_login(Some(pending.clone()));
    Ok(to_oauth_start_response(&pending))
}

pub fn cancel_oauth_login(login_id: Option<&str>) -> Result<(), String> {
    if let Some(login_id) = login_id.and_then(|value| normalize_non_empty(Some(value))) {
        clear_pending_oauth_login_if_matches(&login_id);
        return Ok(());
    }
    set_pending_oauth_login(None);
    Ok(())
}

async fn exchange_oauth_code_for_tokens(
    state: &PendingClaudeOAuthState,
    code: &str,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client
        .post(CLAUDE_OAUTH_TOKEN_URL)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json, text/plain, */*")
        .header(USER_AGENT, "antigravity-cockpit-tools")
        .json(&json!({
            "grant_type": "authorization_code",
            "client_id": CLAUDE_OAUTH_CLIENT_ID,
            "code": code,
            "redirect_uri": CLAUDE_OAUTH_MANUAL_REDIRECT_URL,
            "code_verifier": state.code_verifier,
            "state": state.state,
        }))
        .send()
        .await
        .map_err(|e| format!("交换 Claude OAuth token 失败: {}", e))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 Claude OAuth token 响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!(
            "交换 Claude OAuth token 失败: HTTP {} {}",
            status, body
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("解析 Claude OAuth token 响应失败: {}", e))
}

async fn request_oauth_profile(access_token: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client
        .get(CLAUDE_OAUTH_PROFILE_URL)
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", access_token))
                .map_err(|e| format!("构造 Claude profile Authorization 失败: {}", e))?,
        )
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, "antigravity-cockpit-tools")
        .send()
        .await
        .map_err(|e| format!("请求 Claude OAuth profile 失败: {}", e))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 Claude OAuth profile 响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!(
            "请求 Claude OAuth profile 失败: HTTP {} {}",
            status, body
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("解析 Claude OAuth profile 响应失败: {}", e))
}

fn split_scope_string(scope: Option<String>) -> Vec<String> {
    scope
        .map(|value| {
            value
                .split_whitespace()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            CLAUDE_OAUTH_SCOPES
                .iter()
                .map(|item| item.to_string())
                .collect()
        })
}

fn insert_string_if_present(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value.and_then(|item| normalize_non_empty(Some(item.as_str()))) {
        object.insert(key.to_string(), Value::String(value));
    }
}

fn insert_bool_if_present(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<bool>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), Value::Bool(value));
    }
}

fn first_string(candidates: Vec<Option<String>>) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find_map(|value| normalize_non_empty(Some(value.as_str())))
}

fn subscription_type_from_profile(profile: Option<&Value>) -> Option<String> {
    // 对齐官方 oauth/profile 分支，只识别 4 个枚举：claude_max / claude_pro / claude_enterprise / claude_team。
    // 其它取值一律返回 None，避免产出多余原值。
    match read_string_path(profile?, &["organization", "organization_type"])?.as_str() {
        "claude_max" => Some("Max".to_string()),
        "claude_pro" => Some("Pro".to_string()),
        "claude_enterprise" => Some("Enterprise".to_string()),
        "claude_team" => Some("Team".to_string()),
        _ => None,
    }
}

fn build_oauth_snapshots(
    token_response: &Value,
    profile: Option<&Value>,
    email_hint: Option<&str>,
) -> Result<(Value, Value), String> {
    let access_token = read_string_path(token_response, &["access_token"])
        .ok_or_else(|| "Claude OAuth 响应缺少 access_token".to_string())?;
    let refresh_token = read_string_path(token_response, &["refresh_token"]);
    let expires_in = read_i64_value(token_response.get("expires_in")).unwrap_or(3600);
    let scopes = split_scope_string(read_string_path(token_response, &["scope"]));

    let account_uuid = first_string(vec![
        profile.and_then(|value| read_string_path(value, &["account", "uuid"])),
        read_string_path(token_response, &["account", "uuid"]),
    ]);
    let email = first_string(vec![
        profile.and_then(|value| read_string_path(value, &["account", "email"])),
        profile.and_then(|value| read_string_path(value, &["account", "email_address"])),
        read_string_path(token_response, &["account", "email_address"]),
        email_hint.and_then(|value| normalize_non_empty(Some(value))),
    ])
    .ok_or_else(|| "无法从 Claude OAuth 响应识别邮箱，请填写邮箱后重试".to_string())?;
    let organization_uuid = first_string(vec![
        profile.and_then(|value| read_string_path(value, &["organization", "uuid"])),
        read_string_path(token_response, &["organization", "uuid"]),
    ]);
    let organization_name = first_string(vec![
        profile.and_then(|value| read_string_path(value, &["organization", "name"])),
        profile.and_then(|value| read_string_path(value, &["organization", "display_name"])),
        read_string_path(token_response, &["organization", "name"]),
    ]);
    let display_name =
        profile.and_then(|value| read_string_path(value, &["account", "display_name"]));
    let avatar_url = first_string(vec![
        profile.and_then(|value| read_string_path(value, &["account", "avatar_url"])),
        profile.and_then(|value| read_string_path(value, &["account", "avatarUrl"])),
        read_string_path(token_response, &["account", "avatar_url"]),
    ]);
    let account_created_at =
        profile.and_then(|value| read_string_path(value, &["account", "created_at"]));
    let organization_type =
        profile.and_then(|value| read_string_path(value, &["organization", "organization_type"]));
    let billing_type =
        profile.and_then(|value| read_string_path(value, &["organization", "billing_type"]));
    let rate_limit_tier =
        profile.and_then(|value| read_string_path(value, &["organization", "rate_limit_tier"]));
    let subscription_created_at = profile
        .and_then(|value| read_string_path(value, &["organization", "subscription_created_at"]));
    let has_extra_usage_enabled = profile.and_then(|value| {
        read_bool_value(value.get("organization")?.get("has_extra_usage_enabled"))
    });
    let subscription_type = subscription_type_from_profile(profile);

    let mut credentials_oauth = serde_json::Map::new();
    credentials_oauth.insert("accessToken".to_string(), Value::String(access_token));
    if let Some(refresh_token) = refresh_token {
        credentials_oauth.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    credentials_oauth.insert(
        "expiresAt".to_string(),
        Value::Number(serde_json::Number::from(now_ts_ms() + expires_in * 1000)),
    );
    credentials_oauth.insert(
        "scopes".to_string(),
        Value::Array(scopes.into_iter().map(Value::String).collect()),
    );
    credentials_oauth.insert(
        "subscriptionType".to_string(),
        subscription_type
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    credentials_oauth.insert(
        "rateLimitTier".to_string(),
        rate_limit_tier
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    if let Some(profile) = profile {
        credentials_oauth.insert("profile".to_string(), profile.clone());
    }

    let mut oauth_account = serde_json::Map::new();
    insert_string_if_present(&mut oauth_account, "accountUuid", account_uuid);
    oauth_account.insert("emailAddress".to_string(), Value::String(email));
    insert_string_if_present(&mut oauth_account, "organizationUuid", organization_uuid);
    insert_string_if_present(&mut oauth_account, "organizationName", organization_name);
    insert_string_if_present(&mut oauth_account, "displayName", display_name);
    insert_string_if_present(&mut oauth_account, "avatarUrl", avatar_url);
    insert_bool_if_present(
        &mut oauth_account,
        "hasExtraUsageEnabled",
        has_extra_usage_enabled,
    );
    insert_string_if_present(&mut oauth_account, "billingType", billing_type);
    insert_string_if_present(&mut oauth_account, "organizationType", organization_type);
    insert_string_if_present(&mut oauth_account, "accountCreatedAt", account_created_at);
    insert_string_if_present(
        &mut oauth_account,
        "subscriptionCreatedAt",
        subscription_created_at,
    );
    insert_string_if_present(&mut oauth_account, "subscriptionType", subscription_type);
    insert_string_if_present(&mut oauth_account, "rateLimitTier", rate_limit_tier);

    let credentials = Value::Object(
        [(
            "claudeAiOauth".to_string(),
            Value::Object(credentials_oauth),
        )]
        .into_iter()
        .collect(),
    );
    let config = json!({
        "oauthAccount": Value::Object(oauth_account),
        "hasCompletedOnboarding": true,
    });
    Ok((credentials, config))
}

pub async fn complete_oauth_login(
    login_id: &str,
    callback_or_code: &str,
    email_hint: Option<&str>,
) -> Result<ClaudeAccount, String> {
    let pending = get_pending_oauth_login_for(login_id)?;
    let (code, callback_state) = parse_oauth_callback_input(callback_or_code)?;
    if let Some(callback_state) = callback_state {
        if callback_state != pending.state {
            return Err("Claude OAuth 回调 state 不匹配，请重新开始授权".to_string());
        }
    }
    let token_response = exchange_oauth_code_for_tokens(&pending, &code).await?;
    let access_token = read_string_path(&token_response, &["access_token"])
        .ok_or_else(|| "Claude OAuth 响应缺少 access_token".to_string())?;
    let profile = match request_oauth_profile(&access_token).await {
        Ok(profile) => Some(profile),
        Err(error) => {
            logger::log_warn(&format!(
                "[Claude OAuth] 获取 profile 失败，将尝试使用 token 响应或邮箱兜底: {}",
                error
            ));
            None
        }
    };
    let (credentials, config) =
        build_oauth_snapshots(&token_response, profile.as_ref(), email_hint)?;
    let account = upsert_account_from_snapshots(credentials, config)?;
    clear_pending_oauth_login_if_matches(login_id);
    Ok(account)
}

fn first_string_path_candidates(value: Option<&Value>, paths: &[&[&str]]) -> Option<String> {
    let value = value?;
    paths.iter().find_map(|path| read_string_path(value, path))
}

fn first_f64_path_candidates(value: Option<&Value>, paths: &[&[&str]]) -> Option<f64> {
    let value = value?;
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        read_f64_value(Some(current))
    })
}

fn first_i64_path_candidates(value: Option<&Value>, paths: &[&[&str]]) -> Option<i64> {
    let value = value?;
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        read_i64_value(Some(current))
    })
}

fn first_reset_path_candidates(value: Option<&Value>, paths: &[&[&str]]) -> Option<i64> {
    let value = value?;
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        parse_reset_seconds(Some(current))
    })
}

fn find_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(found) = object
                    .get(*key)
                    .and_then(|item| normalize_non_empty(item.as_str()))
                {
                    return Some(found);
                }
            }
            object
                .values()
                .find_map(|item| find_string_by_key(item, keys))
        }
        Value::Array(items) => items.iter().find_map(|item| find_string_by_key(item, keys)),
        _ => None,
    }
}

/// 对齐官方 Claude.app `fai` 函数的 4 个枚举。
/// 只识别：free / claude_pro / claude_max / raven（raven 进一步看 isEnterprise 拆为 Team / Enterprise）。
/// 其他一律返回 None，与官方 “拿不到 paidAccountTier 则不显示” 一致。
/// 额外兼容本地 profile 中用于细分 Max 档位的 rate limit tier。
fn normalize_desktop_plan_value(value: Option<String>) -> Option<String> {
    let value = value.and_then(|item| normalize_non_empty(Some(item.as_str())))?;
    let key = value
        .trim()
        .to_ascii_lowercase()
        .replace('-', " ")
        .replace('_', " ");
    let normalized = match key.as_str() {
        "default claude max 20x" | "claude max 20x" | "max 20x" => "Max 20x",
        "default claude max 5x" | "claude max 5x" | "max 5x" => "Max 5x",
        "claude max" | "max" => "Max",
        "claude pro" | "pro" => "Pro",
        "default claude ai" | "free" | "claude free" => "Free",
        // OAuth profile organization_type 路径：claude_enterprise / claude_team
        "claude enterprise" | "enterprise" => "Enterprise",
        "claude team" | "team" => "Team",
        // 其它取值（claude_desktop、desktop、personal、individual、apple_subscription 等）一律不识别。
        _ => return None,
    };
    Some(normalized.to_string())
}

/// 从 capabilities 数组中提取小写字符串（对齐 ["chat", "claude_pro"] 这种结构）。
fn capability_strings(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_str().map(|s| s.trim().to_ascii_lowercase()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// 严格按官方 `fai(A, isEnterprise)` 函数输出：
///   - claude_max → Max
///   - claude_pro → Pro
///   - raven      → isEnterprise ? Enterprise : Team
///   - claude_free / free → Free
fn plan_from_capability_list(caps: &[String], is_enterprise: bool) -> Option<String> {
    if caps.iter().any(|c| c == "claude_max") {
        return Some("Max".to_string());
    }
    if caps.iter().any(|c| c == "claude_pro") {
        return Some("Pro".to_string());
    }
    if caps.iter().any(|c| c == "raven") {
        return Some(if is_enterprise {
            "Enterprise".to_string()
        } else {
            "Team".to_string()
        });
    }
    if caps.iter().any(|c| c == "claude_free" || c == "free") {
        return Some("Free".to_string());
    }
    None
}

/// 是否企业版：对齐官方 oauth/profile 分支，看 organization.organization_type === "claude_enterprise"。
fn is_enterprise_from_profile(profile: &Value) -> bool {
    let Some(endpoints) = profile.get("endpoints") else {
        return false;
    };
    let direct_paths: &[&[&str]] = &[
        &["accountProfile", "organization", "organization_type"],
        &["account", "organization", "organization_type"],
        &[
            "bootstrapAppStart",
            "activeOrganization",
            "organization_type",
        ],
        &[
            "bootstrapAppStart",
            "active_organization",
            "organization_type",
        ],
        &["bootstrapAppStart", "organization", "organization_type"],
    ];
    for path in direct_paths {
        if let Some(value) = read_string_path(endpoints, path) {
            if value.eq_ignore_ascii_case("claude_enterprise") {
                return true;
            }
        }
    }
    let memberships_paths: &[&[&str]] = &[
        &["bootstrapAppStart", "account", "memberships"],
        &["accountProfile", "account", "memberships"],
        &["account", "account", "memberships"],
        &["account", "memberships"],
    ];
    for path in memberships_paths {
        let mut current = endpoints;
        let mut ok = true;
        for key in *path {
            match current.get(*key) {
                Some(next) => current = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let Some(memberships) = current.as_array() else {
            continue;
        };
        for membership in memberships {
            if let Some(org_type) = membership
                .get("organization")
                .and_then(|org| read_string_path(org, &["organization_type"]))
            {
                if org_type.eq_ignore_ascii_case("claude_enterprise") {
                    return true;
                }
            }
        }
    }
    false
}

fn infer_desktop_plan_from_capabilities(profile: &Value) -> Option<String> {
    let endpoints = profile.get("endpoints")?;
    let is_enterprise = is_enterprise_from_profile(profile);

    // 1) accountProfile.organization.capabilities
    let direct_paths: &[&[&str]] = &[
        &["accountProfile", "organization", "capabilities"],
        &["account", "organization", "capabilities"],
        &["bootstrapAppStart", "activeOrganization", "capabilities"],
        &["bootstrapAppStart", "active_organization", "capabilities"],
        &["bootstrapAppStart", "organization", "capabilities"],
    ];
    for path in direct_paths {
        let mut current = endpoints;
        let mut ok = true;
        for key in *path {
            match current.get(*key) {
                Some(next) => current = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let caps = capability_strings(Some(current));
            if let Some(plan) = plan_from_capability_list(&caps, is_enterprise) {
                return Some(plan);
            }
        }
    }

    // 2) bootstrapAppStart.account.memberships[*].organization.capabilities
    let memberships_paths: &[&[&str]] = &[
        &["bootstrapAppStart", "account", "memberships"],
        &["accountProfile", "account", "memberships"],
        &["account", "account", "memberships"],
        &["account", "memberships"],
    ];
    for path in memberships_paths {
        let mut current = endpoints;
        let mut ok = true;
        for key in *path {
            match current.get(*key) {
                Some(next) => current = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let Some(memberships) = current.as_array() else {
            continue;
        };
        let mut all_caps: Vec<String> = Vec::new();
        for membership in memberships {
            let caps = capability_strings(
                membership
                    .get("organization")
                    .and_then(|org| org.get("capabilities")),
            );
            all_caps.extend(caps);
        }
        if let Some(plan) = plan_from_capability_list(&all_caps, is_enterprise) {
            return Some(plan);
        }
    }

    None
}

fn is_desktop_plan_placeholder(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "claude desktop" | "desktop"
    )
}

fn normalize_desktop_usage_percentage(value: f64) -> i32 {
    let scaled = if value > 0.0 && value < 1.0 {
        value * 100.0
    } else {
        value
    };
    clamp_percentage(Some(scaled))
}

fn quota_matches(left: &ClaudeQuota, right: &ClaudeQuota) -> bool {
    left.five_hour_percentage == right.five_hour_percentage
        && left.five_hour_reset_time == right.five_hour_reset_time
        && left.seven_day_percentage == right.seven_day_percentage
        && left.seven_day_reset_time == right.seven_day_reset_time
        && left.seven_day_sonnet_percentage == right.seven_day_sonnet_percentage
        && left.seven_day_sonnet_reset_time == right.seven_day_sonnet_reset_time
        && left.extra_usage_percentage == right.extra_usage_percentage
        && left.extra_usage_reset_time == right.extra_usage_reset_time
        && left.extra_usage_used_cents == right.extra_usage_used_cents
        && left.extra_usage_limit_cents == right.extra_usage_limit_cents
}

// ============================================================================
// webProfile 存储瘦身
//
// 贬面背景：claude-desktop-auth-helper 会拽回 bootstrapAppStart，包含
// statsig / growthbook feature flags、system_prompts、完整 memberships 等，原始对象
// 常常能到几 MB。以前直接把整个 profile 填到 claude_usage_raw，导致账号文件 +
// 导出 JSON 极大。这里只保留上层识别实际使用的字段。
// ============================================================================

fn slim_organization_object(org: &Value) -> Value {
    let Some(obj) = org.as_object() else {
        return Value::Null;
    };
    let mut slim = serde_json::Map::new();
    for key in [
        "uuid",
        "name",
        "organization_type",
        "rate_limit_tier",
        "capabilities",
    ] {
        if let Some(v) = obj.get(key) {
            slim.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(slim)
}

fn slim_membership_entry(membership: &Value) -> Value {
    let mut slim = serde_json::Map::new();
    if let Some(org) = membership.get("organization") {
        slim.insert("organization".to_string(), slim_organization_object(org));
    }
    Value::Object(slim)
}

fn slim_account_object(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut slim = serde_json::Map::new();
    for key in [
        "email_address",
        "email",
        "uuid",
        "full_name",
        "display_name",
    ] {
        if let Some(v) = obj.get(key) {
            slim.insert(key.to_string(), v.clone());
        }
    }
    if let Some(memberships) = obj.get("memberships").and_then(|v| v.as_array()) {
        let trimmed: Vec<Value> = memberships.iter().map(slim_membership_entry).collect();
        slim.insert("memberships".to_string(), Value::Array(trimmed));
    }
    Some(Value::Object(slim))
}

/// accountProfile / account 端点响应瘦身：只保留邮箱、uuid、全名、organization、嵌套 account.memberships。
fn slim_account_profile_payload(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut slim = serde_json::Map::new();
    for key in [
        "email_address",
        "email",
        "uuid",
        "full_name",
        "display_name",
    ] {
        if let Some(v) = obj.get(key) {
            slim.insert(key.to_string(), v.clone());
        }
    }
    if let Some(org) = obj.get("organization") {
        slim.insert("organization".to_string(), slim_organization_object(org));
    }
    if let Some(account) = obj.get("account") {
        if let Some(slim_account) = slim_account_object(account) {
            slim.insert("account".to_string(), slim_account);
        }
    }
    Some(Value::Object(slim))
}

/// bootstrapAppStart 瘦身：只保留 active_organization 与 account.memberships。
fn slim_bootstrap_payload(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut slim = serde_json::Map::new();
    for org_key in ["activeOrganization", "active_organization", "organization"] {
        if let Some(org) = obj.get(org_key) {
            slim.insert(org_key.to_string(), slim_organization_object(org));
        }
    }
    if let Some(account) = obj.get("account") {
        if let Some(slim_account) = slim_account_object(account) {
            slim.insert("account".to_string(), slim_account);
        }
    }
    if slim.is_empty() {
        None
    } else {
        Some(Value::Object(slim))
    }
}

/// 生成可安全写入 claude_usage_raw / 导出的 webProfile 瘦身副本。
fn slim_web_profile_for_storage(profile: &Value) -> Value {
    let mut slim = serde_json::Map::new();
    for key in ["version", "fetchContext", "fetchedAt"] {
        if let Some(v) = profile.get(key) {
            slim.insert(key.to_string(), v.clone());
        }
    }
    if let Some(errors) = profile.get("errors") {
        slim.insert("errors".to_string(), errors.clone());
    }
    if let Some(endpoints) = profile.get("endpoints").and_then(|v| v.as_object()) {
        let mut slim_endpoints = serde_json::Map::new();
        // 额度与订阅响应体量可控，原样保留（后续字段识别/展示都靠它）。
        for key in [
            "organizationUsage",
            "subscriptionDetails",
            "overageSpendLimit",
        ] {
            if let Some(v) = endpoints.get(key) {
                slim_endpoints.insert(key.to_string(), v.clone());
            }
        }
        if let Some(ap) = endpoints
            .get("accountProfile")
            .and_then(slim_account_profile_payload)
        {
            slim_endpoints.insert("accountProfile".to_string(), ap);
        }
        if let Some(acc) = endpoints
            .get("account")
            .and_then(slim_account_profile_payload)
        {
            slim_endpoints.insert("account".to_string(), acc);
        }
        if let Some(boot) = endpoints
            .get("bootstrapAppStart")
            .and_then(slim_bootstrap_payload)
        {
            slim_endpoints.insert("bootstrapAppStart".to_string(), boot);
        }
        slim.insert("endpoints".to_string(), Value::Object(slim_endpoints));
    }
    Value::Object(slim)
}

fn desktop_web_usage_to_quota(profile: &Value) -> Option<ClaudeQuota> {
    let five_hour = first_f64_path_candidates(
        Some(profile),
        &[
            &["endpoints", "organizationUsage", "five_hour", "utilization"],
            &["endpoints", "organizationUsage", "five_hour", "percentage"],
            &[
                "endpoints",
                "organizationUsage",
                "five_hour",
                "percent_used",
            ],
            &["endpoints", "organizationUsage", "fiveHour", "utilization"],
            &["endpoints", "organizationUsage", "fiveHour", "percentage"],
            &["endpoints", "organizationUsage", "fiveHour", "percentUsed"],
            &[
                "endpoints",
                "organizationUsage",
                "usage",
                "five_hour",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "usage",
                "fiveHour",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "limits",
                "five_hour",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "limits",
                "fiveHour",
                "utilization",
            ],
            &["endpoints", "organizationUsage", "five_hour_percentage"],
            &["endpoints", "organizationUsage", "fiveHourPercentage"],
            &["endpoints", "organizationUsage", "five_hour_utilization"],
            &["endpoints", "organizationUsage", "fiveHourUtilization"],
            &["endpoints", "organizationUsage", "five_hour_percent_used"],
            &["endpoints", "organizationUsage", "fiveHourPercentUsed"],
        ],
    );
    let seven_day = first_f64_path_candidates(
        Some(profile),
        &[
            &["endpoints", "organizationUsage", "seven_day", "utilization"],
            &["endpoints", "organizationUsage", "seven_day", "percentage"],
            &[
                "endpoints",
                "organizationUsage",
                "seven_day",
                "percent_used",
            ],
            &["endpoints", "organizationUsage", "sevenDay", "utilization"],
            &["endpoints", "organizationUsage", "sevenDay", "percentage"],
            &["endpoints", "organizationUsage", "sevenDay", "percentUsed"],
            &[
                "endpoints",
                "organizationUsage",
                "usage",
                "seven_day",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "usage",
                "sevenDay",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "limits",
                "seven_day",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "limits",
                "sevenDay",
                "utilization",
            ],
            &["endpoints", "organizationUsage", "seven_day_percentage"],
            &["endpoints", "organizationUsage", "sevenDayPercentage"],
            &["endpoints", "organizationUsage", "seven_day_utilization"],
            &["endpoints", "organizationUsage", "sevenDayUtilization"],
            &["endpoints", "organizationUsage", "seven_day_percent_used"],
            &["endpoints", "organizationUsage", "sevenDayPercentUsed"],
        ],
    );
    let seven_day_sonnet = first_f64_path_candidates(
        Some(profile),
        &[
            &[
                "endpoints",
                "organizationUsage",
                "seven_day_sonnet",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "seven_day_sonnet",
                "percentage",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "seven_day_sonnet",
                "percent_used",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "sevenDaySonnet",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "sevenDaySonnet",
                "percentage",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "sevenDaySonnet",
                "percentUsed",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "seven_day_sonnet_percentage",
            ],
            &["endpoints", "organizationUsage", "sevenDaySonnetPercentage"],
            &[
                "endpoints",
                "organizationUsage",
                "seven_day_sonnet_utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "sevenDaySonnetUtilization",
            ],
        ],
    );
    if five_hour.is_none() && seven_day.is_none() && seven_day_sonnet.is_none() {
        return None;
    }

    let extra_usage = first_f64_path_candidates(
        Some(profile),
        &[
            &[
                "endpoints",
                "organizationUsage",
                "extra_usage",
                "utilization",
            ],
            &[
                "endpoints",
                "organizationUsage",
                "extraUsage",
                "utilization",
            ],
            &["endpoints", "organizationUsage", "extra_usage_percentage"],
            &["endpoints", "organizationUsage", "extraUsagePercentage"],
            &["endpoints", "overageSpendLimit", "utilization"],
            &["endpoints", "overageSpendLimit", "percentage"],
            &["endpoints", "overageSpendLimit", "percent_used"],
            &["endpoints", "overageSpendLimit", "percentUsed"],
        ],
    );
    let extra_enabled = read_bool_value(
        profile
            .get("endpoints")
            .and_then(|value| value.get("organizationUsage"))
            .and_then(|value| value.get("extra_usage"))
            .and_then(|value| value.get("is_enabled")),
    )
    .or_else(|| {
        read_bool_value(
            profile
                .get("endpoints")
                .and_then(|value| value.get("organizationUsage"))
                .and_then(|value| value.get("extraUsage"))
                .and_then(|value| value.get("isEnabled")),
        )
    })
    .or_else(|| {
        read_bool_value(
            profile
                .get("endpoints")
                .and_then(|value| value.get("overageSpendLimit"))
                .and_then(|value| value.get("is_enabled")),
        )
    })
    .unwrap_or(extra_usage.is_some());

    let endpoints = profile.get("endpoints");
    Some(ClaudeQuota {
        five_hour_percentage: five_hour
            .map(normalize_desktop_usage_percentage)
            .unwrap_or(0),
        five_hour_reset_time: first_reset_path_candidates(
            Some(profile),
            &[
                &["endpoints", "organizationUsage", "five_hour", "resets_at"],
                &["endpoints", "organizationUsage", "five_hour", "reset_at"],
                &["endpoints", "organizationUsage", "fiveHour", "resetsAt"],
                &["endpoints", "organizationUsage", "fiveHour", "resetAt"],
                &["endpoints", "organizationUsage", "five_hour_reset_time"],
                &["endpoints", "organizationUsage", "fiveHourResetTime"],
            ],
        ),
        seven_day_percentage: seven_day
            .map(normalize_desktop_usage_percentage)
            .unwrap_or(0),
        seven_day_reset_time: first_reset_path_candidates(
            Some(profile),
            &[
                &["endpoints", "organizationUsage", "seven_day", "resets_at"],
                &["endpoints", "organizationUsage", "seven_day", "reset_at"],
                &["endpoints", "organizationUsage", "sevenDay", "resetsAt"],
                &["endpoints", "organizationUsage", "sevenDay", "resetAt"],
                &["endpoints", "organizationUsage", "seven_day_reset_time"],
                &["endpoints", "organizationUsage", "sevenDayResetTime"],
            ],
        ),
        seven_day_sonnet_percentage: seven_day_sonnet.map(normalize_desktop_usage_percentage),
        seven_day_sonnet_reset_time: first_reset_path_candidates(
            Some(profile),
            &[
                &[
                    "endpoints",
                    "organizationUsage",
                    "seven_day_sonnet",
                    "resets_at",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "seven_day_sonnet",
                    "reset_at",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "sevenDaySonnet",
                    "resetsAt",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "sevenDaySonnet",
                    "resetAt",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "seven_day_sonnet_reset_time",
                ],
                &["endpoints", "organizationUsage", "sevenDaySonnetResetTime"],
            ],
        ),
        extra_usage_percentage: extra_enabled.then(|| {
            extra_usage
                .map(normalize_desktop_usage_percentage)
                .unwrap_or(0)
        }),
        extra_usage_reset_time: first_reset_path_candidates(
            Some(profile),
            &[
                &["endpoints", "organizationUsage", "extra_usage", "resets_at"],
                &["endpoints", "organizationUsage", "extraUsage", "resetsAt"],
                &["endpoints", "overageSpendLimit", "resets_at"],
                &["endpoints", "overageSpendLimit", "resetsAt"],
            ],
        ),
        extra_usage_used_cents: first_i64_path_candidates(
            Some(profile),
            &[
                &[
                    "endpoints",
                    "organizationUsage",
                    "extra_usage",
                    "used_credits",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "extraUsage",
                    "usedCredits",
                ],
                &["endpoints", "overageSpendLimit", "used_credits"],
                &["endpoints", "overageSpendLimit", "usedCredits"],
                &["endpoints", "overageSpendLimit", "used_cents"],
                &["endpoints", "overageSpendLimit", "usedCents"],
            ],
        ),
        extra_usage_limit_cents: first_i64_path_candidates(
            Some(profile),
            &[
                &[
                    "endpoints",
                    "organizationUsage",
                    "extra_usage",
                    "monthly_limit",
                ],
                &[
                    "endpoints",
                    "organizationUsage",
                    "extraUsage",
                    "monthlyLimit",
                ],
                &["endpoints", "overageSpendLimit", "monthly_limit"],
                &["endpoints", "overageSpendLimit", "monthlyLimit"],
                &["endpoints", "overageSpendLimit", "limit_cents"],
                &["endpoints", "overageSpendLimit", "limitCents"],
            ],
        ),
        raw_data: Some(json!({
            "source": "claude_desktop_web",
            "organizationUsage": endpoints.and_then(|value| value.get("organizationUsage")).cloned(),
            "subscriptionDetails": endpoints.and_then(|value| value.get("subscriptionDetails")).cloned(),
            "overageSpendLimit": endpoints.and_then(|value| value.get("overageSpendLimit")).cloned(),
        })),
    })
}

fn normalize_cached_desktop_quota_from_raw(account: &mut ClaudeAccount) -> bool {
    if account.auth_mode != ClaudeAuthMode::DesktopOAuth {
        return false;
    }
    let Some(raw) = account.claude_usage_raw.as_ref() else {
        return false;
    };
    let Some(quota) = desktop_web_usage_to_quota(raw) else {
        return false;
    };
    let changed = account
        .quota
        .as_ref()
        .map(|current| !quota_matches(current, &quota))
        .unwrap_or(true);
    if changed {
        account.quota = Some(quota);
    }
    changed
}

fn desktop_web_profile_summary(profile: &Value) -> Value {
    let email = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "accountProfile", "account", "email"],
            &["endpoints", "accountProfile", "account", "email_address"],
            &["endpoints", "accountProfile", "email"],
            &["endpoints", "account", "account", "email"],
            &["endpoints", "account", "email"],
            &["endpoints", "bootstrapAppStart", "account", "email"],
            &["endpoints", "bootstrapAppStart", "user", "email"],
        ],
    )
    .or_else(|| find_string_by_key(profile, &["email", "email_address", "emailAddress"]));
    let avatar_url = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "accountProfile", "account", "avatar_url"],
            &["endpoints", "accountProfile", "account", "avatarUrl"],
            &["endpoints", "accountProfile", "account", "picture"],
            &["endpoints", "account", "avatar_url"],
            &["endpoints", "account", "picture"],
            &["endpoints", "bootstrapAppStart", "account", "avatar_url"],
        ],
    )
    .or_else(|| {
        find_string_by_key(
            profile,
            &[
                "avatar_url",
                "avatarUrl",
                "profile_image_url",
                "profileImageUrl",
                "picture",
                "picture_url",
                "image_url",
            ],
        )
    });
    let account_uuid = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "accountProfile", "account", "uuid"],
            &["endpoints", "account", "uuid"],
            &["endpoints", "account", "account", "uuid"],
            &["endpoints", "bootstrapAppStart", "account", "uuid"],
        ],
    )
    .or_else(|| find_string_by_key(profile, &["account_uuid", "accountUuid"]));
    let organization_uuid = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "accountProfile", "organization", "uuid"],
            &["endpoints", "account", "organization", "uuid"],
            &[
                "endpoints",
                "bootstrapAppStart",
                "activeOrganization",
                "uuid",
            ],
            &[
                "endpoints",
                "bootstrapAppStart",
                "active_organization",
                "uuid",
            ],
            &["endpoints", "bootstrapAppStart", "organization", "uuid"],
        ],
    )
    .or_else(|| find_string_by_key(profile, &["organization_uuid", "organizationUuid"]));
    let organization_name = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "accountProfile", "organization", "name"],
            &[
                "endpoints",
                "accountProfile",
                "organization",
                "display_name",
            ],
            &["endpoints", "account", "organization", "name"],
            &[
                "endpoints",
                "bootstrapAppStart",
                "activeOrganization",
                "name",
            ],
            &[
                "endpoints",
                "bootstrapAppStart",
                "active_organization",
                "name",
            ],
            &["endpoints", "bootstrapAppStart", "organization", "name"],
        ],
    )
    .or_else(|| find_string_by_key(profile, &["organization_name", "organizationName"]));
    let raw_plan = first_string_path_candidates(
        Some(profile),
        &[
            &["endpoints", "subscriptionDetails", "plan_type"],
            &["endpoints", "subscriptionDetails", "planType"],
            &["endpoints", "subscriptionDetails", "plan"],
            &["endpoints", "subscriptionDetails", "tier"],
            &["endpoints", "subscriptionDetails", "subscription_type"],
            &["endpoints", "subscriptionDetails", "subscriptionType"],
            &[
                "endpoints",
                "subscriptionDetails",
                "subscription",
                "plan_type",
            ],
            &[
                "endpoints",
                "subscriptionDetails",
                "subscription",
                "planType",
            ],
            &["endpoints", "subscriptionDetails", "subscription", "plan"],
            &["endpoints", "subscriptionDetails", "subscription", "tier"],
            &["endpoints", "organizationUsage", "plan_type"],
            &["endpoints", "organizationUsage", "planType"],
            &["endpoints", "organizationUsage", "subscription_type"],
            &["endpoints", "organizationUsage", "subscriptionType"],
            &[
                "endpoints",
                "accountProfile",
                "organization",
                "rate_limit_tier",
            ],
            &[
                "endpoints",
                "accountProfile",
                "organization",
                "organization_type",
            ],
            &[
                "endpoints",
                "accountProfile",
                "organization",
                "billing_type",
            ],
            &["endpoints", "account", "organization", "rate_limit_tier"],
            &["endpoints", "account", "organization", "organization_type"],
            &[
                "endpoints",
                "bootstrapAppStart",
                "activeOrganization",
                "rate_limit_tier",
            ],
            &[
                "endpoints",
                "bootstrapAppStart",
                "activeOrganization",
                "organization_type",
            ],
            &[
                "endpoints",
                "bootstrapAppStart",
                "active_organization",
                "rate_limit_tier",
            ],
            &[
                "endpoints",
                "bootstrapAppStart",
                "active_organization",
                "organization_type",
            ],
        ],
    )
    .or_else(|| {
        find_string_by_key(
            profile,
            &[
                "rate_limit_tier",
                "rateLimitTier",
                "subscription_type",
                "subscriptionType",
                "billing_type",
                "billingType",
                "organization_type",
                "organizationType",
                "plan_type",
                "planType",
                "plan_name",
                "planName",
                "subscription_tier",
                "subscriptionTier",
                "plan",
                "tier",
            ],
        )
    });
    // 严格对齐官方：仅 capabilities 识别、OAuth profile organization_type 识别。
    // 拿不到时返回 None，与官方 "没值则不显示" 一致。
    let plan_type = infer_desktop_plan_from_capabilities(profile)
        .or_else(|| normalize_desktop_plan_value(raw_plan.clone()));
    json!({
        "fetchedAt": read_string_path(profile, &["fetchedAt"]),
        "email": email,
        "avatarUrl": avatar_url,
        "accountUuid": account_uuid,
        "organizationUuid": organization_uuid,
        "organizationName": organization_name,
        "planType": plan_type,
        "rawPlan": raw_plan,
        "errors": profile.get("errors").cloned(),
    })
}

fn shorten_profile_error(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut value = String::new();
    for ch in trimmed.chars().take(180) {
        value.push(ch);
    }
    if trimmed.chars().count() > 180 {
        value.push_str("...");
    }
    value
}

fn desktop_web_profile_error_message(profile: &Value) -> Option<String> {
    let errors = profile.get("errors")?.as_object()?;
    let first_error = errors
        .values()
        .filter_map(|value| normalize_non_empty(value.as_str()))
        .next()?;
    if desktop_error_is_cloudflare_challenge(&first_error) {
        return Some(
            "Claude Web 接口被 Cloudflare 校验拦截，暂时无法读取账号资料、订阅或额度；切号不受影响。"
                .to_string(),
        );
    }
    Some(format!(
        "Claude Web 资料接口失败: {}",
        shorten_profile_error(&first_error)
    ))
}

fn desktop_web_usage_error_message(profile: &Value) -> Option<String> {
    let error = profile
        .get("errors")
        .and_then(|value| value.as_object())
        .and_then(|errors| errors.get("organizationUsage"))
        .and_then(|value| normalize_non_empty(value.as_str()))?;
    if error.contains("missing lastActiveOrg") {
        return Some("Claude 账号缺少组织信息，暂时无法刷新额度。".to_string());
    }
    if desktop_error_is_cloudflare_challenge(&error) {
        return Some(
            "Claude Web usage 接口被 Cloudflare 校验拦截，暂时无法刷新额度；已保留旧缓存。"
                .to_string(),
        );
    }
    Some(format!(
        "Claude 额度刷新失败: {}",
        shorten_profile_error(&error)
    ))
}

fn desktop_account_has_real_profile_data(account: &ClaudeAccount) -> bool {
    account
        .email
        .split_once('@')
        .map(|(_, domain)| domain.contains('.'))
        .unwrap_or(false)
        || account.account_uuid.is_some()
        || account.avatar_url.is_some()
        || account
            .plan_type
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .map(|value| !value.eq_ignore_ascii_case("Claude"))
            .unwrap_or(false)
        || account
            .organization_name
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .map(|value| !value.eq_ignore_ascii_case("Claude"))
            .unwrap_or(false)
}

fn apply_desktop_web_profile(account: &mut ClaudeAccount, profile: &Value) -> bool {
    let summary = desktop_web_profile_summary(profile);
    let mut applied = false;
    let quota = desktop_web_usage_to_quota(profile);
    if let Some(quota) = quota {
        account.quota = Some(quota);
        // 仅存瘦身后的 webProfile，避免 bootstrapAppStart 中的 statsig / feature flags
        // / system_prompts 等数 MB 包体颍到账号文件与导出 JSON。
        account.claude_usage_raw = Some(slim_web_profile_for_storage(profile));
        account.usage_updated_at = Some(now_ts_ms());
        applied = true;
    } else {
        // 额度未识别时输出诊断信息，便于定位是接口失败还是字段结构不识别。
        let usage_node = profile
            .get("endpoints")
            .and_then(|v| v.get("organizationUsage"));
        let usage_keys: Vec<String> = usage_node
            .and_then(|v| v.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let usage_err = profile
            .get("errors")
            .and_then(|v| v.get("organizationUsage"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        logger::log_warn(&format!(
            "[Claude] organizationUsage 未识别: account_id={}, usage_present={}, usage_keys={:?}, usage_error={:?}",
            account.id,
            usage_node.is_some(),
            usage_keys,
            usage_err
        ));
    }
    if let Some(email) = read_string_path(&summary, &["email"]) {
        account.email = email;
        applied = true;
    }
    if let Some(account_uuid) = read_string_path(&summary, &["accountUuid"]) {
        account.account_uuid = Some(account_uuid);
        applied = true;
    }
    if let Some(organization_uuid) = read_string_path(&summary, &["organizationUuid"]) {
        account.organization_uuid = Some(organization_uuid);
        applied = true;
    }
    if let Some(organization_name) = read_string_path(&summary, &["organizationName"]) {
        account.organization_name = Some(organization_name);
        applied = true;
    }
    if let Some(plan_type) = read_string_path(&summary, &["planType"]) {
        account.plan_type = Some(plan_type);
        applied = true;
    } else if account
        .plan_type
        .as_deref()
        .map(is_desktop_plan_placeholder)
        .unwrap_or(false)
    {
        account.plan_type = None;
        applied = true;
    }
    if let Some(avatar_url) = read_string_path(&summary, &["avatarUrl"]) {
        account.avatar_url = Some(avatar_url);
        applied = true;
    }
    if applied {
        account.profile_updated_at = Some(now_ts_ms());
    } else if !desktop_account_has_real_profile_data(account) {
        account.profile_updated_at = None;
    }
    if let Some(config) = account.claude_config_raw.as_mut() {
        if !config.is_object() {
            *config = json!({});
        }
        if let Some(object) = config.as_object_mut() {
            let desktop_profile = object
                .entry("desktopProfile".to_string())
                .or_insert_with(|| json!({}));
            if !desktop_profile.is_object() {
                *desktop_profile = json!({});
            }
            if let Some(desktop_object) = desktop_profile.as_object_mut() {
                desktop_object.insert("webProfileSummary".to_string(), summary);
            }
        }
    }
    applied
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<ClaudeAccount> = account_ids
        .iter()
        .filter_map(|id| load_account_file(id))
        .filter(|account| account.auth_mode != ClaudeAuthMode::DesktopOAuth)
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化导出 JSON 失败: {}", e))
}

pub fn read_config_file(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| {
        format!(
            "读取 Claude config 失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str::<Value>(&content)
        .map(Some)
        .map_err(|e| format!("解析 Claude config 失败: {}", e))
}

fn write_config_file(path: &Path, config: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 Claude config 失败: {}", e))?;
    atomic_write::write_string_atomic(path, &content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn normalize_env_key(value: &str) -> Option<String> {
    let key = value.trim().to_ascii_uppercase();
    if key.is_empty() {
        return None;
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_uppercase() => {}
        _ => return None,
    }
    if chars.all(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
        Some(key)
    } else {
        None
    }
}

fn managed_env_store_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn read_settings_managed_env_keys() -> BTreeMap<String, Vec<String>> {
    let Ok(path) = get_claude_code_settings_managed_env_keys_path() else {
        return BTreeMap::new();
    };
    let Ok(Some(value)) = read_config_file(&path) else {
        return BTreeMap::new();
    };
    let mut result = BTreeMap::new();
    let Some(object) = value.as_object() else {
        return result;
    };
    for (path, keys) in object {
        let key_list = keys
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .filter_map(normalize_env_key)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !key_list.is_empty() {
            result.insert(path.clone(), key_list);
        }
    }
    result
}

fn write_settings_managed_env_keys(map: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
    let path = get_claude_code_settings_managed_env_keys_path()?;
    let value = serde_json::to_value(map)
        .map_err(|e| format!("序列化 Claude CLI settings 管理键失败: {}", e))?;
    write_config_file(&path, &value)
}

fn record_settings_managed_env_keys(
    settings_path: &Path,
    keys: BTreeSet<String>,
) -> Result<(), String> {
    let mut map = read_settings_managed_env_keys();
    let map_key = managed_env_store_key(settings_path);
    if keys.is_empty() {
        map.remove(&map_key);
    } else {
        map.insert(map_key, keys.into_iter().collect());
    }
    write_settings_managed_env_keys(&map)
}

fn managed_env_keys_for_settings(settings_path: &Path) -> BTreeSet<String> {
    let mut keys = CLAUDE_CODE_API_ENV_KEYS
        .iter()
        .filter_map(|key| normalize_env_key(key))
        .collect::<BTreeSet<_>>();
    let map = read_settings_managed_env_keys();
    if let Some(recorded) = map.get(&managed_env_store_key(settings_path)) {
        keys.extend(recorded.iter().filter_map(|key| normalize_env_key(key)));
    }
    keys
}

fn clear_api_key_env_from_claude_code_settings(config_dir: &Path) -> Result<(), String> {
    let settings_path = get_claude_code_settings_path(config_dir);
    let recorded_keys = read_settings_managed_env_keys();
    let has_recorded_keys = recorded_keys.contains_key(&managed_env_store_key(&settings_path));
    if !settings_path.exists() {
        if has_recorded_keys {
            record_settings_managed_env_keys(&settings_path, BTreeSet::new())?;
        }
        return Ok(());
    }

    let keys = managed_env_keys_for_settings(&settings_path);
    if keys.is_empty() {
        return Ok(());
    }

    let mut settings = read_config_file(&settings_path)?.unwrap_or_else(|| json!({}));
    if !settings.is_object() {
        settings = json!({});
    }

    if let Some(env_object) = settings
        .get_mut("env")
        .and_then(|value| value.as_object_mut())
    {
        for key in &keys {
            env_object.remove(key);
        }
    }

    write_config_file(&settings_path, &settings)?;
    record_settings_managed_env_keys(&settings_path, BTreeSet::new())
}

pub(crate) fn build_api_key_cli_env_map(
    account: &ClaudeAccount,
) -> Result<BTreeMap<String, String>, String> {
    let api_key = account
        .api_key
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .ok_or_else(|| "Claude API Key 账号缺少 API Key".to_string())?;
    let api_base_url = account
        .api_base_url
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)));
    let key_field =
        normalize_api_key_field(account.api_key_field.as_deref(), api_base_url.as_deref());
    let mut env = BTreeMap::new();
    if let Some(extra_env) = account.api_extra_env.as_ref() {
        for (key, value) in extra_env {
            let Some(key) = normalize_env_key(key) else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if matches!(
                key.as_str(),
                "ANTHROPIC_API_KEY" | "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_BASE_URL"
            ) {
                continue;
            }
            env.insert(key, value.to_string());
        }
    }
    if let Some(api_base_url) = api_base_url {
        env.insert("ANTHROPIC_BASE_URL".to_string(), api_base_url);
    }
    env.insert(key_field, api_key);
    Ok(env)
}

fn inject_api_key_to_claude_code_settings(
    account: &ClaudeAccount,
    config_dir: Option<&Path>,
) -> Result<(), String> {
    let config_dir = get_effective_claude_code_config_dir(config_dir)?;
    let settings_path = get_claude_code_settings_path(&config_dir);
    let env = build_api_key_cli_env_map(account)?;
    let managed_keys = env.keys().cloned().collect::<BTreeSet<_>>();

    fs::create_dir_all(&config_dir).map_err(|e| format!("创建 Claude Code 配置目录失败: {}", e))?;
    let mut settings = read_config_file(&settings_path)?.unwrap_or_else(|| json!({}));
    if !settings.is_object() {
        settings = json!({});
    }

    let keys_to_clear = managed_env_keys_for_settings(&settings_path);
    let object = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json 结构非法".to_string())?;
    let env_value = object.entry("env".to_string()).or_insert_with(|| json!({}));
    if !env_value.is_object() {
        *env_value = json!({});
    }
    let env_object = env_value
        .as_object_mut()
        .ok_or_else(|| "Claude settings.json env 结构非法".to_string())?;
    for key in keys_to_clear {
        env_object.remove(&key);
    }
    for (key, value) in env {
        env_object.insert(key, Value::String(value));
    }

    write_config_file(&settings_path, &settings)?;
    record_settings_managed_env_keys(&settings_path, managed_keys)
}

#[cfg(target_os = "macos")]
fn claude_code_keychain_service_name(config_dir: &Path) -> String {
    let env_config_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .and_then(|value| normalize_non_empty(Some(&value)));
    let default_unscoped_dir = env_config_dir.is_none()
        && dirs::home_dir()
            .map(|home| home.join(".claude") == config_dir)
            .unwrap_or(false);
    let hash_suffix = if default_unscoped_dir {
        String::new()
    } else {
        let value = config_dir.to_string_lossy();
        let digest = Sha256::digest(value.as_bytes());
        let hex = hex_encode(&digest);
        format!("-{}", &hex[..8])
    };
    format!(
        "{}{}{}",
        CLAUDE_CODE_KEYCHAIN_SERVICE_PREFIX, CLAUDE_CODE_KEYCHAIN_CREDENTIALS_SUFFIX, hash_suffix
    )
}

#[cfg(target_os = "macos")]
fn claude_code_keychain_account_name() -> String {
    std::env::var("USER")
        .ok()
        .and_then(|value| normalize_non_empty(Some(&value)))
        .or_else(|| {
            std::env::var("LOGNAME")
                .ok()
                .and_then(|value| normalize_non_empty(Some(&value)))
        })
        .unwrap_or_else(|| "claude-code-user".to_string())
}

#[cfg(target_os = "macos")]
fn read_claude_code_keychain_credentials(config_dir: &Path) -> Option<Value> {
    let service = claude_code_keychain_service_name(config_dir);
    let account = claude_code_keychain_account_name();
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            account.as_str(),
            "-w",
            "-s",
            service.as_str(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim()).ok()
}

#[cfg(target_os = "macos")]
fn write_claude_code_keychain_credentials(
    config_dir: &Path,
    credentials: &Value,
) -> Result<(), String> {
    let service = claude_code_keychain_service_name(config_dir);
    let account = claude_code_keychain_account_name();
    let content = serde_json::to_string(credentials)
        .map_err(|e| format!("序列化 Claude Code Keychain credentials 失败: {}", e))?;
    let hex_content = hex_encode(content.as_bytes());
    let output = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-a",
            account.as_str(),
            "-s",
            service.as_str(),
            "-X",
            hex_content.as_str(),
        ])
        .output()
        .map_err(|e| format!("调用 macOS Keychain 失败: {}", e))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr.trim();
    Err(format!(
        "写入 macOS Keychain 失败: {}",
        if message.is_empty() {
            "unknown error"
        } else {
            message
        }
    ))
}

#[cfg(target_os = "macos")]
fn delete_claude_code_keychain_credentials(config_dir: &Path) {
    let service = claude_code_keychain_service_name(config_dir);
    let account = claude_code_keychain_account_name();
    let _ = std::process::Command::new("security")
        .args([
            "delete-generic-password",
            "-a",
            account.as_str(),
            "-s",
            service.as_str(),
        ])
        .output();
}

fn read_plaintext_claude_code_credentials(config_dir: &Path) -> Option<Value> {
    read_config_file(&get_claude_code_credentials_path(config_dir))
        .ok()
        .flatten()
}

fn read_claude_code_credentials(config_dir: &Path) -> Value {
    #[cfg(target_os = "macos")]
    if let Some(value) = read_claude_code_keychain_credentials(config_dir) {
        return value;
    }
    read_plaintext_claude_code_credentials(config_dir).unwrap_or_else(|| json!({}))
}

fn write_plaintext_claude_code_credentials(
    config_dir: &Path,
    credentials: &Value,
) -> Result<(), String> {
    write_config_file(&get_claude_code_credentials_path(config_dir), credentials)
}

fn write_claude_code_credentials(config_dir: &Path, credentials: &Value) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        match write_claude_code_keychain_credentials(config_dir, credentials) {
            Ok(()) => {
                let _ = remove_path_if_exists(&get_claude_code_credentials_path(config_dir));
                return Ok(());
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "[Claude Code] Keychain 写入失败，回退到 .credentials.json: {}",
                    error
                ));
                write_plaintext_claude_code_credentials(config_dir, credentials)?;
                delete_claude_code_keychain_credentials(config_dir);
                return Ok(());
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        write_plaintext_claude_code_credentials(config_dir, credentials)
    }
}

fn merge_claude_code_oauth_config(mut target: Value, snapshot: &Value) -> Value {
    if !target.is_object() {
        target = json!({});
    }
    if let Some(target_object) = target.as_object_mut() {
        if let Some(oauth_account) = snapshot.get("oauthAccount").cloned() {
            target_object.insert("oauthAccount".to_string(), oauth_account);
        }
        target_object.insert("hasCompletedOnboarding".to_string(), Value::Bool(true));
    }
    target
}

fn inject_oauth_account_to_claude_code(
    account: &ClaudeAccount,
    config_dir: Option<&Path>,
) -> Result<(), String> {
    let config_dir = get_effective_claude_code_config_dir(config_dir)?;
    let credentials_snapshot = account
        .claude_credentials_raw
        .as_ref()
        .ok_or_else(|| "Claude OAuth 账号缺少 credentials 快照".to_string())?;
    let oauth_credentials = credentials_snapshot
        .get("claudeAiOauth")
        .cloned()
        .ok_or_else(|| "Claude OAuth 账号 credentials 缺少 claudeAiOauth".to_string())?;
    let config_snapshot = account
        .claude_config_raw
        .as_ref()
        .ok_or_else(|| "Claude OAuth 账号缺少 config 快照".to_string())?;
    if config_snapshot.get("oauthAccount").is_none() {
        return Err("Claude OAuth 账号 config 缺少 oauthAccount".to_string());
    }

    let mut credentials = read_claude_code_credentials(&config_dir);
    if !credentials.is_object() {
        credentials = json!({});
    }
    if let Some(object) = credentials.as_object_mut() {
        object.insert("claudeAiOauth".to_string(), oauth_credentials);
    }
    write_claude_code_credentials(&config_dir, &credentials)?;

    let global_config_path = get_claude_code_global_config_path(&config_dir)?;
    let target_config = read_config_file(&global_config_path)?.unwrap_or_else(|| json!({}));
    let merged_config = merge_claude_code_oauth_config(target_config, config_snapshot);
    write_config_file(&global_config_path, &merged_config)?;
    Ok(())
}

pub fn inject_to_claude_config(account_id: &str, config_dir: Option<&Path>) -> Result<(), String> {
    let account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if account.auth_mode == ClaudeAuthMode::DesktopGateway {
        if let Some(target_dir) = config_dir {
            restore_desktop_gateway_account_to_profile(account_id, target_dir, false)?;
            return Ok(());
        }
        quit_claude_desktop_for_profile_write()?;
        write_default_desktop_gateway_profile(&account)?;
        crate::modules::claude_instance::ensure_claude_launch_path_configured()?;
        launch_default_claude_desktop()?;

        let mut updated = account.clone();
        updated.last_used = now_ts_ms();
        save_account_and_index(updated)?;
        return Ok(());
    }
    if account.auth_mode == ClaudeAuthMode::DesktopOAuth {
        if config_dir.is_some() {
            return Err("Claude 登录态不能写入旧配置目录，请使用 Claude 实例。".to_string());
        }
        let snapshot_dir = account
            .desktop_profile_dir
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .map(PathBuf::from)
            .ok_or_else(|| "Claude 账号缺少 profile 快照".to_string())?;
        let target_dir = get_default_claude_desktop_user_data_dir()?;
        quit_claude_desktop_for_profile_write()?;
        let _backup_dir = backup_current_desktop_profile(&target_dir)?;
        restore_default_desktop_gateway_official_config()?;
        restore_desktop_profile_snapshot(&snapshot_dir, &target_dir)?;
        restore_default_desktop_gateway_official_config()?;

        let mut updated = account.clone();
        updated.last_used = now_ts_ms();
        save_account_and_index(updated)?;
        launch_default_claude_desktop()?;
        return Ok(());
    }
    if account.auth_mode == ClaudeAuthMode::ApiKey {
        inject_api_key_to_claude_code_settings(&account, config_dir)?;
        let mut updated = account.clone();
        updated.last_used = now_ts_ms();
        save_account_and_index(updated)?;
        return Ok(());
    }
    let config_dir_path = get_effective_claude_code_config_dir(config_dir)?;
    clear_api_key_env_from_claude_code_settings(&config_dir_path)?;
    inject_oauth_account_to_claude_code(&account, config_dir)?;

    let mut updated = account.clone();
    updated.last_used = now_ts_ms();
    save_account_and_index(updated)?;
    Ok(())
}

pub fn inject_to_claude(account_id: &str) -> Result<(), String> {
    inject_to_claude_config(account_id, None)
}

pub fn resolve_current_account_for_platform(
    platform: &str,
    accounts: &[ClaudeAccount],
) -> Option<ClaudeAccount> {
    let current_id = crate::modules::provider_current_state::resolve_existing_current_account_id(
        platform,
        accounts.iter().map(|item| item.id.as_str()),
    );
    if let Some(current_id) = current_id {
        if let Some(account) = accounts.iter().find(|item| item.id == current_id) {
            return Some(account.clone());
        }
    }
    None
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    remove_accounts(&[account_id.to_string()])
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁")?;
    let mut index = load_index()?;
    for account_id in account_ids {
        if let Some(account) = load_account_file(account_id) {
            if account.auth_mode == ClaudeAuthMode::DesktopOAuth {
                if let Some(snapshot_dir) = account
                    .desktop_profile_dir
                    .as_deref()
                    .and_then(|value| normalize_non_empty(Some(value)))
                {
                    let snapshot_path = PathBuf::from(snapshot_dir);
                    if snapshot_path.exists() {
                        fs::remove_dir_all(&snapshot_path).map_err(|e| {
                            format!(
                                "删除 Claude 快照失败: path={}, error={}",
                                snapshot_path.display(),
                                e
                            )
                        })?;
                    }
                }
            }
            if account.auth_mode == ClaudeAuthMode::DesktopGateway {
                if let Some(profile_dir) = account
                    .desktop_gateway_profile_dir
                    .as_deref()
                    .and_then(|value| normalize_non_empty(Some(value)))
                {
                    let profile_path = PathBuf::from(profile_dir);
                    if profile_path.exists() {
                        fs::remove_dir_all(&profile_path).map_err(|e| {
                            format!(
                                "删除 Claude Gateway profile 失败: path={}, error={}",
                                profile_path.display(),
                                e
                            )
                        })?;
                    }
                }
            }
        }
        let path = account_file_path(account_id)?;
        // Must share the atomic_write path lock with deferred CAS rewrites so a
        // concurrent migration cannot resurrect the deleted account file.
        crate::modules::atomic_write::remove_file_locked(&path)
            .map_err(|e| format!("删除 Claude 账号失败: path={}, error={}", path.display(), e))?;
    }
    index
        .accounts
        .retain(|item| !account_ids.iter().any(|id| id == &item.id));
    save_index(&index)?;
    for platform in ["claude_desktop_account", "claude_code_account"] {
        let _ = crate::modules::provider_current_state::resolve_existing_current_account_id(
            platform,
            index.accounts.iter().map(|item| item.id.as_str()),
        );
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁")?;
    let mut account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    account.tags = Some(
        tags.into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect(),
    );
    save_account_and_index(account)
}

pub fn update_account_plan(
    account_id: &str,
    plan_type: Option<&str>,
) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁")?;
    let mut account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    account.plan_type = plan_type
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(|value| value.to_string());
    save_account_and_index(account)
}

pub fn update_account_note(account_id: &str, note: Option<&str>) -> Result<ClaudeAccount, String> {
    let _lock = CLAUDE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "无法获取 Claude 账号锁")?;
    let mut account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    account.account_note = note
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(|value| value.to_string());
    save_account_and_index(account)
}

fn usage_to_quota(raw: &Value) -> ClaudeQuota {
    let five_hour = raw.get("five_hour");
    let seven_day = raw.get("seven_day");
    let seven_day_sonnet = raw
        .get("seven_day_sonnet")
        .or_else(|| raw.get("seven_day_sonnet_4"))
        .or_else(|| raw.get("seven_day_model"));
    let extra_usage = raw.get("extra_usage");

    let extra_enabled = extra_usage
        .and_then(|item| item.get("is_enabled"))
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let extra_usage_percentage = extra_enabled.then(|| {
        clamp_percentage(
            extra_usage
                .and_then(|item| item.get("utilization"))
                .and_then(|item| item.as_f64()),
        )
    });

    ClaudeQuota {
        five_hour_percentage: clamp_percentage(
            five_hour
                .and_then(|item| item.get("utilization"))
                .and_then(|item| item.as_f64()),
        ),
        five_hour_reset_time: parse_reset_seconds(five_hour.and_then(|item| item.get("resets_at"))),
        seven_day_percentage: clamp_percentage(
            seven_day
                .and_then(|item| item.get("utilization"))
                .and_then(|item| item.as_f64()),
        ),
        seven_day_reset_time: parse_reset_seconds(seven_day.and_then(|item| item.get("resets_at"))),
        seven_day_sonnet_percentage: seven_day_sonnet
            .map(|item| clamp_percentage(item.get("utilization").and_then(|value| value.as_f64()))),
        seven_day_sonnet_reset_time: parse_reset_seconds(
            seven_day_sonnet.and_then(|item| item.get("resets_at")),
        ),
        extra_usage_percentage,
        extra_usage_reset_time: parse_reset_seconds(
            extra_usage.and_then(|item| item.get("resets_at")),
        ),
        extra_usage_used_cents: read_i64_value(
            extra_usage.and_then(|item| item.get("used_credits")),
        ),
        extra_usage_limit_cents: read_i64_value(
            extra_usage.and_then(|item| item.get("monthly_limit")),
        ),
        raw_data: Some(raw.clone()),
    }
}

async fn refresh_oauth_credentials(credentials: &Value) -> Result<Option<Value>, String> {
    let Some(refresh_token) = credentials_refresh_token(credentials) else {
        return Ok(None);
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client
        .post(CLAUDE_OAUTH_TOKEN_URL)
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, "antigravity-cockpit-tools")
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLAUDE_OAUTH_CLIENT_ID,
        }))
        .send()
        .await
        .map_err(|e| format!("刷新 Claude OAuth token 失败: {}", e))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 Claude OAuth 响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!(
            "刷新 Claude OAuth token 失败: HTTP {} {}",
            status, body
        ));
    }
    let payload: Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 Claude OAuth 响应失败: {}", e))?;
    let mut next = credentials.clone();
    let oauth = next
        .get_mut("claudeAiOauth")
        .and_then(|item| item.as_object_mut())
        .ok_or_else(|| "Claude credentials 缺少 claudeAiOauth 字段".to_string())?;
    if let Some(access_token) = read_string_path(&payload, &["access_token"]) {
        oauth.insert("accessToken".to_string(), Value::String(access_token));
    }
    if let Some(refresh_token) = read_string_path(&payload, &["refresh_token"]) {
        oauth.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if let Some(expires_in) = read_i64_value(payload.get("expires_in")) {
        oauth.insert(
            "expiresAt".to_string(),
            Value::Number(serde_json::Number::from(now_ts_ms() + expires_in * 1000)),
        );
    }
    if let Some(scope) = read_string_path(&payload, &["scope"]) {
        oauth.insert(
            "scopes".to_string(),
            Value::Array(
                scope
                    .split_whitespace()
                    .map(|item| Value::String(item.to_string()))
                    .collect(),
            ),
        );
    }
    Ok(Some(next))
}

async fn request_usage(access_token: &str) -> Result<Value, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", access_token))
            .map_err(|e| format!("构造 Claude usage Authorization 失败: {}", e))?,
    );
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static(CLAUDE_OAUTH_BETA_HEADER),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("antigravity-cockpit-tools"),
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let resp = client
        .get(CLAUDE_OAUTH_USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求 Claude usage 失败: {}", e))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 Claude usage 响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!("请求 Claude usage 失败: HTTP {} {}", status, body));
    }
    serde_json::from_str(&body).map_err(|e| format!("解析 Claude usage 响应失败: {}", e))
}

pub async fn refresh_account_quota(account_id: &str) -> Result<ClaudeAccount, String> {
    let mut account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if matches!(
        account.auth_mode,
        ClaudeAuthMode::ApiKey | ClaudeAuthMode::DesktopGateway
    ) {
        account.quota = None;
        account.quota_error = Some(ClaudeQuotaErrorInfo {
            code: Some("unsupported_auth_mode".to_string()),
            message: if account.auth_mode == ClaudeAuthMode::DesktopGateway {
                "Claude Gateway 账号不支持 Claude 订阅配额刷新，请在供应商后台查看用量。"
                    .to_string()
            } else {
                "Claude API Key 账号不支持 Claude 订阅配额刷新，请在 Anthropic Console 查看用量。"
                    .to_string()
            },
            timestamp: now_ts(),
        });
        account.usage_updated_at = Some(now_ts_ms());
        return save_account_and_index(account);
    }
    if account.auth_mode == ClaudeAuthMode::DesktopOAuth {
        let snapshot_dir = resolve_valid_desktop_profile_dir(&mut account)?;
        let local_profile_applied = apply_desktop_local_profile(&mut account, &snapshot_dir);
        let silent_result = fetch_desktop_web_profile_silent(&snapshot_dir).await;
        let web_profile_result =
            resolve_desktop_web_profile_with_hidden_probe(account_id, silent_result, || {
                probe_desktop_web_profile_hidden_with_cooldown(account_id, &snapshot_dir)
            })
            .await;
        match web_profile_result {
            Ok(web_profile) => {
                let web_quota_available = desktop_web_usage_to_quota(&web_profile).is_some();
                let usage_error = desktop_web_usage_error_message(&web_profile);
                let profile_applied = apply_desktop_web_profile(&mut account, &web_profile);
                if profile_applied
                    || local_profile_applied
                    || desktop_account_has_real_profile_data(&account)
                {
                    account.status = None;
                    account.status_reason = None;
                    if web_quota_available {
                        account.quota_error = None;
                    } else if let Some(message) = usage_error {
                        account.quota_error = Some(ClaudeQuotaErrorInfo {
                            code: Some("desktop_usage_refresh_failed".to_string()),
                            message,
                            timestamp: now_ts(),
                        });
                    } else {
                        account.quota_error = None;
                    }
                } else {
                    let message =
                        desktop_web_profile_error_message(&web_profile).unwrap_or_else(|| {
                            "Claude 资料接口未返回邮箱、头像或套餐字段。".to_string()
                        });
                    account.quota_error = Some(ClaudeQuotaErrorInfo {
                        code: Some("desktop_profile_failed".to_string()),
                        message: message.clone(),
                        timestamp: now_ts(),
                    });
                    account.status_reason = Some(message);
                }
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "[Claude] 刷新账号资料失败: account_id={}, error={}",
                    account_id, error
                ));
                let message = format!("Claude 资料刷新失败: {}", error);
                if local_profile_applied || desktop_account_has_real_profile_data(&account) {
                    account.quota_error = Some(ClaudeQuotaErrorInfo {
                        code: Some("desktop_usage_refresh_failed".to_string()),
                        message,
                        timestamp: now_ts(),
                    });
                    account.status = None;
                    account.status_reason = None;
                } else {
                    account.quota_error = Some(ClaudeQuotaErrorInfo {
                        code: Some("desktop_profile_failed".to_string()),
                        message: message.clone(),
                        timestamp: now_ts(),
                    });
                    account.status_reason = Some(message);
                }
            }
        }
        return save_account_and_index(account);
    }

    let mut credentials = account
        .claude_credentials_raw
        .clone()
        .ok_or_else(|| "Claude 账号缺少 credentials 快照".to_string())?;

    if token_is_expired(&credentials) {
        match refresh_oauth_credentials(&credentials).await {
            Ok(Some(refreshed)) => {
                credentials = refreshed;
                account.claude_credentials_raw = Some(credentials.clone());
            }
            Ok(None) => {}
            Err(error) => {
                account.quota_error = Some(ClaudeQuotaErrorInfo {
                    code: Some("refresh_failed".to_string()),
                    message: error,
                    timestamp: now_ts(),
                });
                account.usage_updated_at = Some(now_ts_ms());
                return save_account_and_index(account);
            }
        }
    }

    let Some(access_token) = credentials_access_token(&credentials) else {
        account.quota_error = Some(ClaudeQuotaErrorInfo {
            code: Some("missing_access_token".to_string()),
            message: "Claude 账号缺少 accessToken".to_string(),
            timestamp: now_ts(),
        });
        account.usage_updated_at = Some(now_ts_ms());
        return save_account_and_index(account);
    };

    match request_usage(&access_token).await {
        Ok(usage) => {
            account.quota = Some(usage_to_quota(&usage));
            account.claude_usage_raw = Some(usage);
            account.usage_updated_at = Some(now_ts_ms());
            account.quota_error = None;
            account.status = None;
            account.status_reason = None;
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Claude Quota] 刷新失败: account_id={}, error={}",
                account_id, error
            ));
            account.quota_error = Some(ClaudeQuotaErrorInfo {
                code: Some("usage_failed".to_string()),
                message: error,
                timestamp: now_ts(),
            });
            account.usage_updated_at = Some(now_ts_ms());
        }
    }
    save_account_and_index(account)
}

pub async fn refresh_all_quotas() -> Result<Vec<(String, Result<ClaudeAccount, String>)>, String> {
    let accounts = list_accounts_checked()?;
    let mut results = Vec::with_capacity(accounts.len());
    for account in accounts {
        let id = account.id.clone();
        results.push((id.clone(), refresh_account_quota(&id).await));
    }
    Ok(results)
}

