// The member is always the original account. Independent authorization is a
// selected credential, never an account replacement in business configuration.
pub async fn save_local_access_accounts_with_official_bindings(
    account_ids: Vec<String>,
    restrict_free_accounts: bool,
    backup_account_ids: Option<Vec<String>>,
    preferred_account_ids: Option<Vec<String>>,
    session_affinity: Option<bool>,
    session_affinity_ttl_ms: Option<i64>,
    image_generation_account_policies: Option<
        HashMap<String, CodexLocalAccessImageGenerationPolicy>,
    >,
    official_account_bindings: Option<HashMap<String, String>>,
    session_identity_mapping_enabled: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let original = gateway_runtime()
        .lock()
        .await
        .collection
        .clone()
        .map(Ok)
        .unwrap_or_else(new_local_access_collection)?;
    let account_ids = normalize_account_id_list(account_ids);
    if account_ids.iter().any(|id| {
        id.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
            && !(original.account_ids.contains(id) && service_account_paused(&original, id))
    }) {
        return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
    }
    if official_account_bindings
        .as_ref()
        .is_some_and(|bindings| bindings.keys().any(|id| !account_ids.contains(id)))
    {
        return Err("CHATGPT_PLAN_SOURCE_INVALID".into());
    }
    let mut bindings =
        official_account_bindings.unwrap_or_else(|| original.official_account_bindings.clone());
    bindings.retain(|id, _| account_ids.contains(id));
    if !bindings.is_empty() && original.gateway_mode != CodexLocalAccessGatewayMode::Native {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    let mut snapshots = HashMap::new();
    for (source, registration) in &bindings {
        if original.account_ids.contains(source)
            && original.official_account_bindings.get(source) == Some(registration)
        {
            continue;
        }
        crate::modules::chatgpt_plan::access_token(registration, false).await?;
        snapshots.insert(
            source.clone(),
            crate::modules::chatgpt_plan::validate_source_binding_snapshot(registration, source)
                .await?,
        );
    }
    let removed = original
        .account_ids
        .iter()
        .filter(|id| !account_ids.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    crate::modules::chatgpt_plan_service::remember_modes_before_removal(
        &original.account_ids,
        &removed,
    )
    .await?;
    let changes = bindings
        .iter()
        .filter(|(source, registration)| {
            original.official_account_bindings.get(*source) != Some(*registration)
        })
        .map(|(source, registration)| (source.clone(), registration.clone()))
        .collect::<Vec<_>>();
    let local_changes = original
        .official_account_bindings
        .iter()
        .filter(|(source, _)| account_ids.contains(source) && !bindings.contains_key(*source))
        .map(|(source, registration)| (source.clone(), registration.clone()))
        .collect::<Vec<_>>();
    let expected_revision = original.chatgpt_plan_service_revision;
    // Slow credential renewal above never holds the service mutation lock.
    let _guard = image_configuration_update_lock().lock().await;
    let next = tokio::task::spawn_blocking(move || {
        let path = local_access_file_path()?;
        let mut published = None;
        update_string_atomic(&path, |current| {
            let mut next: CodexLocalAccessCollection = match current {
                Some(raw) => {
                    serde_json::from_str(raw).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?
                }
                None => original.clone(),
            };
            if next.account_ids != original.account_ids
                || next.official_account_bindings != original.official_account_bindings
                || next.paused_account_ids != original.paused_account_ids
            {
                return Err("CHATGPT_PLAN_SERVICE_CHANGED".into());
            }
            if next.chatgpt_plan_service_revision != expected_revision {
                return Err("CHATGPT_PLAN_SERVICE_CHANGED".into());
            }
            for id in &account_ids {
                if id.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                    && next.account_ids.contains(id)
                    && service_account_paused(&next, id)
                {
                    continue;
                }
                let account =
                    codex_account::load_account(id).ok_or("CHATGPT_PLAN_SOURCE_INVALID")?;
                if restrict_free_accounts
                    && account
                        .plan_type
                        .as_deref()
                        .is_some_and(|plan| plan.eq_ignore_ascii_case("free"))
                {
                    return Err("CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE".into());
                }
                if !bindings.contains_key(id)
                    && !service_account_paused(&next, id)
                    && !is_local_access_eligible_account(&account, restrict_free_accounts)
                {
                    return Err("CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE".into());
                }
                if !(next.account_ids.contains(id)
                    && next.official_account_bindings.get(id) == bindings.get(id))
                    && next
                        .service_account_identity_fingerprints
                        .get(id)
                        .is_some_and(|expected| {
                            crate::modules::chatgpt_plan::source_identity_fingerprint(&account)
                                .as_ref()
                                != Ok(expected)
                        })
                {
                    return Err("CHATGPT_PLAN_SOURCE_CHANGED".into());
                }
                if bindings.contains_key(id)
                    && !next.service_account_identity_fingerprints.contains_key(id)
                {
                    next.service_account_identity_fingerprints.insert(
                        id.clone(),
                        crate::modules::chatgpt_plan::source_identity_fingerprint(&account)?,
                    );
                }
                if bindings.contains_key(id)
                    && (account.is_api_key_auth()
                        || account.is_agent_identity_auth()
                        || account.is_web_session_auth())
                {
                    return Err("CHATGPT_PLAN_SOURCE_INVALID".into());
                }
            }
            for snapshot in snapshots.values() {
                crate::modules::chatgpt_plan::validate_source_binding_snapshot_for_commit(
                    snapshot,
                )?;
            }
            let removed = next
                .account_ids
                .iter()
                .filter(|id| !account_ids.contains(id))
                .cloned()
                .collect::<HashSet<_>>();
            remove_account_refs_from_collection(&mut next, &removed);
            next.account_ids = account_ids.clone();
            next.official_account_bindings = bindings.clone();
            next.paused_account_ids
                .retain(|id| account_ids.contains(id) && !snapshots.contains_key(id));
            next.restrict_free_accounts = restrict_free_accounts;
            if let Some(enabled) = session_identity_mapping_enabled {
                next.session_identity_mapping_enabled = enabled;
            }
            if let Some(policies) = &image_generation_account_policies {
                for (id, policy) in policies {
                    if account_ids.contains(id) {
                        next.image_generation_account_policies
                            .insert(id.clone(), *policy);
                    }
                }
            }
            if let Some(value) = session_affinity {
                next.session_affinity = value;
                next.session_affinity_default_enabled_migrated = true;
            }
            if let Some(value) = session_affinity_ttl_ms {
                next.session_affinity_ttl_ms =
                    value.clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
            }
            apply_account_usage_priority_ids(
                &mut next,
                backup_account_ids.as_deref(),
                preferred_account_ids.as_deref(),
            );
            enforce_chatgpt_plan_image_policy(&mut next, &[]);
            native_validate_chatgpt_plan_mode(&next, next.gateway_mode)?;
            next.chatgpt_plan_service_revision = next
                .chatgpt_plan_service_revision
                .checked_add(1)
                .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
            next.updated_at = now_ms();
            let content =
                serde_json::to_string_pretty(&next).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            published = Some(next);
            Ok(content)
        })?;
        let next = published.ok_or("CHATGPT_PLAN_STORAGE_FAILED")?;
        let mut runtime = gateway_runtime().blocking_lock();
        if runtime.collection.as_ref().is_none_or(|current| {
            current.chatgpt_plan_service_revision < next.chatgpt_plan_service_revision
        }) {
            sync_runtime_collection(&mut runtime, next.clone());
        }
        Ok::<_, String>(next)
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")??;
    emit_local_access_state_updated();
    if !removed.is_empty() {
        restore_removed_local_access_accounts(&removed).await;
    }
    drop(_guard);
    for (source, registration, mode) in changes
        .into_iter()
        .map(|(s, r)| (s, r, "official"))
        .chain(local_changes.into_iter().map(|(s, r)| (s, r, "codex")))
    {
        if let Err(error) =
            crate::modules::chatgpt_plan::remember_service_mode(&registration, Some(&source), mode)
                .await
        {
            logger::log_warn(&format!(
                "[ChatGPT Plan] Member save committed; preference update deferred: {error}"
            ));
        }
        if mode == "official" {
            crate::modules::chatgpt_plan_service::refresh_catalog_in_background(&registration);
        }
    }
    if next.enabled {
        trigger_gateway_reload_in_background("API service members updated");
    }
    snapshot_state_without_gateway_reload().await
}

/// Background-only migration. A persisted alias alone never transfers an ACL:
/// the independent grant must still prove the same source identity at commit.
pub(crate) async fn migrate_legacy_official_member(id: &str, source: &str) -> Result<(), String> {
    let projection = codex_account::chatgpt_plan_projection_id(id)?;
    let (_, ids) = chatgpt_plan_pool_snapshot().await?;
    if !ids.contains(&projection) {
        return Ok(());
    }
    let association = match crate::modules::chatgpt_plan::validate_source_association_snapshot(
        id, source,
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            pause_chatgpt_plan_service_endpoint(&projection).await?;
            return Err(error);
        }
    };
    let _guard = image_configuration_update_lock().lock().await;
    let source = source.to_owned();
    let id = id.to_owned();
    let changed = tokio::task::spawn_blocking(move || {
        let path = local_access_file_path()?;
        let mut published = None;
        update_string_atomic(&path, |current| {
            let collection: CodexLocalAccessCollection =
                serde_json::from_str(current.ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?)
                    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            if !collection.account_ids.contains(&projection) {
                return Ok(current.unwrap().into());
            }
            if collection.account_ids.contains(&source)
                && collection.official_account_bindings.get(&source) != Some(&id)
            {
                // A newer canonical choice wins. Isolate the ambiguous legacy
                // endpoint without replacing membership, ACL, or its selection.
                if collection.paused_account_ids.contains(&projection) {
                    return Ok(current.unwrap().into());
                }
                let mut next = collection.clone();
                next.paused_account_ids.push(projection.clone());
                next.chatgpt_plan_service_revision = next
                    .chatgpt_plan_service_revision
                    .checked_add(1)
                    .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
                let content = serde_json::to_string_pretty(&next)
                    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
                published = Some(next);
                return Ok(content);
            }
            crate::modules::chatgpt_plan::validate_source_association_snapshot_for_commit(
                &association,
            )?;
            let account =
                codex_account::load_account(&source).ok_or("CHATGPT_PLAN_SOURCE_INVALID")?;
            let old = codex_account::load_account(&projection);
            let paused = collection.paused_account_ids.contains(&projection);
            let mut next = switched_chatgpt_plan_collection_intent(
                &collection,
                &account,
                old.as_ref(),
                Some(&projection),
                false,
                true,
                true,
            )?;
            next.official_account_bindings
                .entry(source.clone())
                .or_insert_with(|| id.clone());
            next.service_account_identity_fingerprints.insert(
                source.clone(),
                crate::modules::chatgpt_plan::source_identity_fingerprint(&account)?,
            );
            if !paused {
                next.paused_account_ids.retain(|entry| entry != &source);
            }
            // Every request revalidates the grant; an expired/removed grant
            // remains unavailable and can never fall through to source OAuth.
            let content =
                serde_json::to_string_pretty(&next).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            published = Some(next);
            Ok(content)
        })?;
        let changed = published.is_some();
        if let Some(next) = published {
            let mut runtime = gateway_runtime().blocking_lock();
            if runtime.collection.as_ref().is_none_or(|c| {
                c.chatgpt_plan_service_revision < next.chatgpt_plan_service_revision
            }) {
                sync_runtime_collection(&mut runtime, next);
            }
        }
        Ok::<_, String>(changed)
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")??;
    if changed {
        emit_local_access_state_updated();
    }
    Ok(())
}
