// OAuth auth-file synchronization for existing per-profile gateways.
fn sync_provider_gateway_runtime_auth_file(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    sidecar_dir: &Path,
) -> Result<bool, String> {
    let auth_path = sidecar_auths_dir(sidecar_dir).join(sidecar_auth_file_name(&account.id));
    if !auth_path.exists() {
        return Ok(false);
    }
    let proxy_signature = sidecar_effective_proxy_signature(collection)?;
    let effective_proxy_url =
        sidecar_proxy_url_for_account(account, proxy_signature.proxy_url.as_deref())?;
    let auth_json =
        sidecar_auth_json_for_account(account, collection, effective_proxy_url.as_deref());
    let auth_content = serde_json::to_string_pretty(&auth_json)
        .map_err(|error| format!("序列化实例 sidecar OAuth 认证失败: {}", error))?;
    let changed = write_secret_string_atomic_if_changed(&auth_path, &auth_content)?;
    harden_sidecar_auth_file_permissions(&auth_path)?;
    Ok(changed)
}

#[derive(Default)]
struct ProviderAuthSyncQueue(Mutex<HashMap<String, bool>>);

impl ProviderAuthSyncQueue {
    fn request(&self, account_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(account_id.to_string(), true)
            .is_none()
    }

    fn begin(&self, account_id: &str) {
        if let Some(pending) = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(account_id)
        {
            *pending = false;
        }
    }

    fn finish(&self, account_id: &str) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if pending.get(account_id) == Some(&true) {
            return false;
        }
        pending.remove(account_id);
        true
    }
}

async fn provider_gateway_auth_targets(
    account_id: &str,
) -> Vec<(CodexLocalAccessCollection, PathBuf)> {
    let runtimes = provider_gateway_runtime_store().lock().await;
    runtimes
        .values()
        .filter(|runtime| runtime.oauth_account_ids.iter().any(|id| id == account_id))
        .filter_map(|runtime| Some((runtime.collection.clone()?, runtime.sidecar_dir.clone()?)))
        .collect()
}

async fn sync_provider_gateway_auth_files_for_account_once(account_id: &str) -> Result<(), String> {
    if provider_gateway_auth_targets(account_id).await.is_empty() {
        return Ok(());
    }
    // The synchronous writer consumes only prepared proxy state. In particular, an
    // account unbound after launch may never have loaded the unified-proxy store.
    crate::modules::codex_proxy_runtime::prepare_accounts(vec![account_id.to_string()]).await?;
    let token_lock = codex_account::codex_token_lock_for(account_id);
    let token_guard = timeout(Duration::from_secs(5), token_lock.lock_owned())
        .await
        .map_err(|_| "PROXY_RUNTIME_STARTING".to_string())?;
    // Read again after waiting: queued credential/proxy changes must not project
    // an obsolete captured account. Keep this per-account lock through the write.
    let account = crate::modules::codex_proxy_runtime::load(account_id).await?;
    let targets = provider_gateway_auth_targets(account_id).await;
    tauri::async_runtime::spawn_blocking(move || {
        let _token_guard = token_guard;
        let mut errors = Vec::new();
        for (collection, sidecar_dir) in targets {
            match sync_provider_gateway_runtime_auth_file(&account, &collection, &sidecar_dir) {
                Ok(true) => logger::log_codex_api_info(&format!(
                    "[CodexLocalAccess][provider-gateway] 已写穿实例 sidecar OAuth 凭据: account_id={}, sidecar_dir={}",
                    account.id, sidecar_dir.display()
                )),
                Ok(false) => {}
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }).await.map_err(|_| "PROXY_RUNTIME_FAILED".to_string())?
}

pub fn sync_provider_gateway_auth_files_for_account_in_background(account: CodexAccount) {
    static QUEUE: OnceLock<ProviderAuthSyncQueue> = OnceLock::new();
    let queue = QUEUE.get_or_init(ProviderAuthSyncQueue::default);
    let account_id = account.id;
    if !queue.request(&account_id) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            queue.begin(&account_id);
            if let Err(error) = sync_provider_gateway_auth_files_for_account_once(&account_id).await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][provider-gateway] 写穿实例 sidecar OAuth 凭据失败: account_id={}, error={}",
                    account_id, error
                ));
            }
            if queue.finish(&account_id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
}
