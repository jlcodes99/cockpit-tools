// Codex Local Access：Account routing, cooldown policy and model pricing calculation。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn build_cooldown_key(account_id: &str, model_key: &str) -> Option<String> {
    let account_id = account_id.trim();
    let model_key = model_key.trim();
    if account_id.is_empty() || model_key.is_empty() {
        return None;
    }
    Some(format!(
        "{}{}{}",
        account_id, COOLDOWN_KEY_SEPARATOR, model_key
    ))
}

fn build_ordered_account_ids(
    account_ids: &[String],
    start: usize,
    preferred_account_id: Option<&str>,
) -> Vec<String> {
    if account_ids.is_empty() {
        return Vec::new();
    }

    let mut ordered = Vec::with_capacity(account_ids.len());
    for offset in 0..account_ids.len() {
        let account_id = &account_ids[(start + offset) % account_ids.len()];
        if ordered.iter().any(|value| value == account_id) {
            continue;
        }
        ordered.push(account_id.clone());
    }
    let priorities = preferred_account_id
        .map(|account_id| vec![account_id.to_string()])
        .unwrap_or_default();
    prioritize_account_ids(ordered, &priorities)
}

fn normalize_plan_key(plan_type: Option<&str>) -> String {
    let normalized = plan_type.unwrap_or("").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return "free".to_string();
    }
    if normalized.contains("enterprise") {
        return "enterprise".to_string();
    }
    if normalized.contains("health") {
        return "health".to_string();
    }
    if normalized.contains("gov") {
        return "gov".to_string();
    }
    if normalized.contains("teacher") {
        return "teachers".to_string();
    }
    if normalized.contains("business") {
        return "business".to_string();
    }
    if normalized.contains("team") {
        return "team".to_string();
    }
    if normalized.contains("edu") {
        return "edu".to_string();
    }
    if normalized.contains("go") {
        return "go".to_string();
    }
    if normalized.contains("plus") {
        return "plus".to_string();
    }
    if normalized.contains("pro") {
        return "pro".to_string();
    }
    if normalized.contains("free") {
        return "free".to_string();
    }
    normalized
}

fn normalize_auth_file_plan_type(plan_type: Option<&str>) -> Option<&'static str> {
    let normalized = plan_type?
        .trim()
        .to_ascii_lowercase()
        .replace(['_', ' '], "-");
    match normalized.as_str() {
        "prolite" | "pro-lite" => Some("prolite"),
        "promax" | "pro-max" => Some("promax"),
        _ => None,
    }
}

fn resolve_plan_rank(account: &CodexAccount) -> Option<i32> {
    let plan_key = normalize_plan_key(account.plan_type.as_deref());
    let auth_file_plan_type = normalize_auth_file_plan_type(account.auth_file_plan_type.as_deref())
        .or_else(|| normalize_auth_file_plan_type(account.plan_type.as_deref()));

    let rank = match plan_key.as_str() {
        "enterprise" => 700,
        "edu" => 700,
        "health" => 700,
        "gov" => 700,
        "teachers" => 700,
        "pro" => match auth_file_plan_type {
            Some("promax") => 600,
            Some("prolite") => 500,
            _ => 500,
        },
        "business" => 300,
        "team" => 300,
        "plus" => 300,
        "go" => 200,
        "free" => 100,
        _ => return None,
    };

    Some(rank)
}

fn resolve_remaining_quota(account: &CodexAccount) -> Option<i32> {
    let quota = account.quota.as_ref()?;
    let mut percentages = Vec::new();
    if quota.hourly_window_present.unwrap_or(true) {
        percentages.push(quota.hourly_percentage.clamp(0, 100));
    }
    if quota.weekly_window_present.unwrap_or(true) {
        percentages.push(quota.weekly_percentage.clamp(0, 100));
    }
    percentages.into_iter().min()
}

fn resolve_subscription_expiry_ms(account: &CodexAccount) -> Option<i64> {
    let raw = account.subscription_active_until.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.chars().all(|ch| ch.is_ascii_digit()) {
        let mut timestamp = raw.parse::<i64>().ok()?;
        if timestamp < 1_000_000_000_000 {
            timestamp *= 1000;
        }
        return Some(timestamp);
    }

    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| parsed.timestamp_millis())
}

fn build_routing_candidates(ordered_account_ids: &[String]) -> Vec<RoutingCandidate> {
    ordered_account_ids
        .iter()
        .map(|account_id| {
            let account = try_get_cached_account_for_routing(account_id)
                .or_else(|| codex_account::load_account(account_id));
            RoutingCandidate {
                account_id: account_id.clone(),
                plan_rank: account.as_ref().and_then(resolve_plan_rank),
                remaining_quota: account.as_ref().and_then(resolve_remaining_quota),
                subscription_expiry_ms: account.as_ref().and_then(resolve_subscription_expiry_ms),
            }
        })
        .collect()
}

