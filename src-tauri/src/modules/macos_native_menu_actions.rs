// macOS Native Menu：Background refresh, account switching and window actions。
// 通过 include! 保持原 imp 模块作用域和 Objective-C FFI 调用路径。
fn normalize_provider_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn find_codex_provider_for_account(
    providers: &[Value],
    account: &crate::models::codex::CodexAccount,
) -> Option<Value> {
    let provider_id = account
        .api_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(provider_id) = provider_id {
        if let Some(provider) = providers.iter().find(|provider| {
            json_path(Some(provider), &["id"])
                .and_then(Value::as_str)
                .map(str::trim)
                == Some(provider_id)
        }) {
            return Some(provider.clone());
        }
    }
    let account_base = account
        .api_base_url
        .as_deref()
        .map(normalize_provider_base_url)
        .filter(|value| !value.is_empty())?;
    providers
        .iter()
        .find(|provider| {
            json_path(Some(provider), &["baseUrl"])
                .and_then(Value::as_str)
                .map(normalize_provider_base_url)
                == Some(account_base.clone())
        })
        .cloned()
}

async fn save_detected_codex_provider_integration_type(
    provider_id: Option<&str>,
    base_url: &str,
    mode: &str,
) -> Result<(), String> {
    if mode != "new_api" && mode != "sub2api" {
        return Ok(());
    }
    let raw = commands::codex::load_codex_model_providers().await?;
    let mut providers: Value =
        serde_json::from_str(&raw).map_err(|err| format!("解析 Codex 模型供应商失败: {}", err))?;
    let Some(items) = providers.as_array_mut() else {
        return Ok(());
    };
    let normalized_base_url = normalize_provider_base_url(base_url);
    let mut changed = false;
    for provider in items {
        let id_matches = provider_id.is_some_and(|target_id| {
            json_path(Some(provider), &["id"])
                .and_then(Value::as_str)
                .map(str::trim)
                == Some(target_id)
        });
        let base_matches = json_path(Some(provider), &["baseUrl"])
            .and_then(Value::as_str)
            .map(normalize_provider_base_url)
            == Some(normalized_base_url.clone());
        if id_matches || base_matches {
            if provider
                .get("integrationType")
                .and_then(Value::as_str)
                .map(str::trim)
                != Some(mode)
            {
                if let Some(object) = provider.as_object_mut() {
                    object.insert(
                        "integrationType".to_string(),
                        Value::String(mode.to_string()),
                    );
                    object.insert(
                        "updatedAt".to_string(),
                        Value::Number(serde_json::Number::from(
                            chrono::Utc::now().timestamp_millis(),
                        )),
                    );
                    changed = true;
                }
            }
            break;
        }
    }
    if changed {
        let data = serde_json::to_string_pretty(&providers)
            .map_err(|err| format!("序列化 Codex 模型供应商失败: {}", err))?;
        commands::codex::save_codex_model_providers(data).await?;
    }
    Ok(())
}

