// Inbound-key preparation recovery. This never broadens a key's account scope.
const UNRESOLVED_PUBLIC_API_KEYS: &str =
    "网关无法解析入站 API Key，已保留原配置；请恢复被引用的账号后重试";
const KEY_RECOVERY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

fn public_key_has_missing_sources(
    collection: &CodexLocalAccessCollection,
    accounts: &HashMap<String, CodexAccount>,
) -> bool {
    collection.enabled
        && collection.api_keys.iter().any(|key| {
            key.enabled
                && !key.key.trim().is_empty()
                && effective_api_key_account_ids(collection, key)
                    .iter()
                    .any(|id| {
                        !accounts.contains_key(id)
                            && !matches!(
                                codex_account::account_storage_status(id),
                                Ok(codex_account::CodexAccountStorageStatus::Deleted)
                            )
                    })
        })
}

// A usable listener may still have pending sources. Keep serving its available
// accounts while the same bounded recovery loop reloads the missing ones.
fn finish_public_key_preparation(
    runtime: &mut GatewayRuntime,
    pending: bool,
) -> Result<(), String> {
    runtime.last_error = pending.then(|| UNRESOLVED_PUBLIC_API_KEYS.to_string());
    if pending {
        Err(UNRESOLVED_PUBLIC_API_KEYS.to_string())
    } else {
        Ok(())
    }
}

async fn defer_gateway_key_preparation_error(
    result: Result<(), String>,
    generation: u64,
) -> Result<(), String> {
    if result
        .as_ref()
        .is_err_and(|error| error == UNRESOLVED_PUBLIC_API_KEYS)
        && !gateway_lifecycle_generation_changed(generation)
        && GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0
    {
        let collection = gateway_runtime().lock().await.collection.clone();
        if let Some(collection) = collection.filter(|collection| collection.enabled) {
            if schedule_gateway_key_recovery(&collection, generation) {
                return Ok(());
            }
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyRecoveryTicket {
    generation: u64,
    collection_fingerprint: Vec<u8>,
}

#[derive(Default)]
struct KeyRecoveryControl {
    active: Option<KeyRecoveryTicket>,
    exhausted: Option<KeyRecoveryTicket>,
    attempt: usize,
    source_updated: bool,
}

fn key_recovery_control() -> &'static Mutex<KeyRecoveryControl> {
    static CONTROL: OnceLock<Mutex<KeyRecoveryControl>> = OnceLock::new();
    CONTROL.get_or_init(|| Mutex::new(KeyRecoveryControl::default()))
}

impl KeyRecoveryControl {
    fn begin(&mut self, ticket: KeyRecoveryTicket) -> bool {
        if self.exhausted.as_ref() == Some(&ticket) {
            return false;
        }
        if self.active.as_ref() != Some(&ticket) {
            self.active = Some(ticket);
            self.attempt = 0;
            self.source_updated = false;
        }
        true
    }

    fn finish(&mut self, ticket: &KeyRecoveryTicket, exhausted: bool) -> bool {
        if self.active.as_ref() == Some(ticket) {
            let retry_updated_source = exhausted && self.source_updated;
            self.active = None;
            self.exhausted = (exhausted && !retry_updated_source).then(|| ticket.clone());
            self.source_updated = false;
            return retry_updated_source;
        }
        false
    }
}

fn gateway_key_recovery_snapshot() -> (bool, usize) {
    key_recovery_control()
        .lock()
        .ok()
        .map(|control| {
            let active = control.active.as_ref().is_some_and(|ticket| {
                !gateway_lifecycle_generation_changed(ticket.generation)
                    && GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0
            });
            (active, if active { control.attempt } else { 0 })
        })
        .unwrap_or_default()
}

fn recovery_collection_signature(collection: &CodexLocalAccessCollection) -> Option<Value> {
    let mut value = serde_json::to_value(collection).ok()?;
    // Saving an unchanged form updates this bookkeeping timestamp only.
    value.as_object_mut()?.remove("updatedAt");
    Some(value)
}

fn canonicalize_recovery_signature(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                canonicalize_recovery_signature(child);
            }
            let mut entries: Vec<_> = std::mem::take(object).into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            object.extend(entries);
        }
        Value::Array(array) => {
            for child in array {
                canonicalize_recovery_signature(child);
            }
        }
        _ => {}
    }
}

fn recovery_account_signature(account: &CodexAccount) -> String {
    // Include account-level routes and restrictions as well as credentials.
    // Canonical objects avoid falsely detecting a change in HashMap iteration order.
    // Serialization failure disables fallback rather than caching a partial signature.
    serde_json::to_value(account)
        .and_then(|mut value| {
            // Display-only edits do not revoke usable credentials or change routing.
            if let Some(object) = value.as_object_mut() {
                // Account JSON exposes only a proxy summary. The private digest must
                // also detect credential changes at an otherwise identical endpoint.
                object.insert("egress_proxy_url".into(), json!(account.egress_proxy_url));
                for field in [
                    "account_note",
                    "tags",
                    "two_factor_secret",
                    "account_password",
                    "phone_number",
                    "mail_url",
                ] {
                    object.remove(field);
                }
            }
            canonicalize_recovery_signature(&mut value);
            serde_json::to_vec(&value)
        })
        .map(|bytes| format!("{:x}", <Sha256 as sha2::Digest>::digest(bytes)))
        .unwrap_or_default()
}

