    use super::{
        app_server_auth_file_snapshot, auth_diagnostic_error_signal, auth_diagnostic_observation,
        build_launch_args, cdp_auth_signal, cdp_body_preview, cdp_console_auth_signal,
        deepseek_balance_endpoint, deepseek_balance_injection_script,
        deepseek_model_injection_script, has_login_route_markers, injection_badge_kind_for_model,
        injection_script, normalize_injection_model_slug, InjectionBadgeKind, InjectionGrokQuotaLine,
        is_auth_diagnostic_url, is_codex_app_target, is_official_login_route,
        is_safe_cdp_websocket_url, refresh_request_token_from_cdp_response,
        remote_debugging_port_from_command_line, sanitize_cdp_headers,
        selected_model_from_cdp_response, should_capture_cdp_response_body, supports_bind_account,
        AuthPageSnapshot, CdpTarget, DeepSeekBalanceSnapshot, InjectionBalanceLine,
        QuotaPlanSummary, QuotaResponse, AUTH_DIAGNOSTIC_SCRIPT,
    };
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexTokens};
    use crate::modules::codex_account::{
        compare_official_oauth_identity, CodexOfficialOAuthIdentity,
        CodexOfficialOAuthIdentityMatch,
    };
    use serde_json::json;

    fn api_service_script(
        quota: &QuotaResponse,
        balance: &[InjectionBalanceLine],
        grok: &[InjectionGrokQuotaLine],
        kind: InjectionBadgeKind,
        fallback: Option<&str>,
    ) -> String {
        injection_script(
            "Provider",
            quota,
            "zh-cn",
            false,
            None,
            balance,
            grok,
            kind,
            fallback,
        )
    }

    #[test]
    fn disabled_keeps_launch_args() {
        let args = vec!["--foo".to_string(), "bar".to_string()];
        let plan = build_launch_args(&args, false).expect("plan");
        assert_eq!(plan.args, args);
        assert_eq!(plan.port, None);
    }

    #[test]
    fn enabled_replaces_debug_flags_with_loopback_port() {
        let args = vec![
            "--remote-debugging-port=9333".to_string(),
            "--remote-debugging-address".to_string(),
            "0.0.0.0".to_string(),
            "--foo".to_string(),
        ];
        let plan = build_launch_args(&args, true).expect("plan");
        assert!(plan.port.is_some());
        assert!(!plan.args.iter().any(|value| value.contains("9333")));
        assert!(plan
            .args
            .iter()
            .any(|value| value == "--remote-debugging-address=127.0.0.1"));
    }

    #[test]
    fn only_api_service_binding_supports_quota_injection() {
        assert!(supports_bind_account(Some("__api_service__")));
        assert!(!supports_bind_account(Some("api-key-account")));
        assert!(!supports_bind_account(Some(
            "__provider_gateway__:custom-provider"
        )));
        assert!(!supports_bind_account(None));
    }

    #[test]
    fn deepseek_script_controls_official_picker_and_returns_pending_model() {
        let payload = json!({
            "selectedModel": "deepseek-v4-flash",
            "models": [
                { "id": "deepseek-v4-flash", "name": "DeepSeek-V4-Flash", "vision": true },
                { "id": "deepseek-v4-pro", "name": "DeepSeek-V4-Pro", "vision": false }
            ],
            "shells": { "gpt-5.5": "deepseek-v4-flash", "gpt-5.4": "deepseek-v4-pro" }
        });
        let script = deepseek_model_injection_script("zh-cn", &payload, None);
        assert!(script.contains("deepseek-v4-flash"));
        assert!(script.contains("deepseek-v4-pro"));
        assert!(script.contains("gpt-5.5"));
        assert!(script.contains("gpt-5.4"));
        assert!(script.contains("item.model = official"));
        assert!(script.contains("list-models-for-host"));
        assert!(script.contains("model/list"));
        assert!(script.contains("deepseek-official-picker"));
        assert!(script.contains("const reasoningLevels = [\"low\", \"high\", \"max\"]"));
        assert!(script.contains("supported_reasoning_levels = levels"));
        assert!(script.contains("supportedReasoningEfforts = levels"));
        assert!(script.contains("applyVisionMetadata"));
        assert!(script.contains("item.input_modalities = modalities"));
        assert!(script.contains("item.supports_image_detail_original = supportsImage"));
        assert!(script.contains("\"vision\":true"));
        // 模型列表判定必须严格，否则会把队列/线程等 data 数组当成模型列表改写。
        assert!(script.contains("typeof item.slug !== \"string\""));
        assert!(script.contains("typeof item.display_name === \"string\""));
        // 只有显式切模型才回写默认模型，发消息时的 model 字段不算切换。
        assert!(script.contains("const switchMethods"));
        assert!(script.contains("if (reportable) reportSelected(params.model)"));
        // 不得再引用已删除的 flashId/proId（会导致脚本每次执行抛异常）。
        assert!(!script.contains("flashId"));
        assert!(!script.contains("proId"));
        assert!(!script.contains("[\"low\", \"medium\", \"high\", \"xhigh\"]"));
        assert!(script.contains("staleBar"));
        assert!(!script.contains("data-cockpit-deepseek-model"));
        assert!(script.contains("pendingSelectedModel"));
        let parsed = selected_model_from_cdp_response(&json!({
            "result": { "result": { "value": { "selectedModel": "deepseek-v4-pro" } } }
        }));
        assert_eq!(parsed.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn deepseek_balance_endpoint_requires_official_host() {
        let mut account = CodexAccount::new_api_key(
            "codex_apikey_deepseek".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-flash".to_string()],
        );
        assert_eq!(
            deepseek_balance_endpoint(&account).as_deref(),
            Some("https://api.deepseek.com/user/balance")
        );

        // 第三方中转没有官方余额接口，不能凭 provider id 猜测余额。
        account.api_base_url = Some("https://relay.example.com/v1".to_string());
        assert!(deepseek_balance_endpoint(&account).is_none());

        account.api_base_url = None;
        assert!(deepseek_balance_endpoint(&account).is_none());
    }

    #[test]
    fn deepseek_balance_script_renders_balance_badge_and_details() {
        let snapshot = DeepSeekBalanceSnapshot {
            is_available: true,
            currency: Some("CNY".to_string()),
            total_balance: Some(110.0),
            granted_balance: Some(10.0),
            topped_up_balance: Some(100.0),
        };
        let script = deepseek_balance_injection_script("zh-cn", Some(&snapshot), false, None);

        assert!(script.contains("window.__cockpitDeepSeekBalance"));
        assert!(script.contains("data-cockpit-deepseek-balance"));
        assert!(script.contains("data-cockpit-deepseek-balance-details"));
        assert!(script.contains("data-cockpit-deepseek-balance-refresh"));
        assert!(script.contains("const balanceLabel = \"余额\""));
        assert!(script.contains("const totalBalanceLabel = \"总余额\""));
        assert!(script.contains("const grantedBalanceLabel = \"赠金余额\""));
        assert!(script.contains("const toppedUpBalanceLabel = \"充值余额\""));
        assert!(script.contains("\"totalBalance\":110.0"));
        // 与 API 服务额度注入各用各的宿主节点与命名空间；余额脚本只清理额度残留，不渲染额度徽章。
        assert!(script.contains("const staleQuotaRoot = window.__cockpitCodexInjection"));
        assert!(script.contains("document.querySelectorAll('[data-cockpit-quota-footer]"));
        assert!(!script.contains("host.setAttribute('data-cockpit-quota-footer'"));
        assert!(script.contains("root.refreshRequestToken"));
        assert!(script.contains("hostHeartbeatTimeoutMs = 8000"));
        // 余额徽章与额度徽章共用同一套锚点（访问权限与背景信息/模型之间的中点）。
        assert!(script.contains("(permissionsRight + rightAnchorRect.left) / 2"));
        assert!(script.contains("host.style.left = badgeAnchorLeft + 'px'"));
        assert!(script.contains("details.style.left = badgeAnchorLeft + 'px'"));
    }

    #[test]
    fn deepseek_balance_script_hides_badge_without_data() {
        let script = deepseek_balance_injection_script("en", None, false, None);

        assert!(script.contains("const balance = null"));
        assert!(script.contains("root.hasBalance = Boolean(balance)"));
        assert!(script.contains("if (!permissions || !footer || !balance)"));
    }

    #[test]
    fn parses_remote_debugging_port_from_running_process_command_line() {
        assert_eq!(
            remote_debugging_port_from_command_line(
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT --remote-debugging-address=127.0.0.1 --remote-debugging-port=64404"
            ),
            Some(64404)
        );
        assert_eq!(
            remote_debugging_port_from_command_line(
                r#"C:\Program Files\ChatGPT\ChatGPT.exe --remote-debugging-port "9333""#
            ),
            Some(9333)
        );
    }

    #[test]
    fn rejects_missing_or_invalid_remote_debugging_port() {
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-address=127.0.0.1"),
            None
        );
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-port=0"),
            None
        );
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-port=70000"),
            None
        );
    }

    #[test]
    fn custom_script_renders_weekly_and_optional_five_hour_fields() {
        let script = api_service_script(
            &QuotaResponse {
                weekly_remaining_percent: Some(1387),
                five_hour_remaining_percent: Some(100),
                account_count: Some(14),
                available_account_count: Some(12),
                abnormal_account_count: Some(2),
                cooldown_account_count: Some(0),
                plans: vec![QuotaPlanSummary {
                    plan: "PLUS".to_string(),
                    count: 14,
                    weekly_remaining_percent: Some(1387),
                    five_hour_remaining_percent: Some(100),
                }],
            },
            &[],
            &[],
            InjectionBadgeKind::CodexQuota,
            None,
        );
        assert!(script.contains("const accountPoolLabel = \"账号\""));
        assert!(script.contains("const accountCount = 14"));
        assert!(script.contains("const weeklyLabel = \"周\""));
        assert!(script.contains("const fiveHourLabel = \"5h\""));
        assert!(script.contains("const balanceLines = []"));
        assert!(script.contains("data-cockpit-quota-footer"));
        assert!(script.contains("document.body.appendChild(host)"));
        assert!(script.contains("position:fixed"));
        // 底部徽章与弹框水平居中在「访问权限」与右侧「背景信息 / 模型」之间的空档中点。
        assert!(script.contains("(permissionsRight + rightAnchorRect.left) / 2"));
        assert!(script.contains("host.style.left = badgeAnchorLeft + 'px'"));
        assert!(script.contains("details.style.left = badgeAnchorLeft + 'px'"));
        assert!(script.contains("[aria-label*=\"上下文用量\"]"));
        assert!(script.contains("permissionsRect.top + permissionsRect.height / 2"));
        assert!(script.contains("justify-content:center"));
        assert!(script.contains("host.innerHTML !== nextHtml"));
        assert!(script.contains("mutations.every"));
        assert!(script.contains("new ResizeObserver"));
        assert!(script.contains("window.addEventListener('resize'"));
        assert!(script.contains("requestAnimationFrame"));
        assert!(script.contains("root.render()"));
        assert!(script.contains("data-cockpit-quota-refresh"));
        assert!(script.contains("data-cockpit-quota-open"));
        assert!(script.contains("data-cockpit-quota-details"));
        assert!(script.contains("data-cockpit-quota-close"));
        assert!(script.contains("const plans = [{\"plan\":\"PLUS\",\"count\":14"));
        assert!(script.contains("const availableText = \"可用 12/14\""));
        assert!(script.contains("const issueText = \"异常 2 · 冷却 0\""));
        assert!(script.contains("var(--color-token-main-surface-primary"));
        assert!(script.contains("var(--color-token-text-secondary"));
        assert!(script.contains("const planColor"));
        assert!(script.contains("background:#10b981"));
        assert!(script.contains("background:#3b82f6"));
        assert!(script.contains("root.quotaDetailsOpen"));
        assert!(script.contains("details.innerHTML !== detailsHtml"));
        assert!(script.contains("details.contains(mutation.target)"));
        assert!(!script.contains("modal-overlay"));
        assert!(!script.contains("common.shared.refreshQuota"));
        assert!(script.contains("root.refreshRequestToken"));
        assert!(script.contains("cockpit-quota-spin"));
        assert!(script.contains("pointer-events:auto"));
        assert!(script.contains("root.hostHeartbeatAt = Date.now()"));
        assert!(script.contains("root.watchdogTimer"));
        assert!(script.contains("hostHeartbeatTimeoutMs = 8000"));
        assert!(script.contains("root.refreshRequestToken = null"));
        assert!(!script.contains("footer.appendChild(host)"));
        assert!(!script.contains("justify-content:flex-end"));
        assert!(!script.contains("grid-row:3"));
        assert!(!script.contains("min-height:18px"));
        assert!(!script.contains("data-cockpit-fast-mode"));
        assert!(!script.contains("Debugger"));
    }

    #[test]
    fn api_service_injection_renders_grok_build_quota_rows() {
        let script = api_service_script(
            &QuotaResponse {
                account_count: Some(3),
                ..QuotaResponse::default()
            },
            &[],
            &[InjectionGrokQuotaLine {
                product: "GrokBuild".to_string(),
                count: 2,
                remaining_percent: 75,
            }],
            InjectionBadgeKind::CodexQuota,
            None,
        );

        assert!(script.contains(
            "const grokLines = [{\"product\":\"GrokBuild\",\"count\":2,\"remainingPercent\":75}]"
        ));
        assert!(script.contains("const grokRows = grokLines.map"));
        // Grok 行与 Codex 套餐行一起渲染在点击弹框里。
        assert!(script.contains("plans.map(renderPlan).join('') + grokRows"));
    }

    #[test]
    fn api_service_injection_shows_balance_line_in_details() {
        let script = api_service_script(
            &QuotaResponse {
                account_count: Some(2),
                ..QuotaResponse::default()
            },
            &[InjectionBalanceLine {
                currency: Some("CNY".to_string()),
                total_balance: 62.36,
            }],
            &[],
            InjectionBadgeKind::CodexQuota,
            None,
        );

        assert!(
            script.contains("const balanceLines = [{\"currency\":\"CNY\",\"totalBalance\":62.36}]")
        );
        assert!(script.contains("const balanceLabel = \"余额\""));
        assert!(script.contains("const balanceRows = balanceLines.map"));
        assert!(script.contains("escapeHtml(formatMoney(line.totalBalance, line.currency))"));
        // 余额只显示在点击弹框里，不再渲染独立的 DeepSeek 余额徽章宿主。
        assert!(script.contains("const staleBalanceRoot = window.__cockpitDeepSeekBalance"));
        assert!(script.contains("document.querySelectorAll('[data-cockpit-deepseek-balance]"));
        assert!(!script.contains("host.setAttribute('data-cockpit-deepseek-balance'"));
        assert!(!script.contains("data-cockpit-deepseek-balance-refresh"));
    }

    #[test]
    fn api_service_injection_badge_follows_selected_model() {
        let grok = [InjectionGrokQuotaLine {
            product: "GrokBuild".to_string(),
            count: 1,
            remaining_percent: 94,
        }];
        let balance = [InjectionBalanceLine {
            currency: Some("CNY".to_string()),
            total_balance: 13.47,
        }];
        let grok_script = api_service_script(
            &QuotaResponse::empty_pool(),
            &balance,
            &grok,
            InjectionBadgeKind::GrokQuota,
            Some("grok-4.6"),
        );
        assert!(grok_script.contains("const hostBadgeKind = \"grok\""));
        assert!(grok_script.contains("const fallbackModel = \"grok-4.6\""));
        assert!(grok_script.contains("const grokBadgePercent = 94"));
        assert!(grok_script.contains("if (badgeKind === 'deepseek')"));
        assert!(grok_script.contains("} else if (badgeKind === 'grok')"));
        assert!(grok_script.contains("readVisibleModel"));
        assert!(grok_script.contains("classifyModel"));
        // Grok 徽章只写产品短名（Build），不带 Grok 前缀。
        assert!(grok_script.contains("const shortProductLabel = (value) =>"));
        assert!(grok_script.contains("text.replace(/^grok[\\s_\\-]*/i, '')"));
        assert!(grok_script.contains("shortProductLabel(grokBadgeProduct) + ' ' + grokPercent"));

        let gpt_script = api_service_script(
            &QuotaResponse {
                weekly_remaining_percent: Some(61),
                five_hour_remaining_percent: Some(100),
                account_count: Some(2),
                ..QuotaResponse::default()
            },
            &balance,
            &grok,
            InjectionBadgeKind::CodexQuota,
            Some("gpt-5.4"),
        );
        assert!(gpt_script.contains("const hostBadgeKind = \"codex\""));
        assert!(gpt_script.contains("const fallbackModel = \"gpt-5.4\""));
        // GPT 原生模型恢复「账号 / 5h / 周」三个并排胶囊。
        assert!(gpt_script.contains("accountPoolLabel + ' ' + Math.round(accountCount)"));
        assert!(gpt_script.contains("fiveHourLabel + ' ' + Math.round(fiveHourPercent) + '%'"));
        assert!(gpt_script.contains("weeklyLabel + ' ' + Math.round(weeklyPercent) + '%'"));
        assert!(gpt_script.contains("detailsHtml = detailHeader('#8b5cf6'"));

        let deepseek_script = api_service_script(
            &QuotaResponse::default(),
            &balance,
            &grok,
            InjectionBadgeKind::DeepSeekBalance,
            Some("deepseek-v4-pro"),
        );
        assert!(deepseek_script.contains("const hostBadgeKind = \"deepseek\""));
        assert!(deepseek_script.contains("const fallbackModel = \"deepseek-v4-pro\""));
        // 余额徽章恢复「余额 ¥xx」样式，弹框只列余额。
        assert!(deepseek_script.contains("const accountBalanceLabel = \"账户余额\""));
        assert!(deepseek_script.contains("const totalBalanceLabel = \"总余额\""));
        assert!(deepseek_script.contains("formatMoney(line.totalBalance, line.currency)"));
        assert!(deepseek_script.contains("detailsHtml = detailHeader('#3b82f6'"));

        // Grok 弹框只讲 Grok 额度，不混账号池与余额。
        assert!(grok_script.contains("const quotaOverviewLabel = \"额度概览\""));
        assert!(grok_script.contains("const grokEmptyLabel = \"暂无可用配额数据\""));
        assert!(grok_script.contains("detailsHtml = detailHeader('#f59e0b'"));
    }

    #[test]
    fn api_service_injection_missing_label_is_translated_once() {
        let script = api_service_script(
            &QuotaResponse::default(),
            &[],
            &[],
            InjectionBadgeKind::CodexQuota,
            None,
        );
        // 翻译键缺失时 i18n 会回落成键名，底部徽章必须使用真实文案。
        assert!(script.contains("const missingLabel = \"—\""));
        assert!(!script.contains("settings.general.codexAppUiInjectionMissingLabel"));
        // 没有数据的窗口不渲染胶囊：池里没有官方 GPT 账号时不会再排「— - —」。
        assert!(!script.contains("quotaParts"));
        assert!(script.contains("if (Number.isFinite(fiveHourPercent)) {"));
        assert!(script.contains("if (Number.isFinite(weeklyPercent)) {"));
    }

    #[test]
    fn api_service_injection_ignores_picker_placeholder_text() {
        let script = api_service_script(
            &QuotaResponse::default(),
            &[],
            &[],
            InjectionBadgeKind::GrokQuota,
            Some("grok-4.6"),
        );
        // 读不到模型名（选择器占位文案、界面未渲染完）时保持宿主给的徽章类型，
        // 不再回落成「其它模型 → GPT 周/5h 额度」。
        assert!(script.contains("if (!slug) return null;"));
        assert!(script.contains("const badgeKind = classifyModel(selectedModel) || hostBadgeKind;"));
        assert!(!script.contains("if (!slug) return hostBadgeKind;"));
        // 模型名来源：选择器触发器 + 展开弹框里的当前模型行。
        assert!(script.contains("[data-model-picker-model-row]"));
        assert!(script.contains("ViewToggleModelLabel_"));
        assert!(script.contains("if (visibleModel) {"));
        assert!(script.contains("root.visibleModel = visibleModel;"));
        // 长时间读不到模型名时交回宿主判断，避免徽章停在旧模型上。
        assert!(script.contains("const modelStickyMs = 15000;"));
        assert!(script.contains("let observedModelThisRender = null;"));
        assert!(script.contains("selectedModel: observedModelThisRender"));
    }

    #[test]
    fn injection_badge_kind_maps_model_slug() {
        assert_eq!(
            injection_badge_kind_for_model(Some("deepseek-v4-pro")),
            InjectionBadgeKind::DeepSeekBalance
        );
        assert_eq!(
            injection_badge_kind_for_model(Some("grok-4.6")),
            InjectionBadgeKind::GrokQuota
        );
        assert_eq!(
            injection_badge_kind_for_model(Some("gpt-5.4")),
            InjectionBadgeKind::CodexQuota
        );
        assert_eq!(
            injection_badge_kind_for_model(None),
            InjectionBadgeKind::CodexQuota
        );
        assert_eq!(
            normalize_injection_model_slug("DeepSeek V4 Pro").as_deref(),
            Some("deepseek-v4-pro")
        );
        assert_eq!(
            selected_model_from_cdp_response(&json!({
                "result": { "result": { "value": { "selectedModel": "grok-4.6" } } }
            }))
            .as_deref(),
            Some("grok-4.6")
        );
    }

    #[test]
    fn five_hour_only_quota_is_not_relabelled_as_weekly() {
        let script = api_service_script(
            &QuotaResponse {
                weekly_remaining_percent: None,
                five_hour_remaining_percent: Some(63),
                account_count: Some(2),
                ..QuotaResponse::default()
            },
            &[],
            &[],
            InjectionBadgeKind::CodexQuota,
            None,
        );
        assert!(script.contains("const weeklyPercent = null"));
        assert!(script.contains("const fiveHourPercent = 63"));
    }

    #[test]
    fn empty_pool_quota_renders_zero_account_and_windows() {
        let quota = QuotaResponse::empty_pool();
        assert_eq!(quota.account_count, Some(0));
        assert_eq!(quota.five_hour_remaining_percent, Some(0));
        assert_eq!(quota.weekly_remaining_percent, Some(0));

        let script = api_service_script(
            &quota,
            &[],
            &[],
            InjectionBadgeKind::CodexQuota,
            None,
        );
        assert!(script.contains("const accountCount = 0"));
        assert!(script.contains("const fiveHourPercent = 0"));
        assert!(script.contains("const weeklyPercent = 0"));
        // GPT 原生模型恢复「账号 / 5h / 周」三个并排胶囊，账号数回到独立徽章。
        assert!(script.contains("const fields = [];"));
        assert!(script.contains("accountPoolLabel + ' ' + Math.round(accountCount)"));
    }

    #[test]
    fn empty_pool_response_normalizes_missing_window_values_to_zero() {
        let quota = QuotaResponse {
            weekly_remaining_percent: None,
            five_hour_remaining_percent: None,
            account_count: Some(0),
            ..QuotaResponse::default()
        }
        .normalize_empty_pool();

        assert_eq!(quota.account_count, Some(0));
        assert_eq!(quota.five_hour_remaining_percent, Some(0));
        assert_eq!(quota.weekly_remaining_percent, Some(0));
    }

    #[test]
    fn cdp_response_extracts_refresh_request_token() {
        let response = json!({
            "id": 1,
            "result": {
                "result": {
                    "type": "object",
                    "value": {"refreshRequestToken": "request-123"}
                }
            }
        });
        assert_eq!(
            refresh_request_token_from_cdp_response(&response).as_deref(),
            Some("request-123")
        );
        assert!(refresh_request_token_from_cdp_response(&json!({"id": 1})).is_none());
    }

    #[test]
    fn auth_diagnostic_cdp_websocket_must_be_loopback_and_same_port() {
        assert!(is_safe_cdp_websocket_url(
            "ws://127.0.0.1:9333/devtools/page/1",
            9333
        ));
        assert!(is_safe_cdp_websocket_url(
            "ws://[::1]:9333/devtools/page/1",
            9333
        ));
        assert!(!is_safe_cdp_websocket_url(
            "ws://192.168.1.2:9333/devtools/page/1",
            9333
        ));
        assert!(!is_safe_cdp_websocket_url(
            "ws://127.0.0.1:9444/devtools/page/1",
            9333
        ));
    }

    #[test]
    fn auth_diagnostic_observation_only_reports_safe_page_state() {
        let target = CdpTarget {
            target_id: "page-1".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: Some("ws://127.0.0.1:9333/devtools/page/1".to_string()),
        };
        let snapshot = AuthPageSnapshot {
            route: "/login".to_string(),
            title: "ChatGPT".to_string(),
            ready_state: "complete".to_string(),
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[target], Some(snapshot));

        assert!(observed.cdp_available);
        assert_eq!(observed.target_count, 1);
        assert_eq!(observed.route, "/login");
        assert!(observed.login_signal());
    }

    #[test]
    fn auth_diagnostic_requires_route_or_complete_login_ui_signal() {
        assert!(!AUTH_DIAGNOSTIC_SCRIPT.contains("innerText"));
        assert!(!AUTH_DIAGNOSTIC_SCRIPT.contains("loginText"));
        assert!(is_official_login_route("/login"));
        assert!(is_official_login_route("/login/"));
        assert!(!is_official_login_route("/auth/login"));
        assert!(!is_official_login_route("/index.html"));
        assert!(has_login_route_markers(&[
            "login_title".to_string(),
            "login_primary_action".to_string(),
        ]));
        assert!(!has_login_route_markers(&["login_title".to_string()]));
        let target = CdpTarget {
            target_id: "page-1".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: Some("ws://127.0.0.1:9333/devtools/page/1".to_string()),
        };
        let normal_page = AuthPageSnapshot {
            route: "/index.html".to_string(),
            title: "Sign in to ChatGPT".to_string(),
            ready_state: "complete".to_string(),
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[target], Some(normal_page));
        assert!(!observed.login_signal());

        let login_page = AuthPageSnapshot {
            route: "/index.html".to_string(),
            login_ui_signal: true,
            login_ui_markers: vec![
                "login_title".to_string(),
                "login_primary_action".to_string(),
            ],
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[], Some(login_page));
        assert!(observed.login_signal());
        assert!(observed.login_ui_signal);
    }

    #[test]
    fn auth_diagnostic_ignores_non_codex_targets() {
        let codex_target = CdpTarget {
            target_id: "codex".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: None,
        };
        let external_target = CdpTarget {
            target_id: "external".to_string(),
            target_type: "page".to_string(),
            url: "https://chatgpt.com/auth/login".to_string(),
            websocket_url: None,
        };
        assert!(is_codex_app_target(&codex_target));
        assert!(!is_codex_app_target(&external_target));
    }

    #[test]
    fn auth_diagnostic_profile_identity_uses_account_id_before_email() {
        let mut account = CodexAccount::new(
            "local-account".to_string(),
            "expected@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.account_id = Some("chatgpt-account".to_string());
        account.user_id = Some("chatgpt-user".to_string());

        let matching = CodexOfficialOAuthIdentity {
            email: "different@example.com".to_string(),
            user_id: Some("chatgpt-user".to_string()),
            account_id: Some("chatgpt-account".to_string()),
            organization_id: None,
        };
        assert_eq!(
            compare_official_oauth_identity(&matching, &account),
            CodexOfficialOAuthIdentityMatch::Matched
        );

        let mismatched = CodexOfficialOAuthIdentity {
            account_id: Some("other-account".to_string()),
            ..matching.clone()
        };
        assert_eq!(
            compare_official_oauth_identity(&mismatched, &account),
            CodexOfficialOAuthIdentityMatch::Mismatched
        );

        let incomplete = CodexOfficialOAuthIdentity {
            email: "expected@example.com".to_string(),
            user_id: None,
            account_id: None,
            organization_id: None,
        };
        assert_eq!(
            compare_official_oauth_identity(&incomplete, &account),
            CodexOfficialOAuthIdentityMatch::Unknown
        );
    }

    #[test]
    fn auth_network_diagnostics_redact_credentials_and_queries() {
        assert_eq!(
            super::sanitize_cdp_url(
                "https://auth.openai.com/oauth/token?client_secret=secret&code=oauth-code"
            ),
            "https://auth.openai.com/oauth/token?<redacted-query>"
        );
        let headers = sanitize_cdp_headers(Some(&json!({
            "authorization": "Bearer secret",
            "cookie": "session=secret",
            "x-api-key": "secret-api-key",
            "session_token": "secret-session-token",
            "x-request-id": "request-1"
        })));
        assert_eq!(headers["authorization"], "<redacted>");
        assert_eq!(headers["cookie"], "<redacted>");
        assert_eq!(headers["x-api-key"], "<redacted>");
        assert_eq!(headers["session_token"], "<redacted>");
        assert_eq!(headers["x-request-id"], "request-1");
    }

    #[test]
    fn auth_network_diagnostics_extract_redacted_json_error_body() {
        let body = json!({
            "error": "refresh_token_reused",
            "access_token": "eyJsecret",
            "message": "reauth required"
        });
        let response = json!({
            "result": {"body": serde_json::to_string(&body).expect("body")}
        });
        let preview = cdp_body_preview(&response).expect("preview");
        assert!(preview.contains("refresh_token_reused"));
        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains("eyJsecret"));
    }

    #[test]
    fn auth_network_diagnostics_matches_official_relogin_signals() {
        let invalid_refresh = json!({
            "result": {"body": r#"{"error":{"code":"invalid_refresh_token","message":"Invalid refresh token."}}"#}
        });
        assert_eq!(
            cdp_auth_signal(&invalid_refresh),
            Some("invalid_refresh_token")
        );

        let auth_error = json!({
            "result": {"body": r#"{"data":{"reason":"cloudRequirements","errorCode":"Auth"}}"#}
        });
        assert_eq!(cdp_auth_signal(&auth_error), Some("auth_error_code"));

        let relogin = json!({
            "result": {"body": r#"{"data":{"reason":"cloudConfigBundle","action":"relogin"}}"#}
        });
        assert_eq!(cdp_auth_signal(&relogin), Some("relogin_action"));

        let normal = json!({
            "result": {"body": r#"{"data":{"reason":"cloudRequirements"}}"#}
        });
        assert_eq!(cdp_auth_signal(&normal), None);
    }

    #[test]
    fn auth_network_diagnostics_marks_relevant_endpoints() {
        assert!(is_auth_diagnostic_url(
            "https://chatgpt.com/backend-api/cloudRequirements"
        ));
        assert!(is_auth_diagnostic_url(
            "https://auth.openai.com/oauth/token"
        ));
        assert!(!is_auth_diagnostic_url("https://example.com/assets/app.js"));
    }

    #[test]
    fn auth_network_diagnostics_captures_auth_body_even_for_success_status() {
        assert!(should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/cloudRequirements",
            200
        ));
        assert!(should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/conversations",
            401
        ));
        assert!(!should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/conversations",
            200
        ));
    }

    #[test]
    fn auth_diagnostic_extracts_only_known_official_error_signals() {
        assert_eq!(
            auth_diagnostic_error_signal("401 Unauthorized: invalid_refresh_token"),
            Some("invalid_refresh_token")
        );
        assert_eq!(
            auth_diagnostic_error_signal("Invalid refresh token."),
            Some("invalid_refresh_token")
        );
        assert_eq!(
            auth_diagnostic_error_signal("auth_status_result nullReason=auth_token_missing"),
            Some("auth_token_missing")
        );
        assert_eq!(
            auth_diagnostic_error_signal("no_token_attached"),
            Some("no_token_attached")
        );
        assert_eq!(
            auth_diagnostic_error_signal("requiresOpenaiAuth=true"),
            Some("requiresOpenaiAuth")
        );
        assert_eq!(auth_diagnostic_error_signal("ordinary 401 response"), None);
        assert_eq!(auth_diagnostic_error_signal("ERR_SSL_PROTOCOL_ERROR"), None);
    }

    #[test]
    fn auth_diagnostic_reads_console_argument_values_without_logging_them() {
        let params = json!({
            "args": [
                {"type": "string", "value": "app_server_connection.auth_status_result"},
                {"type": "string", "value": "code=invalid_refresh_token"}
            ]
        });
        assert_eq!(
            cdp_console_auth_signal(&params),
            Some("invalid_refresh_token")
        );

        let nested = json!({
            "args": [{"type": "object", "description": "{\"requiresOpenaiAuth\":true}"}]
        });
        assert_eq!(cdp_console_auth_signal(&nested), Some("requiresOpenaiAuth"));
        assert_eq!(
            cdp_console_auth_signal(&json!({"args": [{"value": "Bearer eyJsecret"}]})),
            None
        );
    }

    #[test]
    fn app_server_auth_snapshot_never_contains_auth_contents() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-codex-app-server-diagnostic-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create diagnostic directory");
        std::fs::write(
            directory.join("auth.json"),
            r#"{"access_token":"secret-access","refresh_token":"secret-refresh"}"#,
        )
        .expect("write diagnostic auth");

        let snapshot = app_server_auth_file_snapshot(&directory);
        assert!(snapshot.starts_with("exists=true,size="));
        assert!(snapshot.contains("sha256="));
        assert!(!snapshot.contains("secret-access"));
        assert!(!snapshot.contains("secret-refresh"));
        let _ = std::fs::remove_dir_all(directory);
    }
