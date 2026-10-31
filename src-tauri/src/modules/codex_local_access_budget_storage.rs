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
