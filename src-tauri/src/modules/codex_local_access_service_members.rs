// Membership is durable user intent; routability is a separate, reversible state.
fn service_account_paused(collection: &CodexLocalAccessCollection, id: &str) -> bool {
    collection
        .paused_account_ids
        .iter()
        .any(|paused| paused == id)
}

fn active_service_account_ids(
    collection: &CodexLocalAccessCollection,
    ids: Vec<String>,
) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !service_account_paused(collection, id))
        .collect()
}

fn service_member_snapshot(
    collection: &CodexLocalAccessCollection,
) -> Vec<crate::models::codex_local_access::CodexLocalAccessServiceMember> {
    collection
        .account_ids
        .iter()
        .map(
            |id| crate::models::codex_local_access::CodexLocalAccessServiceMember {
                source_account_id: collection
                    .official_account_source_ids
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.clone()),
                account_id: id.clone(),
                mode: if collection.official_account_bindings.contains_key(id)
                    || id.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                {
                    "official"
                } else {
                    "codex"
                }
                .into(),
                paused: service_account_paused(collection, id),
            },
        )
        .collect()
}

/// Blocking, display-only migration. Never adds an account or grants a scope.
pub(crate) fn remember_chatgpt_plan_service_origin(id: &str, source: &str) -> Result<(), String> {
    let projection = codex_account::chatgpt_plan_projection_id(id)?;
    let path = local_access_file_path()?;
    let mut published = None;
    update_string_atomic(&path, |current| {
        let current = current.ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
        let mut collection: CodexLocalAccessCollection =
            serde_json::from_str(current).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
        if !collection.account_ids.contains(&projection) { return Ok(current.into()); }
        if let Some(existing) = collection.official_account_source_ids.get(&projection) {
            return if existing == source {
                Ok(current.into())
            } else {
                Err("CHATGPT_PLAN_BINDING_CONFLICT".into())
            };
        }
        collection
            .official_account_source_ids
            .insert(projection.clone(), source.to_owned());
        collection.chatgpt_plan_service_revision = collection
            .chatgpt_plan_service_revision
            .checked_add(1)
            .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
        let content =
            serde_json::to_string_pretty(&collection).map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
        published = Some(collection);
        Ok(content)
    })?;
    if let Some(collection) = published {
        let mut runtime = gateway_runtime().blocking_lock();
        if runtime.collection.as_ref().is_some_and(|current| {
            current.chatgpt_plan_service_revision < collection.chatgpt_plan_service_revision
        }) {
            sync_runtime_collection(&mut runtime, collection);
        }
        drop(runtime);
        emit_local_access_state_updated();
    }
    Ok(())
}

pub(crate) async fn chatgpt_plan_state_snapshot() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    snapshot_state_without_gateway_reload().await
}

/// Safety path for a missing/replaced source: preserve the old endpoint and its
/// scopes, but never transfer those grants to a different principal.
pub(crate) async fn pause_chatgpt_plan_service_endpoint(
    id: &str,
) -> Result<CodexLocalAccessState, String> {
    let _guard = image_configuration_update_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let id = id.to_owned();
    let changed = tokio::task::spawn_blocking(move || {
        let path = local_access_file_path()?;
        let mut next = None;
        update_string_atomic(&path, |current| {
            let mut collection: CodexLocalAccessCollection =
                serde_json::from_str(current.ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?)
                    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            if !collection.account_ids.contains(&id) || collection.paused_account_ids.contains(&id)
            {
                return Ok(current.unwrap().into());
            }
            if collection.account_ids.contains(&id) && !collection.paused_account_ids.contains(&id)
            {
                collection.paused_account_ids.push(id.clone());
                collection.chatgpt_plan_service_revision = collection
                    .chatgpt_plan_service_revision
                    .checked_add(1)
                    .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
                collection.updated_at = now_ms();
            }
            let content = serde_json::to_string_pretty(&collection)
                .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            next = Some(collection);
            Ok(content)
        })?;
        let Some(collection) = next else {
            return Ok::<_, String>(false);
        };
        let mut runtime = gateway_runtime().blocking_lock();
        if runtime.collection.as_ref().is_none_or(|current| {
            current.chatgpt_plan_service_revision <= collection.chatgpt_plan_service_revision
        }) {
            sync_runtime_collection(&mut runtime, collection);
        }
        Ok::<_, String>(true)
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")??;
    if changed {
        emit_local_access_state_updated();
    }
    snapshot_state_without_gateway_reload().await
}

pub(crate) async fn chatgpt_plan_selected_bindings() -> Result<HashMap<String, String>, String> {
    ensure_runtime_loaded_without_start().await?;
    Ok(gateway_runtime()
        .lock()
        .await
        .collection
        .as_ref()
        .map(|c| c.official_account_bindings.clone())
        .unwrap_or_default())
}

/// Remove only this selected credential. Membership and its explicit ACL remain
/// unchanged, including when the source identity can no longer be validated.
pub(crate) async fn pause_official_service_binding(
    source: &str,
    registration: &str,
) -> Result<(), String> {
    let _guard = image_configuration_update_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let source = source.to_owned();
    let registration = registration.to_owned();
    tokio::task::spawn_blocking(move || {
        let path = local_access_file_path()?;
        let mut published = None;
        update_string_atomic(&path, |current| {
            let mut collection: CodexLocalAccessCollection =
                serde_json::from_str(current.ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?)
                    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            if collection.official_account_bindings.get(&source) != Some(&registration) {
                return Ok(current.unwrap().into());
            }
            collection.official_account_bindings.remove(&source);
            if collection.account_ids.contains(&source)
                && !collection.paused_account_ids.contains(&source)
            {
                collection.paused_account_ids.push(source.clone());
            }
            collection.chatgpt_plan_service_revision = collection
                .chatgpt_plan_service_revision
                .checked_add(1)
                .ok_or("CHATGPT_PLAN_SERVICE_CHANGED")?;
            collection.updated_at = now_ms();
            let content = serde_json::to_string_pretty(&collection)
                .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")?;
            published = Some(collection);
            Ok(content)
        })?;
        if let Some(collection) = published {
            let mut runtime = gateway_runtime().blocking_lock();
            if runtime.collection.as_ref().is_none_or(|c| {
                c.chatgpt_plan_service_revision < collection.chatgpt_plan_service_revision
            }) {
                sync_runtime_collection(&mut runtime, collection);
            }
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE_FAILED")??;
    emit_local_access_state_updated();
    Ok(())
}
