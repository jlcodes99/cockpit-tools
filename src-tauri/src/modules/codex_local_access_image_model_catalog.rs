fn normalize_image_model_id(raw: &str) -> Result<String, String> {
    let model = raw.trim();
    if model.is_empty() {
        return Err("codex.localAccess.imageGenerationModel.required".into());
    }
    if model.chars().count() > 200 {
        return Err("codex.localAccess.imageGenerationModel.tooLong".into());
    }
    if model
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err("codex.localAccess.imageGenerationModel.invalid".into());
    }
    Ok(model.to_string())
}

fn normalize_image_model_ids(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter_map(|model| normalize_image_model_id(&model).ok())
        .filter(|model| seen.insert(model.clone()))
        .collect()
}

fn merge_discovered_image_models(
    collection: &mut CodexLocalAccessCollection,
    discovered: Vec<String>,
) -> bool {
    let mut changed = false;
    let deleted = collection
        .image_generation_deleted_models
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut known = collection
        .image_generation_models
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    for model in normalize_image_model_ids(discovered) {
        if !deleted.contains(&model) && known.insert(model.clone()) {
            collection.image_generation_models.push(model);
            changed = true;
        }
    }
    changed
}

fn sanitize_image_model_catalog(collection: &mut CodexLocalAccessCollection) -> bool {
    let before_models = collection.image_generation_models.clone();
    let before_deleted = collection.image_generation_deleted_models.clone();
    collection.image_generation_models = normalize_image_model_ids(before_models.clone());
    collection.image_generation_deleted_models = normalize_image_model_ids(before_deleted.clone());
    collection
        .image_generation_models
        .retain(|model| !collection.image_generation_deleted_models.contains(model));
    // Old account-level full variants become candidates only. Tombstones always win over
    // discovery/migration; neither path changes the user's current selected model.
    let candidates = collection
        .image_generation_account_models
        .values()
        .cloned()
        .chain(std::iter::once(collection.image_generation_model.clone()))
        .collect();
    merge_discovered_image_models(collection, candidates);
    before_models != collection.image_generation_models
        || before_deleted != collection.image_generation_deleted_models
}

fn update_image_model_catalog(
    collection: &mut CodexLocalAccessCollection,
    discovered: Option<Vec<String>>,
    add_model: Option<String>,
    remove_model: Option<String>,
) -> Result<bool, String> {
    let add = add_model
        .as_deref()
        .map(normalize_image_model_id)
        .transpose()?;
    let remove = remove_model
        .as_deref()
        .map(normalize_image_model_id)
        .transpose()?;
    if remove
        .as_deref()
        .is_some_and(|model| image_model_is_declared_in_use(collection, model))
    {
        return Err("codex.localAccess.imageGenerationModel.deleteSelected".into());
    }
    let mut changed = sanitize_image_model_catalog(collection);
    if let Some(discovered) = discovered {
        changed |= merge_discovered_image_models(collection, discovered);
    }
    if let Some(model) = add {
        let count = collection.image_generation_deleted_models.len();
        collection
            .image_generation_deleted_models
            .retain(|deleted| deleted != &model);
        changed |= count != collection.image_generation_deleted_models.len();
        changed |= merge_discovered_image_models(collection, vec![model]);
    }
    if let Some(model) = remove {
        let count = collection.image_generation_models.len();
        collection
            .image_generation_models
            .retain(|value| value != &model);
        changed |= count != collection.image_generation_models.len();
        if !collection.image_generation_deleted_models.contains(&model) {
            collection.image_generation_deleted_models.push(model);
            changed = true;
        }
    }
    Ok(changed)
}

fn apply_selected_image_model(
    collection: &mut CodexLocalAccessCollection,
    model: &str,
) -> Result<bool, String> {
    let model = normalize_image_model_id(model)?;
    let mut changed = if matches!(model.as_str(), "gpt-image-2" | "gpt-image-2.5") {
        false // A family selection does not restore the deleted bare actual model.
    } else {
        update_image_model_catalog(collection, None, Some(model.clone()), None)?
    };
    changed |= collection.image_generation_model != model;
    collection.image_generation_model = model;
    Ok(changed)
}

