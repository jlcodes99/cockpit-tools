fn injection_script(
    provider_name: &str,
    quota: &QuotaResponse,
    locale: &str,
    refresh_in_progress: bool,
    handled_refresh_token: Option<&str>,
    balance_lines: &[InjectionBalanceLine],
    grok_lines: &[InjectionGrokQuotaLine],
    badge_kind: InjectionBadgeKind,
    fallback_model: Option<&str>,
    usage_state: &Value,
    refresh_error: Option<&str>,
) -> String {
    let provider = serde_json::to_string(provider_name).unwrap_or_else(|_| "\"Codex\"".to_string());
    let weekly = quota.weekly_remaining_percent;
    let five_hour = quota.five_hour_remaining_percent;
    let account_count = quota.account_count;
    let available_account_count = quota.available_account_count.or(account_count);
    let abnormal_account_count = quota.abnormal_account_count.unwrap_or(0);
    let cooldown_account_count = quota.cooldown_account_count.unwrap_or(0);
    let plans = serde_json::to_string(&quota.plans).unwrap_or_else(|_| "[]".to_string());
    let grok_lines_value = serde_json::to_string(grok_lines).unwrap_or_else(|_| "[]".to_string());
    let grok_badge_line = grok_lines.iter().max_by_key(|line| line.remaining_percent);
    let grok_badge_percent =
        serde_json::to_string(&grok_badge_line.map(|line| line.remaining_percent))
            .unwrap_or_else(|_| "null".to_string());
    let grok_badge_product =
        serde_json::to_string(&grok_badge_line.map(|line| line.product.clone()))
            .unwrap_or_else(|_| "null".to_string());
    let badge_kind_value =
        serde_json::to_string(badge_kind.as_js_str()).unwrap_or_else(|_| "\"codex\"".to_string());
    let fallback_model_value =
        serde_json::to_string(&fallback_model).unwrap_or_else(|_| "null".to_string());
    let weekly = serde_json::to_string(&weekly).unwrap_or_else(|_| "null".to_string());
    let five_hour = serde_json::to_string(&five_hour).unwrap_or_else(|_| "null".to_string());
    let account_count_value =
        serde_json::to_string(&account_count).unwrap_or_else(|_| "null".to_string());
    let available_account_count_value =
        serde_json::to_string(&available_account_count).unwrap_or_else(|_| "null".to_string());
    let abnormal_account_count_value =
        serde_json::to_string(&abnormal_account_count).unwrap_or_else(|_| "0".to_string());
    let cooldown_account_count_value =
        serde_json::to_string(&cooldown_account_count).unwrap_or_else(|_| "0".to_string());
    let account_pool_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionPoolLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"Accounts\"".to_string());
    let plan_label = serde_json::to_string(&i18n::translate(locale, "floatingCard.plan", &[]))
        .unwrap_or_default();
    let weekly_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionWeeklyLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"Weekly\"".to_string());
    let five_hour_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionFiveHourLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"5h\"".to_string());
    let account_pool_title = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.accountPoolHealth.title",
        &[],
    ))
    .unwrap_or_else(|_| "\"Account Pool\"".to_string());
    let quota_empty_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.quotaPool.empty",
        &[],
    ))
    .unwrap_or_else(|_| "\"No quota stats yet\"".to_string());
    let available_text = {
        let available = available_account_count.unwrap_or(0).to_string();
        let total = account_count.unwrap_or(0).to_string();
        serde_json::to_string(&i18n::translate(
            locale,
            "codex.localAccess.accountPoolHealth.availableRatio",
            &[("available", available.as_str()), ("total", total.as_str())],
        ))
        .unwrap_or_else(|_| "\"Available\"".to_string())
    };
    let issue_text = {
        let abnormal = abnormal_account_count.to_string();
        let cooldown = cooldown_account_count.to_string();
        serde_json::to_string(&i18n::translate(
            locale,
            "codex.localAccess.accountPoolHealth.issueSummary",
            &[
                ("abnormal", abnormal.as_str()),
                ("cooldown", cooldown.as_str()),
            ],
        ))
        .unwrap_or_else(|_| "\"Issues\"".to_string())
    };
    let refresh_label =
        serde_json::to_string(&i18n::translate(locale, "common.shared.refreshQuota", &[]))
            .unwrap_or_else(|_| "\"Refresh quota\"".to_string());
    let close_label = serde_json::to_string(&i18n::translate(locale, "common.close", &[]))
        .unwrap_or_else(|_| "\"Close\"".to_string());
    let missing_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionMissingLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"\u{2014}\"".to_string());
    let refresh_in_progress = if refresh_in_progress { "true" } else { "false" };
    let handled_refresh_token =
        serde_json::to_string(&handled_refresh_token).unwrap_or_else(|_| "null".to_string());
    let balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.balance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Balance\"".to_string());
    let account_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.accountBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Account Balance\"".to_string());
    let total_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.totalBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Total balance\"".to_string());
    // Grok 弹框只讲 Grok 额度：标题复用额度概览，空态复用 Grok 平台的空数据文案。
    let quota_overview_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.title",
        &[],
    ))
    .unwrap_or_else(|_| "\"Quota Overview\"".to_string());
    let grok_empty_label =
        serde_json::to_string(&i18n::translate(locale, "grok.quotaQuery.empty", &[]))
            .unwrap_or_else(|_| "\"No quota\"".to_string());
    let balance_lines = serde_json::to_string(balance_lines).unwrap_or_else(|_| "[]".to_string());
    let official_label = serde_json::to_string(&i18n::translate(
        locale,
        "chatgptPlan.officialFlow.badge",
        &[],
    ))
    .unwrap_or_default();
    let badge_layout = include_str!("codex_app_injection_layout.js");
    let usage_metrics = include_str!("codex_app_injection_usage.js");
    let renderer = include_str!("codex_app_injection_statusline.js");
    let dialog_renderer = include_str!("codex_app_injection_dialog.js");
    let dialog_styles = serde_json::to_string(include_str!("codex_app_injection_dialog.css")).unwrap_or_default();
    let usage_state = usage_state.to_string();
    let refresh_error = serde_json::to_string(&refresh_error).unwrap_or_else(|_| "null".into());
    let usage_labels = serde_json::Value::Object(
        [
            "available",
            "issues",
            "cooling",
            "overview",
            "requestsTitle",
            "quotaTitle",
            "statistics",
            "verifiedStatistics",
            "excludedHistory",
            "model",
            "provider",
            "periodToday",
            "periodWeek",
            "scopeChat",
            "scopeService",
            "newestFirst",
            "shownRows",
            "more",
            "allLoaded",
            "loading",
            "dataFailed",
            "retry",
            "noRequests",
            "unlinkedSession",
            "statsUnavailable",
            "serviceUnavailable",
            "averageSpeed",
            "averageCache",
            "tierAdjusted",
            "requestTime",
            "account",
            "token",
            "performance",
            "result",
            "input",
            "output",
            "cached",
            "total",
            "firstResponse",
            "totalDuration",
            "success",
            "failed",
            "cancelled",
            "incomplete",
            "recovered",
            "official",
            "local",
            "serviceUnknown",
            "refreshData",
            "requestsCount",
            "totalTokens",
            "estimatedValue",
            "avgLatency",
            "successRate",
            "title",
            "session",
            "speed",
            "cache",
            "cacheShort",
            "speedHint",
            "cacheHint",
            "aggregateHint",
            "waiting",
            "refreshFailed",
            "refreshPartial",
            "hostUnavailable",
            "refreshing",
            "recent",
            "recentAccount",
            "noRecentAccount",
            "recentAccountHint",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_string(),
                json!(i18n::translate(
                    locale,
                    &format!("common.codexUsage.{}", name),
                    &[]
                )),
            )
        })
        .chain([
            ("requestTier", "codex.apiService.logs.requestedTier"),
            ("responseTier", "codex.apiService.logs.responseTier"),
            ("legacyTier", "codex.apiService.logs.legacyTier"),
            ("tierHint", "codex.apiService.logs.serviceTierHint"),
            ("tierFast", "codex.speed.fast"),
            ("tierUltrafast", "codex.speed.ultrafast"),
            ("tierStandard", "codex.speed.standard"),
            ("tierAuto", "codex.contextOverride.automatic"),
            ("tierUnknown", "common.requestProxy.unknown"),
        ].into_iter().map(|(name, key)| (name.to_string(), json!(i18n::translate(locale, key, &[])))))
        .collect(),
    )
    .to_string();
    format!(
        r#"(() => {{
      const providerName = {provider};
      const weeklyPercent = {weekly};
      const fiveHourPercent = {five_hour};
      const accountCount = {account_count_value};
      const availableAccountCount = {available_account_count_value};
      const abnormalAccountCount = {abnormal_account_count_value};
      const cooldownAccountCount = {cooldown_account_count_value};
      const plans = {plans};
      const officialLabel = {official_label};
      const grokLines = {grok_lines_value};
      const grokBadgePercent = {grok_badge_percent};
      const grokBadgeProduct = {grok_badge_product};
      const hostBadgeKind = {badge_kind_value};
      const fallbackModel = {fallback_model_value};
      const accountPoolLabel = {account_pool_label};
      const planLabel = {plan_label};
      const weeklyLabel = {weekly_label};
      const fiveHourLabel = {five_hour_label};
      const accountPoolTitle = {account_pool_title};
      const quotaEmptyLabel = {quota_empty_label};
      const availableText = {available_text};
      const issueText = {issue_text};
      const refreshLabel = {refresh_label};
      const closeLabel = {close_label};
      const missingLabel = {missing_label};
      const balanceLabel = {balance_label};
      const accountBalanceLabel = {account_balance_label};
      const totalBalanceLabel = {total_balance_label};
      const quotaOverviewLabel = {quota_overview_label};
      const grokEmptyLabel = {grok_empty_label};
      const balanceLines = {balance_lines};
      const refreshInProgress = {refresh_in_progress};
      const handledRefreshToken = {handled_refresh_token};
      const hostHeartbeatTimeoutMs = 8000;
      const badgeLayout = {badge_layout};
      const usageMetrics = {usage_metrics};
      const usageState = {usage_state};
      const usageLabels = {usage_labels};
      const refreshError = {refresh_error};
      const usageDialog = {dialog_renderer};
      const dialogStyles = {dialog_styles};
      return (
{renderer}
      );
    }})()"#
    )
}
