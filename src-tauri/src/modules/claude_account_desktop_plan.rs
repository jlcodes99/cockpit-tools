// Select one organization before interpreting subscription data. Never combine tenants.
fn desktop_plan_organization(profile: &Value) -> Option<&Value> {
    let endpoints = profile.get("endpoints")?;
    let active = read_string_path(profile, &["lastActiveOrg"]);
    let direct = [
        &["bootstrapAppStart", "activeOrganization"][..],
        &["bootstrapAppStart", "active_organization"][..],
        &["bootstrapAppStart", "organization"][..],
        &["accountProfile", "organization"][..],
        &["account", "organization"][..],
    ];
    for path in direct {
        if let Some(org) = path.iter().try_fold(endpoints, |v, key| v.get(*key)) {
            if org.is_object()
                && active
                    .as_deref()
                    .is_none_or(|id| org.get("uuid").and_then(Value::as_str) == Some(id))
            {
                return Some(org);
            }
        }
    }
    for path in [
        &["bootstrapAppStart", "account", "memberships"][..],
        &["accountProfile", "account", "memberships"][..],
        &["accountProfile", "memberships"][..],
        &["account", "account", "memberships"][..],
        &["account", "memberships"][..],
    ] {
        let Some(items) = path
            .iter()
            .try_fold(endpoints, |v, key| v.get(*key))
            .and_then(Value::as_array)
        else {
            continue;
        };
        if let Some(id) = active.as_deref() {
            if let Some(org) = items
                .iter()
                .filter_map(|v| v.get("organization"))
                .find(|org| org.get("uuid").and_then(Value::as_str) == Some(id))
            {
                return Some(org);
            }
        } else if items.len() == 1 {
            return items[0].get("organization");
        }
    }
    None
}

fn desktop_max_subtier_from_rate_limit(sources: &[Option<&Value>]) -> Option<(String, String)> {
    for value in sources.iter().copied().flatten() {
        if let Some(raw) = read_string_path(value, &["rate_limit_tier"]) {
            if let Some(plan) = normalize_desktop_plan_value(Some(raw.clone())) {
                if matches!(plan.as_str(), "Max 5x" | "Max 20x") {
                    return Some((plan, raw));
                }
            }
        }
    }
    None
}

fn refine_desktop_max_plan(
    resolved: (String, Option<String>),
    sources: &[Option<&Value>],
) -> (Option<String>, Option<String>) {
    if resolved.0 == "Max" {
        if let Some((subtier, raw)) = desktop_max_subtier_from_rate_limit(sources) {
            return (Some(subtier), Some(raw));
        }
    }
    (Some(resolved.0), resolved.1)
}

fn desktop_plan_from_profile(profile: &Value) -> (Option<String>, Option<String>) {
    let org = desktop_plan_organization(profile);
    let enterprise = org
        .and_then(|v| v.get("organization_type"))
        .and_then(Value::as_str)
        == Some("claude_enterprise")
        || capability_strings(org.and_then(|v| v.get("capabilities")))
            .iter()
            .any(|v| v == "raven_enterprise");
    let endpoints = profile.get("endpoints");
    let subscription = endpoints.and_then(|v| v.get("subscriptionDetails"));
    let nested_subscription = subscription.and_then(|v| v.get("subscription"));
    let usage = endpoints.and_then(|v| v.get("organizationUsage"));
    let account_profile = endpoints.and_then(|v| v.get("accountProfile"));
    let bootstrap = endpoints.and_then(|v| v.get("bootstrapAppStart"));
    // Display names are server-authored labels, including custom enterprise plans.
    for value in [org, subscription, usage, account_profile, bootstrap]
        .into_iter()
        .flatten()
    {
        for key in ["planDisplayName", "plan_display_name"] {
            if let Some(label) = read_string_path(value, &[key]) {
                return (Some(label.clone()), Some(label));
            }
        }
    }

    let sources = [
        org,
        subscription,
        nested_subscription,
        usage,
        account_profile,
        bootstrap,
    ];

    // Explicit paid/subscription tier fields take precedence over rate_limit_tier,
    // because standard Pro accounts still carry `rate_limit_tier = "default_claude_ai"`.
    for value in sources.into_iter().flatten() {
        for key in [
            "paidAccountTier",
            "subscription_type",
            "subscriptionType",
            "plan_type",
            "planType",
            "plan",
            "tier",
        ] {
            if let Some(raw) = read_string_path(value, &[key]) {
                if raw == "raven" {
                    let plan = if enterprise { "Enterprise" } else { "Team" }.to_string();
                    return (Some(plan), Some(raw));
                }
                if let Some(plan) = normalize_desktop_plan_value(Some(raw.clone())) {
                    return refine_desktop_max_plan((plan, Some(raw)), &sources);
                }
            }
        }
    }

    let caps = capability_strings(org.and_then(|v| v.get("capabilities")));
    let cap_plan = plan_from_capability_list(&caps, enterprise);
    if let Some(plan) = cap_plan.as_ref().filter(|plan| plan.as_str() != "Free") {
        return refine_desktop_max_plan((plan.clone(), None), &sources);
    }

    for value in sources.into_iter().flatten() {
        if let Some(raw) = read_string_path(value, &["organization_type"]) {
            if raw == "raven" {
                let plan = if enterprise { "Enterprise" } else { "Team" }.to_string();
                return (Some(plan), Some(raw));
            }
            if let Some(plan) = normalize_desktop_plan_value(Some(raw.clone())) {
                return refine_desktop_max_plan((plan, Some(raw)), &sources);
            }
        }
    }

    if let Some(plan) = cap_plan {
        return (Some(plan), None);
    }

    for value in sources.into_iter().flatten() {
        if let Some(raw) = read_string_path(value, &["rate_limit_tier"]) {
            if let Some(plan) = normalize_desktop_plan_value(Some(raw.clone())) {
                return (Some(plan), Some(raw));
            }
        }
    }

    (None, None)
}