fn image_model_is_declared_in_use(collection: &CodexLocalAccessCollection, model: &str) -> bool {
    model == collection.image_generation_model.trim()
        || collection.image_generation_account_ids.iter().any(|id| {
            collection
                .image_generation_account_models
                .get(id)
                .map(|actual| actual.trim())
                .unwrap_or(collection.image_generation_model.trim())
                == model
        })
}

fn copy_image_model_catalog_fields(
    target: &mut CodexLocalAccessCollection,
    source: &CodexLocalAccessCollection,
) {
    target.image_generation_model = source.image_generation_model.clone();
    target.image_generation_models = source.image_generation_models.clone();
    target.image_generation_deleted_models = source.image_generation_deleted_models.clone();
    target.image_generation_model_catalog_revision = source.image_generation_model_catalog_revision;
}

/// Only this field transaction owns image directory changes. It reads the latest file
/// under the ordinary writer's path lock, retaining every unrelated setting.
fn persist_image_model_settings(
    path: &Path,
    fallback: &CodexLocalAccessCollection,
    selected_model: Option<String>,
    main_model: Option<Option<String>>,
    catalog_update: Option<(Option<Vec<String>>, Option<String>, Option<String>)>,
) -> Result<CodexLocalAccessCollection, String> {
    persist_image_model_settings_with_sources(
        path,
        fallback,
        selected_model,
        main_model,
        catalog_update,
        None,
    )
}

fn persist_image_model_settings_with_sources(
    path: &Path,
    fallback: &CodexLocalAccessCollection,
    selected_model: Option<String>,
    main_model: Option<Option<String>>,
    catalog_update: Option<(Option<Vec<String>>, Option<String>, Option<String>)>,
    source_models: Option<Vec<String>>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut document = match current {
            Some(current) => serde_json::from_str::<Value>(current)
                .map_err(|error| format!("读取最新生图型号配置失败: {}", error))?,
            None => serde_json::to_value(fallback).map_err(|error| error.to_string())?,
        };
        let mut latest: CodexLocalAccessCollection = serde_json::from_value(document.clone())
            .map_err(|error| format!("读取最新生图型号配置失败: {}", error))?;
        let normalized_main = normalize_image_generation_main_model(
            latest.image_generation_main_model.clone(),
        ).unwrap_or(None);
        let migrated_main = normalized_main != latest.image_generation_main_model;
        latest.image_generation_main_model = normalized_main;
        let update_main = main_model.is_some() || migrated_main;
        let mut catalog_changed = false;
        if let Some(models) = source_models {
            for model in &models {
                normalize_image_model_id(model)?;
                if !image_model_matches_selection(model, &latest.image_generation_model)
                    || latest
                        .image_generation_deleted_models
                        .iter()
                        .any(|deleted| deleted.trim() == model.trim())
                {
                    return Err("codex.localAccess.imageAccounts.modelRequired".to_string());
                }
            }
            catalog_changed |= merge_discovered_image_models(&mut latest, models);
        }
        if let Some((discovered, add, remove)) = catalog_update {
            catalog_changed |= update_image_model_catalog(&mut latest, discovered, add, remove)?;
        }
        if let Some(model) = selected_model {
            catalog_changed |= apply_selected_image_model(&mut latest, &model)?;
        }
        let mut changed = catalog_changed || migrated_main;
        if let Some(model) = main_model {
            let model = normalize_image_generation_main_model(model)?;
            changed |= latest.image_generation_main_model != model;
            latest.image_generation_main_model = model;
        }
        if catalog_changed {
            latest.image_generation_model_catalog_revision = latest
                .image_generation_model_catalog_revision
                .saturating_add(1);
        }
        if changed {
            latest.updated_at = latest.updated_at.max(now_ms());
        }
        let fields = serde_json::to_value(&latest).map_err(|error| error.to_string())?;
        let object = document.as_object_mut().ok_or("生图型号配置不是对象")?;
        for key in [
            "imageGenerationModel",
            "imageGenerationModels",
            "imageGenerationDeletedModels",
            "imageGenerationModelCatalogRevision",
            "updatedAt",
        ] {
            object.insert(key.to_string(), fields[key].clone());
        }
        if update_main {
            match fields.get("imageGenerationMainModel") {
                Some(value) => {
                    object.insert("imageGenerationMainModel".into(), value.clone());
                }
                None => {
                    object.remove("imageGenerationMainModel");
                }
            }
        }
        let content = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
        published = Some(latest);
        Ok(content)
    })?;
    published.ok_or_else(|| "生图型号事务未返回配置".to_string())
}