fn compare_routing_candidates(
    left: &RoutingCandidate,
    right: &RoutingCandidate,
    strategy: CodexLocalAccessRoutingStrategy,
    original_index: &HashMap<String, usize>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let compare_option_desc = |a: Option<i32>, b: Option<i32>| match (a, b) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    let compare_option_asc = |a: Option<i32>, b: Option<i32>| match (a, b) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    let compare_option_i64_asc = |a: Option<i64>, b: Option<i64>| match (a, b) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };

    let ordering = match strategy {
        CodexLocalAccessRoutingStrategy::Auto => {
            compare_option_desc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::Random => Ordering::Equal,
        CodexLocalAccessRoutingStrategy::SingleAccount => Ordering::Equal,
        CodexLocalAccessRoutingStrategy::QuotaHighFirst => {
            compare_option_desc(left.remaining_quota, right.remaining_quota)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
        }
        CodexLocalAccessRoutingStrategy::QuotaLowFirst => {
            compare_option_asc(left.remaining_quota, right.remaining_quota)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
        }
        CodexLocalAccessRoutingStrategy::PlanHighFirst => {
            compare_option_desc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::PlanLowFirst => {
            compare_option_asc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::ExpirySoonFirst => {
            compare_option_i64_asc(left.subscription_expiry_ms, right.subscription_expiry_ms)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::Custom => Ordering::Equal,
    };

    ordering.then_with(|| {
        let left_index = original_index
            .get(&left.account_id)
            .copied()
            .unwrap_or(usize::MAX);
        let right_index = original_index
            .get(&right.account_id)
            .copied()
            .unwrap_or(usize::MAX);
        left_index.cmp(&right_index)
    })
}

fn normalize_custom_routing_rule(
    rule: CodexLocalAccessCustomRoutingRule,
) -> Option<CodexLocalAccessCustomRoutingRule> {
    let account_id = rule.account_id.trim().to_string();
    if account_id.is_empty() {
        return None;
    }

    Some(CodexLocalAccessCustomRoutingRule {
        account_id,
        priority: rule
            .priority
            .clamp(CUSTOM_ROUTING_PRIORITY_MIN, CUSTOM_ROUTING_PRIORITY_MAX),
        weight: rule
            .weight
            .clamp(CUSTOM_ROUTING_WEIGHT_MIN, CUSTOM_ROUTING_WEIGHT_MAX),
        is_backup: rule.is_backup && !rule.is_preferred,
        is_preferred: rule.is_preferred,
    })
}

fn normalize_custom_routing_rules(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
    account_ids: &[String],
) -> Vec<CodexLocalAccessCustomRoutingRule> {
    let valid_account_ids: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for rule in rules {
        let Some(rule) = normalize_custom_routing_rule(rule) else {
            continue;
        };
        if !valid_account_ids.contains(rule.account_id.as_str()) {
            continue;
        }
        if seen.insert(rule.account_id.clone()) {
            normalized.push(rule);
        }
    }

    normalized
}

fn normalize_account_model_rule(
    rule: CodexLocalAccessAccountModelRule,
) -> Option<CodexLocalAccessAccountModelRule> {
    let account_id = rule.account_id.trim().to_string();
    if account_id.is_empty() {
        return None;
    }
    let excluded_models = normalize_model_rule_list(rule.excluded_models);
    if excluded_models.is_empty() {
        return None;
    }
    Some(CodexLocalAccessAccountModelRule {
        account_id,
        excluded_models,
    })
}

fn normalize_account_model_rules(
    rules: Vec<CodexLocalAccessAccountModelRule>,
    account_ids: &[String],
) -> Vec<CodexLocalAccessAccountModelRule> {
    let valid_account_ids: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let mut merged: HashMap<String, Vec<String>> = HashMap::new();

    for rule in rules {
        let Some(rule) = normalize_account_model_rule(rule) else {
            continue;
        };
        if !valid_account_ids.contains(rule.account_id.as_str()) {
            continue;
        }
        merged
            .entry(rule.account_id)
            .or_default()
            .extend(rule.excluded_models);
    }

    let mut normalized = Vec::new();
    for account_id in account_ids {
        let Some(excluded_models) = merged.remove(account_id) else {
            continue;
        };
        let excluded_models = normalize_model_rule_list(excluded_models);
        if excluded_models.is_empty() {
            continue;
        }
        normalized.push(CodexLocalAccessAccountModelRule {
            account_id: account_id.clone(),
            excluded_models,
        });
    }
    normalized
}

fn account_excluded_models<'a>(
    collection: &'a CodexLocalAccessCollection,
    account_id: &str,
) -> Option<&'a [String]> {
    collection
        .account_model_rules
        .iter()
        .find(|rule| rule.account_id == account_id)
        .map(|rule| rule.excluded_models.as_slice())
}

fn account_model_rule_blocks_model(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
    model_key: &str,
) -> bool {
    let model_key = model_key.trim();
    if model_key.is_empty() {
        return false;
    }
    account_excluded_models(collection, account_id)
        .map(|rules| model_matches_any_rule(model_key, rules))
        .unwrap_or(false)
}

fn merge_collection_and_account_excluded_models(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
) -> Vec<String> {
    let mut rules = collection.excluded_models.clone();
    if let Some(account_rules) = account_excluded_models(collection, account_id) {
        rules.extend(account_rules.iter().cloned());
    }
    normalize_model_rule_list(rules)
}

fn normalize_quota_limit_name_to_model_pattern(limit_name: &str) -> Option<String> {
    let trimmed = limit_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase().replace(' ', "-"))
}

fn normalize_quota_entitlement_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', ' '], "-")
}

fn is_gpt_reserve_entitlement_name(value: &str) -> bool {
    let normalized = normalize_quota_entitlement_key(value);
    normalized == CODEX_GPT_RESERVE_MODEL_ID
        || normalized == "gptreserve"
        || normalized.starts_with("gpt-reserve-")
        || normalized.starts_with("gptreserve-")
}

fn additional_rate_limit_allowed(entry: &Value) -> Option<bool> {
    entry
        .get("rate_limit")
        .or_else(|| entry.get("rateLimit"))
        .and_then(|rate_limit| rate_limit.get("allowed"))
        .and_then(Value::as_bool)
        .or_else(|| entry.get("allowed").and_then(Value::as_bool))
}

fn additional_rate_limit_matches_gpt_reserve(entry: &Value, object_key: Option<&str>) -> bool {
    [
        object_key,
        entry.get("limit_name").and_then(Value::as_str),
        entry.get("limitName").and_then(Value::as_str),
        entry.get("name").and_then(Value::as_str),
        entry.get("metered_feature").and_then(Value::as_str),
        entry.get("meteredFeature").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .any(is_gpt_reserve_entitlement_name)
}

fn account_has_gpt_reserve_entitlement(account: &CodexAccount) -> bool {
    // Listing is independent of eligibility. Dispatch follows the official
    // quota-side predicate, without emulating desktop UI feature gates.
    if account.is_api_key_auth() {
        return false;
    }
    let Some(raw) = account
        .quota
        .as_ref()
        .and_then(|quota| quota.raw_data.as_ref())
    else {
        return false;
    };
    if raw.pointer("/rate_limit/allowed").and_then(Value::as_bool) != Some(false)
        || raw.pointer("/rate_limit_upsell/banner_type").and_then(Value::as_str)
            != Some("luna_reserve")
    {
        return false;
    }
    let Some(additional) = raw
        .get("additional_rate_limits")
        .or_else(|| raw.get("additionalRateLimits"))
    else {
        return false;
    };

    match additional {
        Value::Array(entries) => entries.iter().any(|entry| {
            additional_rate_limit_matches_gpt_reserve(entry, None)
                && additional_rate_limit_allowed(entry) == Some(true)
        }),
        Value::Object(entries) => entries.iter().any(|(name, entry)| {
            additional_rate_limit_matches_gpt_reserve(entry, Some(name))
                && additional_rate_limit_allowed(entry) == Some(true)
        }),
        _ => false,
    }
}

fn metered_features_in_quota_raw(raw: &Value) -> HashSet<String> {
    let mut features = HashSet::new();
    let Some(limits) = raw.get("additional_rate_limits").and_then(Value::as_array) else {
        return features;
    };
    for entry in limits {
        let Some(feature) = entry
            .get("metered_feature")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        features.insert(feature.to_ascii_lowercase());
    }
    features
}

fn quota_disallowed_model_patterns(account: &CodexAccount) -> Vec<String> {
    let Some(raw) = account
        .quota
        .as_ref()
        .and_then(|quota| quota.raw_data.as_ref())
    else {
        return Vec::new();
    };
    let Some(limits) = raw.get("additional_rate_limits").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut patterns = Vec::new();
    for entry in limits {
        let allowed = entry
            .get("rate_limit")
            .and_then(|value| value.get("allowed"))
            .and_then(Value::as_bool);
        if allowed != Some(false) {
            continue;
        }
        let Some(limit_name) = entry.get("limit_name").and_then(Value::as_str) else {
            continue;
        };
        if let Some(pattern) = normalize_quota_limit_name_to_model_pattern(limit_name) {
            patterns.push(pattern);
        }
    }
    patterns
}

fn metered_feature_model_patterns_for_pool(
    collection: &CodexLocalAccessCollection,
    account_overrides: &HashMap<String, CodexAccount>,
) -> HashMap<String, String> {
    let persisted_accounts = codex_account::list_accounts_checked().ok();
    let mut patterns = HashMap::new();
    for account_id in effective_sidecar_account_ids(collection) {
        let account = account_overrides.get(&account_id).or_else(|| {
            persisted_accounts
                .as_ref()
                .and_then(|accounts| accounts.iter().find(|account| account.id == account_id))
        });
        let Some(raw) = account
            .and_then(|account| account.quota.as_ref())
            .and_then(|quota| quota.raw_data.as_ref())
        else {
            continue;
        };
        let Some(limits) = raw.get("additional_rate_limits").and_then(Value::as_array) else {
            continue;
        };
        for entry in limits {
            let feature = entry
                .get("metered_feature")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase);
            let limit_name = entry.get("limit_name").and_then(Value::as_str);
            if let (Some(feature), Some(limit_name)) = (feature, limit_name) {
                if let Some(pattern) = normalize_quota_limit_name_to_model_pattern(limit_name) {
                    patterns.entry(feature).or_insert(pattern);
                }
            }
        }
    }
    patterns
}