async fn refresh_codex_api_key_usage_for_menu(
    app: AppHandle,
    account_id: String,
) -> Result<(), String> {
    let mut account = modules::codex_account::list_accounts()
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| "未找到 Codex API Key 账号".to_string())?;
    if !account.is_api_key_auth() {
        commands::codex::refresh_codex_quota(app, account_id).await?;
        return Ok(());
    }
    let api_key = account
        .openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Codex API Key 为空".to_string())?;
    let providers_raw = commands::codex::load_codex_model_providers().await?;
    let providers: Vec<Value> = serde_json::from_str(&providers_raw).unwrap_or_default();
    let provider = find_codex_provider_for_account(&providers, &account);
    let base_url = provider
        .as_ref()
        .and_then(|provider| json_path(Some(provider), &["baseUrl"]))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            account
                .api_base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| "Codex API Base URL 为空".to_string())?
        .to_string();
    let integration_type = provider
        .as_ref()
        .and_then(|provider| json_path(Some(provider), &["integrationType"]))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let summary = commands::codex::codex_query_model_provider_usage(
        base_url.clone(),
        api_key.to_string(),
        integration_type,
    )
    .await?;
    let summary_value = serde_json::to_value(&summary)
        .map_err(|err| format!("序列化 Codex API Key 用量失败: {}", err))?;
    let now = chrono::Utc::now().timestamp();
    let mut raw_data = account
        .quota
        .as_ref()
        .and_then(|quota| quota.raw_data.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    if !raw_data.is_object() {
        raw_data = serde_json::json!({});
    }
    if let Some(object) = raw_data.as_object_mut() {
        object.insert("provider_usage".to_string(), summary_value);
    }
    account.quota = Some(crate::models::codex::CodexQuota {
        hourly_percentage: 0,
        hourly_reset_time: None,
        hourly_window_minutes: None,
        hourly_window_present: Some(false),
        weekly_percentage: 0,
        weekly_reset_time: None,
        weekly_window_minutes: None,
        weekly_window_present: Some(false),
        reset_credits_available: None,
        reset_credits: Vec::new(),
        reset_credits_next_expires_at: None,
        raw_data: Some(raw_data),
    });
    account.quota_error = None;
    account.usage_updated_at = Some(now);
    modules::codex_account::save_account(&account)?;
    if let Some(mode) = summary.mode.as_deref() {
        let provider_id = provider
            .as_ref()
            .and_then(|provider| json_path(Some(provider), &["id"]))
            .and_then(Value::as_str);
        save_detected_codex_provider_integration_type(provider_id, &base_url, mode).await?;
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

async fn refresh_all_codex_usage_for_menu(app: AppHandle) -> Result<i32, String> {
    let accounts = modules::codex_account::list_accounts();
    let mut refreshed = 0;
    let mut last_error: Option<String> = None;
    for account in accounts {
        match refresh_codex_api_key_usage_for_menu(app.clone(), account.id.clone()).await {
            Ok(_) => refreshed += 1,
            Err(err) => last_error = Some(err),
        }
    }
    if refreshed > 0 {
        Ok(refreshed)
    } else {
        Err(last_error.unwrap_or_else(|| "没有可刷新的 Codex 账号".to_string()))
    }
}

async fn refresh_codex_api_service_pool_for_menu(app: AppHandle) -> Result<i32, String> {
    let target_ids = modules::codex_local_access::api_service_refreshable_account_ids();
    if target_ids.is_empty() {
        return Err("API 服务账号池暂无可刷新的额度".to_string());
    }
    let success_count = commands::codex::refresh_codex_quotas_batch(
        app.clone(),
        target_ids,
        Some(true),
        Some(false),
    )
    .await?;
    if success_count <= 0 {
        return Err("API 服务账号池额度刷新失败".to_string());
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count)
}

fn spawn_refresh(platform: PlatformId, account_id: Option<String>) {
    let Some(app) = crate::get_app_handle().cloned() else {
        return;
    };

    tauri::async_runtime::spawn(async move {
        let refresh_result = match (platform, account_id) {
            (PlatformId::Antigravity, Some(account_id)) => {
                commands::account::fetch_account_quota(account_id)
                    .await
                    .map(|_| 0)
                    .map_err(|err| err.to_string())
            }
            (PlatformId::Antigravity, None) => {
                commands::account::refresh_current_quota(app.clone())
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Codex, Some(account_id))
                if modules::codex_instance::is_api_service_bind_account_id(&account_id) =>
            {
                refresh_codex_api_service_pool_for_menu(app.clone()).await
            }
            (PlatformId::Codex, Some(account_id)) => {
                refresh_codex_api_key_usage_for_menu(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Codex, None) => refresh_all_codex_usage_for_menu(app.clone()).await,
            (PlatformId::Claude, Some(account_id)) => {
                commands::claude::refresh_claude_quota(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Claude, None) => {
                commands::claude::refresh_all_claude_quotas(app.clone()).await
            }
            (PlatformId::GitHubCopilot, Some(account_id)) => {
                commands::github_copilot::refresh_github_copilot_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::GitHubCopilot, None) => {
                commands::github_copilot::refresh_all_github_copilot_tokens(app.clone()).await
            }
            (PlatformId::Windsurf, Some(account_id)) => {
                commands::windsurf::refresh_windsurf_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Windsurf, None) => {
                commands::windsurf::refresh_all_windsurf_tokens(app.clone()).await
            }
            (PlatformId::Kiro, Some(account_id)) => {
                commands::kiro::refresh_kiro_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Kiro, None) => commands::kiro::refresh_all_kiro_tokens(app.clone()).await,
            (PlatformId::Cursor, Some(account_id)) => {
                commands::cursor::refresh_cursor_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Cursor, None) => {
                commands::cursor::refresh_all_cursor_tokens(app.clone()).await
            }
            (PlatformId::Grok, Some(account_id)) => {
                commands::grok::refresh_grok_account(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Grok, None) => {
                commands::grok::refresh_all_grok_accounts(app.clone()).await
            }
            (PlatformId::Codebuddy, Some(account_id)) => {
                commands::codebuddy::refresh_codebuddy_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Codebuddy, None) => {
                commands::codebuddy::refresh_all_codebuddy_tokens(app.clone()).await
            }
            (PlatformId::CodebuddyCn, Some(account_id)) => {
                commands::codebuddy_cn::refresh_codebuddy_cn_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::CodebuddyCn, None) => {
                commands::codebuddy_cn::refresh_all_codebuddy_cn_tokens(app.clone()).await
            }
            (PlatformId::Qoder, Some(account_id)) => {
                commands::qoder::refresh_qoder_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Qoder, None) => {
                commands::qoder::refresh_all_qoder_tokens(app.clone()).await
            }
            (PlatformId::Zcode, Some(account_id)) => {
                commands::zcode::refresh_zcode_account(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Zcode, None) => {
                commands::zcode::refresh_all_zcode_accounts(app.clone()).await
            }
            (
                PlatformId::Trae
                | PlatformId::TraeSolo
                | PlatformId::TraeCn
                | PlatformId::TraeSoloCn,
                Some(account_id),
            ) => commands::trae::refresh_trae_token(app.clone(), account_id)
                .await
                .map(|_| 0),
            (
                PlatformId::Trae
                | PlatformId::TraeSolo
                | PlatformId::TraeCn
                | PlatformId::TraeSoloCn,
                None,
            ) => commands::trae::refresh_all_trae_tokens(app.clone()).await,
            (PlatformId::Workbuddy, Some(account_id)) => {
                commands::workbuddy::refresh_workbuddy_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Workbuddy, None) => {
                commands::workbuddy::refresh_all_workbuddy_tokens(app.clone()).await
            }
            (PlatformId::Zed, Some(account_id)) => {
                commands::zed::refresh_zed_token(app.clone(), account_id)
                    .await
                    .map(|_| 0)
            }
            (PlatformId::Zed, None) => commands::zed::refresh_all_zed_tokens(app.clone()).await,
        };
        let _ = refresh_result;
        refresh_native_menu_snapshot();
    });
}

fn refresh_native_menu_snapshot() {
    let Some(snapshot) = build_snapshot() else {
        return;
    };
    let Ok(snapshot_json) = serde_json::to_string(&snapshot) else {
        return;
    };
    let snapshot_json = to_cstring(&snapshot_json);
    unsafe {
        macos_native_menu_update_snapshot(snapshot_json.as_ptr());
    }
    if let Some(app) = crate::get_app_handle() {
        let _ = update_status_item(app);
    }
}

fn spawn_switch_account(platform: PlatformId, account_id: String) {
    let Some(app) = crate::get_app_handle().cloned() else {
        return;
    };

    tauri::async_runtime::spawn(async move {
        let status_app = app.clone();
        let _ = match platform {
            PlatformId::Antigravity => commands::account::switch_account(app, account_id, None)
                .await
                .map(|_| ()),
            PlatformId::Codex
                if modules::codex_instance::is_api_service_bind_account_id(&account_id) =>
            {
                commands::codex::codex_local_access_activate(app, None, None)
                    .await
                    .map(|_| ())
            }
            PlatformId::Codex => {
                { commands::codex::switch_codex_account(app, account_id, None, None, None) }
                    .await
                    .map(|_| ())
            }
            PlatformId::Claude => {
                commands::claude::switch_claude_account(app, account_id).map(|_| ())
            }
            PlatformId::GitHubCopilot => {
                commands::github_copilot::inject_github_copilot_to_vscode(app, account_id)
                    .await
                    .map(|_| ())
            }
            PlatformId::Windsurf => commands::windsurf::inject_windsurf_to_vscode(app, account_id)
                .await
                .map(|_| ()),
            PlatformId::Kiro => commands::kiro::inject_kiro_to_vscode(app, account_id)
                .await
                .map(|_| ()),
            PlatformId::Cursor => commands::cursor::inject_cursor_account(app, account_id)
                .await
                .map(|_| ()),
            PlatformId::Grok => commands::grok::switch_grok_account(app, account_id).map(|_| ()),
            PlatformId::Codebuddy => {
                commands::codebuddy::inject_codebuddy_to_vscode(app, account_id)
                    .await
                    .map(|_| ())
            }
            PlatformId::CodebuddyCn => {
                commands::codebuddy_cn::inject_codebuddy_cn_to_vscode(app, account_id)
                    .await
                    .map(|_| ())
            }
            PlatformId::Qoder => commands::qoder::inject_qoder_account(app, account_id)
                .await
                .map(|_| ()),
            PlatformId::Zcode => commands::zcode::inject_zcode_account(app, account_id).map(|_| ()),
            PlatformId::Trae
            | PlatformId::TraeSolo
            | PlatformId::TraeCn
            | PlatformId::TraeSoloCn => commands::trae::inject_trae_account(
                app,
                account_id,
                Some(platform.as_str().to_string()),
            )
            .await
            .map(|_| ()),
            PlatformId::Workbuddy => {
                commands::workbuddy::inject_workbuddy_to_vscode(app, account_id)
                    .await
                    .map(|_| ())
            }
            PlatformId::Zed => commands::zed::inject_zed_account(app, account_id)
                .await
                .map(|_| ()),
        };
        let _ = update_status_item(&status_app);
    });
}

fn open_main_window_page(page: &str) {
    if let Some(app) = crate::get_app_handle() {
        let _ = modules::floating_card_window::show_main_window_and_navigate(app, page);
    }
}

fn open_main_window() {
    if let Some(app) = crate::get_app_handle() {
        let _ = modules::floating_card_window::show_main_window(app);
    }
}