fn can_preserve_gateway_accounts(signatures: &HashMap<String, String>) -> bool {
    !signatures.is_empty()
        && signatures.iter().all(|(id, signature)| {
            matches!(
                codex_account::account_storage_status(id),
                Ok(codex_account::CodexAccountStorageStatus::Present)
            ) && codex_account::load_account(id).is_some_and(|account| {
                !signature.is_empty()
                    && !codex_account::is_grok_upstream_provider(&account)
                    && !account.requires_reauth
                    && !codex_account::account_has_remote_api_auth_rejection(&account)
                    && sidecar_local_account_usable_for_start(&account)
                    && recovery_account_signature(&account) == *signature
            })
        })
}

async fn can_preserve_gateway_for_key_recovery(collection: &CodexLocalAccessCollection) -> bool {
    let signatures = {
        let runtime = gateway_runtime().lock().await;
        let same_configuration = runtime.applied_collection.as_ref().is_some_and(|applied| {
            recovery_collection_signature(applied).is_some()
                && recovery_collection_signature(applied)
                    == recovery_collection_signature(collection)
        });
        if !runtime.running || !same_configuration {
            return false;
        }
        runtime.applied_account_signatures.clone()
    };
    // Fail closed for missing/revoked credentials and bounded local verification.
    matches!(
        timeout(
            Duration::from_secs(2),
            tauri::async_runtime::spawn_blocking(move || {
                can_preserve_gateway_accounts(&signatures)
            })
        )
        .await,
        Ok(Ok(true))
    )
}

async fn key_recovery_ticket_is_current(ticket: &KeyRecoveryTicket) -> bool {
    if gateway_lifecycle_generation_changed(ticket.generation)
        || GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) > 0
        || !key_recovery_control()
            .lock()
            .ok()
            .is_some_and(|control| control.active.as_ref() == Some(ticket))
    {
        return false;
    }
    let matching_collection = gateway_runtime()
        .lock()
        .await
        .collection
        .as_ref()
        .is_some_and(|collection| {
            collection.enabled
                && collection_content_fingerprint(collection).ok().as_ref()
                    == Some(&ticket.collection_fingerprint)
        });
    // Stop can advance the generation while this task waits for the runtime lock.
    matching_collection
        && !gateway_lifecycle_generation_changed(ticket.generation)
        && GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0
}

fn schedule_gateway_key_recovery(collection: &CodexLocalAccessCollection, generation: u64) -> bool {
    let Ok(fingerprint) = collection_content_fingerprint(collection) else {
        return false;
    };
    let ticket = KeyRecoveryTicket {
        generation,
        collection_fingerprint: fingerprint,
    };
    if !collection.enabled
        || gateway_lifecycle_generation_changed(generation)
        || GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) > 0
    {
        return false;
    }
    {
        let Ok(mut control) = key_recovery_control().lock() else {
            return false;
        };
        if gateway_lifecycle_generation_changed(generation)
            || GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) > 0
        {
            return false;
        }
        let already_running = control.active.as_ref() == Some(&ticket);
        if !control.begin(ticket.clone()) {
            return false;
        }
        if already_running {
            return true;
        }
    }
    emit_local_access_state_updated();
    tauri::async_runtime::spawn(run_gateway_key_recovery(ticket));
    true
}

async fn run_gateway_key_recovery(ticket: KeyRecoveryTicket) {
    run_gateway_key_recovery_with(ticket, &KEY_RECOVERY_DELAYS, || {
        ensure_gateway_matches_runtime_locked()
    })
    .await;
}