fn implicit_metered_feature_exclusions(
    account: &CodexAccount,
    feature_patterns: &HashMap<String, String>,
) -> Vec<String> {
    if feature_patterns.is_empty() {
        return Vec::new();
    }
    let Some(raw) = account
        .quota
        .as_ref()
        .and_then(|quota| quota.raw_data.as_ref())
    else {
        return Vec::new();
    };
    let present = metered_features_in_quota_raw(raw);
    feature_patterns
        .iter()
        .filter_map(|(feature, pattern)| {
            if present.contains(feature) {
                None
            } else {
                Some(pattern.clone())
            }
        })
        .collect()
}

fn sidecar_excluded_models_for_account(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    metered_feature_patterns: &HashMap<String, String>,
) -> Vec<String> {
    let mut excluded = merge_collection_and_account_excluded_models(collection, &account.id);
    excluded.extend(quota_disallowed_model_patterns(account));
    excluded.extend(implicit_metered_feature_exclusions(
        account,
        metered_feature_patterns,
    ));
    if !account_has_gpt_reserve_entitlement(account) {
        excluded.push(CODEX_GPT_RESERVE_MODEL_ID.to_string());
    }
    if !account.api_model_mappings.is_empty() {
        let mapped = account_api_model_mapping_ids(account);
        excluded.extend(
            supported_codex_model_ids()
                .into_iter()
                .filter(|model| !mapped.contains(&model.to_ascii_lowercase())),
        );
    }
    normalize_model_rule_list(excluded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AccountUsagePriority {
    Lowest,
    Normal,
    Highest,
}

fn account_usage_priority(
    rule: Option<&CodexLocalAccessCustomRoutingRule>,
) -> AccountUsagePriority {
    match rule {
        Some(rule) if rule.is_preferred => AccountUsagePriority::Highest,
        Some(rule) if rule.is_backup => AccountUsagePriority::Lowest,
        _ => AccountUsagePriority::Normal,
    }
}

fn custom_rule_map(
    rules: &[CodexLocalAccessCustomRoutingRule],
) -> HashMap<&str, (i32, u32, AccountUsagePriority)> {
    rules
        .iter()
        .map(|rule| {
            (
                rule.account_id.as_str(),
                (
                    rule.priority
                        .clamp(CUSTOM_ROUTING_PRIORITY_MIN, CUSTOM_ROUTING_PRIORITY_MAX),
                    rule.weight
                        .clamp(CUSTOM_ROUTING_WEIGHT_MIN, CUSTOM_ROUTING_WEIGHT_MAX),
                    account_usage_priority(Some(rule)),
                ),
            )
        })
        .collect()
}

fn weighted_group_order(
    group: &[String],
    weights: &HashMap<&str, (i32, u32, AccountUsagePriority)>,
    start: usize,
) -> Vec<String> {
    if group.len() <= 1 {
        return group.to_vec();
    }

    let total_weight = group.iter().fold(0usize, |sum, account_id| {
        let weight = weights
            .get(account_id.as_str())
            .map(|(_, weight, _)| *weight)
            .unwrap_or(CUSTOM_ROUTING_WEIGHT_MIN) as usize;
        sum.saturating_add(weight.max(1))
    });
    if total_weight == 0 {
        return group.to_vec();
    }

    let mut slot = start % total_weight;
    let mut first_index = 0usize;
    for (index, account_id) in group.iter().enumerate() {
        let weight = weights
            .get(account_id.as_str())
            .map(|(_, weight, _)| *weight)
            .unwrap_or(CUSTOM_ROUTING_WEIGHT_MIN) as usize;
        if slot < weight {
            first_index = index;
            break;
        }
        slot -= weight;
    }

    (0..group.len())
        .map(|offset| group[(first_index + offset) % group.len()].clone())
        .collect()
}

fn apply_custom_routing_strategy(
    account_ids: &[String],
    rules: &[CodexLocalAccessCustomRoutingRule],
    start: usize,
) -> Vec<String> {
    let rule_map = custom_rule_map(rules);
    let mut priority_groups: Vec<(AccountUsagePriority, i32, Vec<String>)> = Vec::new();

    for account_id in account_ids {
        let (priority, usage_priority) = rule_map
            .get(account_id.as_str())
            .map(|(priority, _, usage_priority)| (*priority, *usage_priority))
            .unwrap_or((CUSTOM_ROUTING_PRIORITY_MIN, AccountUsagePriority::Normal));
        if let Some((_, _, group)) =
            priority_groups
                .iter_mut()
                .find(|(group_usage_priority, group_priority, _)| {
                    *group_usage_priority == usage_priority && *group_priority == priority
                })
        {
            group.push(account_id.clone());
        } else {
            priority_groups.push((usage_priority, priority, vec![account_id.clone()]));
        }
    }

    priority_groups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    let mut ordered = Vec::with_capacity(account_ids.len());
    for (_, _, group) in priority_groups {
        ordered.extend(weighted_group_order(&group, &rule_map, start));
    }
    ordered
}

fn apply_account_usage_priority(
    account_ids: Vec<String>,
    rules: &[CodexLocalAccessCustomRoutingRule],
) -> Vec<String> {
    let rules_by_account_id = rules
        .iter()
        .map(|rule| (rule.account_id.as_str(), rule))
        .collect::<HashMap<_, _>>();
    let mut highest = Vec::new();
    let mut normal = Vec::new();
    let mut lowest = Vec::new();
    for account_id in account_ids {
        match account_usage_priority(rules_by_account_id.get(account_id.as_str()).copied()) {
            AccountUsagePriority::Highest => highest.push(account_id),
            AccountUsagePriority::Normal => normal.push(account_id),
            AccountUsagePriority::Lowest => lowest.push(account_id),
        }
    }
    highest.extend(normal);
    highest.extend(lowest);
    highest
}

fn apply_routing_strategy(
    account_ids: &[String],
    strategy: CodexLocalAccessRoutingStrategy,
    custom_rules: &[CodexLocalAccessCustomRoutingRule],
    start: usize,
) -> Vec<String> {
    if strategy == CodexLocalAccessRoutingStrategy::Random {
        let mut shuffled = account_ids.to_vec();
        shuffled.shuffle(&mut rand::thread_rng());
        return apply_account_usage_priority(shuffled, custom_rules);
    }

    if strategy == CodexLocalAccessRoutingStrategy::SingleAccount {
        return apply_account_usage_priority(account_ids.to_vec(), custom_rules);
    }

    if strategy == CodexLocalAccessRoutingStrategy::Custom {
        return apply_custom_routing_strategy(account_ids, custom_rules, start);
    }

    let original_index: HashMap<String, usize> = account_ids
        .iter()
        .enumerate()
        .map(|(index, account_id)| (account_id.clone(), index))
        .collect();
    let mut candidates = build_routing_candidates(account_ids);
    candidates
        .sort_by(|left, right| compare_routing_candidates(left, right, strategy, &original_index));
    let ordered = candidates
        .into_iter()
        .map(|candidate| candidate.account_id)
        .collect();
    apply_account_usage_priority(ordered, custom_rules)
}

fn effective_routing_strategy(
    collection: &CodexLocalAccessCollection,
    scoped_account_ids: &[String],
) -> CodexLocalAccessRoutingStrategy {
    if scoped_account_ids == collection.account_ids {
        collection.routing_strategy
    } else {
        CodexLocalAccessRoutingStrategy::Auto
    }
}

fn max_credential_attempts_for_strategy(
    collection: &CodexLocalAccessCollection,
    total: usize,
    strategy: CodexLocalAccessRoutingStrategy,
) -> usize {
    if strategy == CodexLocalAccessRoutingStrategy::SingleAccount {
        return 1;
    }

    let configured_max_credentials = collection.max_retry_credentials as usize;
    if configured_max_credentials == 0 {
        total
    } else {
        configured_max_credentials.min(total)
    }
    .min(MAX_RETRY_CREDENTIALS_PER_REQUEST)
    .max(1)
}

fn prioritize_account_ids(
    account_ids: Vec<String>,
    priority_account_ids: &[String],
) -> Vec<String> {
    let mut ordered = Vec::with_capacity(account_ids.len());
    for priority_account_id in priority_account_ids {
        if priority_account_id.trim().is_empty()
            || !account_ids
                .iter()
                .any(|account_id| account_id == priority_account_id)
            || ordered
                .iter()
                .any(|account_id| account_id == priority_account_id)
        {
            continue;
        }
        ordered.push(priority_account_id.clone());
    }
    for account_id in account_ids {
        if ordered.iter().any(|value| value == &account_id) {
            continue;
        }
        ordered.push(account_id);
    }
    ordered
}

fn pin_account_to_front_for_strategy(
    account_ids: Vec<String>,
    priority_account_ids: &[String],
    _strategy: CodexLocalAccessRoutingStrategy,
    custom_rules: &[CodexLocalAccessCustomRoutingRule],
) -> Vec<String> {
    let rules_by_account_id = custom_rules
        .iter()
        .map(|rule| (rule.account_id.as_str(), rule))
        .collect::<HashMap<_, _>>();
    let mut highest = Vec::new();
    let mut normal = Vec::with_capacity(account_ids.len());
    let mut lowest = Vec::new();
    for account_id in account_ids {
        match account_usage_priority(rules_by_account_id.get(account_id.as_str()).copied()) {
            AccountUsagePriority::Highest => highest.push(account_id),
            AccountUsagePriority::Normal => normal.push(account_id),
            AccountUsagePriority::Lowest => lowest.push(account_id),
        }
    }

    highest = prioritize_account_ids(highest, priority_account_ids);
    normal = prioritize_account_ids(normal, priority_account_ids);
    lowest = prioritize_account_ids(lowest, priority_account_ids);
    highest.extend(normal);
    highest.extend(lowest);
    highest
}

fn format_retry_after_duration(wait: Duration) -> String {
    let seconds = wait.as_secs().max(1);
    format!("{} 秒", seconds)
}

fn build_cooldown_unavailable_message(model_key: &str, wait: Duration) -> String {
    let wait_text = format_retry_after_duration(wait);
    if model_key.trim().is_empty() {
        format!("当前 API 服务账号均在冷却中，请 {} 后重试", wait_text)
    } else {
        format!(
            "模型 {} 的可用账号均在冷却中，请 {} 后重试",
            model_key, wait_text,
        )
    }
}

fn parse_codex_retry_after(status: StatusCode, error_body: &str) -> Option<Duration> {
    if status != StatusCode::TOO_MANY_REQUESTS || error_body.trim().is_empty() {
        return None;
    }

    let payload = serde_json::from_str::<Value>(error_body).ok()?;
    let error = payload.get("error")?;
    if error.get("type").and_then(Value::as_str).map(str::trim) != Some("usage_limit_reached") {
        return None;
    }

    let now_seconds = chrono::Utc::now().timestamp();
    if let Some(resets_at) = error.get("resets_at").and_then(Value::as_i64) {
        if resets_at > now_seconds {
            let delta = resets_at.saturating_sub(now_seconds) as u64;
            if delta > 0 {
                return Some(Duration::from_secs(delta));
            }
        }
    }

    error
        .get("resets_in_seconds")
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds as u64))
}

