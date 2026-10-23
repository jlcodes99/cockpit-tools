fn quota_display_account(id: &str, plan: &str, hourly: i32, weekly: i32) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.into(),
        "fixture@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "fixture".into(),
            id_token: "fixture".into(),
            refresh_token: None,
        },
    );
    account.plan_type = Some(plan.into());
    account.usage_updated_at = Some(1000);
    account.quota = Some(
        serde_json::from_value(
            json!({"hourly_percentage":hourly,"weekly_percentage":weekly,
        "hourly_window_minutes":300,"weekly_window_minutes":10080}),
        )
        .unwrap(),
    );
    account
}

fn quota_display_official() -> CodexAccount {
    let mut account = quota_display_account("chatgpt_plan_grant", "unknown", 99, 99);
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    account.api_provider_id = Some(codex_account::CHATGPT_PLAN_PROVIDER_ID.into());
    // These projection values must never masquerade as source usage.
    account.usage_updated_at = Some(9999);
    account
}

#[test]
fn quota_reply_official_uses_bound_source_data_and_timestamp_without_special_plan_group() {
    let source = quota_display_account("source", "plus", 65, 80);
    let official = quota_display_official();
    let display = HashMap::from([(
        official.id.clone(),
        official_quota_plans::DisplayAccount::from_source(&source),
    )]);
    let result = native_quota_summary(
        &[official.id.clone()],
        &[official.clone()],
        &HashSet::new(),
        1001,
        &display,
    );
    assert_eq!(result["accountCount"], 1);
    assert_eq!(result["includedAccountCount"], 1);
    assert_eq!(result["missingAccountCount"], 0);
    assert_eq!(result["availableAccountCount"], 1);
    assert_eq!(result["fiveHourRemainingPercent"], 65);
    assert_eq!(result["weeklyRemainingPercent"], 80);
    assert_eq!(result["updatedAt"], 1000);
    assert_eq!(result["stale"], false);
    assert_eq!(result["plans"][0]["plan"], "plus");
    assert!(result["plans"][0].get("official").is_none());
    let stale = native_quota_summary(
        &[official.id.clone()],
        &[official.clone()],
        &HashSet::new(),
        1901,
        &display,
    );
    assert_eq!(stale["stale"], true);
    let unknown = native_quota_summary(
        &[official.id.clone()],
        &[official],
        &HashSet::new(),
        1001,
        &HashMap::new(),
    );
    assert_eq!(unknown["missingAccountCount"], 1);
    assert!(unknown.get("fiveHourRemainingPercent").is_none());
    assert!(unknown.get("updatedAt").is_none());
}

#[test]
fn quota_reply_deduplicates_source_channels_and_uses_only_scoped_channel_health() {
    let source = quota_display_account("source", "plus", 65, 80);
    let mut official = quota_display_official();
    let outside = quota_display_account("outside", "team", 100, 100);
    let display = HashMap::from([(
        official.id.clone(),
        official_quota_plans::DisplayAccount::from_source(&source),
    )]);
    let scope = vec![source.id.clone(), official.id.clone(), official.id.clone()];
    let accounts = vec![source.clone(), official.clone(), outside.clone()];
    let cooling = HashSet::from([official.id.clone()]);
    let merged = native_quota_summary(&scope, &accounts, &cooling, 1001, &display);
    assert_eq!(merged["accountCount"], 1);
    assert_eq!(merged["includedAccountCount"], 1);
    assert_eq!(merged["availableAccountCount"], 1);
    assert_eq!(merged["cooldownAccountCount"], 0);
    assert_eq!(merged["fiveHourRemainingPercent"], 65);
    assert_eq!(merged["fiveHourCapacityPercent"], 100);
    assert_eq!(merged["weeklyCapacityPercent"], 100);
    assert_eq!(merged["plans"].as_array().unwrap().len(), 1);
    assert_eq!(merged["plans"][0]["count"], 1);
    let isolated =
        native_quota_summary(&[official.id.clone()], &accounts, &cooling, 1001, &display);
    assert_eq!(isolated["cooldownAccountCount"], 1);
    assert_eq!(isolated["availableAccountCount"], 0);
    official.requires_reauth = true;
    let isolated = native_quota_summary(
        &[official.id.clone()],
        &[source, official, outside.clone()],
        &HashSet::new(),
        1001,
        &display,
    );
    assert_eq!(isolated["abnormalAccountCount"], 1);
    assert_eq!(isolated["availableAccountCount"], 0);
    assert_eq!(isolated["fiveHourRemainingPercent"], 65);
    let empty = native_quota_summary(&[], &accounts, &HashSet::new(), 1001, &display);
    assert_eq!(empty["accountCount"], 0);
    assert_eq!(empty["plans"], json!([]));
    let unrelated = native_quota_summary(
        &[outside.id.clone()],
        &accounts,
        &HashSet::new(),
        1001,
        &display,
    );
    assert_eq!(unrelated["plans"][0]["plan"], "team");
    assert_eq!(unrelated["fiveHourRemainingPercent"], 100);
}

