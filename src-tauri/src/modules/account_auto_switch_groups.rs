use super::{
    collect_group_quotas, default_auto_switch_groups, reported_auto_switch_groups,
    summary_bucket_family, Account, AutoSwitchGroupDefinition, AutoSwitchGroupQuota, QuotaData,
    QuotaSummaryGroup, AUTO_SWITCH_SCOPE_ANY_GROUP,
};

pub(super) enum AutoSwitchGroups {
    Legacy(Vec<AutoSwitchGroupDefinition>),
    Summary(Vec<QuotaSummaryGroup>),
}

impl AutoSwitchGroups {
    pub(super) fn resolve(
        scope: &str,
        legacy_groups: Vec<AutoSwitchGroupDefinition>,
        account: &Account,
    ) -> Self {
        if scope == AUTO_SWITCH_SCOPE_ANY_GROUP {
            if let Some(groups) = account.quota.as_ref().and_then(fresh_quota_summary_groups) {
                return Self::Summary(groups.to_vec());
            }
        }
        let reported = collect_group_quotas(account, &legacy_groups);
        Self::Legacy(reported_auto_switch_groups(scope, legacy_groups, &reported))
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Legacy(groups) => groups.len(),
            Self::Summary(groups) => groups.len(),
        }
    }

    pub(super) fn labels(&self) -> (Vec<String>, Vec<String>) {
        match self {
            Self::Legacy(groups) => groups
                .iter()
                .map(|g| (g.id.clone(), g.name.clone()))
                .unzip(),
            Self::Summary(groups) => groups
                .iter()
                .map(|g| (g.id.clone(), g.name.clone()))
                .unzip(),
        }
    }

    pub(super) fn collect_quotas(&self, account: &Account) -> Vec<AutoSwitchGroupQuota> {
        let monitored = match self {
            Self::Legacy(groups) => return collect_group_quotas(account, groups),
            Self::Summary(groups) => groups,
        };
        let Some(quota) = account.quota.as_ref() else {
            return Vec::new();
        };
        match fresh_quota_summary_groups(quota) {
            Some(reported) => monitored
                .iter()
                .filter_map(|target| {
                    reported
                        .iter()
                        .find(|group| group.id == target.id)
                        .and_then(summary_quota)
                })
                .collect(),
            None => monitored
                .iter()
                .filter_map(|target| legacy_quota(account, target))
                .collect(),
        }
    }
}

fn fresh_quota_summary_groups(quota: &QuotaData) -> Option<&[QuotaSummaryGroup]> {
    if quota.quota_summary_stale {
        return None;
    }
    quota.quota_summary_groups.as_deref()
}

fn summary_quota(group: &QuotaSummaryGroup) -> Option<AutoSwitchGroupQuota> {
    let minimum = |period: &str| {
        group
            .windows
            .iter()
            .filter(|window| window.period == period)
            .map(|w| w.percentage)
            .min()
    };
    Some(AutoSwitchGroupQuota {
        id: group.id.clone(),
        name: group.name.clone(),
        percentage: group.windows.iter().map(|w| w.percentage).min()?,
        five_hour: minimum("5h"),
        weekly: minimum("weekly"),
        other_window: group
            .windows
            .iter()
            .filter(|window| !matches!(window.period.as_str(), "5h" | "weekly"))
            .map(|w| w.percentage)
            .min(),
        model_average: None,
    })
}

