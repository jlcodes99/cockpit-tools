use crate::modules::account_query_identity::{
    BackfillCursor, BackfillStatus, IdentitySummary, QueryIdentity,
};

pub(crate) fn query_identity(account: &TraeAccount) -> QueryIdentity {
    QueryIdentity {
        version: 1,
        subject: normalize_non_empty(account.user_id.as_deref()),
        email: normalize_identity_email(Some(&account.email)),
        credential_digest: None,
        platform: resolve_account_platform_kind(account).provider_key().into(),
    }
}

pub(crate) fn schedule_identity_maintenance() {
    if cfg!(test) {
        return;
    }
    let Ok(store) = index_store() else {
        return;
    };
    let mut backfill = BackfillCursor::default();
    crate::modules::schedule_identity_job(
        format!("trae-identity:{}", store.legacy_path().display()),
        move || {
            let status = backfill.step(
                &store,
                &TRAE_ACCOUNT_INDEX_LOCK,
                resolve_account_file_path,
                |id| load_account(id).map(|a| a.summary()),
            )?;
            Ok(matches!(
                status,
                BackfillStatus::Inactive | BackfillStatus::Done { .. }
            ))
        },
    );
}

fn summary_matches_payload(
    summary: &crate::models::trae::TraeAccountSummary,
    platform: TraePlatformKind,
    user: Option<&str>,
    email: Option<&str>,
) -> bool {
    if let Some(identity) = summary.identity() {
        identity.platform == platform.provider_key()
            && match (identity.subject.as_deref(), user) {
                (Some(a), Some(b)) => a == b,
                _ => matches!((identity.email.as_deref(), email), (Some(a), Some(b)) if a == b),
            }
    } else {
        load_account(&summary.id).is_some_and(|a| {
            resolve_account_platform_kind(&a) == platform
                && account_matches_import_identity(&a, user, email)
        })
    }
}