fn publish_image_model_settings(
    runtime: &mut GatewayRuntime,
    published: &CodexLocalAccessCollection,
    update_main: bool,
) {
    if let Some(current) = runtime.collection.as_mut() {
        if published.image_generation_model_catalog_revision
            >= current.image_generation_model_catalog_revision
        {
            copy_image_model_catalog_fields(current, published);
        }
        if update_main || normalize_image_generation_main_model(current.image_generation_main_model.clone())
            .unwrap_or(None) != current.image_generation_main_model {
            current.image_generation_main_model = published.image_generation_main_model.clone();
        }
        current.updated_at = current.updated_at.max(published.updated_at);
    }
}

pub async fn update_local_access_image_generation_model_catalog(
    discovered_models: Option<Vec<String>>,
    add_model: Option<String>,
    remove_model: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    // A profile keeps this lifecycle lock from reading the image template through
    // publishing its runtime. Its prepared collection is not yet in the store.
    // Never accept a deletion in that window, or its old manifest could still start.
    // Acquire provider -> image, and fail promptly instead of blocking discovery,
    // image settings or the dialog behind an in-flight gateway launch/reload.
    let provider_guard = if remove_model.is_some() {
        Some(
            provider_gateway_lifecycle_lock()
                .try_lock()
                .map_err(|_| "codex.modelConfig.errors.busy".to_string())?,
        )
    } else {
        None
    };
    let _guard = image_configuration_update_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let collection = gateway_runtime()
        .lock()
        .await
        .collection
        .clone()
        .map(Ok)
        .unwrap_or_else(new_local_access_collection)?;
    if let Some(model) = remove_model.as_deref() {
        let model = normalize_image_model_id(model)?;
        if provider_gateway_runtime_store()
            .lock()
            .await
            .values()
            .any(|runtime| {
                runtime
                    .collection
                    .as_ref()
                    .is_some_and(|profile| image_model_is_declared_in_use(profile, &model))
            })
        {
            return Err("codex.localAccess.imageGenerationModel.deleteSelected".into());
        }
    }
    let path = local_access_file_path()?;
    let published = tauri::async_runtime::spawn_blocking(move || {
        persist_image_model_settings(
            &path,
            &collection,
            None,
            None,
            Some((discovered_models, add_model, remove_model)),
        )
    })
    .await
    .map_err(|error| format!("保存生图型号目录任务失败: {}", error))??;
    let mut runtime = gateway_runtime().lock().await;
    if runtime.collection.is_none() {
        sync_runtime_collection(&mut runtime, published.clone());
    }
    publish_image_model_settings(&mut runtime, &published, false);
    drop(runtime);
    drop(_guard);
    drop(provider_guard);
    snapshot_state_without_gateway_reload().await
}

/// Instance account updates run on the command's blocking worker. The shared lock
/// serializes model/source intent, while the field transaction retains unrelated settings.
pub(crate) fn register_image_generation_source_models(
    collection: &CodexLocalAccessCollection,
    models: Vec<String>,
) -> Result<(), String> {
    if models.is_empty() {
        return Ok(());
    }
    let _guard = image_configuration_update_lock().blocking_lock();
    let published = persist_image_model_settings_with_sources(
        &local_access_file_path()?,
        collection,
        None,
        None,
        None,
        Some(models),
    )?;
    let mut runtime = gateway_runtime().blocking_lock();
    if runtime.collection.is_none() {
        sync_runtime_collection(&mut runtime, published.clone());
    }
    publish_image_model_settings(&mut runtime, &published, false);
    Ok(())
}
