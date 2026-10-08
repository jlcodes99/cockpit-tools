// Credential rotation policy and bounded background synchronization for Claude CLI.

fn read_claude_code_credentials_for_sync(config_dir: &Path) -> Result<Value, String> {
    #[cfg(target_os = "macos")]
    if let Some(value) = read_claude_code_keychain_credentials(config_dir)? {
        return Ok(value);
    }
    // A failed Keychain read must not make an old plaintext fallback authoritative.
    Ok(
        read_config_file(&get_claude_code_credentials_path(config_dir))?
            .unwrap_or_else(|| json!({})),
    )
}

fn write_claude_code_credentials_for_sync(
    config_dir: &Path,
    credentials: &Value,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Background maintenance must not delete a Keychain item or replace it
        // with plaintext just because the system Keychain is temporarily unavailable.
        write_claude_code_keychain_credentials(config_dir, credentials)?;
        let _ = remove_path_if_exists(&get_claude_code_credentials_path(config_dir));
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    write_plaintext_claude_code_credentials(config_dir, credentials)
}

fn should_sync_cli_oauth_credentials(account: Option<&Value>, instance: &Value) -> bool {
    let Some(instance_access) = credentials_access_token(instance) else {
        return false;
    };
    let Some(account) = account else {
        return true;
    };
    let account_refresh = credentials_refresh_token(account);
    let instance_refresh = credentials_refresh_token(instance);
    if account_refresh.is_some() && instance_refresh.is_none() {
        return false;
    }
    let Some(account_access) = credentials_access_token(account) else {
        return true;
    };
    let same_tokens = account_access == instance_access && account_refresh == instance_refresh;
    // A changed token is not proof of a rotation: it can also be the revoked
    // credential left in an idle CLI after Cockpit refreshed or reauthorized.
    match (
        credentials_expires_at(account),
        credentials_expires_at(instance),
    ) {
        (Some(saved), Some(local)) => local > saved || (local == saved && same_tokens),
        (None, Some(_)) => true,
        (None, None) => same_tokens,
        _ => false,
    }
}

fn merge_newer_account_oauth_credentials(account: &Value, instance: &Value) -> Option<Value> {
    let account_access = credentials_access_token(account)?;
    let account_refresh = credentials_refresh_token(account)?;
    let instance_access = credentials_access_token(instance)?;
    if account_access == instance_access
        && Some(account_refresh) == credentials_refresh_token(instance)
    {
        return None;
    }
    let account_expiry = credentials_expires_at(account)?;
    if credentials_expires_at(instance).is_some_and(|local| local >= account_expiry) {
        return None;
    }
    let mut merged = instance.clone();
    merged.as_object_mut()?.insert(
        "claudeAiOauth".to_string(),
        account.get("claudeAiOauth")?.clone(),
    );
    Some(merged)
}

fn cli_config_matches_account(account: &ClaudeAccount, config: &Value) -> bool {
    let Ok(instance) =
        derive_account_from_snapshots(json!({"claudeAiOauth": {}}), config.clone(), None)
    else {
        return false;
    };
    cli_accounts_same_identity(account, &instance)
        && match (&account.organization_uuid, &instance.organization_uuid) {
            (Some(expected), Some(actual)) => expected == actual,
            _ => true,
        }
}

fn bound_cli_config_dirs(
    store: &crate::models::InstanceStore,
    account_id: &str,
    default_dir: Option<PathBuf>,
) -> BTreeSet<PathBuf> {
    let mut dirs: BTreeSet<_> = store
        .instances
        .iter()
        .filter(|instance| {
            instance.launch_mode == crate::models::InstanceLaunchMode::Cli
                && instance.bind_account_id.as_deref() == Some(account_id)
        })
        .map(|instance| PathBuf::from(&instance.user_data_dir))
        .collect();
    if store.default_settings.launch_mode == crate::models::InstanceLaunchMode::Cli
        && store.default_settings.bind_account_id.as_deref() == Some(account_id)
    {
        dirs.extend(default_dir);
    }
    dirs
}

