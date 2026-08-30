// cockpit-core Codex 账号：File import, account metadata mutations and quota auto-switch。
// 通过 include! 保持原模块作用域和凭据调用路径。
/// 从本地文件导入 Codex 账号（支持多种 JSON 格式）
pub async fn import_from_files(file_paths: Vec<String>) -> Result<CodexFileImportResult, String> {
    use std::path::Path;

    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;

    logger::log_info(&format!(
        "Codex: 开始从 {} 个文件导入账号...",
        file_paths.len()
    ));

    // 原有文件导入候选: (CodexTokens, account_id_hint, label)
    let mut candidates: Vec<(CodexTokens, Option<String>, String)> = Vec::new();
    // 旧规则未识别到账号时，才用 Token/JSON 粘贴框的解析逻辑处理整个文件内容。
    let mut fallback_files: Vec<(String, String)> = Vec::new();

    for file_path in &file_paths {
        let path = Path::new(file_path);
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                logger::log_error(&format!("读取文件失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        // 从文件名推断 email 作为 label
        let filename_label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let parsed: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                logger::log_warn(&format!(
                    "Codex 文件旧规则 JSON 解析失败，将尝试 Token/JSON 导入逻辑 {:?}: {}",
                    file_path, e
                ));
                fallback_files.push((content, filename_label));
                continue;
            }
        };

        let before_count = candidates.len();
        match &parsed {
            serde_json::Value::Object(_) => {
                if let Some((tokens, hint)) = extract_codex_tokens_from_value(&parsed) {
                    candidates.push((tokens, hint, filename_label.clone()));
                } else {
                    logger::log_error(&format!("未找到有效 Token {:?}", file_path));
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let Some((tokens, hint)) = extract_codex_tokens_from_value(item) {
                        let label = item
                            .get("email")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&filename_label)
                            .to_string();
                        candidates.push((tokens, hint, label));
                    }
                }
            }
            _ => {
                logger::log_error(&format!("不支持的 JSON 格式 {:?}", file_path));
            }
        }

        if candidates.len() == before_count {
            logger::log_info(&format!(
                "Codex 文件旧规则未找到账号，将尝试 Token/JSON 导入逻辑 {:?}",
                file_path
            ));
            fallback_files.push((content, filename_label));
        }
    }

    if candidates.is_empty() && fallback_files.is_empty() {
        return Err(
            "未找到有效的 Codex Token（需要 accessToken/access_token、id_token + access_token，或 refresh_token）"
                .to_string(),
        );
    }

    logger::log_info(&format!(
        "Codex: 发现 {} 个旧格式候选账号，{} 个文件待尝试 Token/JSON 导入逻辑...",
        candidates.len(),
        fallback_files.len()
    ));

    let mut imported = Vec::new();
    let mut failed: Vec<CodexFileImportFailure> = Vec::new();
    let total = candidates.len() + fallback_files.len();
    let mut progress_index = 0usize;

    for (tokens, account_id_hint, label) in candidates {
        // 发送进度事件
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match upsert_account_with_hints(tokens, account_id_hint, None) {
            Ok(account) => {
                logger::log_info(&format!("Codex 导入成功: {}", account.email));
                imported.push(account);
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    for (content, label) in fallback_files {
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match import_from_json(&content).await {
            Ok(accounts) => {
                for account in accounts {
                    logger::log_info(&format!("Codex 导入成功: {}", account.email));
                    imported.push(account);
                }
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    logger::log_info(&format!(
        "Codex 文件导入完成，成功 {} 个，失败 {} 个",
        imported.len(),
        failed.len()
    ));

    Ok(CodexFileImportResult { imported, failed })
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.tags = Some(tags);
    save_account(&account)?;

    Ok(account)
}

pub fn update_account_note(account_id: &str, note: String) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.account_note = normalize_optional_value(Some(note));
    save_account(&account)?;

    Ok(account)
}

pub fn update_api_key_credentials(
    account_id: &str,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持编辑凭据".to_string());
    }

    let (normalized_key, normalized_base_url) =
        validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        normalized_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let old_id = account.id.clone();
    let new_id = build_api_key_account_id(&normalized_key);
    let mut index = load_account_index();
    let was_current = get_current_account()
        .map(|current| current.id == old_id)
        .unwrap_or(false);

    if new_id != old_id && index.accounts.iter().any(|item| item.id == new_id) {
        return Err("该 API Key 已存在，请直接使用已有账号".to_string());
    }

    if new_id != old_id {
        account.id = new_id.clone();
    }

    apply_api_key_fields(&mut account, &normalized_key, provider_config);
    account.update_last_used();
    save_account(&account)?;

    if old_id != account.id {
        delete_account_file(&old_id)?;
    }

    let mut summary_found = false;
    for summary in &mut index.accounts {
        if summary.id == old_id {
            summary.id = account.id.clone();
            summary.email = account.email.clone();
            summary.plan_type = account.plan_type.clone();
            summary.last_used = account.last_used;
            summary_found = true;
            break;
        }
    }

    if !summary_found {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    if index.current_account_id.as_deref() == Some(old_id.as_str()) {
        index.current_account_id = Some(account.id.clone());
    }
    save_account_index(&index)?;

    if old_id != account.id {
        if let Err(err) =
            crate::modules::codex_instance::replace_bind_account_references(&old_id, &account.id)
        {
            logger::log_warn(&format!(
                "Codex API Key 账号编辑后同步实例绑定失败: old_id={}, new_id={}, error={}",
                old_id, account.id, err
            ));
        }
    }

    if was_current {
        let codex_home = get_codex_home();
        write_account_bundle_to_dir(&codex_home, &account)?;
    }

    logger::log_info(&format!(
        "Codex API Key 账号凭据已更新: old_id={}, new_id={}, has_base_url={}",
        old_id,
        account.id,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));

    Ok(account)
}

pub fn update_account_name(account_id: &str, name: String) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持重命名".to_string());
    }

    account.account_name = normalize_optional_value(Some(name));
    save_account(&account)?;

    Ok(account)
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_account_scope_mode(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase();
    if normalized == CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED.to_string()
    } else {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL.to_string()
    }
}

fn normalize_auto_switch_selected_account_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn resolve_monitored_auto_switch_account_ids(
    scope_mode: &str,
    selected_account_ids: &[String],
    accounts: &[CodexAccount],
) -> HashSet<String> {
    if scope_mode != CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        return accounts.iter().map(|account| account.id.clone()).collect();
    }

    let selected = normalize_auto_switch_selected_account_ids(selected_account_ids);
    if selected.is_empty() {
        return HashSet::new();
    }

    let existing: HashSet<&str> = accounts.iter().map(|account| account.id.as_str()).collect();
    selected
        .into_iter()
        .filter(|account_id| existing.contains(account_id.as_str()))
        .collect()
}

fn format_codex_quota_metric_label(window_minutes: Option<i64>, fallback: &str) -> String {
    const HOUR_MINUTES: i64 = 60;
    const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
    const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

    let Some(minutes) = window_minutes.filter(|value| *value > 0) else {
        return fallback.to_string();
    };

    if minutes >= WEEK_MINUTES - 1 {
        let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
        return if weeks <= 1 {
            "Weekly".to_string()
        } else {
            format!("{} Week", weeks)
        };
    }

    if minutes >= DAY_MINUTES - 1 {
        let days = (minutes + DAY_MINUTES - 1) / DAY_MINUTES;
        return format!("{}d", days);
    }

    if minutes >= HOUR_MINUTES {
        let hours = (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES;
        return format!("{}h", hours);
    }

    format!("{}m", minutes)
}

#[derive(Debug, Clone)]
struct CodexQuotaMetric {
    key: &'static str,
    label: String,
    percentage: i32,
}

fn extract_quota_metrics(account: &CodexAccount) -> Vec<CodexQuotaMetric> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };

    let has_presence =
        quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
    let mut metrics = Vec::new();

    if !has_presence || quota.hourly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    if !has_presence || quota.weekly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "secondary_window",
            label: format_codex_quota_metric_label(quota.weekly_window_minutes, "Weekly"),
            percentage: quota.weekly_percentage.clamp(0, 100),
        });
    }

    if metrics.is_empty() {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    metrics
}

