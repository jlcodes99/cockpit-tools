use crate::modules::account_query_identity::{IdentitySummary, QueryIdentity};

pub(crate) fn query_identity(account: &TraeAccount) -> QueryIdentity {
    QueryIdentity {
        version: 1,
        subject: normalize_non_empty(account.user_id.as_deref()),
        email: normalize_identity_email(Some(&account.email)),
        credential_digest: None,
        platform: resolve_account_platform_kind(account).provider_key().into(),
    }
}

fn schedule_identity_maintenance() {
    if cfg!(test) {
        return;
    }
    let Ok(path) = get_accounts_index_path() else {
        return;
    };
    crate::modules::background_maintenance::schedule(
        format!("trae-identity:{}", path.display()),
        move || {
            if get_accounts_index_path()? != path {
                return Ok(true);
            }
            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut index: TraeAccountIndex =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;
            let (ready, changed) = crate::modules::account_query_identity::backfill(
                &mut index.accounts,
                |id| Ok(get_accounts_dir()?.join(format!("{id}.json"))),
                |id| load_account(id).map(|a| a.summary()),
            )?;
            if changed {
                let Ok(_guard) = TRAE_ACCOUNT_INDEX_LOCK.try_lock() else {
                    return Ok(false);
                };
                if !crate::modules::account_query_identity::commit_backfill(
                    &path, &content, &index,
                )? {
                    return Ok(false);
                }
            }
            Ok(ready)
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