fn empty_stats_snapshot() -> CodexLocalAccessStats {
    let now = now_ms();
    let window_starts = calendar_stats_window_starts(now);
    CodexLocalAccessStats {
        since: now,
        updated_at: now,
        totals: CodexLocalAccessUsageStats::default(),
        accounts: Vec::new(),
        models: Vec::new(),
        api_keys: Vec::new(),
        daily: CodexLocalAccessStatsWindow {
            since: window_starts.day,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
            trend: Vec::new(),
            trend_hourly: false,
        },
        weekly: CodexLocalAccessStatsWindow {
            since: window_starts.week,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
            trend: Vec::new(),
            trend_hourly: false,
        },
        monthly: CodexLocalAccessStatsWindow {
            since: window_starts.month,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
            trend: Vec::new(),
            trend_hourly: false,
        },
        events: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatsWindowStarts {
    day: i64,
    week: i64,
    month: i64,
}

fn local_date_start_ms(date: NaiveDate) -> i64 {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .expect("a calendar date must have a midnight");
    match Local.from_local_datetime(&midnight) {
        LocalResult::Single(value) => value.timestamp_millis(),
        LocalResult::Ambiguous(first, second) => {
            first.timestamp_millis().min(second.timestamp_millis())
        }
        LocalResult::None => (1..=24 * 60 * 60)
            .find_map(|seconds| {
                let candidate = midnight.checked_add_signed(ChronoDuration::seconds(seconds))?;
                Local
                    .from_local_datetime(&candidate)
                    .earliest()
                    .map(|value| value.timestamp_millis())
            })
            .unwrap_or_else(|| Local.from_utc_datetime(&midnight).timestamp_millis()),
    }
}

fn calendar_stats_window_starts(now_ms: i64) -> StatsWindowStarts {
    let local_now = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms)
        .unwrap_or_else(chrono::Utc::now)
        .with_timezone(&Local);
    let day = local_now.date_naive();
    let week = day
        .checked_sub_signed(ChronoDuration::days(
            day.weekday().num_days_from_monday() as i64
        ))
        .unwrap_or(day);
    let month = NaiveDate::from_ymd_opt(day.year(), day.month(), 1).unwrap_or(day);
    StatsWindowStarts {
        day: local_date_start_ms(day),
        week: local_date_start_ms(week),
        month: local_date_start_ms(month),
    }
}

fn empty_stats_window(since: i64, updated_at: i64) -> CodexLocalAccessStatsWindow {
    CodexLocalAccessStatsWindow {
        since,
        updated_at,
        totals: CodexLocalAccessUsageStats::default(),
        accounts: Vec::new(),
        models: Vec::new(),
        api_keys: Vec::new(),
        trend: Vec::new(),
        trend_hourly: false,
    }
}

fn sort_usage_accounts(accounts: &mut [CodexLocalAccessAccountStats]) {
    accounts.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.account_id.cmp(&right.account_id))
    });
}

fn sort_usage_models(models: &mut [CodexLocalAccessModelStats]) {
    models.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
}

fn sort_usage_api_keys(api_keys: &mut [CodexLocalAccessApiKeyStats]) {
    api_keys.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.api_key_id.cmp(&right.api_key_id))
    });
}

fn merge_missing_usage_accounts(
    runtime: &mut Vec<CodexLocalAccessAccountStats>,
    maintained: &[CodexLocalAccessAccountStats],
) {
    for item in maintained {
        if !runtime
            .iter()
            .any(|existing| existing.account_id == item.account_id)
        {
            runtime.push(item.clone());
        }
    }
}

fn merge_missing_usage_models(
    runtime: &mut Vec<CodexLocalAccessModelStats>,
    maintained: &[CodexLocalAccessModelStats],
) {
    for item in maintained {
        if !runtime
            .iter()
            .any(|existing| existing.model_id == item.model_id)
        {
            runtime.push(item.clone());
        }
    }
}