async fn run_gateway_key_recovery_with<F, Fut>(
    ticket: KeyRecoveryTicket,
    delays: &[Duration],
    mut prepare: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut exhausted = false;
    for (index, delay) in delays.iter().copied().enumerate() {
        tokio::select! {
            _ = wait_for_gateway_lifecycle_generation_change(ticket.generation) => break,
            _ = tokio::time::sleep(delay) => {}
        }
        if !key_recovery_ticket_is_current(&ticket).await {
            break;
        }
        // Waiting/retry delays never hold the lifecycle lock or a runtime lock.
        let guard = tokio::select! {
            _ = wait_for_gateway_lifecycle_generation_change(ticket.generation) => break,
            result = timeout(Duration::from_secs(2), gateway_lifecycle_lock().lock()) => {
                match result {
                    Ok(guard) => guard,
                    Err(_) => {
                        exhausted = index + 1 == delays.len();
                        continue;
                    }
                }
            }
        };
        if !key_recovery_ticket_is_current(&ticket).await {
            break;
        }
        if let Ok(mut control) = key_recovery_control().lock() {
            if control.active.as_ref() == Some(&ticket) {
                control.attempt = index + 1;
            }
        }
        emit_local_access_state_updated();
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 后台恢复入站 Key 配置: attempt={}/{}",
            index + 1,
            KEY_RECOVERY_DELAYS.len()
        ));
        let result = prepare().await;
        drop(guard);
        if !key_recovery_ticket_is_current(&ticket).await {
            break;
        }
        match result {
            Ok(()) => {
                let collection = {
                    let mut runtime = gateway_runtime().lock().await;
                    runtime.last_error = None;
                    runtime
                        .collection
                        .clone()
                        .filter(|collection| collection.enabled && runtime.running)
                };
                if let Some(collection) = collection {
                    trigger_sidecar_account_refresh_in_background(collection);
                }
                logger::log_codex_api_info("[CodexLocalAccess] 入站 Key 配置已自动恢复");
                break;
            }
            Err(error) if error == UNRESOLVED_PUBLIC_API_KEYS => {
                exhausted = index + 1 == delays.len();
            }
            Err(error) => {
                gateway_runtime().lock().await.last_error = Some(error.clone());
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 入站 Key 恢复遇到其他失败: {error}"
                ));
                exhausted = true;
                break;
            }
        }
    }
    let retry_updated_source = key_recovery_control()
        .lock()
        .ok()
        .is_some_and(|mut control| control.finish(&ticket, exhausted));
    if retry_updated_source
        && !gateway_lifecycle_generation_changed(ticket.generation)
        && GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0
    {
        let collection = gateway_runtime().lock().await.collection.clone();
        if let Some(collection) = collection.filter(|collection| {
            collection.enabled
                && collection_content_fingerprint(collection).ok().as_ref()
                    == Some(&ticket.collection_fingerprint)
        }) {
            schedule_gateway_key_recovery(&collection, ticket.generation);
        }
    }
    if exhausted && !retry_updated_source {
        logger::log_codex_api_warn(
            "[CodexLocalAccess] 入站 Key 自动恢复未成功，保留配置等待修复绑定或手动重试",
        );
    }
    emit_local_access_state_updated();
}

fn account_can_restore_public_key(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> bool {
    collection.enabled
        && !account.requires_reauth
        && !codex_account::account_has_remote_api_auth_rejection(account)
        && sidecar_local_account_usable_for_start(account)
        && is_local_access_eligible_account(account, collection.restrict_free_accounts)
        && sidecar_auth_id_for_account(account).is_some()
        && collection.api_keys.iter().any(|key| {
            key.enabled
                && !key.key.trim().is_empty()
                && effective_api_key_account_ids(collection, key).contains(&account.id)
        })
}

pub(crate) fn notify_gateway_key_account_restored(account: &CodexAccount) {
    // Saves are frequent. Only pending recovery needs a notification; a source
    // saved during the final attempt must also be observed before exhaustion.
    let ticket = key_recovery_control()
        .lock()
        .ok()
        .and_then(|control| {
            control
                .active
                .as_ref()
                .or(control.exhausted.as_ref())
                .cloned()
        })
        .filter(|ticket| !gateway_lifecycle_generation_changed(ticket.generation));
    let Some(ticket) = ticket else {
        return;
    };
    let account = account.clone();
    tauri::async_runtime::spawn(async move {
        let collection = {
            let runtime = gateway_runtime().lock().await;
            if runtime.last_error.as_deref() != Some(UNRESOLVED_PUBLIC_API_KEYS) {
                return;
            }
            // Maintenance writes for an already loaded account cannot restart
            // the retry budget of a different, still-unavailable source.
            if runtime.applied_account_signatures.contains_key(&account.id) {
                return;
            }
            runtime.collection.clone().filter(|collection| {
                !gateway_lifecycle_generation_changed(ticket.generation)
                    && collection_content_fingerprint(collection).ok().as_ref()
                        == Some(&ticket.collection_fingerprint)
                    && account_can_restore_public_key(collection, &account)
            })
        };
        let Some(collection) = collection else {
            return;
        };
        // A newly saved, usable source is a new recovery opportunity, even after
        // the bounded retry budget expired. Routine status reads never reset it.
        if let Ok(mut control) = key_recovery_control().lock() {
            if gateway_lifecycle_generation_changed(ticket.generation)
                || control.active.as_ref().or(control.exhausted.as_ref()) != Some(&ticket)
            {
                return;
            }
            if control.active.is_some() {
                control.source_updated = true;
                return;
            }
            control.exhausted = None;
        }
        schedule_gateway_key_recovery(&collection, ticket.generation);
    });
}
