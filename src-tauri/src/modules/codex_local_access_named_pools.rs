use crate::models::codex_local_access::CodexLocalAccessAccountPool;

/// Materialize pool membership into the ordinary per-key routing scope.
/// The gateway and downgrade paths can use account_ids without resolving a name.
fn apply_named_account_pool_scopes(collection: &mut CodexLocalAccessCollection) -> bool {
    let pools = &collection.account_pools;
    let mut changed = false;
    for key in &mut collection.api_keys {
        let Some(pool_id) = key.account_pool_id.as_deref() else {
            continue;
        };
        let ids = pools
            .iter()
            .find(|pool| pool.id == pool_id)
            .map_or(&[][..], |pool| pool.account_ids.as_slice());
        if key.inherit_account_pool != Some(false) || key.account_ids != ids {
            key.inherit_account_pool = Some(false);
            key.account_ids = ids.to_vec();
            changed = true;
        }
        let previous = key.priority_account_ids.len();
        key.priority_account_ids.retain(|id| ids.contains(id));
        changed |= previous != key.priority_account_ids.len();
    }
    changed
}

fn save_named_pool_settings_at_path(
    path: &Path,
    fallback: &CodexLocalAccessCollection,
    change: impl FnOnce(&mut CodexLocalAccessCollection) -> Result<(), String>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut collection = match current {
            Some(raw) => serde_json::from_str::<CodexLocalAccessCollection>(raw)
                .map_err(|error| format!("读取账号池配置失败: {error}"))?,
            None => fallback.clone(),
        };
        normalize_collection_api_keys(&mut collection);
        change(&mut collection)?;
        normalize_collection_api_keys(&mut collection);
        collection.account_pool_revision = collection
            .account_pool_revision
            .checked_add(1)
            .ok_or("ACCOUNT_POOL_REVISION_EXHAUSTED")?;
        collection.updated_at = now_ms().max(collection.updated_at);
        let raw = serde_json::to_string_pretty(&collection)
            .map_err(|error| format!("保存账号池配置失败: {error}"))?;
        published = Some(collection);
        Ok(raw)
    })?;
    published.ok_or_else(|| "账号池配置未保存".into())
}

async fn change_named_account_pools(
    change: impl FnOnce(&mut CodexLocalAccessCollection) -> Result<(), String> + Send + 'static,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    tokio::task::spawn_blocking(move || {
        let path = local_access_file_path()?;
        // Serialize the disk transaction and runtime publication. Older full
        // settings snapshots are rejected by the account-pool revision.
        let mut runtime = gateway_runtime().blocking_lock();
        let current = runtime.collection.as_ref().ok_or("本地接入集合尚未创建")?;
        let collection = save_named_pool_settings_at_path(&path, current, change)?;
        sync_runtime_collection(&mut runtime, collection);
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| error.to_string())??;
    emit_local_access_state_updated();
    // The native request handler reads the published collection on each request.
    snapshot_state_without_gateway_reload().await
}

fn upsert_named_account_pool(
    collection: &mut CodexLocalAccessCollection,
    pool_id: Option<&str>,
    label: &str,
    account_ids: Vec<String>,
) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("请输入账号池名称".into());
    }
    let mut pool = CodexLocalAccessAccountPool {
        id: String::new(),
        label: label.to_owned(),
        account_ids: normalize_account_id_list(account_ids),
    };
    if let Some(id) = pool_id {
        let existing = collection
            .account_pools
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or("账号池不存在")?;
        pool.id = existing.id.clone();
        *existing = pool;
    } else {
        pool.id = uuid::Uuid::new_v4().to_string();
        collection.account_pools.push(pool);
    }
    Ok(())
}

fn add_key_with_named_account_pool(
    collection: &mut CodexLocalAccessCollection,
    label: Option<&str>,
    pool_id: &str,
) -> Result<(), String> {
    let pool = collection
        .account_pools
        .iter()
        .find(|pool| pool.id == pool_id)
        .ok_or("账号池不存在")?;
    let mut key = build_local_access_api_key(label);
    key.account_pool_id = Some(pool.id.clone());
    key.inherit_account_pool = Some(false);
    key.account_ids.clone_from(&pool.account_ids);
    collection.api_keys.push(key);
    Ok(())
}

fn delete_named_account_pool(
    collection: &mut CodexLocalAccessCollection,
    pool_id: &str,
) -> Result<(), String> {
    let index = collection
        .account_pools
        .iter()
        .position(|pool| pool.id == pool_id)
        .ok_or("账号池不存在")?;
    collection.account_pools.remove(index);
    for key in &mut collection.api_keys {
        if key.account_pool_id.as_deref() == Some(pool_id) {
            // Keep the last explicit scope, including if the pool was empty.
            key.account_pool_id = None;
        }
    }
    Ok(())
}

pub async fn save_local_access_account_pool(
    pool_id: Option<String>,
    label: String,
    account_ids: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    change_named_account_pools(move |collection| {
        upsert_named_account_pool(collection, pool_id.as_deref(), &label, account_ids)
    })
    .await
}