fn sync_account_oauth_to_bound_cli_instances(account_id: &str) -> Result<(), String> {
    let store = crate::modules::claude_instance::load_instance_store()?;
    let dirs = bound_cli_config_dirs(
        &store,
        account_id,
        get_default_claude_code_config_dir().ok(),
    );
    for config_dir in dirs {
        // Refresh snapshots per instance, so a queued task cannot propagate a
        // captured token after a newer refresh, reauthorization or account switch.
        let Some(account) = load_account(account_id) else {
            break;
        };
        if account.auth_mode != ClaudeAuthMode::OAuth || !config_dir.is_dir() {
            continue;
        }
        let Some(snapshot) = account.claude_credentials_raw.as_ref() else {
            continue;
        };
        let config_path = get_claude_code_global_config_path(&config_dir)?;
        let Some(config) = read_config_file(&config_path)? else {
            continue;
        };
        if !cli_config_matches_account(&account, &config) {
            continue;
        }
        let instance = match read_claude_code_credentials_for_sync(&config_dir) {
            Ok(value) => value,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Claude CLI] 跳过凭证同步，实例凭证无法安全读取: account_id={}, error={}",
                    account_id, error
                ));
                continue;
            }
        };
        let Some(merged) = merge_newer_account_oauth_credentials(snapshot, &instance) else {
            continue;
        };
        // A Keychain probe can take seconds. Recheck binding, identity and saved
        // credentials after it to avoid propagating a snapshot superseded meanwhile.
        let current_store = crate::modules::claude_instance::load_instance_store()?;
        if !bound_cli_config_dirs(
            &current_store,
            account_id,
            get_default_claude_code_config_dir().ok(),
        )
        .contains(&config_dir)
            || !read_config_file(&config_path)?
                .as_ref()
                .is_some_and(|config| cli_config_matches_account(&account, config))
            || load_account(account_id)
                .as_ref()
                .and_then(|current| current.claude_credentials_raw.as_ref())
                != Some(snapshot)
            || read_claude_code_credentials_for_sync(&config_dir)? != instance
        {
            continue;
        }
        match write_claude_code_credentials_for_sync(&config_dir, &merged) {
            Ok(()) => logger::log_info(&format!(
                "[Claude CLI] 已同步账号凭证到绑定实例: account_id={}, config_dir={}",
                account_id,
                config_dir.display()
            )),
            Err(error) => logger::log_warn(&format!(
                "[Claude CLI] 绑定实例凭证同步失败，下次刷新时重试: account_id={}, error={}",
                account_id, error
            )),
        }
    }
    Ok(())
}

#[derive(Default)]
struct CliCredentialSyncQueue {
    // true means another request arrived while this account's worker was running.
    accounts: HashMap<String, bool>,
}

impl CliCredentialSyncQueue {
    fn enqueue(&mut self, account_id: &str) -> bool {
        match self.accounts.entry(account_id.to_string()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.insert(true);
                false
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(false);
                true
            }
        }
    }

    fn finish_pass(&mut self, account_id: &str) -> bool {
        if self.accounts.get(account_id) == Some(&true) {
            self.accounts.insert(account_id.to_string(), false);
            true
        } else {
            self.accounts.remove(account_id);
            false
        }
    }
}

static CLAUDE_CLI_CREDENTIAL_SYNC: std::sync::LazyLock<Mutex<CliCredentialSyncQueue>> =
    std::sync::LazyLock::new(|| Mutex::new(CliCredentialSyncQueue::default()));

fn schedule_cli_oauth_credential_sync(account_id: &str) {
    let should_start = CLAUDE_CLI_CREDENTIAL_SYNC
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .enqueue(account_id);
    if !should_start {
        return;
    }
    let account_id = account_id.to_string();
    tauri::async_runtime::spawn(async move {
        loop {
            let worker_id = account_id.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                sync_account_oauth_to_bound_cli_instances(&worker_id)
            })
            .await;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => logger::log_warn(&format!(
                    "[Claude CLI] 后台凭证同步失败，下次刷新时重试: account_id={}, error={}",
                    account_id, error
                )),
                Err(error) => logger::log_warn(&format!(
                    "[Claude CLI] 后台凭证同步任务失败: account_id={}, error={}",
                    account_id, error
                )),
            }
            if !CLAUDE_CLI_CREDENTIAL_SYNC
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .finish_pass(&account_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
}
