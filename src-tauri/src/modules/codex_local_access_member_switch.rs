// Explicit authorization changes affect only this service's member references.
// Credential files, provider profiles and response/checkpoint ownership stay separate.
fn switch_member_ids(ids: &mut Vec<String>, old: &str, new: &str) {
    for id in ids.iter_mut() {
        if id == old {
            *id = new.to_owned();
        }
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
}

fn switched_chatgpt_plan_collection(
    current: &CodexLocalAccessCollection,
    source: &CodexAccount,
    projection: Option<&CodexAccount>,
    projection_id: Option<&str>,
    official: bool,
) -> Result<CodexLocalAccessCollection, String> {
    switched_chatgpt_plan_collection_intent(
        current,
        source,
        projection,
        projection_id,
        official,
        false,
        false,
    )
}

fn switched_chatgpt_plan_collection_intent(
    current: &CodexLocalAccessCollection,
    source: &CodexAccount,
    projection: Option<&CodexAccount>,
    projection_id: Option<&str>,
    official: bool,
    require_existing: bool,
    paused: bool,
) -> Result<CodexLocalAccessCollection, String> {
    if source.is_api_key_auth() || source.is_agent_identity_auth() || source.is_web_session_auth() {
        return Err("CHATGPT_PLAN_SOURCE_INVALID".into());
    }
    if official && current.gateway_mode != CodexLocalAccessGatewayMode::Native {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    if official && current.restrict_free_accounts && is_free_plan_type(source.plan_type.as_deref())
    {
        return Err("CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE".into());
    }
    let legacy_member =
        projection_id.is_some_and(|id| current.account_ids.iter().any(|member| member == id));
    if require_existing && !current.account_ids.contains(&source.id) && !legacy_member {
        if current
            .account_ids
            .iter()
            .any(|id| current.official_account_source_ids.get(id) == Some(&source.id))
        {
            return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
        }
        return Err("CHATGPT_PLAN_SERVICE_MEMBER_REMOVED".into());
    }
    if !official
        && !paused
        && !is_local_access_eligible_account(source, current.restrict_free_accounts)
    {
        return Err("CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE".into());
    }
    if !paused
        && current
            .service_account_identity_fingerprints
            .get(&source.id)
            .is_some_and(|expected| {
                crate::modules::chatgpt_plan::source_identity_fingerprint(source).as_ref()
                    != Ok(expected)
            })
    {
        return Err("CHATGPT_PLAN_SOURCE_CHANGED".into());
    }
    // Only a caller holding an actual binding snapshot can reach this legacy
    // transfer. Ordinary mode changes keep every business identity unchanged.
    let mut next = if legacy_member {
        legacy_switched_chatgpt_plan_collection_intent(
            current,
            source,
            projection,
            projection_id,
            false,
            true,
            true,
        )?
    } else {
        current.clone()
    };
    if !next.account_ids.contains(&source.id) {
        next.account_ids.push(source.id.clone());
    }
    next.paused_account_ids
        .retain(|id| id != &source.id && Some(id.as_str()) != projection_id);
    if paused {
        next.paused_account_ids.push(source.id.clone());
    }
    if official {
        let registration = projection_id
            .and_then(|id| id.strip_prefix(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX))
            .ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
        uuid::Uuid::parse_str(registration).map_err(|_| "CHATGPT_PLAN_INVALID_ID")?;
        next.official_account_bindings
            .insert(source.id.clone(), registration.into());
        next.service_account_identity_fingerprints.insert(
            source.id.clone(),
            crate::modules::chatgpt_plan::source_identity_fingerprint(source)?,
        );
    } else {
        next.official_account_bindings.remove(&source.id);
        if let Some(policy) = next.chatgpt_plan_codex_image_policies.remove(&source.id) {
            next.image_generation_account_policies
                .insert(source.id.clone(), policy);
        }
    }
    if legacy_member {
        if let Some(projection) = projection_id {
            next.official_account_source_ids
                .entry(projection.into())
                .or_insert_with(|| source.id.clone());
        }
    }
    enforce_chatgpt_plan_image_policy(&mut next, &[]);
    native_validate_chatgpt_plan_mode(&next, next.gateway_mode)?;
    next.chatgpt_plan_service_revision = current
        .chatgpt_plan_service_revision
        .checked_add(1)
        .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
    next.updated_at = now_ms();
    Ok(next)
}

fn legacy_switched_chatgpt_plan_collection_intent(
    current: &CodexLocalAccessCollection,
    source: &CodexAccount,
    projection: Option<&CodexAccount>,
    projection_id: Option<&str>,
    official: bool,
    require_existing: bool,
    paused: bool,
) -> Result<CodexLocalAccessCollection, String> {
    if projection_id.is_none()
        && !current.account_ids.contains(&source.id)
        && current
            .account_ids
            .iter()
            .any(|id| current.official_account_source_ids.get(id) == Some(&source.id))
    {
        // Historical display provenance is not a transferable authorization.
        return Err("CHATGPT_PLAN_BINDING_REQUIRED".into());
    }
    if require_existing
        && !current
            .account_ids
            .iter()
            .any(|id| id == &source.id || Some(id.as_str()) == projection_id)
    {
        return Err("CHATGPT_PLAN_SERVICE_MEMBER_REMOVED".into());
    }
    if source.is_api_key_auth() || source.is_agent_identity_auth() || source.is_web_session_auth() {
        return Err("CHATGPT_PLAN_SOURCE_INVALID".into());
    }
    if official && current.gateway_mode != CodexLocalAccessGatewayMode::Native {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    let target = if official {
        let projection = projection.ok_or("CHATGPT_PLAN_ACCOUNT_NOT_FOUND")?;
        if !codex_account::is_chatgpt_plan_account(projection)
            || Some(projection.id.as_str()) != projection_id
        {
            return Err("CHATGPT_PLAN_BINDING_MISMATCH".into());
        }
        projection
    } else {
        source
    };
    if !paused && !is_local_access_eligible_account(target, current.restrict_free_accounts) {
        return Err("CHATGPT_PLAN_SERVICE_ACCOUNT_INELIGIBLE".into());
    }
    let opposite = if official {
        Some(source.id.as_str())
    } else {
        projection_id
    };
    let old = opposite.filter(|id| current.account_ids.iter().any(|member| member == *id));
    let mut next = current.clone();
    if let Some(projection_id) = projection_id {
        if next
            .official_account_source_ids
            .get(projection_id)
            .is_some_and(|old| old != &source.id)
        {
            return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
        }
        next.official_account_source_ids
            .insert(projection_id.into(), source.id.clone());
    }
    next.paused_account_ids
        .retain(|id| id != &source.id && Some(id.as_str()) != projection_id);
    if paused {
        next.paused_account_ids.push(target.id.clone());
    }
    if let Some(old) = old {
        switch_member_ids(&mut next.account_ids, old, &target.id);
        // Explicit key scopes grant this logical member, not unrelated members.
        // A profile/provider key is a separate authorization boundary.
        for key in &mut next.api_keys {
            if key.provider_gateway.is_some() {
                continue;
            }
            switch_member_ids(&mut key.account_ids, old, &target.id);
            switch_member_ids(&mut key.priority_account_ids, old, &target.id);
            if key.preferred_account_id.as_deref() == Some(old) {
                key.preferred_account_id = Some(target.id.clone());
            }
            // Deliberately leave model_routing provider identities unchanged.
        }
        let target_has_rule = next
            .custom_routing_rules
            .iter()
            .any(|rule| rule.account_id == target.id);
        if target_has_rule {
            next.custom_routing_rules
                .retain(|rule| rule.account_id != old);
        } else {
            for rule in &mut next.custom_routing_rules {
                if rule.account_id == old {
                    rule.account_id = target.id.clone();
                }
            }
        }
        // Exclusions may only become stricter when legacy dual members collide.
        for rule in &mut next.account_model_rules {
            if rule.account_id == old {
                rule.account_id = target.id.clone();
            }
        }
        next.account_model_rules =
            normalize_account_model_rules(next.account_model_rules, &next.account_ids);
        // Capabilities are credential/transport specific; never copy a Codex
        // supported declaration to the independent official credential.
        prune_capability_fallback_references(&mut next);
    } else if !next.account_ids.contains(&target.id) {
        next.account_ids.push(target.id.clone());
    }
    if official {
        if current.account_ids.contains(&source.id) {
            next.chatgpt_plan_codex_image_policies
                .entry(source.id.clone())
                .or_insert_with(|| {
                    current
                        .image_generation_account_policies
                        .get(&source.id)
                        .copied()
                        .unwrap_or_default()
                });
        }
        // Keep an independently selected Codex image source and its original
        // policy. Only the official conversation member is image-disabled.
        if !next.image_generation_account_ids.contains(&source.id) {
            next.image_generation_account_policies.remove(&source.id);
        }
        next.image_generation_account_policies.insert(
            target.id.clone(),
            CodexLocalAccessImageGenerationPolicy::Disabled,
        );
    } else {
        if let Some(policy) = next.chatgpt_plan_codex_image_policies.remove(&source.id) {
            next.image_generation_account_policies
                .insert(source.id.clone(), policy);
        }
        if let Some(projection_id) = projection_id {
            next.image_generation_account_policies.remove(projection_id);
        }
    }
    // bound_oauth_account_id, image-source IDs and provider routes retain their
    // actual OAuth identity; no client auth/config or response owner is aliased.
    enforce_chatgpt_plan_image_policy(&mut next, &[]);
    native_validate_chatgpt_plan_mode(&next, next.gateway_mode)?;
    next.chatgpt_plan_service_revision = current
        .chatgpt_plan_service_revision
        .checked_add(1)
        .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
    next.updated_at = now_ms();
    Ok(next)
}

fn commit_chatgpt_plan_member_switch(
    source_id: &str,
    projection_id: Option<&str>,
    official: bool,
    binding_snapshot: Option<&crate::modules::chatgpt_plan::SourceBindingSnapshot>,
    association_snapshot: Option<&crate::modules::chatgpt_plan::SourceAssociationSnapshot>,
    require_existing: bool,
    paused: bool,
) -> Result<bool, String> {
    const MAX_ATTEMPTS: usize = 8;
    for _ in 0..MAX_ATTEMPTS {
        let base = gateway_runtime()
            .blocking_lock()
            .collection
            .clone()
            .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
        let fingerprint = collection_content_fingerprint(&base)?;
        let Some((path, expected_hash)) = collection_disk_snapshot_for_cas(&fingerprint)? else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        let source = codex_account::load_account(source_id)
            .filter(|account| account.id == source_id)
            .ok_or("CHATGPT_PLAN_SOURCE_INVALID")?;
        let projection = projection_id.and_then(codex_account::load_account);
        let next = switched_chatgpt_plan_collection_intent(
            &base,
            &source,
            projection.as_ref(),
            projection_id,
            official,
            require_existing,
            paused,
        )?;
        if official {
            crate::modules::chatgpt_plan::validate_source_binding_snapshot_for_commit(
                binding_snapshot.ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?,
            )?;
        } else if projection_id.is_some() {
            crate::modules::chatgpt_plan::validate_source_association_snapshot_for_commit(
                association_snapshot.ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?,
            )?;
        }
        let content =
            serde_json::to_string_pretty(&next).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
        let enabled_non_native =
            next.enabled && next.gateway_mode != CodexLocalAccessGatewayMode::Native;
        let runtime_matches = {
            let runtime = gateway_runtime().blocking_lock();
            let current = runtime
                .collection
                .as_ref()
                .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
            collection_content_fingerprint(current)? == fingerprint
        };
        // Disk locking, hashing and fsync stay on this worker, outside the
        // runtime lock. A disk-first writer changes the CAS hash; a stale
        // whole-snapshot writer is rejected by the service revision guard.
        let committed = runtime_matches
            && write_string_atomic_if_hash_matches(&path, expected_hash, || Ok(content))?;
        if committed {
            let mut runtime = gateway_runtime().blocking_lock();
            // A newer field transaction may already have read and published
            // the committed disk revision before this worker reacquired the
            // runtime lock. Do not overwrite that newer publication.
            if runtime.collection.as_ref().is_none_or(|current| {
                current.chatgpt_plan_service_revision < next.chatgpt_plan_service_revision
            }) {
                sync_runtime_collection(&mut runtime, next);
            }
            drop(runtime);
            logger::log_codex_api_info(&format!(
                "API service authorization member switched: source_id={}, target_id={}, mode={}",
                source_id,
                if official {
                    projection_id.unwrap_or_default()
                } else {
                    source_id
                },
                if official { "official" } else { "codex" },
            ));
            return Ok(enabled_non_native);
        }
        std::thread::yield_now();
    }
    Err("CHATGPT_PLAN_SERVICE_CHANGED".into())
}

pub(crate) async fn switch_chatgpt_plan_service_member(
    source_id: &str,
    projection_id: Option<&str>,
    official: bool,
    binding_snapshot: Option<crate::modules::chatgpt_plan::SourceBindingSnapshot>,
    association_snapshot: Option<crate::modules::chatgpt_plan::SourceAssociationSnapshot>,
) -> Result<CodexLocalAccessState, String> {
    switch_chatgpt_plan_service_member_intent(
        source_id,
        projection_id,
        official,
        binding_snapshot,
        association_snapshot,
        false,
        false,
    )
    .await
}

pub(crate) async fn switch_chatgpt_plan_service_member_intent(
    source_id: &str,
    projection_id: Option<&str>,
    official: bool,
    binding_snapshot: Option<crate::modules::chatgpt_plan::SourceBindingSnapshot>,
    association_snapshot: Option<crate::modules::chatgpt_plan::SourceAssociationSnapshot>,
    require_existing: bool,
    paused: bool,
) -> Result<CodexLocalAccessState, String> {
    let _guard = image_configuration_update_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let source_id = source_id.to_owned();
    let projection_id = projection_id.map(str::to_owned);
    let reload_non_native = tokio::task::spawn_blocking(move || {
        commit_chatgpt_plan_member_switch(
            &source_id,
            projection_id.as_deref(),
            official,
            binding_snapshot.as_ref(),
            association_snapshot.as_ref(),
            require_existing,
            paused,
        )
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")??;
    // Publish the committed membership even if a subsequent preference write
    // fails; observers must see the actual transaction result.
    emit_local_access_state_updated();
    // Native reads its collection for every request. Keep its listener and
    // existing technical owner/checkpoint state running without a restart.
    if reload_non_native {
        trigger_gateway_reload_in_background("API 服务 Codex 授权成员更新");
    }
    snapshot_state_without_gateway_reload().await
}

#[cfg(test)]
#[path = "codex_local_access_member_switch_tests.rs"]
mod member_switch_tests;