fn average_quota_percentage(metrics: &[CodexQuotaMetric]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let sum: i32 = metrics.iter().map(|metric| metric.percentage).sum();
    sum as f64 / metrics.len() as f64
}

fn metric_crossed_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage <= primary_threshold,
        "secondary_window" => metric.percentage <= secondary_threshold,
        _ => false,
    }
}

fn metric_above_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage > primary_threshold,
        "secondary_window" => metric.percentage > secondary_threshold,
        _ => true,
    }
}

fn metric_margin_over_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<i32> {
    match metric.key {
        "primary_window" => Some(metric.percentage - primary_threshold),
        "secondary_window" => Some(metric.percentage - secondary_threshold),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CodexSwitchCandidate {
    account: CodexAccount,
    min_margin: i32,
    min_percentage: i32,
    average_percentage: f64,
}

fn build_switch_candidate(
    account: &CodexAccount,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexSwitchCandidate> {
    let metrics = extract_quota_metrics(account);
    if metrics.is_empty() {
        return None;
    }
    if !metrics
        .iter()
        .all(|metric| metric_above_threshold(metric, primary_threshold, secondary_threshold))
    {
        return None;
    }

    let min_margin = metrics
        .iter()
        .filter_map(|metric| {
            metric_margin_over_threshold(metric, primary_threshold, secondary_threshold)
        })
        .min()?;
    let min_percentage = metrics.iter().map(|metric| metric.percentage).min()?;
    let average_percentage = average_quota_percentage(&metrics);

    Some(CodexSwitchCandidate {
        account: account.clone(),
        min_margin,
        min_percentage,
        average_percentage,
    })
}

fn pick_best_candidate(mut candidates: Vec<CodexSwitchCandidate>) -> Option<CodexAccount> {
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        b.min_margin
            .cmp(&a.min_margin)
            .then_with(|| b.min_percentage.cmp(&a.min_percentage))
            .then_with(|| {
                b.average_percentage
                    .partial_cmp(&a.average_percentage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.account.last_used.cmp(&b.account.last_used))
    });

    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.account)
}

fn build_quota_alert_cooldown_key(
    account_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> String {
    format!(
        "codex:{}:{}:{}",
        account_id, primary_threshold, secondary_threshold
    )
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < CODEX_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, primary_threshold: i32, secondary_threshold: i32) {
    if let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(
            account_id,
            primary_threshold,
            secondary_threshold,
        ));
    }
}

pub(crate) fn resolve_current_account_id(accounts: &[CodexAccount]) -> Option<String> {
    if let Some(account) = get_current_account() {
        return Some(account.id);
    }

    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let trimmed = bind_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .map(|account| account.id.clone())
}

fn pick_quota_alert_recommendation(
    accounts: &[CodexAccount],
    current_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexAccount> {
    let candidates: Vec<CodexSwitchCandidate> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter_map(|account| {
            build_switch_candidate(account, primary_threshold, secondary_threshold)
        })
        .collect();

    pick_best_candidate(candidates)
}

pub fn pick_auto_switch_target_if_needed() -> Result<Option<CodexAccount>, String> {
    if CODEX_AUTO_SWITCH_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        logger::log_info("[AutoSwitch][Codex] 自动切号进行中，跳过本次检查");
        return Ok(None);
    }

    let result = (|| {
        let cfg = crate::modules::config::get_user_config();
        if !cfg.codex_auto_switch_enabled {
            return Ok(None);
        }

        let primary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_primary_threshold);
        let secondary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_secondary_threshold);
        let account_scope_mode =
            normalize_auto_switch_account_scope_mode(&cfg.codex_auto_switch_account_scope_mode);

        let accounts = list_accounts();
        let monitored_account_ids = resolve_monitored_auto_switch_account_ids(
            &account_scope_mode,
            &cfg.codex_auto_switch_selected_account_ids,
            &accounts,
        );
        if monitored_account_ids.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 可监控账号范围为空(scope={})，跳过自动切号",
                account_scope_mode
            ));
            return Ok(None);
        }
        let current_id = match resolve_current_account_id(&accounts) {
            Some(id) => id,
            None => return Ok(None),
        };
        if !monitored_account_ids.contains(&current_id) {
            logger::log_info(&format!(
                "[AutoSwitch][Codex] 当前账号不在监控范围内(current_id={}, scope={})，跳过自动切号",
                current_id, account_scope_mode
            ));
            return Ok(None);
        }

        let current = match accounts.iter().find(|account| account.id == current_id) {
            Some(account) => account,
            None => return Ok(None),
        };

        let current_metrics = extract_quota_metrics(current);
        if current_metrics.is_empty() {
            return Ok(None);
        }

        let should_switch = current_metrics
            .iter()
            .any(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold));
        if !should_switch {
            return Ok(None);
        }

        let candidates: Vec<CodexSwitchCandidate> = accounts
            .iter()
            .filter(|account| monitored_account_ids.contains(&account.id))
            .filter(|account| account.id != current_id)
            .filter_map(|account| {
                build_switch_candidate(account, primary_threshold, secondary_threshold)
            })
            .collect();

        if candidates.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 当前账号命中阈值 (primary<={}%, secondary<={}%)，但没有可切换候选账号",
                primary_threshold, secondary_threshold
            ));
            return Ok(None);
        }

        Ok(pick_best_candidate(candidates))
    })();

    CODEX_AUTO_SWITCH_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.codex_quota_alert_enabled {
        return Ok(None);
    }

    let primary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_primary_threshold);
    let secondary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_secondary_threshold);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id(&accounts) {
        Some(id) => id,
        None => return Ok(None),
    };

    let current = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };

    let metrics = extract_quota_metrics(current);
    let low_models: Vec<(String, i32)> = metrics
        .into_iter()
        .filter(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold))
        .map(|metric| (metric.label, metric.percentage))
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, primary_threshold, secondary_threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key =
        build_quota_alert_cooldown_key(&current_id, primary_threshold, secondary_threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(
        &accounts,
        &current_id,
        primary_threshold,
        secondary_threshold,
    );
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "codex".to_string(),
        current_account_id: current_id,
        current_email: current.email.clone(),
        threshold: primary_threshold,
        threshold_display: Some(format!(
            "primary_window<={}%, secondary_window<={}%",
            primary_threshold, secondary_threshold
        )),
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(|account| account.email.clone()),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