fn merge_missing_usage_api_keys(
    runtime: &mut Vec<CodexLocalAccessApiKeyStats>,
    maintained: &[CodexLocalAccessApiKeyStats],
) {
    for item in maintained {
        if !runtime
            .iter()
            .any(|existing| existing.api_key_id == item.api_key_id)
        {
            runtime.push(item.clone());
        }
    }
}

fn model_pricing(
    model_id: &str,
    long_context_threshold_tokens: Option<u64>,
    standard: CodexLocalAccessPrice,
    standard_long: Option<CodexLocalAccessPrice>,
    priority: Option<CodexLocalAccessPrice>,
    priority_long: Option<CodexLocalAccessPrice>,
) -> CodexLocalAccessModelPricing {
    CodexLocalAccessModelPricing {
        model_id: model_id.to_string(),
        long_context_threshold_tokens,
        input_usd_per_million: standard.input_usd_per_million,
        output_usd_per_million: standard.output_usd_per_million,
        cached_input_usd_per_million: Some(standard.cached_input_usd_per_million),
        standard_long_input_usd_per_million: standard_long.map(|price| price.input_usd_per_million),
        standard_long_output_usd_per_million: standard_long
            .map(|price| price.output_usd_per_million),
        standard_long_cached_input_usd_per_million: standard_long
            .map(|price| price.cached_input_usd_per_million),
        priority_input_usd_per_million: priority.map(|price| price.input_usd_per_million),
        priority_output_usd_per_million: priority.map(|price| price.output_usd_per_million),
        priority_cached_input_usd_per_million: priority
            .map(|price| price.cached_input_usd_per_million),
        priority_long_input_usd_per_million: priority_long.map(|price| price.input_usd_per_million),
        priority_long_output_usd_per_million: priority_long
            .map(|price| price.output_usd_per_million),
        priority_long_cached_input_usd_per_million: priority_long
            .map(|price| price.cached_input_usd_per_million),
    }
}

/// Billing service_tier for cost estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexBillingServiceTier {
    Standard,
    Priority,
    Flex,
}

#[derive(Debug, Clone, Copy)]
struct CodexLocalAccessPrice {
    input_usd_per_million: f64,
    cached_input_usd_per_million: f64,
    output_usd_per_million: f64,
}

#[derive(Debug, Clone, Copy)]
struct CodexLocalAccessPriceBookEntry {
    model_id: &'static str,
    /// When true, apply session long-context multipliers above the threshold.
    session_long_context: bool,
    standard: CodexLocalAccessPrice,
    priority: Option<CodexLocalAccessPrice>,
}

#[derive(Debug, Clone)]
struct RequestLogRepriceChange {
    event_key: String,
    timestamp: i64,
    account_id: String,
    api_key_id: String,
    model_id: String,
    estimated_cost_delta_usd: f64,
}

const fn codex_price(
    input_usd_per_million: f64,
    cached_input_usd_per_million: f64,
    output_usd_per_million: f64,
) -> CodexLocalAccessPrice {
    CodexLocalAccessPrice {
        input_usd_per_million,
        cached_input_usd_per_million,
        output_usd_per_million,
    }
}