pub async fn delete_local_access_account_pool(
    pool_id: String,
) -> Result<CodexLocalAccessState, String> {
    change_named_account_pools(move |collection| delete_named_account_pool(collection, &pool_id))
        .await
}

pub async fn create_local_access_api_key_in_pool(
    label: Option<String>,
    pool_id: String,
) -> Result<CodexLocalAccessState, String> {
    change_named_account_pools(move |collection| {
        add_key_with_named_account_pool(collection, label.as_deref(), &pool_id)
    })
    .await
}

pub async fn detach_local_access_api_key_from_pool(
    key_id: String,
) -> Result<CodexLocalAccessState, String> {
    change_named_account_pools(move |collection| {
        let key = collection
            .api_keys
            .iter_mut()
            .find(|key| key.id == key_id)
            .ok_or("API Key 不存在")?;
        key.account_pool_id = None;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod named_account_pool_tests {
    use super::*;

    #[test]
    fn creating_a_key_in_a_pool_saves_its_exact_scope_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex_local_access.json");
        let mut initial = new_local_access_collection().unwrap();
        initial.account_ids = vec!["default".into(), "outside".into()];
        let before_key = save_named_pool_settings_at_path(&path, &initial, |collection| {
            upsert_named_account_pool(
                collection,
                None,
                "Plus",
                vec!["plus".into(), "plus".into()],
            )?;
            upsert_named_account_pool(collection, None, "Team", vec!["team".into()])
        })
        .unwrap();
        let pool_id = before_key.account_pools[0].id.clone();
        let created = save_named_pool_settings_at_path(&path, &before_key, |collection| {
            add_key_with_named_account_pool(collection, Some("Client Plus"), &pool_id)
        })
        .unwrap();
        let from_disk: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let original = &from_disk.api_keys[0];
        let key = from_disk
            .api_keys
            .iter()
            .find(|key| key.label == "Client Plus")
            .unwrap();
        assert_eq!(
            native_scoped_account_ids(&from_disk, Some(original)),
            ["default", "outside"]
        );
        assert_eq!(native_scoped_account_ids(&from_disk, Some(key)), ["plus"]);
        assert_eq!(effective_api_key_account_ids(&from_disk, key), ["plus"]);
        assert_eq!(key.inherit_account_pool, Some(false));
        assert_eq!(key.account_pool_id.as_deref(), Some(pool_id.as_str()));
        assert_eq!(
            created.account_pool_revision,
            before_key.account_pool_revision + 1
        );
        assert!(save_collection_to_path(&path, &before_key).is_err());
        let bytes = fs::read(&path).unwrap();
        assert!(
            save_named_pool_settings_at_path(&path, &created, |collection| {
                add_key_with_named_account_pool(collection, None, "missing")
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn changes_follow_the_pool_and_removal_preserves_the_last_explicit_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local-access.json");
        let mut initial = new_local_access_collection().unwrap();
        initial.account_ids = vec!["default".into()];
        let current = save_named_pool_settings_at_path(&path, &initial, |collection| {
            upsert_named_account_pool(collection, None, "Plus", vec!["first".into()])?;
            let id = collection.account_pools[0].id.clone();
            add_key_with_named_account_pool(collection, Some("A"), &id)?;
            add_key_with_named_account_pool(collection, Some("B"), &id)?;
            collection.api_keys[1].priority_account_ids = vec!["first".into()];
            Ok(())
        })
        .unwrap();
        let id = &current.account_pools[0].id;
        let edited = save_named_pool_settings_at_path(&path, &current, |collection| {
            upsert_named_account_pool(collection, Some(id), "Renamed", vec!["second".into()])
        })
        .unwrap();
        let key = &edited.api_keys[1];
        assert_eq!(native_scoped_account_ids(&edited, Some(key)), ["second"]);
        assert!(key.priority_account_ids.is_empty());
        assert_eq!(
            native_scoped_account_ids(&edited, Some(&edited.api_keys[2])),
            ["second"]
        );
        assert!(validate_api_key_account_scope_update(&edited, key, None, Some(true)).is_err());
        let mut removed_with_members = edited.clone();
        delete_named_account_pool(&mut removed_with_members, id).unwrap();
        normalize_collection_api_keys(&mut removed_with_members);
        for key in &removed_with_members.api_keys[1..] {
            assert!(key.account_pool_id.is_none());
            assert_eq!(
                native_scoped_account_ids(&removed_with_members, Some(key)),
                ["second"]
            );
        }
        let empty = save_named_pool_settings_at_path(&path, &edited, |collection| {
            upsert_named_account_pool(collection, Some(id), "Empty", vec![])
        })
        .unwrap();
        let deleted = save_named_pool_settings_at_path(&path, &empty, |collection| {
            delete_named_account_pool(collection, id)
        })
        .unwrap();
        assert!(deleted.account_pools.is_empty());
        for key in &deleted.api_keys[1..] {
            assert!(key.account_pool_id.is_none());
            assert!(native_scoped_account_ids(&deleted, Some(key)).is_empty());
            assert_eq!(key.inherit_account_pool, Some(false));
        }
    }
}
