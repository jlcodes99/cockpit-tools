// Credential edits change account IDs; preserve scopes without starting a service.
fn replace_collection_account_references(
    collection: &mut CodexLocalAccessCollection,
    old_id: &str,
    new_id: &str,
) -> bool {
    fn replace(value: &mut String, old: &str, new: &str) -> bool {
        if value.trim() != old {
            return false;
        }
        *value = new.to_string();
        true
    }
    fn replace_list(values: &mut [String], old: &str, new: &str) -> bool {
        values
            .iter_mut()
            .fold(false, |changed, value| replace(value, old, new) | changed)
    }
    let mut changed = replace_list(&mut collection.account_ids, old_id, new_id);
    changed |= replace_list(&mut collection.image_generation_account_ids, old_id, new_id);
    if let Some(id) = &mut collection.bound_oauth_account_id {
        changed |= replace(id, old_id, new_id);
    }
    if let Some(policy) = collection.image_generation_account_policies.remove(old_id) {
        collection
            .image_generation_account_policies
            .insert(new_id.to_string(), policy);
        changed = true;
    }
    for rule in &mut collection.custom_routing_rules {
        changed |= replace(&mut rule.account_id, old_id, new_id);
    }
    for rule in &mut collection.account_model_rules {
        changed |= replace(&mut rule.account_id, old_id, new_id);
    }
    for key in &mut collection.api_keys {
        changed |= replace_list(&mut key.account_ids, old_id, new_id);
        changed |= replace_list(&mut key.priority_account_ids, old_id, new_id);
        if let Some(id) = &mut key.preferred_account_id {
            changed |= replace(id, old_id, new_id);
        }
        if let Some(routing) = &mut key.model_routing {
            for route in &mut routing.routes {
                changed |= replace(&mut route.provider_account_id, old_id, new_id);
            }
        }
    }
    changed
}

pub fn replace_account_references_after_key_edit(old_id: &str, new_id: &str) -> Result<(), String> {
    let old_id = old_id.trim();
    let new_id = new_id.trim();
    if old_id.is_empty() || new_id.is_empty() || old_id == new_id {
        return Ok(());
    }
    let path = local_access_file_path()?;
    // Conditional atomic writes prevent credential edits from overwriting newer settings.
    // Malformed configurations remain in place and the old account remains readable.
    let mut persisted = false;
    for _ in 0..3 {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                persisted = true;
                break;
            }
            Err(error) => return Err(format!("读取账号池引用失败: {}", error)),
        };
        let mut collection: CodexLocalAccessCollection = serde_json::from_slice(&bytes)
            .map_err(|error| format!("解析账号池引用失败: {}", error))?;
        if !replace_collection_account_references(&mut collection, old_id, new_id) {
            persisted = true;
            break;
        }
        collection.updated_at = now_ms();
        let hash = <Sha256 as sha2::Digest>::digest(&bytes).into();
        if write_string_atomic_if_hash_matches(&path, hash, || {
            serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())
        })? {
            persisted = true;
            break;
        }
        std::thread::yield_now();
    }
    if !persisted {
        return Err("账号池配置正在更新，请重试凭据编辑".to_string());
    }
    let reload = tauri::async_runtime::block_on(async {
        let mut runtime = timeout(Duration::from_secs(2), gateway_runtime().lock())
            .await
            .map_err(|_| "账号池运行态繁忙，请重试凭据编辑".to_string())?;
        let changed = runtime.collection.as_mut().is_some_and(|collection| {
            replace_collection_account_references(collection, old_id, new_id)
        });
        if changed {
            runtime.prepared_accounts.remove(old_id);
            runtime.account_health.remove(old_id);
        }
        Ok::<_, String>(changed && runtime.running)
    })?;
    if reload {
        trigger_gateway_reload_in_background("API Key 账号引用迁移");
    }
    Ok(())
}