/// Default Codex/OpenAI price book for local cost estimation (USD / 1M tokens).
/// Bump `DEFAULT_MODEL_PRICING_VERSION` when defaults change so saved overrides
/// reseal and historical estimates reprice.
const CODEX_LOCAL_ACCESS_PRICE_BOOK: &[CodexLocalAccessPriceBookEntry] = &[
    // Keep in sync with supported Codex models and public OpenAI rates.
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-6-astra",
        session_long_context: true,
        standard: codex_price(10.0, 1.0, 50.0),
        priority: Some(codex_price(20.0, 2.0, 100.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.6-sol",
        session_long_context: true,
        standard: codex_price(5.0, 0.5, 30.0),
        priority: Some(codex_price(10.0, 1.0, 60.0)),
    },
    CodexLocalAccessPriceBookEntry {
        // Bare gpt-5.6 uses sol-tier rates.
        model_id: "gpt-5.6",
        session_long_context: true,
        standard: codex_price(5.0, 0.5, 30.0),
        priority: Some(codex_price(10.0, 1.0, 60.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.6-terra",
        session_long_context: true,
        standard: codex_price(2.0, 0.2, 12.0),
        priority: Some(codex_price(4.0, 0.4, 24.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.6-luna",
        session_long_context: true,
        standard: codex_price(0.2, 0.02, 1.2),
        priority: Some(codex_price(0.4, 0.04, 2.4)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.5",
        session_long_context: true,
        standard: codex_price(5.0, 0.5, 30.0),
        priority: Some(codex_price(10.0, 1.0, 60.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "codex-auto-review",
        session_long_context: true,
        standard: codex_price(5.0, 0.5, 30.0),
        priority: Some(codex_price(10.0, 1.0, 60.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4",
        session_long_context: true,
        standard: codex_price(2.5, 0.25, 15.0),
        priority: Some(codex_price(5.0, 0.5, 30.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4-mini",
        session_long_context: false,
        standard: codex_price(0.75, 0.075, 4.5),
        priority: Some(codex_price(1.5, 0.15, 9.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4-nano",
        session_long_context: false,
        standard: codex_price(0.2, 0.02, 1.25),
        priority: None,
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.3-codex",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.3-codex-spark",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.2",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.2-codex",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        priority: Some(codex_price(2.5, 0.25, 20.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex-max",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        // No explicit priority rates -> fall back to x2 at billing time.
        priority: None,
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex-mini",
        session_long_context: false,
        standard: codex_price(0.25, 0.025, 2.0),
        priority: Some(codex_price(0.45, 0.045, 3.6)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5-codex",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        priority: None,
    },
];

fn derived_standard_long_price(standard: CodexLocalAccessPrice) -> CodexLocalAccessPrice {
    // OpenAI above-272k: input x2, cache x2, output x1.5.
    codex_price(
        standard.input_usd_per_million * CODEX_LOCAL_ACCESS_LONG_CONTEXT_INPUT_MULTIPLIER,
        standard.cached_input_usd_per_million * CODEX_LOCAL_ACCESS_LONG_CONTEXT_CACHE_MULTIPLIER,
        standard.output_usd_per_million * CODEX_LOCAL_ACCESS_LONG_CONTEXT_OUTPUT_MULTIPLIER,
    )
}

fn price_book_entry_to_model_pricing(
    entry: &CodexLocalAccessPriceBookEntry,
) -> CodexLocalAccessModelPricing {
    let long_threshold = if entry.session_long_context {
        Some(CODEX_LOCAL_ACCESS_LONG_CONTEXT_THRESHOLD_TOKENS)
    } else {
        None
    };
    let standard_long = if entry.session_long_context {
        Some(derived_standard_long_price(entry.standard))
    } else {
        None
    };
    model_pricing(
        entry.model_id,
        long_threshold,
        entry.standard,
        standard_long,
        entry.priority,
        None,
    )
}

/// Canonicalize OpenAI/Codex model id aliases for pricing lookup.
fn canonicalize_openai_model_alias_spelling(model: &str) -> String {
    let mut normalized = model.trim().to_ascii_lowercase();
    if let Some((_, tail)) = normalized.rsplit_once('/') {
        normalized = tail.trim().to_string();
    } else {
        normalized = normalized.trim().to_string();
    }
    if normalized.is_empty() {
        return String::new();
    }
    normalized = normalized.replace('_', "-");
    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }
    if let Some(rest) = normalized.strip_prefix("gpt5") {
        normalized = format!("gpt-5{rest}");
    }
    if !normalized.starts_with("gpt-") && !normalized.contains("codex") {
        return String::new();
    }
    let replacements = [
        ("gpt-5.4mini", "gpt-5.4-mini"),
        ("gpt-5.4nano", "gpt-5.4-nano"),
        ("gpt-5.3-codexspark", "gpt-5.3-codex-spark"),
        ("gpt-5.3codexspark", "gpt-5.3-codex-spark"),
        ("gpt-5.3codex", "gpt-5.3-codex"),
    ];
    for (from, to) in replacements {
        normalized = normalized.replace(from, to);
    }
    normalized
}

fn normalize_known_openai_codex_model(model: &str) -> Option<String> {
    let mut normalized = canonicalize_openai_model_alias_spelling(model);
    if normalized.is_empty() {
        return None;
    }
    if let Some(stripped) = normalized.strip_suffix("-openai-compact") {
        if let Some(mapped) = normalize_known_openai_codex_model(stripped) {
            return Some(mapped);
        }
    }
    // Drop dated snapshot suffixes like -2026-03-05 for family matching.
    // Dated snapshot suffixes like -YYYYMMDD / -YYYY-MM-DD are stripped below.
    if normalized.len() > 11 {
        let tail = &normalized[normalized.len() - 11..];
        if tail.as_bytes().first() == Some(&b'-')
            && tail.as_bytes().get(5) == Some(&b'-')
            && tail.as_bytes().get(8) == Some(&b'-')
            && tail
                .bytes()
                .enumerate()
                .all(|(i, b)| matches!(i, 0 | 5 | 8) || b.is_ascii_digit())
        {
            normalized = normalized[..normalized.len() - 11].to_string();
        }
    }

    if normalized.contains("gpt-6-astra") {
        return Some("gpt-6-astra".to_string());
    }
    if normalized.contains("gpt-5.6-sol") {
        return Some("gpt-5.6-sol".to_string());
    }
    if normalized.contains("gpt-5.6-terra") {
        return Some("gpt-5.6-terra".to_string());
    }
    if normalized.contains("gpt-5.6-luna") {
        return Some("gpt-5.6-luna".to_string());
    }
    if normalized.contains("gpt-5.6") {
        // Bare gpt-5.6 uses sol-tier rates.
        return Some("gpt-5.6".to_string());
    }
    if normalized.contains("gpt-5.5") {
        return Some("gpt-5.5".to_string());
    }
    if normalized.contains("gpt-5.4-mini") {
        return Some("gpt-5.4-mini".to_string());
    }
    if normalized.contains("gpt-5.4-nano") {
        return Some("gpt-5.4-nano".to_string());
    }
    if normalized.contains("gpt-5.4") {
        return Some("gpt-5.4".to_string());
    }
    if normalized.contains("gpt-5.2") {
        // Prefer codex variant when present.
        if normalized.contains("codex") {
            return Some("gpt-5.2-codex".to_string());
        }
        return Some("gpt-5.2".to_string());
    }
    if normalized.contains("gpt-5.3-codex-spark") {
        return Some("gpt-5.3-codex-spark".to_string());
    }
    if normalized.contains("gpt-5.3-codex") || normalized.contains("gpt-5.3") {
        return Some("gpt-5.3-codex".to_string());
    }
    if normalized.contains("gpt-5.1-codex-mini") {
        return Some("gpt-5.1-codex-mini".to_string());
    }
    if normalized.contains("gpt-5.1-codex-max") {
        return Some("gpt-5.1-codex-max".to_string());
    }
    if normalized.contains("gpt-5.1-codex") {
        return Some("gpt-5.1-codex".to_string());
    }
    if normalized.contains("gpt-5-codex") || normalized == "gpt-5-codex" {
        return Some("gpt-5-codex".to_string());
    }
    if normalized.contains("codex") {
        return Some("gpt-5.3-codex".to_string());
    }
    if normalized.contains("gpt-5") {
        return Some("gpt-5.4".to_string());
    }
    None
}

fn price_book_entry_for_model(model_id: &str) -> Option<&'static CodexLocalAccessPriceBookEntry> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(entry) = CODEX_LOCAL_ACCESS_PRICE_BOOK
        .iter()
        .find(|item| item.model_id.eq_ignore_ascii_case(trimmed))
    {
        return Some(entry);
    }
    let normalized = normalize_known_openai_codex_model(trimmed)?;
    CODEX_LOCAL_ACCESS_PRICE_BOOK
        .iter()
        .find(|item| item.model_id == normalized.as_str())
}

fn parse_billing_service_tier(service_tier: Option<&str>) -> CodexBillingServiceTier {
    match service_tier.and_then(normalize_proxy_service_tier) {
        Some("priority") => CodexBillingServiceTier::Priority,
        Some("flex") => CodexBillingServiceTier::Flex,
        _ => CodexBillingServiceTier::Standard,
    }
}

fn is_openai_session_long_context_model(model_id: &str) -> bool {
    let normalized = normalize_known_openai_codex_model(model_id)
        .unwrap_or_else(|| model_id.trim().to_ascii_lowercase());
    matches!(
        normalized.as_str(),
        "gpt-5.4"
            | "gpt-5.5"
            | "gpt-6-astra"
            | "gpt-5.6"
            | "gpt-5.6-sol"
            | "gpt-5.6-terra"
            | "gpt-5.6-luna"
            | "codex-auto-review"
    ) || normalized.starts_with("gpt-5.6-")
}

fn should_apply_session_long_context(
    model_id: &str,
    pricing: &CodexLocalAccessModelPricing,
    usage: Option<&UsageCapture>,
) -> bool {
    let threshold = pricing
        .long_context_threshold_tokens
        .filter(|value| *value > 0)
        .or_else(|| {
            is_openai_session_long_context_model(model_id)
                .then_some(CODEX_LOCAL_ACCESS_LONG_CONTEXT_THRESHOLD_TOKENS)
        });
    let Some(threshold) = threshold else {
        return false;
    };
    // OpenAI total prompt tokens include cached tokens.
    usage.map(|item| item.input_tokens).unwrap_or(0) > threshold
}

fn pricing_has_explicit_priority_rates(pricing: &CodexLocalAccessModelPricing) -> bool {
    pricing
        .priority_input_usd_per_million
        .is_some_and(|value| value > 0.0)
        || pricing
            .priority_output_usd_per_million
            .is_some_and(|value| value > 0.0)
        || pricing
            .priority_cached_input_usd_per_million
            .is_some_and(|value| value > 0.0)
}

/// Resolve unit prices after service_tier + long-context policy
/// (absolute priority rates, else x2/x0.5; long multiplies input/cache/output).
fn compute_effective_unit_prices(
    pricing: &CodexLocalAccessModelPricing,
    model_id: &str,
    usage: Option<&UsageCapture>,
    service_tier: Option<&str>,
) -> CodexLocalAccessPrice {
    let mut input_price = pricing.input_usd_per_million;
    let mut output_price = pricing.output_usd_per_million;
    let mut cache_price = pricing
        .cached_input_usd_per_million
        .unwrap_or(pricing.input_usd_per_million);
    let mut tier_multiplier = 1.0_f64;

    match parse_billing_service_tier(service_tier) {
        CodexBillingServiceTier::Priority if pricing_has_explicit_priority_rates(pricing) => {
            if let Some(value) = pricing
                .priority_input_usd_per_million
                .filter(|value| *value > 0.0)
            {
                input_price = value;
            }
            if let Some(value) = pricing
                .priority_output_usd_per_million
                .filter(|value| *value > 0.0)
            {
                output_price = value;
            }
            if let Some(value) = pricing
                .priority_cached_input_usd_per_million
                .filter(|value| *value > 0.0)
            {
                cache_price = value;
            }
        }
        CodexBillingServiceTier::Priority => {
            tier_multiplier = 2.0;
        }
        CodexBillingServiceTier::Flex => {
            tier_multiplier = 0.5;
        }
        CodexBillingServiceTier::Standard => {}
    }

    if should_apply_session_long_context(model_id, pricing, usage) {
        input_price *= CODEX_LOCAL_ACCESS_LONG_CONTEXT_INPUT_MULTIPLIER;
        cache_price *= CODEX_LOCAL_ACCESS_LONG_CONTEXT_CACHE_MULTIPLIER;
        output_price *= CODEX_LOCAL_ACCESS_LONG_CONTEXT_OUTPUT_MULTIPLIER;
    }

    CodexLocalAccessPrice {
        input_usd_per_million: input_price * tier_multiplier,
        cached_input_usd_per_million: cache_price * tier_multiplier,
        output_usd_per_million: output_price * tier_multiplier,
    }
}

fn default_model_pricing_presets() -> Vec<CodexLocalAccessModelPricing> {
    CODEX_LOCAL_ACCESS_PRICE_BOOK
        .iter()
        .map(price_book_entry_to_model_pricing)
        .collect()
}

fn effective_price_book_model_ids(collection: Option<&CodexLocalAccessCollection>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut model_ids = Vec::new();
    for preset in default_model_pricing_presets() {
        let key = normalize_model_key(&preset.model_id);
        if seen.insert(key) {
            model_ids.push(preset.model_id);
        }
    }
    if let Some(collection) = collection {
        for pricing in &collection.model_pricings {
            let model_id = pricing.model_id.trim();
            if model_id.is_empty() {
                continue;
            }
            let key = normalize_model_key(model_id);
            if seen.insert(key) {
                model_ids.push(model_id.to_string());
            }
        }
    }
    model_ids
}

fn normalize_price_value(value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 {
        0.0
    } else {
        value.min(MAX_MODEL_PRICE_USD_PER_MILLION)
    }
}

fn normalize_positive_tokens(value: Option<u64>) -> Option<u64> {
    value.filter(|item| *item > 0)
}

fn normalize_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Vec<CodexLocalAccessModelPricing> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for pricing in model_pricings {
        let model_id = pricing.model_id.trim().to_string();
        if model_id.is_empty() {
            continue;
        }
        let key = model_id.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let preset = price_book_entry_for_model(&model_id);
        let has_custom_rates = pricing.long_context_threshold_tokens.is_some()
            || pricing.priority_input_usd_per_million.is_some()
            || pricing.priority_cached_input_usd_per_million.is_some()
            || pricing.priority_output_usd_per_million.is_some()
            || (pricing.input_usd_per_million > 0.0 || pricing.output_usd_per_million > 0.0);
        // Drop empty overrides that only restate defaults without custom fields.
        if preset.is_some() && !has_custom_rates && pricing.cached_input_usd_per_million.is_none() {
            continue;
        }

        let session_long = is_openai_session_long_context_model(&model_id)
            || preset
                .map(|item| item.session_long_context)
                .unwrap_or(false);
        let long_context_threshold_tokens = if session_long {
            Some(
                normalize_positive_tokens(pricing.long_context_threshold_tokens)
                    .or_else(|| {
                        preset.and_then(|item| {
                            item.session_long_context
                                .then_some(CODEX_LOCAL_ACCESS_LONG_CONTEXT_THRESHOLD_TOKENS)
                        })
                    })
                    .unwrap_or(CODEX_LOCAL_ACCESS_LONG_CONTEXT_THRESHOLD_TOKENS),
            )
        } else {
            None
        };

        let standard = price_from_base_triple(
            normalize_price_value(pricing.input_usd_per_million),
            pricing
                .cached_input_usd_per_million
                .map(normalize_price_value),
            normalize_price_value(pricing.output_usd_per_million),
        );
        // standard_long absolute fields are display-only / legacy; billing uses
        // multipliers. Persist derived display values for session-long models.
        let standard_long = session_long.then(|| derived_standard_long_price(standard));

        normalized.push(CodexLocalAccessModelPricing {
            model_id,
            long_context_threshold_tokens,
            input_usd_per_million: standard.input_usd_per_million,
            output_usd_per_million: standard.output_usd_per_million,
            cached_input_usd_per_million: Some(standard.cached_input_usd_per_million),
            standard_long_input_usd_per_million: standard_long
                .map(|price| price.input_usd_per_million),
            standard_long_output_usd_per_million: standard_long
                .map(|price| price.output_usd_per_million),
            standard_long_cached_input_usd_per_million: standard_long
                .map(|price| price.cached_input_usd_per_million),
            priority_input_usd_per_million: pricing
                .priority_input_usd_per_million
                .map(normalize_price_value),
            priority_output_usd_per_million: pricing
                .priority_output_usd_per_million
                .map(normalize_price_value),
            priority_cached_input_usd_per_million: pricing
                .priority_cached_input_usd_per_million
                .map(normalize_price_value),
            priority_long_input_usd_per_million: None,
            priority_long_output_usd_per_million: None,
            priority_long_cached_input_usd_per_million: None,
        });
    }
    normalized
}

fn prices_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-9
}

fn optional_price_matches_legacy(value: Option<f64>, expected: f64) -> bool {
    match value {
        None => true,
        Some(price) => prices_close(price, expected),
    }
}

/// Previous official book rates that should follow the new book going forward.
/// Custom overrides that are not these snapshots are kept.
fn is_superseded_default_56_pricing(pricing: &CodexLocalAccessModelPricing) -> bool {
    let model_id = normalize_known_openai_codex_model(&pricing.model_id)
        .unwrap_or_else(|| pricing.model_id.trim().to_ascii_lowercase());
    let (input, cached, output, priority_input, priority_cached, priority_output) =
        match model_id.as_str() {
            "gpt-5.6-terra" => (2.5, 0.25, 15.0, 5.0, 0.5, 30.0),
            "gpt-5.6-luna" => (1.0, 0.1, 6.0, 2.0, 0.2, 12.0),
            _ => return false,
        };
    if !prices_close(pricing.input_usd_per_million, input)
        || !prices_close(pricing.output_usd_per_million, output)
        || !optional_price_matches_legacy(pricing.cached_input_usd_per_million, cached)
    {
        return false;
    }
    optional_price_matches_legacy(pricing.priority_input_usd_per_million, priority_input)
        && optional_price_matches_legacy(
            pricing.priority_cached_input_usd_per_million,
            priority_cached,
        )
        && optional_price_matches_legacy(pricing.priority_output_usd_per_million, priority_output)
}

fn drop_superseded_default_56_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Vec<CodexLocalAccessModelPricing> {
    model_pricings
        .into_iter()
        .filter(|pricing| !is_superseded_default_56_pricing(pricing))
        .collect()
}

fn price_from_base_triple(
    input_usd_per_million: f64,
    cached_input_usd_per_million: Option<f64>,
    output_usd_per_million: f64,
) -> CodexLocalAccessPrice {
    CodexLocalAccessPrice {
        input_usd_per_million,
        cached_input_usd_per_million: cached_input_usd_per_million.unwrap_or(input_usd_per_million),
        output_usd_per_million,
    }
}

fn selected_model_pricing(
    pricing: &CodexLocalAccessModelPricing,
    selected_price: CodexLocalAccessPrice,
) -> CodexLocalAccessModelPricing {
    let mut selected = pricing.clone();
    selected.input_usd_per_million = selected_price.input_usd_per_million;
    selected.cached_input_usd_per_million = Some(selected_price.cached_input_usd_per_million);
    selected.output_usd_per_million = selected_price.output_usd_per_million;
    selected
}

fn same_model_pricing_fields(
    left: &CodexLocalAccessModelPricing,
    right: &CodexLocalAccessModelPricing,
) -> bool {
    left.long_context_threshold_tokens == right.long_context_threshold_tokens
        && left.input_usd_per_million == right.input_usd_per_million
        && left.output_usd_per_million == right.output_usd_per_million
        && left.cached_input_usd_per_million == right.cached_input_usd_per_million
        && left.standard_long_input_usd_per_million == right.standard_long_input_usd_per_million
        && left.standard_long_output_usd_per_million == right.standard_long_output_usd_per_million
        && left.standard_long_cached_input_usd_per_million
            == right.standard_long_cached_input_usd_per_million
        && left.priority_input_usd_per_million == right.priority_input_usd_per_million
        && left.priority_output_usd_per_million == right.priority_output_usd_per_million
        && left.priority_cached_input_usd_per_million == right.priority_cached_input_usd_per_million
        && left.priority_long_input_usd_per_million == right.priority_long_input_usd_per_million
        && left.priority_long_output_usd_per_million == right.priority_long_output_usd_per_million
        && left.priority_long_cached_input_usd_per_million
            == right.priority_long_cached_input_usd_per_million
}

fn changed_model_pricing_ids(
    previous: &[CodexLocalAccessModelPricing],
    next: &[CodexLocalAccessModelPricing],
) -> Vec<String> {
    let mut previous_map: HashMap<String, &CodexLocalAccessModelPricing> = HashMap::new();
    for pricing in previous {
        previous_map.insert(normalize_model_key(&pricing.model_id), pricing);
    }
    let mut next_map: HashMap<String, &CodexLocalAccessModelPricing> = HashMap::new();
    for pricing in next {
        next_map.insert(normalize_model_key(&pricing.model_id), pricing);
    }

    let mut changed = HashSet::new();
    for model_id in previous_map.keys().chain(next_map.keys()) {
        match (previous_map.get(model_id), next_map.get(model_id)) {
            (Some(previous), Some(current)) if same_model_pricing_fields(previous, current) => {}
            _ => {
                if let Some(previous) = previous_map.get(model_id) {
                    changed.insert(previous.model_id.trim().to_string());
                }
                if let Some(current) = next_map.get(model_id) {
                    changed.insert(current.model_id.trim().to_string());
                }
            }
        }
    }

    let mut changed = changed.into_iter().collect::<Vec<_>>();
    changed.sort_unstable();
    changed
}

fn normalize_reprice_model_ids(model_ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for model_id in model_ids {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        if seen.insert(normalize_model_key(model_id)) {
            normalized.push(model_id.to_string());
        }
    }
    normalized.sort_unstable();
    normalized
}

fn find_custom_model_pricing<'a>(
    collection: &'a CodexLocalAccessCollection,
    model_id: &str,
) -> Option<&'a CodexLocalAccessModelPricing> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(exact) = collection
        .model_pricings
        .iter()
        .find(|item| item.model_id.eq_ignore_ascii_case(trimmed))
    {
        return Some(exact);
    }
    let normalized = normalize_known_openai_codex_model(trimmed)?;
    collection.model_pricings.iter().find(|item| {
        item.model_id.eq_ignore_ascii_case(normalized.as_str())
            || normalize_known_openai_codex_model(&item.model_id).as_deref()
                == Some(normalized.as_str())
    })
}

fn resolve_base_model_pricing(
    collection: Option<&CodexLocalAccessCollection>,
    model_id: &str,
) -> Option<CodexLocalAccessModelPricing> {
    if let Some(collection) = collection {
        if let Some(custom) = find_custom_model_pricing(collection, model_id) {
            return Some(custom.clone());
        }
    }
    price_book_entry_for_model(model_id).map(price_book_entry_to_model_pricing)
}

fn resolve_effective_model_pricing(
    collection: Option<&CodexLocalAccessCollection>,
    model_id: Option<&str>,
    usage: Option<&UsageCapture>,
    service_tier: Option<&str>,
) -> Option<CodexLocalAccessModelPricing> {
    let Some(model_id) = model_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return None;
    };
    let base = resolve_base_model_pricing(collection, model_id)?;
    let selected_price = compute_effective_unit_prices(&base, model_id, usage, service_tier);
    Some(selected_model_pricing(&base, selected_price))
}

fn calculate_usage_cost_usd(
    usage: Option<&UsageCapture>,
    pricing: Option<&CodexLocalAccessModelPricing>,
) -> f64 {
    let (Some(usage), Some(pricing)) = (usage, pricing) else {
        return 0.0;
    };
    if let Some(breakdown) = usage.token_breakdown.as_ref() {
        if breakdown.schema_version == 2
            && breakdown.input.total_tokens
                == breakdown
                    .input
                    .uncached_tokens
                    .saturating_add(breakdown.input.cache_read_tokens)
                    .saturating_add(breakdown.input.cache_write_tokens)
            && breakdown.output.total_tokens
                == breakdown
                    .output
                    .non_reasoning_tokens
                    .saturating_add(breakdown.output.reasoning_tokens)
            && breakdown.total_tokens
                == breakdown
                    .input
                    .total_tokens
                    .saturating_add(breakdown.output.total_tokens)
                    .saturating_add(breakdown.unclassified_tokens)
            && breakdown.quality == "complete"
        {
            let cached_input_price = pricing
                .cached_input_usd_per_million
                .unwrap_or(pricing.input_usd_per_million);
            let cost = (breakdown.input.uncached_tokens as f64 * pricing.input_usd_per_million
                + breakdown.input.cache_read_tokens as f64 * cached_input_price
                + breakdown.input.cache_write_tokens as f64 * pricing.input_usd_per_million
                + breakdown.output.total_tokens as f64 * pricing.output_usd_per_million)
                / 1_000_000.0;
            return if cost.is_finite() && cost > 0.0 {
                cost
            } else {
                0.0
            };
        }
    }
    calculate_usage_cost_usd_from_tokens(
        usage.input_tokens,
        usage.output_tokens,
        usage.cached_tokens,
        pricing,
    )
}

pub fn estimate_model_token_cost_usd(
    model: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    let Some(pricing) = resolve_base_model_pricing(None, model) else {
        return 0.0;
    };
    calculate_usage_cost_usd_from_tokens(input_tokens, output_tokens, cached_input_tokens, &pricing)
}

fn calculate_usage_cost_usd_from_tokens(
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    pricing: &CodexLocalAccessModelPricing,
) -> f64 {
    let cached_tokens = cached_tokens.min(input_tokens);
    let normal_input_tokens = input_tokens.saturating_sub(cached_tokens);
    let cached_input_price = pricing
        .cached_input_usd_per_million
        .unwrap_or(pricing.input_usd_per_million);
    let cost = (normal_input_tokens as f64 * pricing.input_usd_per_million
        + cached_tokens as f64 * cached_input_price
        + output_tokens as f64 * pricing.output_usd_per_million)
        / 1_000_000.0;
    if cost.is_finite() && cost > 0.0 {
        cost
    } else {
        0.0
    }
}

fn trim_recent_events(events: &mut Vec<CodexLocalAccessUsageEvent>, retention_since: i64) {
    events.retain(|event| event.timestamp > 0 && event.timestamp >= retention_since);
    events.sort_by_key(|event| event.timestamp);
}
