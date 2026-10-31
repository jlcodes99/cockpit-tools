use crate::modules::account_query_identity::{
    fingerprint, DedupScan, IdentitySummary, QueryIdentity, ScanResult,
};

pub(crate) fn query_identity(account: &WindsurfAccount) -> QueryIdentity {
    QueryIdentity {
        version: 1,
        subject: Some(account.github_id.to_string()),
        email: None,
        credential_digest: fingerprint(resolve_account_api_key(account).as_deref()),
        platform: "windsurf".into(),
    }
}

fn identity_duplicates(left: &QueryIdentity, right: &QueryIdentity) -> bool {
    left.subject == right.subject
        && match (&left.credential_digest, &right.credential_digest) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

fn schedule_identity_maintenance() {
    if cfg!(test) {
        return;
    }
    let Ok(path) = get_accounts_index_path() else {
        return;
    };
    let mut scan = DedupScan::default();
    crate::modules::background_maintenance::schedule(
        format!("windsurf-identity:{}", path.display()),
        move || {
            if get_accounts_index_path()? != path {
                return Ok(true);
            }
            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut index: WindsurfAccountIndex =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;
            if index.identity_maintenance_version == 1
                && index.accounts.iter().all(|s| s.identity().is_some())
            {
                return Ok(true);
            }
            let (ready, changed) = crate::modules::account_query_identity::backfill(
                &mut index.accounts,
                |id| Ok(get_accounts_dir()?.join(format!("{id}.json"))),
                |id| load_account(id).map(|a| a.summary()),
            )?;
            if changed {
                let Ok(_guard) = WINDSURF_ACCOUNT_INDEX_LOCK.try_lock() else {
                    return Ok(false);
                };
                crate::modules::account_query_identity::commit_backfill(&path, &content, &index)?;
                return Ok(false);
            }
            if !ready {
                return Ok(false);
            }
            let Ok(_guard) = WINDSURF_ACCOUNT_INDEX_LOCK.try_lock() else {
                return Ok(false);
            };
            if fs::read_to_string(&path).map_err(|e| e.to_string())? != content {
                return Ok(false);
            }
            match scan.step(
                &index.accounts,
                index.identity_revision,
                identity_duplicates,
            ) {
                ScanResult::Pending => Ok(false),
                ScanResult::Complete => {
                    index.identity_maintenance_version = 1;
                    save_account_index(&index)?;
                    Ok(true)
                }
                ScanResult::Pair(left, right) => {
                    let a = load_account(&index.accounts[left].id)
                        .ok_or("Windsurf duplicate detail unavailable")?;
                    let b = load_account(&index.accounts[right].id)
                        .ok_or("Windsurf duplicate detail unavailable")?;
                    if a.github_id != b.github_id || !account_apis_are_compatible(&a, &b) {
                        return Err("Windsurf identity changed before merge".into());
                    }
                    let old_ids = [a.id.clone(), b.id.clone()];
                    let store = crate::modules::windsurf_instance::load_instance_store()?;
                    let mut bound_ids: HashSet<String> = store
                        .instances
                        .iter()
                        .filter_map(|v| v.bind_account_id.clone())
                        .collect();
                    if let Some(id) = store.default_settings.bind_account_id {
                        bound_ids.insert(id);
                    }
                    if old_ids.iter().all(|id| bound_ids.contains(id)) {
                        return Ok(false);
                    }
                    let mut merged = merge_account_group(vec![a, b]);
                    if let Some(id) = old_ids.iter().find(|id| bound_ids.contains(*id)) {
                        merged.id = id.clone();
                    }
                    let removed_id = old_ids
                        .iter()
                        .find(|id| **id != merged.id)
                        .ok_or("duplicate merge has no removed identity")?;
                    let removed_path = get_accounts_dir()?.join(format!("{removed_id}.json"));
                    let removed_hash =
                        crate::modules::account_query_identity::backup_duplicate(&removed_path)?;
                    save_account_file(&merged)?;
                    refresh_summary(&mut index, &merged);
                    index
                        .accounts
                        .retain(|s| s.id == merged.id || !old_ids.contains(&s.id));
                    index.identity_revision = index.identity_revision.wrapping_add(1);
                    save_account_index(&index)?;
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
    summary: &crate::models::windsurf::WindsurfAccountSummary,
    github_id: u64,
    api_key: Option<&str>,
) -> bool {
    if let Some(identity) = summary.identity() {
        identity_duplicates(
            identity,
            &QueryIdentity {
                version: 1,
                subject: Some(github_id.to_string()),
                credential_digest: fingerprint(api_key),
                platform: "windsurf".into(),
                email: None,
            },
        )
    } else {
        load_account(&summary.id).is_some_and(|account| {
            account.github_id == github_id
                && match (api_key, resolve_account_api_key(&account)) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                }
        })
    }
}

#[cfg(test)]
mod query_identity_tests {
    use super::*;
    #[test]
    fn api_key_and_github_identity_conflicts_are_not_merged() {
        let a = QueryIdentity {
            subject: Some("42".into()),
            credential_digest: fingerprint(Some("key-a")),
            ..Default::default()
        };
        let mut b = a.clone();
        b.credential_digest = fingerprint(Some("key-b"));
        assert!(!identity_duplicates(&a, &b));
        b.credential_digest = None;
        assert!(identity_duplicates(&a, &b));
        b.subject = Some("43".into());
        assert!(!identity_duplicates(&a, &b));
    }
}
