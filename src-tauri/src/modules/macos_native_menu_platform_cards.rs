// macOS Native Menu：Platform account cards, resource quotas and localized rendering。
// 通过 include! 保持原 imp 模块作用域和 Objective-C FFI 调用路径。
    fn resource_account_roots<'a>(
        quota_raw: Option<&'a Value>,
        usage_raw: Option<&'a Value>,
    ) -> Vec<&'a Value> {
        let quota_root = json_path(quota_raw, &["userResource"]).or(usage_raw);
        let accounts = json_path(quota_root, &["data", "Response", "Data", "Accounts"])
            .and_then(|value| value.as_array());
        accounts
            .into_iter()
            .flatten()
            .filter(|item| item.is_object())
            .collect()
    }

    fn is_active_resource(raw: &Value) -> bool {
        matches!(
            raw.get("Status")
                .and_then(parse_json_number)
                .map(|value| value as i64),
            Some(0 | 3)
        )
    }

    fn resource_package_code(raw: &Value) -> Option<String> {
        raw.get("PackageCode")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn parse_cycle_total(raw: &Value) -> f64 {
        raw.get("CycleCapacitySizePrecise")
            .and_then(parse_json_number)
            .or_else(|| raw.get("CycleCapacitySize").and_then(parse_json_number))
            .or_else(|| raw.get("CapacitySizePrecise").and_then(parse_json_number))
            .or_else(|| raw.get("CapacitySize").and_then(parse_json_number))
            .unwrap_or(0.0)
    }

    fn parse_cycle_remain(raw: &Value) -> f64 {
        raw.get("CycleCapacityRemainPrecise")
            .and_then(parse_json_number)
            .or_else(|| raw.get("CycleCapacityRemain").and_then(parse_json_number))
            .or_else(|| raw.get("CapacityRemainPrecise").and_then(parse_json_number))
            .or_else(|| raw.get("CapacityRemain").and_then(parse_json_number))
            .unwrap_or(0.0)
    }

    fn aggregate_resource_entries(entries: &[&Value]) -> Option<Value> {
        if entries.is_empty() {
            return None;
        }
        let mut merged = (*entries.first()?).clone();
        let total: f64 = entries.iter().map(|item| parse_cycle_total(item)).sum();
        let remain: f64 = entries.iter().map(|item| parse_cycle_remain(item)).sum();
        if let Some(object) = merged.as_object_mut() {
            object.insert(
                "CycleCapacitySizePrecise".to_string(),
                Value::String(total.to_string()),
            );
            object.insert(
                "CycleCapacityRemainPrecise".to_string(),
                Value::String(remain.to_string()),
            );
        }
        Some(merged)
    }

    fn to_resource_quota_entry(raw: &Value, extra_code: &str) -> ResourceQuotaEntry {
        let package_code = resource_package_code(raw);
        let package_name = raw
            .get("PackageName")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let total = parse_cycle_total(raw);
        let remain = parse_cycle_remain(raw);
        let used = (total - remain).max(0.0);
        let used_percent = if total > 0.0 {
            clamp_percent((used / total) * 100.0)
        } else {
            0
        };
        let cycle_end_at = raw
            .get("CycleEndTime")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_ts);
        let deduction_end_time = raw.get("DeductionEndTime").and_then(parse_json_number);
        let expire_at = deduction_end_time
            .and_then(parse_timestamp_number)
            .or_else(|| {
                raw.get("ExpiredTime")
                    .and_then(|value| value.as_str())
                    .and_then(parse_rfc3339_ts)
            })
            .or(cycle_end_at);
        let refresh_at =
            if cycle_end_at.is_some() && expire_at.is_some() && cycle_end_at != expire_at {
                cycle_end_at.map(|value| value + 1)
            } else {
                None
            };
        let is_base_package = package_code.as_deref() != Some(extra_code);

        ResourceQuotaEntry {
            package_code,
            package_name,
            total,
            remain,
            used,
            used_percent,
            refresh_at,
            expire_at,
            is_base_package,
        }
    }

    fn resolve_codebuddy_plan_badge(
        account: &crate::models::codebuddy::CodebuddyAccount,
    ) -> String {
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";

        let profile_type = account
            .profile_raw
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(profile_type.as_str(), "ultimate" | "exclusive" | "premise") {
            return "ENTERPRISE".to_string();
        }

        let accounts =
            resource_account_roots(account.quota_raw.as_ref(), account.usage_raw.as_ref());
        let active: Vec<&Value> = accounts
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if active.iter().any(|item| {
            matches!(
                resource_package_code(item).as_deref(),
                Some(PRO_MON | PRO_YEAR)
            )
        }) {
            return "PRO".to_string();
        }
        if active
            .iter()
            .any(|item| resource_package_code(item).as_deref() == Some(GIFT))
        {
            return "TRIAL".to_string();
        }
        if active.is_empty() {
            let source = first_non_empty(&[
                account.payment_type.as_deref(),
                account.plan_type.as_deref(),
            ])
            .unwrap_or("");
            let normalized = source.to_ascii_lowercase();
            if normalized.contains("enterprise") {
                return "ENTERPRISE".to_string();
            }
            if normalized.contains("trial") {
                return "TRIAL".to_string();
            }
            if normalized.contains("pro") {
                return "PRO".to_string();
            }
            if normalized.contains("free") {
                return "FREE".to_string();
            }
            if !source.is_empty() {
                return source.to_ascii_uppercase();
            }
            return "UNKNOWN".to_string();
        }
        "FREE".to_string()
    }

    fn resolve_workbuddy_plan_badge(
        account: &crate::models::workbuddy::WorkbuddyAccount,
    ) -> String {
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";

        let profile_type = account
            .profile_raw
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(profile_type.as_str(), "ultimate" | "exclusive" | "premise") {
            return "ENTERPRISE".to_string();
        }

        let accounts =
            resource_account_roots(account.quota_raw.as_ref(), account.usage_raw.as_ref());
        let active: Vec<&Value> = accounts
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if active.iter().any(|item| {
            matches!(
                resource_package_code(item).as_deref(),
                Some(PRO_MON | PRO_YEAR)
            )
        }) {
            return "PRO".to_string();
        }
        if active
            .iter()
            .any(|item| resource_package_code(item).as_deref() == Some(GIFT))
        {
            return "TRIAL".to_string();
        }
        if active.is_empty() {
            let source = first_non_empty(&[
                account.payment_type.as_deref(),
                account.plan_type.as_deref(),
            ])
            .unwrap_or("");
            let normalized = source.to_ascii_lowercase();
            if normalized.contains("enterprise") {
                return "ENTERPRISE".to_string();
            }
            if normalized.contains("trial") {
                return "TRIAL".to_string();
            }
            if normalized.contains("pro") {
                return "PRO".to_string();
            }
            if normalized.contains("free") {
                return "FREE".to_string();
            }
            if !source.is_empty() {
                return source.to_ascii_uppercase();
            }
            return "UNKNOWN".to_string();
        }
        "FREE".to_string()
    }

    fn build_resource_quota_model(
        quota_raw: Option<&Value>,
        usage_raw: Option<&Value>,
    ) -> ResourceQuotaModel {
        const FREE: &str = "TCACA_code_001_PqouKr6QWV";
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";
        const ACTIVITY: &str = "TCACA_code_007_nzdH5h4Nl0";
        const FREE_MON: &str = "TCACA_code_008_cfWoLwvjU4";
        const EXTRA: &str = "TCACA_code_009_0XmEQc2xOf";

        let all: Vec<&Value> = resource_account_roots(quota_raw, usage_raw)
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if all.is_empty() {
            return ResourceQuotaModel {
                resources: Vec::new(),
                extra: ResourceQuotaEntry {
                    package_code: Some(EXTRA.to_string()),
                    is_base_package: false,
                    ..Default::default()
                },
            };
        }

        let pro: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| {
                matches!(
                    resource_package_code(item).as_deref(),
                    Some(PRO_MON | PRO_YEAR)
                )
            })
            .collect();
        let extras: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(EXTRA))
            .collect();
        let trial_or_free_mon: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| {
                matches!(
                    resource_package_code(item).as_deref(),
                    Some(GIFT | FREE_MON)
                )
            })
            .collect();
        let free: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(FREE))
            .collect();
        let activity: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(ACTIVITY))
            .collect();

        let merged_trial_or_free_mon = aggregate_resource_entries(&trial_or_free_mon);
        let merged_free = aggregate_resource_entries(&free);
        let mut ordered = Vec::new();
        if let Some(item) = merged_trial_or_free_mon.as_ref() {
            ordered.push(item);
        }
        ordered.extend(pro.iter().copied());
        ordered.extend(activity.iter().copied());
        if let Some(item) = merged_free.as_ref() {
            ordered.push(item);
        }

        let resources = ordered
            .into_iter()
            .map(|item| to_resource_quota_entry(item, EXTRA))
            .collect();
        let extra = aggregate_resource_entries(&extras)
            .map(|value| to_resource_quota_entry(&value, EXTRA))
            .unwrap_or(ResourceQuotaEntry {
                package_code: Some(EXTRA.to_string()),
                is_base_package: false,
                ..Default::default()
            });

        ResourceQuotaModel { resources, extra }
    }

    fn resolve_codebuddy_resource_label(lang: &str, resource: &ResourceQuotaEntry) -> String {
        match resource.package_code.as_deref() {
            Some("TCACA_code_009_0XmEQc2xOf") => {
                translate_or(lang, "codebuddy.extraCredit.title", "加量包", &[])
            }
            Some("TCACA_code_007_nzdH5h4Nl0") => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.activity",
                "活动赠送包",
                &[],
            ),
            Some(
                "TCACA_code_001_PqouKr6QWV"
                | "TCACA_code_006_DbXS0lrypC"
                | "TCACA_code_008_cfWoLwvjU4",
            ) => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.base",
                "基础体验包",
                &[],
            ),
            Some("TCACA_code_002_AkiJS3ZHF5" | "TCACA_code_003_FAnt7lcmRT") => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.pro",
                "专业版订阅",
                &[],
            ),
            _ => resource.package_name.clone().unwrap_or_else(|| {
                translate_or(
                    lang,
                    "codebuddy.quotaQuery.packageUnknown",
                    "套餐信息未知",
                    &[],
                )
            }),
        }
    }

    fn resolve_workbuddy_resource_label(lang: &str, resource: &ResourceQuotaEntry) -> String {
        match resource.package_code.as_deref() {
            Some("TCACA_code_009_0XmEQc2xOf") => {
                translate_or(lang, "workbuddy.extraCredit.title", "加量包", &[])
            }
            Some("TCACA_code_007_nzdH5h4Nl0") => translate_or(
                lang,
                "workbuddy.quotaQuery.packageTitle.activity",
                "活动赠送包",
                &[],
            ),
            Some(
                "TCACA_code_001_PqouKr6QWV"
                | "TCACA_code_006_DbXS0lrypC"
                | "TCACA_code_008_cfWoLwvjU4",
            ) => translate_or(
                lang,
                "workbuddy.quotaQuery.packageTitle.base",
                "基础体验包",
                &[],
            ),
            Some("TCACA_code_002_AkiJS3ZHF5" | "TCACA_code_003_FAnt7lcmRT") => resource
                .package_name
                .clone()
                .unwrap_or_else(|| "PRO".to_string()),
            _ => resource.package_name.clone().unwrap_or_else(|| {
                translate_or(
                    lang,
                    "workbuddy.quotaQuery.packageUnknown",
                    "套餐信息未知",
                    &[],
                )
            }),
        }
    }

    fn build_qoder_subscription_info(
        account: &crate::models::qoder::QoderAccount,
    ) -> QoderSubscriptionInfo {
        let roots = [
            account.auth_credit_usage_raw.as_ref(),
            account.auth_user_plan_raw.as_ref(),
            account.auth_user_info_raw.as_ref(),
        ];
        let _plan_tag = first_string_from_roots(
            &roots,
            &[
                &["plan_tier_name"],
                &["tier_name"],
                &["tierName"],
                &["planTierName"],
                &["plan"],
                &["userTag"],
                &["user_tag"],
            ],
        )
        .map(str::to_string)
        .or_else(|| account.plan_type.clone())
        .unwrap_or_else(|| "UNKNOWN".to_string());

        let parse_bucket =
            |sources: &[Option<&Value>], fallback: QoderQuotaBucket| -> QoderQuotaBucket {
                let raw = sources.iter().find_map(|value| *value);
                let used = raw
                    .and_then(|value| {
                        json_path(Some(value), &["used"])
                            .or_else(|| json_path(Some(value), &["usage"]))
                            .or_else(|| json_path(Some(value), &["consumed"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.used);
                let total = raw
                    .and_then(|value| {
                        json_path(Some(value), &["total"])
                            .or_else(|| json_path(Some(value), &["quota"]))
                            .or_else(|| json_path(Some(value), &["limit"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.total);
                let remaining = raw
                    .and_then(|value| {
                        json_path(Some(value), &["remaining"])
                            .or_else(|| json_path(Some(value), &["available"]))
                            .or_else(|| json_path(Some(value), &["left"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.remaining)
                    .or_else(|| match (used, total) {
                        (Some(used), Some(total)) => Some((total - used).max(0.0)),
                        _ => None,
                    });
                let percentage = raw
                    .and_then(|value| {
                        json_path(Some(value), &["percentage"])
                            .or_else(|| json_path(Some(value), &["usagePercent"]))
                            .or_else(|| json_path(Some(value), &["usage_percentage"]))
                            .and_then(parse_json_number)
                    })
                    .map(clamp_percent)
                    .or(fallback.percentage)
                    .or_else(|| match (used, total) {
                        (Some(used), Some(total)) if total > 0.0 => {
                            Some(clamp_percent((used / total) * 100.0))
                        }
                        _ => None,
                    });

                QoderQuotaBucket {
                    used,
                    total,
                    remaining,
                    percentage,
                }
            };

        let user_quota = parse_bucket(
            &[
                json_path(account.auth_credit_usage_raw.as_ref(), &["userQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["userQuota"]),
                json_path(account.auth_user_info_raw.as_ref(), &["userQuota"]),
            ],
            QoderQuotaBucket {
                used: account.credits_used,
                total: account.credits_total,
                remaining: account.credits_remaining,
                percentage: account.credits_usage_percent.map(clamp_percent),
            },
        );
        let add_on_quota = parse_bucket(
            &[
                json_path(account.auth_credit_usage_raw.as_ref(), &["addOnQuota"]),
                json_path(account.auth_credit_usage_raw.as_ref(), &["addonQuota"]),
                json_path(account.auth_credit_usage_raw.as_ref(), &["add_on_quota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["addOnQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["addonQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["add_on_quota"]),
            ],
            QoderQuotaBucket::default(),
        );

        let shared_credit_root = [
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["orgResourcePackage"],
            ),
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["organizationResourcePackage"],
            ),
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["sharedCreditPackage"],
            ),
            json_path(account.auth_credit_usage_raw.as_ref(), &["resourcePackage"]),
            json_path(account.auth_user_plan_raw.as_ref(), &["orgResourcePackage"]),
        ];
        let shared_credit_package_used =
            shared_credit_root.into_iter().flatten().find_map(|value| {
                json_path(Some(value), &["used"])
                    .or_else(|| json_path(Some(value), &["usage"]))
                    .or_else(|| json_path(Some(value), &["consumed"]))
                    .or_else(|| json_path(Some(value), &["count"]))
                    .and_then(parse_json_number)
            });

        QoderSubscriptionInfo {
            user_quota,
            add_on_quota,
            shared_credit_package_used,
            total_usage_percentage: first_number_from_roots(
                &roots,
                &[&["totalUsagePercentage"], &["total_usage_percentage"]],
            )
            .map(clamp_percent),
        }
    }

    fn build_trae_usage_summary(account: &crate::models::trae::TraeAccount) -> TraeUsageSummary {
        let Some(usage_root) = account
            .trae_usage_raw
            .as_ref()
            .and_then(|value| value.as_object())
        else {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        };

        if usage_root
            .get("code")
            .and_then(parse_json_number)
            .map(|value| value as i64)
            != Some(0)
        {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        }

        let packs = usage_root
            .get("user_entitlement_pack_list")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if packs.is_empty() {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        }

        let product_type = |pack: &Value| -> i64 {
            json_path(Some(pack), &["entitlement_base_info", "product_type"])
                .or_else(|| json_path(Some(pack), &["product_type"]))
                .and_then(parse_json_number)
                .map(|value| value as i64)
                .unwrap_or(-1)
        };

        let valid_packs: Vec<Value> = packs
            .into_iter()
            .filter(|pack| product_type(pack) != 3)
            .collect();
        let find_pack = |target: i64| valid_packs.iter().find(|pack| product_type(pack) == target);

        let selected_pack = find_pack(6)
            .or_else(|| find_pack(4))
            .or_else(|| find_pack(1))
            .or_else(|| find_pack(9))
            .or_else(|| find_pack(8))
            .or_else(|| find_pack(0));
        let pay_go_pack = find_pack(7);

        let spent_usd = selected_pack
            .and_then(|pack| {
                json_path(Some(pack), &["usage", "basic_usage_amount"])
                    .or_else(|| json_path(Some(pack), &["usage", "basic_usage"]))
            })
            .and_then(parse_json_number)
            .unwrap_or(0.0);
        let total_usd = selected_pack
            .and_then(|pack| {
                json_path(
                    Some(pack),
                    &["entitlement_base_info", "quota", "basic_usage_limit"],
                )
                .or_else(|| {
                    json_path(
                        Some(pack),
                        &["entitlement_base_info", "quota", "basic_quota"],
                    )
                })
            })
            .and_then(parse_json_number)
            .unwrap_or(0.0);
        let reset_at = selected_pack
            .and_then(|pack| {
                json_path(Some(pack), &["entitlement_base_info", "quota_reset_time"])
                    .or_else(|| json_path(Some(pack), &["entitlement_base_info", "end_time"]))
                    .or_else(|| json_path(Some(pack), &["next_reset_time"]))
            })
            .and_then(parse_timestamp_like)
            .or(account.plan_reset_at);
        let pay_as_you_go_usd = pay_go_pack
            .and_then(|pack| {
                json_path(Some(pack), &["usage", "basic_usage_amount"])
                    .or_else(|| json_path(Some(pack), &["usage", "basic_usage"]))
            })
            .and_then(parse_json_number);

        TraeUsageSummary {
            used_percent: if total_usd > 0.0 {
                Some(clamp_percent((spent_usd / total_usd) * 100.0))
            } else {
                None
            },
            spent_usd: Some(spent_usd),
            total_usd: Some(total_usd),
            reset_at,
            pay_as_you_go_open: Some(pay_go_pack.is_some()),
            pay_as_you_go_usd,
        }
    }

    fn build_platform_cards(
        platform: PlatformId,
        lang: &str,
    ) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        match platform {
            PlatformId::Antigravity => build_antigravity_cards(lang),
            PlatformId::Codex => build_codex_cards(lang),
            PlatformId::Claude => build_claude_cards(lang, true),
            PlatformId::GitHubCopilot => build_ghcp_cards(lang),
            PlatformId::Windsurf => build_windsurf_cards(lang),
            PlatformId::Kiro => build_kiro_cards(lang),
            PlatformId::Cursor => build_cursor_cards(lang),
            PlatformId::Grok => build_grok_cards(lang),
            PlatformId::Qoder => build_qoder_cards(lang),
            PlatformId::Zcode => build_zcode_cards(lang),
            PlatformId::Trae
            | PlatformId::TraeSolo
            | PlatformId::TraeCn
            | PlatformId::TraeSoloCn => build_trae_cards(lang, platform),
            PlatformId::Codebuddy => build_codebuddy_cards(lang),
            PlatformId::CodebuddyCn => build_codebuddy_cn_cards(lang),
            PlatformId::Workbuddy => build_workbuddy_cards(lang),
            PlatformId::Zed => build_zed_cards(lang),
        }
    }

    fn build_antigravity_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let accounts = modules::account::list_accounts().unwrap_or_default();
        let current_id = modules::account::get_current_account()
            .ok()
            .flatten()
            .map(|account| account.id);

        let mut sorted = accounts;
        sorted.sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let recommended = current_id
            .as_deref()
            .and_then(|id| modules::account::pick_quota_alert_recommendation(&sorted, id))
            .map(|account| account.id);

        let cards = sorted
            .into_iter()
            .map(|account| {
                let quota = account.quota.as_ref();
                let quota_rows = build_antigravity_quota_rows(lang, quota);
                AccountCard {
                    id: account.id,
                    title: account.email,
                    plan: resolve_antigravity_plan_label(quota),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    remaining_percent: min_quota_progress(&quota_rows, true),
                    quota_rows,
                }
            })
            .collect();

        (cards, current_id, recommended)
    }

    fn localize_codex_pool_window_label(lang: &str, label: &str) -> String {
        // 与前端 formatCodexQuotaPoolWindowLabel 对齐：Weekly → 周。
        if label.eq_ignore_ascii_case("Weekly") {
            return translate_or(lang, "codex.localAccess.quotaPool.weeklyShort", "周", &[]);
        }
        label.to_string()
    }

    fn build_codex_api_service_card(lang: &str) -> AccountCard {
        let pool = modules::codex_local_access::menu_bar_api_service_quota();
        let mut rows = Vec::new();

        for window in &pool.windows {
            let tone_pct = window.percentage.clamp(0, 100);
            rows.push(make_progress_row(
                localize_codex_pool_window_label(lang, &window.label),
                format!("{}%", window.percentage),
                tone_pct,
                None,
                codex_remaining_tone(tone_pct),
            ));
        }
        if rows.is_empty() {
            rows.push(make_text_row(
                translate_or(lang, "common.shared.quota.noData", "No quota data", &[]),
                "-".to_string(),
                Some(modules::i18n::translate(lang, "common.refresh", &[])),
            ));
        } else {
            rows.insert(
                0,
                make_text_row(
                    translate_or(
                        lang,
                        "settings.general.codexAppUiInjectionPoolLabel",
                        "Accounts",
                        &[],
                    ),
                    pool.account_count.to_string(),
                    None,
                ),
            );
        }

        AccountCard {
            id: modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
            title: translate_or(lang, "codex.localAccess.title", "API Service", &[]),
            plan: Some("API".to_string()),
            updated_at: Some(chrono::Utc::now().timestamp()),
            quota_rows: rows,
            remaining_percent: pool.remaining_percent,
        }
    }

    fn build_codex_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codex_account::list_accounts();
        let api_service_current = is_codex_api_service_current();
        let current_id = if api_service_current {
            Some(modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string())
        } else {
            modules::codex_account::resolve_current_account_id(&accounts)
        };
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let recommended = current_id.as_deref().and_then(|id| {
            if modules::codex_instance::is_api_service_bind_account_id(id) {
                return None;
            }
            accounts
                .iter()
                .filter(|account| account.id != id && account.quota.is_some())
                .max_by(|left, right| {
                    let score_left = left
                        .quota
                        .as_ref()
                        .map(|quota| quota.hourly_percentage + quota.weekly_percentage)
                        .unwrap_or(-1);
                    let score_right = right
                        .quota
                        .as_ref()
                        .map(|quota| quota.hourly_percentage + quota.weekly_percentage)
                        .unwrap_or(-1);
                    score_left.cmp(&score_right)
                })
                .map(|account| account.id.clone())
        });

        let mut cards: Vec<AccountCard> = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if account.is_api_key_auth() {
                    rows = build_codex_api_key_usage_rows(lang, &account);
                } else if let Some(quota) = account.quota.as_ref() {
                    let has_presence_flags = quota.hourly_window_present.is_some()
                        || quota.weekly_window_present.is_some();
                    if !has_presence_flags || quota.hourly_window_present == Some(true) {
                        let percentage = quota.hourly_percentage.clamp(0, 100);
                        rows.push(make_progress_row(
                            format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, quota.hourly_reset_time),
                            codex_remaining_tone(percentage),
                        ));
                    }
                    if !has_presence_flags || quota.weekly_window_present == Some(true) {
                        let percentage = quota.weekly_percentage.clamp(0, 100);
                        rows.push(make_progress_row(
                            format_codex_quota_metric_label(quota.weekly_window_minutes, "Weekly"),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, quota.weekly_reset_time),
                            codex_remaining_tone(percentage),
                        ));
                    }
                    if let Some(mut code_review) = parse_code_review_metric(Some(quota)) {
                        code_review.label = "Code Review".to_string();
                        code_review.subtext = quota
                            .raw_data
                            .as_ref()
                            .and_then(|raw| raw.get("code_review_rate_limit"))
                            .and_then(|rate_limit| {
                                rate_limit
                                    .get("primary_window")
                                    .or_else(|| rate_limit.get("secondary_window"))
                            })
                            .and_then(|window| {
                                window
                                    .get("reset_at")
                                    .and_then(parse_json_number)
                                    .map(|value| value.floor() as i64)
                                    .or_else(|| {
                                        window
                                            .get("reset_after_seconds")
                                            .and_then(parse_json_number)
                                            .map(|value| {
                                                chrono::Utc::now().timestamp()
                                                    + value.floor() as i64
                                            })
                                    })
                            })
                            .and_then(|ts| format_reset_subtext(lang, Some(ts)));
                        rows.push(code_review);
                    }
                }
                let remaining_percent = min_quota_progress(&rows, !account.is_api_key_auth());
                AccountCard {
                    id: account.id,
                    title: if matches!(
                        account.auth_mode,
                        crate::models::codex::CodexAuthMode::Apikey
                    ) && account
                        .account_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
                    {
                        account.account_name.unwrap_or(account.email)
                    } else {
                        account.email
                    },
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();

        // 有 API 服务账号池或当前正是 API 服务时，插入虚拟卡片展示池额度。
        if api_service_current || modules::codex_local_access::api_service_collection_has_accounts()
        {
            cards.insert(0, build_codex_api_service_card(lang));
        }

        (cards, current_id, recommended)
    }

    fn is_claude_desktop_account(account: &crate::models::claude::ClaudeAccount) -> bool {
        matches!(
            account.auth_mode,
            crate::models::claude::ClaudeAuthMode::DesktopOAuth
                | crate::models::claude::ClaudeAuthMode::DesktopGateway
        )
    }

    fn build_claude_cards(
        lang: &str,
        desktop: bool,
    ) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let fallback_title = if desktop { "Claude" } else { "Claude CLI" };
        let mut accounts = modules::claude_account::list_accounts()
            .into_iter()
            .filter(|account| is_claude_desktop_account(account) == desktop)
            .collect::<Vec<_>>();
        let current_platform = if desktop {
            "claude_desktop_account"
        } else {
            "claude_code_account"
        };
        let current_id = modules::claude_account::resolve_current_account_for_platform(
            current_platform,
            &accounts,
        )
        .map(|account| account.id);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let quota = account.quota.as_ref()?;
                    let values = [quota.five_hour_percentage, quota.seven_day_percentage];
                    let avg = values.iter().copied().sum::<i32>() as f64 / values.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .min_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| right.2.cmp(&left.2))
                })
                .map(|item| item.0)
        });

        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let Some(quota) = account.quota.as_ref() {
                    let show_remaining =
                        crate::modules::config::get_user_config().claude_quota_display_remaining;
                    let five_hour_used = quota.five_hour_percentage.clamp(0, 100);
                    let five_hour_display = if show_remaining {
                        (100 - five_hour_used).clamp(0, 100)
                    } else {
                        five_hour_used
                    };
                    rows.push(make_progress_row(
                        translate_or(lang, "claude.quota.fiveHour", "Current session", &[]),
                        format!("{five_hour_display}%"),
                        five_hour_display,
                        format_reset_subtext(lang, quota.five_hour_reset_time),
                        usage_warning_tone(five_hour_used),
                    ));

                    let seven_day_used = quota.seven_day_percentage.clamp(0, 100);
                    let seven_day_display = if show_remaining {
                        (100 - seven_day_used).clamp(0, 100)
                    } else {
                        seven_day_used
                    };
                    rows.push(make_progress_row(
                        translate_or(
                            lang,
                            "claude.quota.sevenDay",
                            "Current week (all models)",
                            &[],
                        ),
                        format!("{seven_day_display}%"),
                        seven_day_display,
                        format_reset_subtext(lang, quota.seven_day_reset_time),
                        usage_warning_tone(seven_day_used),
                    ));
                } else if let Some(error) = account.quota_error.as_ref() {
                    rows.push(make_text_row(
                        translate_or(lang, "common.shared.columns.status", "Status", &[]),
                        error.message.clone(),
                        None,
                    ));
                }

                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id,
                    title: first_non_empty(&[
                        Some(account.email.as_str()),
                        account.organization_name.as_deref(),
                    ])
                    .unwrap_or(fallback_title)
                    .to_string(),
                    plan: first_non_empty(&[
                        account.plan_type.as_deref(),
                        account.organization_name.as_deref(),
                    ])
                    .map(str::to_string),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();

        (cards, current_id, recommended)
    }

    fn build_ghcp_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::github_copilot_account::list_accounts();
        let current_id = modules::github_copilot_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::github_copilot_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = compute_copilot_usage(
                    &account.copilot_token,
                    account.copilot_plan.as_deref(),
                    account.copilot_limited_user_quotas.as_ref(),
                    account.copilot_quota_snapshots.as_ref(),
                    account.copilot_limited_user_reset_date,
                    account.copilot_quota_reset_date.as_deref(),
                );
                let rows = vec![
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "common.shared.instances.quota.inline",
                            "Inline Suggestions",
                            &[],
                        ),
                        usage.inline,
                        usage.reset_ts,
                    ),
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "common.shared.instances.quota.chat",
                            "Chat messages",
                            &[],
                        ),
                        usage.chat,
                        usage.reset_ts,
                    ),
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "githubCopilot.columns.premium",
                            "Premium requests",
                            &[],
                        ),
                        usage.premium,
                        None,
                    ),
                ];
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id,
                    title: account
                        .github_email
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account.copilot_plan,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_windsurf_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::windsurf_account::list_accounts();
        let current_id = modules::windsurf_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::windsurf_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let rows = match resolve_windsurf_usage_mode(&account) {
                    WindsurfUsageMode::Quota => {
                        let summary = resolve_windsurf_quota_usage_summary(&account);
                        let mut rows = Vec::new();
                        if let Some(percentage) = summary.daily_used_percent {
                            rows.push(make_progress_row(
                                translate_or(
                                    lang,
                                    "windsurf.usageSummary.dailyQuota",
                                    "Daily quota usage",
                                    &[],
                                ),
                                format!("{percentage}%"),
                                percentage,
                                format_reset_subtext(lang, summary.daily_reset_ts),
                                usage_warning_tone(percentage),
                            ));
                        }
                        if let Some(percentage) = summary.weekly_used_percent {
                            rows.push(make_progress_row(
                                translate_or(
                                    lang,
                                    "windsurf.usageSummary.weeklyQuota",
                                    "Weekly quota usage",
                                    &[],
                                ),
                                format!("{percentage}%"),
                                percentage,
                                format_reset_subtext(lang, summary.weekly_reset_ts),
                                usage_warning_tone(percentage),
                            ));
                        }
                        rows.push(make_text_row(
                            translate_or(
                                lang,
                                "windsurf.usageSummary.extraUsageBalance",
                                "Extra usage balance",
                                &[],
                            ),
                            format_micros_usd(summary.overage_balance_micros.unwrap_or(0.0)),
                            None,
                        ));
                        rows
                    }
                    WindsurfUsageMode::Credits => {
                        let summary = resolve_windsurf_credits_summary(&account);
                        let cycle_text = format_reset_subtext(lang, summary.plan_end_ts);
                        let mut rows = Vec::new();
                        let credits_left_text = if let Some(value) = summary.credits_left {
                            let formatted = format_quota_number(value);
                            translate_or(
                                lang,
                                "windsurf.credits.left",
                                "{{value}} credits left",
                                &[("value", formatted.as_str())],
                            )
                        } else {
                            translate_or(
                                lang,
                                "windsurf.credits.leftUnknown",
                                "Credits left -",
                                &[],
                            )
                        };
                        rows.push(make_text_row(
                            translate_or(lang, "windsurf.credits.title", "Plan", &[]),
                            credits_left_text,
                            None,
                        ));

                        let prompt_progress = match (summary.prompt_total, summary.prompt_used) {
                            (Some(total), Some(used)) if total > 0.0 => {
                                clamp_percent((used / total) * 100.0)
                            }
                            _ => 0,
                        };
                        let prompt_value = match (summary.prompt_left, summary.prompt_total) {
                            (Some(left), Some(total)) if total > 0.0 => {
                                let remaining = format_quota_number(left);
                                let total_text = format_quota_number(total);
                                translate_or(
                                    lang,
                                    "windsurf.credits.promptLeft",
                                    "{{remaining}}/{{total}} prompt credits left",
                                    &[
                                        ("remaining", remaining.as_str()),
                                        ("total", total_text.as_str()),
                                    ],
                                )
                            }
                            (Some(left), _) if left > 0.0 => {
                                let remaining = format_quota_number(left);
                                translate_or(
                                    lang,
                                    "windsurf.credits.promptLeftNoTotal",
                                    "{{remaining}} prompt credits left",
                                    &[("remaining", remaining.as_str())],
                                )
                            }
                            _ => translate_or(
                                lang,
                                "windsurf.credits.promptLeftUnknown",
                                "Prompt credits left -",
                                &[],
                            ),
                        };
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "windsurf.credits.promptCreditsLeftLabel",
                                "prompt credits left",
                                &[],
                            ),
                            prompt_value,
                            prompt_progress,
                            cycle_text.clone(),
                            usage_warning_tone(prompt_progress),
                        ));

                        let add_on_progress = match (summary.add_on_total, summary.add_on_used) {
                            (Some(total), Some(used)) if total > 0.0 => {
                                clamp_percent((used / total) * 100.0)
                            }
                            _ => 0,
                        };
                        let add_on_left = format_quota_number(summary.add_on_left.unwrap_or(0.0));
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "windsurf.credits.addOnCreditsAvailableLabel",
                                "add-on credits available",
                                &[],
                            ),
                            translate_or(
                                lang,
                                "windsurf.credits.addOnAvailable",
                                "{{count}} add-on credits available",
                                &[("count", add_on_left.as_str())],
                            ),
                            add_on_progress,
                            cycle_text,
                            usage_warning_tone(add_on_progress),
                        ));
                        rows
                    }
                };
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id,
                    title: account
                        .github_email
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account.copilot_plan,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_kiro_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::kiro_account::list_accounts();
        let current_id = modules::kiro_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter(|account| !modules::kiro_account::is_banned_account(account))
                .filter_map(|account| {
                    let metrics = modules::kiro_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let (Some(total), Some(used)) = (account.credits_total, account.credits_used) {
                    if total > 0.0 {
                        let percentage = clamp_percent((used / total) * 100.0);
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "common.shared.columns.promptCredits",
                                "User Prompt credits",
                                &[],
                            ),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, account.usage_reset_at),
                            usage_warning_tone(percentage),
                        ));
                    }
                }
                if let (Some(total), Some(used)) = (account.bonus_total, account.bonus_used) {
                    if total > 0.0 || used > 0.0 {
                        let percentage = if total > 0.0 {
                            clamp_percent((used / total) * 100.0)
                        } else {
                            0
                        };
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "common.shared.columns.addOnPromptCredits",
                                "Add-on prompt credits",
                                &[],
                            ),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, account.usage_reset_at),
                            usage_warning_tone(percentage),
                        ));
                    }
                }
                let account_id = account.id.clone();
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account_id.clone(),
                    title: if account.email.trim().is_empty() {
                        account_id
                    } else {
                        account.email
                    },
                    plan: account.plan_name.or(account.plan_tier),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_cursor_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::cursor_account::list_accounts();
        let current_id = modules::cursor_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter(|account| !modules::cursor_account::is_banned_account(account))
                .filter_map(|account| {
                    let metrics = modules::cursor_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = read_cursor_tray_usage(&account);
                let mut rows = Vec::new();
                if let Some(percentage) = usage.total_used_percent {
                    rows.push(make_progress_row(
                        "Total Usage".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        format_reset_subtext(lang, usage.reset_ts),
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(percentage) = usage.auto_used_percent {
                    rows.push(make_progress_row(
                        "Auto + Composer".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        None,
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(percentage) = usage.api_used_percent {
                    rows.push(make_progress_row(
                        "API Usage".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        None,
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(value) = usage.on_demand_text.clone() {
                    let progress = if value == "Unlimited" || value == "Disabled" {
                        None
                    } else {
                        usage.on_demand_percent
                    };
                    rows.push(QuotaRow {
                        label: translate_or(lang, "cursor.quota.onDemand", "On-Demand", &[]),
                        value: if value == "Unlimited" {
                            translate_or(lang, "common.shared.unlimited", "Unlimited", &[])
                        } else if value == "Disabled" {
                            translate_or(lang, "common.disabled", "Disabled", &[])
                        } else {
                            value
                        },
                        progress,
                        progress_tone: progress.map(cursor_usage_tone),
                        subtext: None,
                    });
                }
                let account_id = account.id.clone();
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account_id.clone(),
                    title: if account.email.trim().is_empty() {
                        account_id
                    } else {
                        account.email
                    },
                    plan: account.membership_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_grok_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::grok_account::list_accounts_checked().unwrap_or_default();
        let current_id = modules::grok_account::current_account_id().ok().flatten();
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let remaining_metrics = |account: &crate::models::grok::GrokAccountView| {
            let Some(quota) = account.quota.as_ref() else {
                return Vec::new();
            };
            let mut values = Vec::new();
            if let Some(used) = quota.weekly_limit_percent {
                values.push((100 - clamp_percent(used)).clamp(0, 100));
            }
            values.extend(quota.products.iter().filter_map(|product| {
                product
                    .usage_percent
                    .map(|used| (100 - clamp_percent(used)).clamp(0, 100))
            }));
            if let (Some(used), Some(cap)) = (quota.on_demand_used, quota.on_demand_cap) {
                if cap > 0.0 {
                    values.push((100 - clamp_percent(used / cap * 100.0)).clamp(0, 100));
                }
            }
            values
        };

        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter(|account| {
                    account.quota_query_last_error.is_none()
                        && account
                            .status
                            .as_deref()
                            .map(|status| matches!(status, "normal" | "ok"))
                            .unwrap_or(true)
                })
                .filter_map(|account| {
                    let minimum = remaining_metrics(account).into_iter().min()?;
                    if minimum <= 0 {
                        return None;
                    }
                    Some((account.id.clone(), minimum, account.last_used))
                })
                .max_by_key(|item| (item.1, item.2))
                .map(|item| item.0)
        });

        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let Some(quota) = account.quota.as_ref() {
                    let reset_at = quota
                        .period_end
                        .as_ref()
                        .and_then(|value| parse_timestamp_like(&Value::String(value.clone())));
                    let reset_subtext = format_reset_subtext(lang, reset_at);
                    let left_value = |remaining: i32| {
                        translate_or(
                            lang,
                            "common.shared.quota.leftPercent",
                            "{{value}}% left",
                            &[("value", &remaining.to_string())],
                        )
                    };

                    if let Some(used) = quota.weekly_limit_percent {
                        let remaining = (100 - clamp_percent(used)).clamp(0, 100);
                        rows.push(make_progress_row(
                            translate_or(lang, "grok.quota.weekly", "Weekly usage", &[]),
                            left_value(remaining),
                            remaining,
                            reset_subtext.clone(),
                            remaining_balance_tone(remaining),
                        ));
                    }
                    for product in &quota.products {
                        if let Some(used) = product.usage_percent {
                            let remaining = (100 - clamp_percent(used)).clamp(0, 100);
                            rows.push(make_progress_row(
                                product.product.clone(),
                                left_value(remaining),
                                remaining,
                                reset_subtext.clone(),
                                remaining_balance_tone(remaining),
                            ));
                        }
                    }
                    if let (Some(used), Some(cap)) = (quota.on_demand_used, quota.on_demand_cap) {
                        let remaining = if cap > 0.0 {
                            (100 - clamp_percent(used / cap * 100.0)).clamp(0, 100)
                        } else {
                            0
                        };
                        rows.push(make_progress_row(
                            translate_or(lang, "grok.quota.onDemand", "On-demand", &[]),
                            format!(
                                "{} / {}",
                                format_quota_number(used),
                                format_quota_number(cap)
                            ),
                            remaining,
                            reset_subtext,
                            remaining_balance_tone(remaining),
                        ));
                    }
                    if let Some(balance) = quota.prepaid_balance {
                        rows.push(make_text_row(
                            translate_or(lang, "grok.quota.balance", "Balance", &[]),
                            format_quota_number(balance),
                            None,
                        ));
                    }
                }

                let remaining_percent = min_quota_progress(&rows, true);
                AccountCard {
                    id: account.id,
                    title: account.email,
                    plan: account.plan_type.or_else(|| {
                        account
                            .quota
                            .as_ref()
                            .and_then(|quota| quota.subscription_tier.clone())
                    }),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_qoder_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::qoder_account::list_accounts();
        let current_id = modules::qoder_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let subscription = build_qoder_subscription_info(&account);
                let mut rows = Vec::new();
                let remaining_percent = subscription
                    .total_usage_percentage
                    .map(|value| (100 - value).clamp(0, 100))
                    .or_else(|| {
                        match (
                            subscription.user_quota.remaining,
                            subscription.user_quota.total,
                        ) {
                            (Some(remaining), Some(total)) if total > 0.0 => {
                                Some(clamp_percent((remaining / total) * 100.0))
                            }
                            _ => None,
                        }
                    });
                let used_percent = remaining_percent.map(|value| (100 - value).clamp(0, 100));
                if remaining_percent.is_some()
                    || subscription.user_quota.total.is_some()
                    || subscription.user_quota.used.is_some()
                    || subscription.user_quota.remaining.is_some()
                {
                    let value = remaining_percent
                        .map(|value| {
                            translate_or(
                                lang,
                                "common.shared.remaining",
                                "剩余 {{value}}",
                                &[("value", format!("{value}%").as_str())],
                            )
                        })
                        .unwrap_or_else(|| "--".to_string());
                    let used = format_quota_number(subscription.user_quota.used.unwrap_or(0.0));
                    let total = format_quota_number(subscription.user_quota.total.unwrap_or(0.0));
                    rows.push(make_progress_row(
                        translate_or(
                            lang,
                            "qoder.usageOverview.includedCredits",
                            "套餐内 Credits",
                            &[],
                        ),
                        value,
                        used_percent.unwrap_or(0),
                        Some(translate_or(
                            lang,
                            "qoder.usageOverview.usedOfTotal",
                            "{{used}} / {{total}}",
                            &[("used", used.as_str()), ("total", total.as_str())],
                        )),
                        cursor_usage_tone(used_percent.unwrap_or(0)),
                    ));
                }

                if subscription.add_on_quota.total.unwrap_or(0.0) > 0.0
                    || subscription.add_on_quota.remaining.unwrap_or(0.0) > 0.0
                {
                    let total = subscription.add_on_quota.total.unwrap_or(0.0);
                    let remaining = subscription.add_on_quota.remaining.unwrap_or(0.0);
                    let remaining_percent = if total > 0.0 {
                        clamp_percent((remaining / total) * 100.0)
                    } else {
                        0
                    };
                    let remaining_text = format_quota_number(remaining);
                    let total_text = format_quota_number(total);
                    rows.push(make_progress_row(
                        translate_or(
                            lang,
                            "common.shared.columns.creditPackage",
                            "Credit Package",
                            &[],
                        ),
                        translate_or(
                            lang,
                            "qoder.usageOverview.usedOfTotal",
                            "{{used}} / {{total}}",
                            &[
                                ("used", remaining_text.as_str()),
                                ("total", total_text.as_str()),
                            ],
                        ),
                        (100 - remaining_percent).clamp(0, 100),
                        None,
                        cursor_usage_tone((100 - remaining_percent).clamp(0, 100)),
                    ));
                }

                if let Some(shared_used) = subscription.shared_credit_package_used {
                    rows.push(make_text_row(
                        translate_or(
                            lang,
                            "common.shared.columns.sharedCreditPackage",
                            "Shared Credit Package",
                            &[],
                        ),
                        format_quota_number(shared_used),
                        None,
                    ));
                }
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .display_name
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.email),
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_zcode_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::zcode_account::list_accounts_checked().unwrap_or_default();
        let current_id = modules::zcode_account::current_account_id().ok().flatten();
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let Some(balances) = account.quota_raw.as_ref().and_then(Value::as_array) {
                    for balance in balances {
                        let total = balance
                            .get("total_units")
                            .and_then(parse_json_number)
                            .unwrap_or(0.0)
                            .max(0.0);
                        let used = balance
                            .get("used_units")
                            .and_then(parse_json_number)
                            .unwrap_or(0.0)
                            .max(0.0);
                        let remaining = balance
                            .get("remaining_units")
                            .or_else(|| balance.get("available_units"))
                            .and_then(parse_json_number)
                            .unwrap_or_else(|| (total - used).max(0.0));
                        let used_percent = if total > 0.0 {
                            clamp_percent((used / total) * 100.0)
                        } else {
                            0
                        };
                        let remaining_percent = if total > 0.0 {
                            clamp_percent((remaining / total) * 100.0)
                        } else {
                            0
                        };
                        let remaining_text = format!("{remaining_percent}%");
                        let reset_at = balance
                            .get("period_end")
                            .or_else(|| balance.get("expires_at"))
                            .and_then(parse_json_number)
                            .map(|value| value as i64);
                        rows.push(make_progress_row(
                            balance
                                .get("show_name")
                                .and_then(Value::as_str)
                                .unwrap_or("ZCode")
                                .to_string(),
                            translate_or(
                                lang,
                                "common.shared.remaining",
                                "剩余 {{value}}",
                                &[("value", remaining_text.as_str())],
                            ),
                            used_percent,
                            Some(format!(
                                "{} / {}",
                                format_quota_number(used),
                                format_quota_number(total)
                            )),
                            cursor_usage_tone(used_percent),
                        ));
                        if let Some(subtext) = format_reset_subtext(lang, reset_at) {
                            if let Some(row) = rows.last_mut() {
                                row.subtext = Some(match row.subtext.take() {
                                    Some(usage) => format!("{} · {}", usage, subtext),
                                    None => subtext,
                                });
                            }
                        }
                    }
                }
                let title = if account.email.trim().is_empty()
                    || account.email.eq_ignore_ascii_case("unknown@zcode.local")
                {
                    account
                        .display_name
                        .clone()
                        .or(account.user_id.clone())
                        .unwrap_or_else(|| account.id.clone())
                } else {
                    account.email.clone()
                };
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id,
                    title,
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_trae_cards(
        lang: &str,
        platform: PlatformId,
    ) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::trae_account::list_accounts();
        let current_id = modules::trae_account::TraePlatformKind::parse(Some(platform.as_str()))
            .ok()
            .and_then(|kind| {
                modules::trae_account::resolve_current_account_id_for_platform(&accounts, kind)
            });
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = build_trae_usage_summary(&account);
                let mut rows = Vec::new();
                if usage.used_percent.is_some()
                    || usage.spent_usd.is_some()
                    || usage.total_usd.is_some()
                    || usage.reset_at.is_some()
                {
                    let remaining = usage.used_percent.map(|value| (100 - value).clamp(0, 100));
                    let spent_text = format_currency_dollars(usage.spent_usd.unwrap_or(0.0));
                    let total_text = format_currency_dollars(usage.total_usd.unwrap_or(0.0));
                    rows.push(make_progress_row(
                        translate_or(lang, "trae.columns.usage", "Usage", &[]),
                        remaining
                            .map(|value| {
                                translate_or(
                                    lang,
                                    "common.shared.remaining",
                                    "剩余 {{value}}",
                                    &[("value", format!("{value}%").as_str())],
                                )
                            })
                            .unwrap_or_else(|| "--".to_string()),
                        usage.used_percent.unwrap_or(0),
                        Some(if usage.spent_usd.is_some() || usage.total_usd.is_some() {
                            translate_or(
                                lang,
                                "trae.quota.usedOfTotal",
                                "${{used}} / ${{total}}",
                                &[
                                    ("used", spent_text.trim_start_matches('$')),
                                    ("total", total_text.trim_start_matches('$')),
                                ],
                            )
                        } else {
                            format_reset_subtext(lang, usage.reset_at).unwrap_or_default()
                        }),
                        cursor_usage_tone(usage.used_percent.unwrap_or(0)),
                    ));
                }
                if let Some(opened) = usage.pay_as_you_go_open {
                    rows.push(make_text_row(
                        translate_or(lang, "trae.quota.payAsYouGoLabel", "On-Demand Usage", &[]),
                        usage
                            .pay_as_you_go_usd
                            .map(format_currency_dollars)
                            .unwrap_or_else(|| {
                                if opened {
                                    translate_or(lang, "common.enabled", "Enabled", &[])
                                } else {
                                    translate_or(lang, "common.disabled", "Disabled", &[])
                                }
                            }),
                        None,
                    ));
                }
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .nickname
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.email),
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_codebuddy_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codebuddy_account::list_accounts();
        let current_id = modules::codebuddy_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows: Vec<QuotaRow> = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_codebuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "codebuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "codebuddy.quotaQuery.updatedAt",
                                "codebuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_codebuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_codebuddy_cn_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codebuddy_cn_account::list_accounts();
        let current_id = modules::codebuddy_cn_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows: Vec<QuotaRow> = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_codebuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "codebuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "codebuddy.quotaQuery.updatedAt",
                                "codebuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_codebuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_workbuddy_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::workbuddy_account::list_accounts();
        let current_id = modules::workbuddy_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows: Vec<QuotaRow> = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_workbuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "workbuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "workbuddy.quotaQuery.updatedAt",
                                "workbuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_workbuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_zed_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::zed_account::list_accounts();
        let current_id = modules::zed_account::resolve_current_account_id();
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::zed_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if account.edit_predictions_used.is_some()
                    || account
                        .edit_predictions_limit_raw
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
                {
                    let used = account.edit_predictions_used.unwrap_or(0).max(0);
                    let total = account
                        .edit_predictions_limit_raw
                        .as_deref()
                        .and_then(|value| value.trim().parse::<f64>().ok())
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or(0.0);
                    let progress = if total > 0.0 {
                        clamp_percent((used as f64 / total) * 100.0)
                    } else {
                        0
                    };
                    rows.push(make_progress_row(
                        translate_or(lang, "zed.page.editPredictions", "Edit Predictions", &[]),
                        format!("{used} / {}", format_quota_number(total)),
                        progress,
                        None,
                        remaining_balance_tone((100 - progress).clamp(0, 100)),
                    ));
                }
                if let Some(overdue) = account.has_overdue_invoices {
                    rows.push(make_text_row(
                        translate_or(lang, "zed.page.overdueField", "Overdue", &[]),
                        if overdue {
                            translate_or(lang, "zed.page.overdueYes", "Yes", &[])
                        } else {
                            translate_or(lang, "zed.page.overdueNo", "No", &[])
                        },
                        None,
                    ));
                }
                let remaining_percent = min_quota_progress(&rows, false);
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .display_name
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account
                        .plan_raw
                        .as_deref()
                        .map(format_zed_plan_label)
                        .filter(|value| !value.is_empty()),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                    remaining_percent,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

