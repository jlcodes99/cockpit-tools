// Desktop account switches from UI, auto-switch and WebSocket share this transaction.
// Reject overlapping switches rather than queueing stale account selections.
static DESKTOP_SWITCH_GATE: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)));

fn try_begin_desktop_switch(
    gate: &std::sync::Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    gate.clone()
        .try_acquire_owned()
        .map_err(|_| "ANTIGRAVITY_SWITCH_BUSY".to_string())
}

/// Only commit Tools' account identity after the real client credential is written.
/// A launch failure is separate: the credential switch has already succeeded.
fn complete_desktop_switch(
    inject: impl FnOnce() -> Result<(), String>,
    commit: impl FnOnce() -> Result<(), String>,
    launch_on_switch: bool,
    launch: impl FnOnce() -> Result<(), String>,
) -> Result<Option<String>, String> {
    inject()?;
    commit().map_err(|error| format!("ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED: {error}"))?;
    Ok(if launch_on_switch {
        launch().err()
    } else {
        None
    })
}

pub(crate) async fn switch_account_desktop(
    account_id: String,
    launch_on_switch: bool,
) -> Result<models::Account, String> {
    let permit = try_begin_desktop_switch(&DESKTOP_SWITCH_GATE)?;
    modules::logger::log_info(&format!(
        "[Antigravity] 开始桌面切号: account_id={}, launch={}",
        account_id, launch_on_switch
    ));
    let account = tokio::time::timeout(
        Duration::from_secs(60),
        modules::account::prepare_account_for_injection(&account_id),
    )
    .await
    .map_err(|_| "ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT".to_string())??;

    // Version probes, process control, credential access and disk writes must not
    // occupy a Tokio runtime thread. The blocking worker owns the single-flight
    // permit, so dropping a caller cannot start a competing credential write.
    let (account, launch_error, _permit) = tokio::task::spawn_blocking(move || {
        if launch_on_switch {
            modules::process::ensure_antigravity_legacy_launch_path_configured()?;
        }
        let auth_mode = resolve_antigravity_desktop_auth_mode()?;
        let mut account = account;
        account.update_last_used();
        let launch_error = complete_desktop_switch(
            || {
                // Persist prepared account data before changing the real credential.
                // The current-account pointer and default binding are committed later.
                modules::save_account(&account)?;
                if launch_on_switch {
                    let default_dir = legacy_antigravity_user_data_dir()?
                        .to_string_lossy()
                        .to_string();
                    modules::process::close_antigravity_legacy_instances(
                        &[default_dir.clone()],
                        &default_dir,
                        20,
                    )?;
                    let _ = modules::antigravity_legacy_instance::update_default_pid(None);
                }
                match auth_mode {
                    AntigravityDesktopAuthMode::SystemCredential => {
                        modules::logger::log_info("[Antigravity] 使用系统凭据认证模式写入账号");
                        modules::antigravity_credential::write_antigravity_system_credential(
                            &account,
                        )
                    }
                    AntigravityDesktopAuthMode::LegacyStateDb => {
                        modules::logger::log_info("[Antigravity] 使用旧版 SQLite 认证模式写入账号");
                        let db_path = legacy_antigravity_state_db_path()?;
                        modules::db::inject_account_token_to_path(&db_path, &account).map(|_| ())
                    }
                }
            },
            || {
                if let Err(error) = modules::antigravity_legacy_instance::update_default_settings(
                    Some(Some(account.id.clone())),
                    None,
                    Some(false),
                ) {
                    // Preserve the UI's existing best-effort binding behavior.
                    modules::logger::log_warn(&format!(
                        "更新 Antigravity 默认实例绑定账号失败: {error}"
                    ));
                }
                modules::set_current_account_id(&account.id)
            },
            launch_on_switch,
            || {
                let settings = modules::antigravity_legacy_instance::load_default_settings()?;
                let args = modules::process::parse_extra_args(&settings.extra_args);
                let pid = modules::process::start_antigravity_legacy_with_args("", &args)?;
                if let Err(error) =
                    modules::antigravity_legacy_instance::update_default_pid(Some(pid))
                {
                    modules::logger::log_warn(&format!(
                        "更新 Antigravity 默认实例 PID 失败: {}",
                        error
                    ));
                }
                Ok(())
            },
        )?;
        Ok::<_, String>((account, launch_error, permit))
    })
    .await
    .map_err(|error| format!("Antigravity 桌面切号任务失败: {}", error))??;

    modules::websocket::broadcast_account_switched(&account.id, &account.email);
    if let Some(error) = launch_error {
        if error.starts_with("APP_PATH_NOT_FOUND:") {
            return Err(error);
        }
        return Err(format!("账号已切换，但启动 Antigravity 失败: {}", error));
    }
    Ok(account)
}

#[cfg(test)]
#[path = "account_desktop_switch_tests.rs"]
mod desktop_switch_tests;
