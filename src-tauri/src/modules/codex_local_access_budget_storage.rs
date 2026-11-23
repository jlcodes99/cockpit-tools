// Small per-key accounting snapshots are independent of display history and
// collection settings. Reads happen on first API use, never on account listing.
static BUDGET_RESTORED: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));
static BUDGET_RESTORE_FLIGHTS: std::sync::LazyLock<
    Mutex<HashMap<String, std::sync::Weak<TokioMutex<()>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn key_budget_path(id: &str) -> Result<PathBuf, String> {
    // File names never interpolate externally supplied account/key identifiers.
    use sha2::{Digest, Sha256};
    Ok(account::get_data_dir()?
        .join("codex_key_budgets")
        .join(format!("{:x}.json", Sha256::digest(id.as_bytes()))))
}

fn save_key_budget_to_path(path: &Path, usage: &CodexApiKeyBudgetUsage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    update_string_atomic(path, |current| {
        let mut usage = usage.clone();
        if let Some(current) = current {
            let previous: CodexApiKeyBudgetUsage =
                serde_json::from_str(current).map_err(|error| error.to_string())?;
            merge_key_budget_usage(&mut usage, &previous);
        }
        serde_json::to_string(&usage).map_err(|error| error.to_string())
    })
}

fn save_collection_key_usage_to_path(
    path: &Path,
    snapshots: &[CodexLocalAccessApiKey],
) -> Result<(), String> {
    let snapshots: HashMap<_, _> = snapshots.iter().map(|key| (key.id.as_str(), key)).collect();
    // Request accounting owns only the counters. A late flush must not replay
    // routing settings or API key definitions captured before a user edit.
    update_string_atomic(path, |current| {
        let current = current.ok_or("API 服务配置文件不存在")?;
        let mut collection: Value = serde_json::from_str(current)
            .map_err(|error| format!("读取 API 服务配置失败: {error}"))?;
        let Some(keys) = collection
            .as_object_mut()
            .ok_or("API 服务配置应为 JSON 对象")?
            .get_mut("apiKeys")
        else {
            return Ok(current.to_owned());
        };
        let keys = keys.as_array_mut().ok_or("API 服务密钥列表格式错误")?;
        let mut changed = false;
        for key in keys {
            let Some(snapshot) = key
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| snapshots.get(id))
            else {
                continue;
            };
            let previous: CodexLocalAccessApiKey = serde_json::from_value(key.clone())
                .map_err(|error| format!("读取 API Key 用量失败: {error}"))?;
            if snapshot.token_used > previous.token_used {
                key["tokenUsed"] = json!(snapshot.token_used);
                changed = true;
            }
            let mut usage = previous.budget_usage.clone();
            merge_key_budget_usage(&mut usage, &snapshot.budget_usage);
            if usage != previous.budget_usage {
                key["budgetUsage"] = serde_json::to_value(usage).map_err(|error| error.to_string())?;
                changed = true;
            }
        }
        if !changed {
            return Ok(current.to_owned());
        }
        serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())
    })
}

async fn persist_key_budget(id: String, usage: CodexApiKeyBudgetUsage) -> Result<(), String> {
    static IO: std::sync::LazyLock<Arc<Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(Semaphore::new(4)));
    let permit = IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        save_key_budget_to_path(&key_budget_path(&id)?, &usage)
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn restore_key_budget(
    bridge: &NativeHostBridge,
    caller: &native_contract::Caller,
) -> Result<(), native_contract::ServiceError> {
    if bridge.collection_override.is_some()
        || matches!(
            caller.id.as_str(),
            NATIVE_INTERNAL_CALLER | NATIVE_PRIMARY_CALLER
        )
    {
        return Ok(());
    }
    let restored = || {
        BUDGET_RESTORED
            .lock()
            .is_ok_and(|keys| keys.contains(&caller.id))
    };
    if restored() {
        return Ok(());
    }
    let flight = {
        let mut flights = BUDGET_RESTORE_FLIGHTS.lock().map_err(|_| {
            native_error(
                503,
                "budget_storage_unavailable",
                "Budget accounting unavailable",
            )
        })?;
        flights.retain(|_, value| value.strong_count() > 0);
        let flight = flights
            .get(&caller.id)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| Arc::new(TokioMutex::new(())));
        flights.insert(caller.id.clone(), Arc::downgrade(&flight));
        flight
    };
    let _guard = tokio::time::timeout(Duration::from_secs(3), flight.lock())
        .await
        .map_err(|_| native_error(503, "budget_storage_unavailable", "Budget accounting busy"))?;
    if restored() {
        return Ok(());
    }
    let id = caller.id.clone();
    let task = tokio::task::spawn_blocking(move || {
        let path = key_budget_path(&id)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<CodexApiKeyBudgetUsage>(&text)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    });
    let usage = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .map_err(|_| {
            native_error(
                503,
                "budget_storage_unavailable",
                "Budget accounting read timed out",
            )
        })?
        .map_err(|_| {
            native_error(
                503,
                "budget_storage_unavailable",
                "Budget accounting read failed",
            )
        })?
        .map_err(|_| {
            native_error(
                503,
                "budget_storage_unavailable",
                "Budget accounting read failed",
            )
        })?;
    if let Some(usage) = usage {
        let mut runtime = gateway_runtime().lock().await;
        if let Some(key) = runtime.collection.as_mut().and_then(|collection| {
            collection
                .api_keys
                .iter_mut()
                .find(|key| key.id == caller.id)
        }) {
            merge_key_budget_usage(&mut key.budget_usage, &usage);
            runtime.collection_dirty = true;
        }
    }
    if let Ok(mut restored) = BUDGET_RESTORED.lock() {
        restored.insert(caller.id.clone());
    }
    Ok(())
}