#[test]
fn quota_reply_merges_equal_source_plans_and_keeps_unknown_windows_unknown() {
    let mut source = quota_display_account("source", "plus", 65, 0);
    let official = quota_display_official();
    let other = quota_display_account("other", "plus", 40, 50);
    let mut display = HashMap::from([(
        official.id.clone(),
        official_quota_plans::DisplayAccount::from_source(&source),
    )]);
    let accounts = vec![official.clone(), other.clone()];
    let scope = vec![official.id.clone(), other.id.clone()];
    let result = native_quota_summary(&scope, &accounts, &HashSet::new(), 1001, &display);
    assert_eq!(result["plans"].as_array().unwrap().len(), 1);
    assert_eq!(result["plans"][0]["count"], 2);
    assert_eq!(
        result["fiveHourRemainingPercent"], 40,
        "zero weekly quota gates source hourly quota once"
    );
    source.quota.as_mut().unwrap().hourly_window_present = Some(false);
    source.quota.as_mut().unwrap().weekly_window_present = Some(false);
    display.insert(
        official.id.clone(),
        official_quota_plans::DisplayAccount::from_source(&source),
    );
    let missing = native_quota_summary(
        &[official.id.clone()],
        &accounts,
        &HashSet::new(),
        1001,
        &display,
    );
    assert_eq!(missing["missingAccountCount"], 1);
    assert!(missing.get("remainingPercent").is_none());
    assert!(missing.get("fiveHourCapacityPercent").is_none());
    assert!(missing.get("weeklyCapacityPercent").is_none());
    assert_eq!(
        missing["availableAccountCount"], 1,
        "unknown source quota is not a failed official channel"
    );
}

#[test]
fn quota_capacity_counts_only_windows_that_contribute_remaining_percent() {
    let first = quota_display_account("first", "plus", 100, 90);
    let second = quota_display_account("second", "plus", 100, 80);
    let third = quota_display_account("third", "plus", 95, 70);
    let accounts = vec![first, second, third];
    let response = native_quota_summary(
        &[
            "first".into(),
            "second".into(),
            "third".into(),
            "missing".into(),
        ],
        &accounts,
        &HashSet::new(),
        1001,
        &HashMap::new(),
    );
    assert_eq!(response["accountCount"], 4);
    assert_eq!(response["fiveHourRemainingPercent"], 295);
    assert_eq!(response["fiveHourCapacityPercent"], 300);
    assert_eq!(response["weeklyRemainingPercent"], 240);
    assert_eq!(response["weeklyCapacityPercent"], 300);
    assert_eq!(response["missingAccountCount"], 1);
}

#[test]
fn quota_capacity_keeps_missing_invalid_and_unclassified_windows_unknown() {
    let mut hourly = quota_display_account("hourly", "plus", 60, 80);
    hourly.quota.as_mut().unwrap().weekly_window_present = Some(false);
    let mut weekly = quota_display_account("weekly", "plus", 60, 80);
    weekly.quota.as_mut().unwrap().hourly_window_present = Some(false);
    let mut invalid = quota_display_account("invalid", "plus", 101, -1);
    let mut unclassified = quota_display_account("unclassified", "plus", 50, 70);
    unclassified.quota.as_mut().unwrap().hourly_window_minutes = Some(1440);
    unclassified.quota.as_mut().unwrap().weekly_window_minutes = Some(1440);
    let accounts = vec![
        hourly.clone(),
        weekly.clone(),
        invalid.clone(),
        unclassified.clone(),
    ];
    let response = native_quota_summary(
        &[
            hourly.id.clone(),
            weekly.id.clone(),
            invalid.id.clone(),
            unclassified.id.clone(),
        ],
        &accounts,
        &HashSet::new(),
        1001,
        &HashMap::new(),
    );
    assert_eq!(response["fiveHourRemainingPercent"], 60);
    assert_eq!(response["fiveHourCapacityPercent"], 100);
    assert_eq!(response["weeklyRemainingPercent"], 80);
    assert_eq!(response["weeklyCapacityPercent"], 100);
    invalid.quota = None;
    for account in [invalid, unclassified] {
        let unknown = native_quota_summary(
            &[account.id.clone()],
            &[account],
            &HashSet::new(),
            1001,
            &HashMap::new(),
        );
        assert!(unknown.get("fiveHourCapacityPercent").is_none());
        assert!(unknown.get("weeklyCapacityPercent").is_none());
    }
}

#[test]
fn quota_capacity_distinguishes_exhausted_window_from_empty_pool() {
    let account = quota_display_account("exhausted", "plus", 100, 0);
    let response = native_quota_summary(
        &[account.id.clone()],
        &[account.clone()],
        &HashSet::from([account.id.clone()]),
        1001,
        &HashMap::new(),
    );
    assert_eq!(response["fiveHourRemainingPercent"], 0);
    assert_eq!(response["weeklyRemainingPercent"], 0);
    assert_eq!(response["fiveHourCapacityPercent"], 100);
    assert_eq!(response["weeklyCapacityPercent"], 100);
    assert_eq!(response["cooldownAccountCount"], 1);
    let empty = native_quota_summary(&[], &[account], &HashSet::new(), 1001, &HashMap::new());
    assert_eq!(empty["fiveHourCapacityPercent"], 0);
    assert_eq!(empty["weeklyCapacityPercent"], 0);
    assert_eq!(empty["accountCount"], 0);
}

#[test]
fn quota_capacity_tracks_multiple_qualifying_windows_without_account_count_guessing() {
    let mut account = quota_display_account("two-short-windows", "plus", 60, 80);
    account.quota.as_mut().unwrap().weekly_window_minutes = Some(120);
    let response = native_quota_summary(
        &[account.id.clone()],
        &[account],
        &HashSet::new(),
        1001,
        &HashMap::new(),
    );
    assert_eq!(response["accountCount"], 1);
    assert_eq!(response["fiveHourRemainingPercent"], 140);
    assert_eq!(response["fiveHourCapacityPercent"], 200);
    assert!(response.get("weeklyCapacityPercent").is_none());
}
