use crate::modules::account_query_identity::{
    fingerprint, BackfillCursor, BackfillStatus, DedupScan, IdentitySummary, QueryIdentity,
    ScanResult,
};

pub(crate) fn query_identity(account: &KiroAccount) -> QueryIdentity {
    QueryIdentity {
        version: 1,
        subject: normalize_user_identity(account.user_id.as_deref()),
        email: normalize_email_identity(Some(&account.email)),
        credential_digest: fingerprint(account.refresh_token.as_deref()),
        platform: "kiro".into(),
    }
}

fn identity_duplicates(left: &QueryIdentity, right: &QueryIdentity) -> bool {
    if matches!((&left.subject, &right.subject), (Some(a), Some(b)) if a != b)
        || matches!((&left.email, &right.email), (Some(a), Some(b)) if a != b)
    {
        return false;
    }
    (left.subject.is_some() && left.subject == right.subject)
        || (left.email.is_some()
            && left.email == right.email
            && left.subject.is_some() == right.subject.is_some())
        || (left.credential_digest.is_some() && left.credential_digest == right.credential_digest)
}

pub(crate) fn schedule_identity_maintenance() {
    if cfg!(test) {
        return;
    } // Fixture tests drive migration steps explicitly.
    let Ok(store) = index_store() else {
        return;
    };
    let mut scan = DedupScan::default();
    let mut backfill = BackfillCursor::default();
    let mut snapshot: Option<KiroAccountIndex> = None;
    let mut pending = None;
    crate::modules::schedule_identity_job(
        format!("kiro-identity:{}", store.legacy_path().display()),
        move || {
            let status = backfill.step(
                &store,
                &KIRO_ACCOUNT_INDEX_LOCK,
                resolve_account_file_path,
                |id| load_account(id).map(|a| a.summary()),
            )?;
            let (revision, version) = match status {
                BackfillStatus::Inactive => return Ok(true),
                BackfillStatus::Pending => return Ok(false),
                BackfillStatus::Done { revision, version } => (revision, version),
            };
            if version == 1 {
                return Ok(true);
            }
            if snapshot
                .as_ref()
                .and_then(|index| index.storage_revision.get())
                != Some(revision)
            {
                snapshot = match store.load()? {
                    crate::modules::antigravity_account_index_store::IndexRead::Active(index) => {
                        Some(index)
                    }
                    _ => None,
                };
                scan = DedupScan::default();
                pending = None;
            }
            let Some(index) = snapshot.as_mut() else {
                return Ok(false);
            };
            if index.storage_revision.get() != Some(revision) {
                return Ok(false);
            }
            let outcome = pending.take().unwrap_or_else(|| {
                scan.step(
                    &index.accounts,
                    index.identity_revision,
                    identity_duplicates,
                )
            });
            if matches!(outcome, ScanResult::Pending) {
                return Ok(false);
            }
            let Ok(_guard) = KIRO_ACCOUNT_INDEX_LOCK.try_lock() else {
                pending = Some(outcome);
                return Ok(false);
            };
            let Some(_writer) = store.try_lock_writes()? else {
                pending = Some(outcome);
                return Ok(false);
            };
            if store.active_revision()? != Some(revision) {
                return Ok(false);
            }
            match outcome {
                ScanResult::Pending => Ok(false),
                ScanResult::Complete => {
                    index.identity_maintenance_version = 1;
                    save_account_index(index)?;
                    Ok(true)
                }
                ScanResult::Pair(left, right) => {
                    let a = load_account(&index.accounts[left].id)
                        .ok_or("Kiro duplicate detail unavailable")?;
                    let b = load_account(&index.accounts[right].id)
                        .ok_or("Kiro duplicate detail unavailable")?;
                    if !accounts_are_duplicates(&a, &b) {
                        return Err("Kiro identity changed before merge".into());
                    }
                    let accounts = vec![a, b];
                    let store = crate::modules::kiro_instance::load_instance_store()?;
                    let mut bound_ids: HashSet<String> = store
                        .instances
                        .iter()
                        .filter_map(|v| v.bind_account_id.clone())
                        .collect();
                    if let Some(id) = store.default_settings.bind_account_id {
                        bound_ids.insert(id);
                    }
                    let bound = accounts
                        .iter()
                        .find(|a| bound_ids.contains(&a.id))
                        .map(|a| a.id.clone());
                    // Two bound profiles are deliberate identities until their
                    // bindings are explicitly reconciled; never break a profile.
                    if accounts.iter().all(|a| bound_ids.contains(&a.id)) {
                        return Ok(false);
                    }
                    let primary =
                        choose_primary_account_index(&[0, 1], &accounts, bound.as_deref());
                    let mut merged = accounts[primary].clone();
                    let removed = &accounts[1 - primary];
                    merge_duplicate_account(&mut merged, removed);
                    let removed_path = resolve_account_file_path(&removed.id)?;
                    let removed_hash =
                        crate::modules::account_query_identity::backup_duplicate(&removed_path)?;
                    // Save before removing anything. A failed index save leaves both
                    // original files recoverable and the next batch can retry.
                    save_account_file_indexed(&merged, index)?;
                    refresh_summary(index, &merged);
                    index.accounts.retain(|s| s.id != removed.id);
                    index.identity_revision = index.identity_revision.wrapping_add(1);
                    save_account_index(index)?;
                    snapshot = None;
                    crate::modules::atomic_write::remove_file_if_hash_matches(
                        &removed_path,
                        removed_hash,
                    )?;
                    Ok(false)
                }
            }
        },
    );
}

fn summary_matches_payload(
    summary: &crate::models::kiro::KiroAccountSummary,
    user: Option<&String>,
    email: Option<&String>,
    refresh: Option<&String>,
) -> bool {
    if let Some(identity) = summary.identity() {
        let digest = fingerprint(refresh.map(String::as_str));
        account_matches_payload_identity(
            identity.subject.as_ref(),
            identity.email.as_ref(),
            identity.credential_digest.as_ref(),
            user,
            email,
            digest.as_ref(),
        )
    } else {
        // Temporary fallback is limited to records without a versioned snapshot.
        load_account(&summary.id).is_some_and(|account| {
            account_matches_payload_identity(
                normalize_user_identity(account.user_id.as_deref()).as_ref(),
                normalize_email_identity(Some(&account.email)).as_ref(),
                normalize_token_identity(account.refresh_token.as_deref()).as_ref(),
                user,
                email,
                refresh,
            )
        })
    }
}

#[cfg(test)]
mod query_identity_tests {
    use super::*;
    #[test]
    fn explicit_subject_conflicts_and_missing_subject_rules_are_preserved() {
        let a = QueryIdentity {
            version: 1,
            subject: Some("a".into()),
            email: Some("user@example.com".into()),
            credential_digest: fingerprint(Some("secret")),
            platform: "kiro".into(),
        };
        let mut b = a.clone();
        b.subject = Some("b".into());
        assert!(!identity_duplicates(&a, &b));
        b.subject = None;
        b.credential_digest = None;
        assert!(!identity_duplicates(&a, &b));
        let mut c = a.clone();
        c.subject = None;
        c.credential_digest = None;
        assert!(identity_duplicates(&b, &c));
        c.email = Some("other@example.com".into());
        assert!(!identity_duplicates(&b, &c));
    }
}