#[cfg(test)]
mod budget_storage_tests {
    use super::*;

    #[test]
    fn background_key_usage_cannot_roll_back_routing_settings_or_restore_deleted_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex_local_access.json");
        let mut snapshot: CodexLocalAccessCollection = serde_json::from_value(json!({
            "enabled": true, "port": 1455, "apiKey": "fixture",
            "accountIds": [], "createdAt": 1, "updatedAt": 1,
            "sessionAffinityTtlMs": 3600000, "maxRetryCredentials": 0,
            "apiKeys": [
                {"id":"keep","key":"old","label":"old","tokenUsed":120},
                {"id":"removed","key":"removed","label":"removed","tokenUsed":99}
            ]
        }))
        .unwrap();
        snapshot.api_keys[0].budget_usage.lifetime_microusd = 300;
        save_collection_to_path(&path, &snapshot).unwrap();
        let mut current: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        current["sessionAffinityTtlMs"] = json!(300_000);
        current["maxRetryCredentials"] = json!(2);
        current["fixtureNewSetting"] = json!(true);
        current["apiKeys"][0]["key"] = json!("rotated");
        current["apiKeys"][0]["label"] = json!("renamed");
        current["apiKeys"][0]["tokenUsed"] = json!(90);
        current["apiKeys"][0]["budgetUsage"]["lifetimeMicrousd"] = json!(400);
        current["apiKeys"].as_array_mut().unwrap().pop();
        std::fs::write(&path, serde_json::to_vec(&current).unwrap()).unwrap();

        save_collection_key_usage_to_path(&path, &snapshot.api_keys).unwrap();
        let restarted = load_collection_for_runtime_from_path(&path).unwrap().unwrap();
        assert_eq!(restarted.max_retry_credentials, 2);
        assert_eq!(restarted.session_affinity_ttl_ms, 300_000);
        assert_eq!(restarted.api_keys.len(), 1);
        let key = &restarted.api_keys[0];
        assert_eq!(key.id, "keep");
        assert_eq!(key.key, "rotated");
        assert_eq!(key.label, "renamed");
        assert_eq!(key.token_used, 120);
        assert_eq!(key.budget_usage.lifetime_microusd, 400);
        let stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["fixtureNewSetting"], true);

        snapshot.api_keys[0].token_used = 100;
        snapshot.api_keys[0].budget_usage.lifetime_microusd = 500;
        save_collection_key_usage_to_path(&path, &snapshot.api_keys).unwrap();
        let after_late_snapshot = load_collection_for_runtime_from_path(&path).unwrap().unwrap();
        assert_eq!(after_late_snapshot.api_keys[0].token_used, 120);
        assert_eq!(after_late_snapshot.api_keys[0].budget_usage.lifetime_microusd, 500);
    }

    #[test]
    fn late_snapshot_cannot_refund_settled_amount_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.json");
        let newer = CodexApiKeyBudgetUsage {
            lifetime_microusd: 1_250_001,
            ..Default::default()
        };
        save_key_budget_to_path(&path, &newer).unwrap();
        save_key_budget_to_path(&path, &CodexApiKeyBudgetUsage::default()).unwrap();
        let restored: CodexApiKeyBudgetUsage =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(restored.lifetime_microusd, 1_250_001);
    }
}