fn legacy_quota(account: &Account, target: &QuotaSummaryGroup) -> Option<AutoSwitchGroupQuota> {
    let family = target
        .windows
        .iter()
        .find_map(|window| summary_bucket_family(&window.id))?;
    let definitions: Vec<_> = default_auto_switch_groups()
        .into_iter()
        .filter(|group| group.summary_family == family)
        .collect();
    let quotas = collect_group_quotas(account, &definitions);
    if quotas.len() != definitions.len() {
        return None;
    }
    Some(AutoSwitchGroupQuota {
        id: target.id.clone(),
        name: target.name.clone(),
        percentage: quotas.iter().map(|group| group.percentage).min()?,
        five_hour: quotas.iter().filter_map(|group| group.five_hour).min(),
        weekly: quotas.iter().filter_map(|group| group.weekly).min(),
        other_window: None,
        model_average: quotas.iter().filter_map(|group| group.model_average).min(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TokenData;
    use serde_json::json;

    use super::super::{
        can_be_auto_switch_candidate, evaluate_auto_switch_trigger, AutoSwitchWindowThresholds,
        AUTO_SWITCH_SCOPE_SELECTED_GROUPS,
    };

    fn account(id: &str, groups: serde_json::Value) -> Account {
        let mut account = Account::new(
            id.into(),
            "fixture@example.test".into(),
            TokenData::new("access".into(), "refresh".into(), 3600, None, None, None),
        );
        let mut quota = QuotaData::new();
        quota.quota_summary_groups = QuotaSummaryGroup::parse(&json!({ "groups": groups }));
        for group in quota.quota_summary_groups.iter().flatten() {
            for window in &group.windows {
                quota.models.push(crate::models::quota::ModelQuota {
                    name: window.id.clone(),
                    display_name: None,
                    percentage: window.percentage,
                    reset_time: String::new(),
                    summary_exhausted: None,
                });
            }
        }
        account.quota = Some(quota);
        account
    }

    #[test]
    fn shared_gemini_window_replaces_local_model_groups_after_refresh() {
        let mut current = account(
            "current",
            json!([
                {"displayName":"Gemini Models","buckets":[
                    {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.75},
                    {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.1}
                ]}
            ]),
        );
        let quota = current.quota.as_mut().unwrap();
        quota.add_model("gemini-4-argon-high".into(), None, 90, String::new());
        quota.add_model("gemini-4-carbon-medium".into(), None, 90, String::new());
        let groups = AutoSwitchGroups::resolve(
            AUTO_SWITCH_SCOPE_ANY_GROUP,
            default_auto_switch_groups(),
            &current,
        );
        let result = evaluate_auto_switch_trigger(
            &current,
            20,
            AUTO_SWITCH_SCOPE_ANY_GROUP,
            &groups,
            false,
            0,
            None,
        )
        .unwrap();
        assert_eq!(result.hit_groups.len(), 1);
        assert_eq!(result.hit_groups[0].group_name, "Gemini Models");
        assert_eq!(result.hit_groups[0].percentage, 10);

        let selected = default_auto_switch_groups()
            .into_iter()
            .filter(|group| group.id == "g4_argon")
            .collect();
        let selected =
            AutoSwitchGroups::resolve(AUTO_SWITCH_SCOPE_SELECTED_GROUPS, selected, &current);
        assert!(evaluate_auto_switch_trigger(
            &current,
            20,
            AUTO_SWITCH_SCOPE_SELECTED_GROUPS,
            &selected,
            false,
            0,
            None,
        )
        .is_none());

        let next = account(
            "next",
            json!([
                {"buckets":[{"bucketId":"gemini:weekly","remainingFraction":0.9}]}
            ]),
        );
        assert!(can_be_auto_switch_candidate(
            &next, "current", 20, &groups, false, 0, None
        ));
        let mut legacy = next.clone();
        legacy.quota.as_mut().unwrap().quota_summary_groups = None;
        assert!(can_be_auto_switch_candidate(
            &legacy, "current", 20, &groups, false, 0, None
        ));
    }

    #[test]
    fn independent_and_new_google_groups_use_their_reported_windows() {
        let current = account(
            "current",
            json!([
                {"displayName":"Gemini Models","buckets":[
                    {"bucketId":"gemini-weekly","remainingFraction":0.8}
                ]},
                {"displayName":"Gemini 4 Argon","buckets":[
                    {"bucketId":"gemini-argon-wtu-weekly","remainingFraction":0.04}
                ]},
                {"displayName":"New group","buckets":[
                    {"bucketId":"new-group-short","window":"5h","remainingFraction":0.09}
                ]}
            ]),
        );
        let groups = AutoSwitchGroups::resolve(
            AUTO_SWITCH_SCOPE_ANY_GROUP,
            default_auto_switch_groups(),
            &current,
        );
        let windows = Some(AutoSwitchWindowThresholds {
            five_hour: 10,
            weekly: 5,
        });
        let result = evaluate_auto_switch_trigger(
            &current,
            0,
            AUTO_SWITCH_SCOPE_ANY_GROUP,
            &groups,
            false,
            0,
            windows,
        )
        .unwrap();
        assert_eq!(
            result
                .hit_groups
                .iter()
                .map(|group| group.group_name.as_str())
                .collect::<Vec<_>>(),
            ["Gemini 4 Argon", "New group"]
        );
        let mut next = current.clone();
        next.id = "next".into();
        for group in next
            .quota
            .as_mut()
            .unwrap()
            .quota_summary_groups
            .iter_mut()
            .flatten()
        {
            for window in &mut group.windows {
                window.percentage = 90;
            }
        }
        assert!(can_be_auto_switch_candidate(
            &next, "current", 0, &groups, false, 0, windows
        ));
        next.quota
            .as_mut()
            .unwrap()
            .quota_summary_groups
            .as_mut()
            .unwrap()
            .pop();
        assert!(!can_be_auto_switch_candidate(
            &next, "current", 0, &groups, false, 0, windows
        ));
    }
}
