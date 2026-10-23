use super::*;
use crate::models::codex::{CodexAccount, CodexTokens};

fn source(id: &str, percent: i32) -> CodexAccount {
    let mut source = CodexAccount::new(
        id.into(),
        "fixture@example.invalid".into(),
        CodexTokens {
            access_token: "fixture".into(),
            id_token: "fixture".into(),
            refresh_token: None,
        },
    );
    source.plan_type = Some("plus".into());
    source.usage_updated_at = Some(1234);
    source.quota = Some(
        serde_json::from_value(serde_json::json!({
            "hourly_percentage":percent,"weekly_percentage":80,"raw_data":{"fixture":"not retained"}
        }))
        .unwrap(),
    );
    source
}

fn grant() -> chatgpt_plan::ChatgptPlanAccountSummary {
    serde_json::from_value(serde_json::json!({
        "id":"grant","label":"fixture","subject":"subject","client_id":"client",
        "source_account_id":"source","source_binding_valid":true,"connected":true,"needs_reauth":false,
        "created_at":1,"updated_at":1
    })).unwrap()
}

#[test]
fn pending_metadata_keeps_only_the_same_verified_subscription_and_quota() {
    let source = source("source", 60);
    let mut grant = grant();
    let wanted = vec!["chatgpt_plan_grant".into()];
    let mut cache = DisplayCache::default();
    let fresh = commit(
        &mut cache,
        1,
        &wanted,
        &[grant.clone()],
        Some(&[source.clone()]),
    );
    assert_eq!(fresh[&wanted[0]].plan.as_deref(), Some("plus"));
    assert_eq!(fresh[&wanted[0]].usage_updated_at, Some(1234));
    assert_eq!(
        fresh[&wanted[0]].quota.as_ref().unwrap().hourly_percentage,
        60
    );
    assert!(fresh[&wanted[0]].quota.as_ref().unwrap().raw_data.is_none());
    grant.source_binding_valid = false;
    let mut changed = source.clone();
    changed.plan_type = Some("team".into());
    changed.quota.as_mut().unwrap().hourly_percentage = 5;
    let pending = commit(&mut cache, 2, &wanted, &[grant.clone()], Some(&[changed]));
    assert_eq!(pending[&wanted[0]].plan.as_deref(), Some("plus"));
    assert_eq!(
        pending[&wanted[0]]
            .quota
            .as_ref()
            .unwrap()
            .hourly_percentage,
        60
    );
    grant.source_binding_error = Some("CHATGPT_PLAN_SOURCE_CHANGED".into());
    assert!(commit(&mut cache, 3, &wanted, &[grant.clone()], None).is_empty());
    grant.source_binding_error = None;
    grant.source_binding_valid = true;
    assert!(
        commit(
            &mut cache,
            1,
            &wanted,
            &[grant.clone()],
            Some(&[source.clone()])
        )
        .is_empty(),
        "late reads cannot restore an invalidated source"
    );
    grant.id = "other".into();
    let other = vec!["chatgpt_plan_other".into()];
    assert_eq!(
        commit(&mut cache, 1, &other, &[grant], Some(&[source]))[&other[0]].source_id,
        "source"
    );
}

#[test]
fn binding_change_unlink_missing_source_and_duplicate_grants_clear_display() {
    let wanted = vec!["chatgpt_plan_grant".into()];
    for mutation in 0..7 {
        let mut cache = DisplayCache::default();
        let mut grant = grant();
        let original = source("source", 60);
        assert_eq!(
            commit(
                &mut cache,
                1,
                &wanted,
                &[grant.clone()],
                Some(&[original.clone()])
            )
            .len(),
            1
        );
        let mut grants = Vec::new();
        let mut sources = vec![original];
        match mutation {
            0 => grant.source_account_id = None,
            1 => {
                grant.source_account_id = Some("other-source".into());
                grant.source_binding_valid = false;
            }
            2 => {
                grant.subject = "changed-subject".into();
                grant.source_binding_valid = false;
            }
            3 => {
                grant.client_id = "changed-client".into();
                grant.source_binding_valid = false;
            }
            4 => sources.clear(),
            5 => {
                let mut duplicate = grant.clone();
                duplicate.id = "other-grant".into();
                grants.push(duplicate);
            }
            _ => grants.push(grant.clone()),
        }
        grants.push(grant);
        assert!(
            commit(&mut cache, 2, &wanted, &grants, Some(&sources)).is_empty(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn absent_source_quota_is_not_fabricated_and_transient_reads_keep_confirmed_data() {
    let wanted = vec!["chatgpt_plan_grant".into()];
    let mut cache = DisplayCache::default();
    let grant = grant();
    let mut source = source("source", 35);
    commit(
        &mut cache,
        1,
        &wanted,
        &[grant.clone()],
        Some(&[source.clone()]),
    );
    let transient = commit(&mut cache, 2, &wanted, &[grant.clone()], None);
    assert_eq!(
        transient[&wanted[0]]
            .quota
            .as_ref()
            .unwrap()
            .hourly_percentage,
        35
    );
    source.plan_type = None;
    source.auth_file_plan_type = Some("pro-special".into());
    source.quota = None;
    source.usage_updated_at = None;
    let fresh = commit(&mut cache, 3, &wanted, &[grant], Some(&[source]));
    assert_eq!(fresh[&wanted[0]].plan.as_deref(), Some("pro-special"));
    assert!(fresh[&wanted[0]].quota.is_none());
    assert!(fresh[&wanted[0]].usage_updated_at.is_none());
    assert!(
        commit(&mut cache, 4, &[], &[], None).is_empty(),
        "empty key scope cannot expose cached data"
    );
}

#[test]
fn unverified_grants_never_adopt_matching_email_or_source_values() {
    let wanted = vec!["chatgpt_plan_grant".into()];
    let mut grant = grant();
    grant.source_binding_valid = false;
    let mut cache = DisplayCache::default();
    assert!(commit(
        &mut cache,
        1,
        &wanted,
        &[grant.clone()],
        Some(&[source("source", 100)])
    )
    .is_empty());
    grant.source_account_id = None;
    assert!(commit(
        &mut cache,
        2,
        &wanted,
        &[grant],
        Some(&[source("source", 100)])
    )
    .is_empty());
}
