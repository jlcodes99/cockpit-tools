use crate::modules::codex_proxy_catalog::{self as catalog, CatalogView};
use std::collections::BTreeMap;
#[cfg(test)]
#[path = "codex_proxy_catalog_preflight_tests.rs"]
mod preflight_tests;
#[tauri::command]
pub async fn codex_proxy_catalog_list() -> Result<CatalogView, String> {
    catalog::list().await
}
#[tauri::command]
pub async fn codex_proxy_catalog_import(
    request_id: String,
    name: String,
    input: String,
    kind: String,
    options: Option<crate::modules::codex_proxy_manual_import::ImportOptions>,
) -> Result<CatalogView, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    catalog::import(request_id, name, input, kind, options.unwrap_or_default()).await
}

#[tauri::command]
pub async fn codex_proxy_catalog_preview(
    input: String,
    options: crate::modules::codex_proxy_manual_import::ImportOptions,
) -> Result<crate::modules::codex_proxy_manual_import::ImportPreview, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    catalog::preview(input, options).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_rename(
    source_id: String,
    name: String,
) -> Result<CatalogView, String> {
    let view = catalog::rename(source_id.clone(), name).await?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, false).await;
    Ok(view)
}
#[tauri::command]
pub async fn codex_proxy_catalog_latency(
    request_id: String,
    source_id: String,
    node_id: String,
    revision: String,
    group_id: Option<String>,
) -> Result<crate::modules::codex_proxy_probe::LatencyResult, String> {
    catalog::latency(request_id, source_id, node_id, revision, group_id).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_refresh(
    request_id: String,
    source_id: String,
) -> Result<CatalogView, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    let view = catalog::refresh(request_id, source_id.clone()).await?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, false).await;
    Ok(view)
}
#[tauri::command]
pub fn codex_proxy_catalog_cancel(request_id: String) -> Result<(), String> {
    catalog::cancel(request_id)
}
/// 删除来源前的只读影响预览：不修改账号、统一代理或来源，也不返回任何凭据。
#[tauri::command]
pub async fn codex_proxy_catalog_dependencies(
    source_id: String,
) -> Result<crate::modules::codex_proxy_catalog::SourceDependenciesView, String> {
    catalog::dependencies(source_id).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_remove(source_id: String) -> Result<CatalogView, String> {
    remove_source(source_id).await
}
/// 订阅、手写来源与策略共用同一条删除链路：先解绑引用该来源的账号，再删除来源本身。
// Only IDs are retained; never restore an old binding after a partial deletion.
// On host restart the sidecar is rebuilt from the authoritative account store.
static PENDING_REMOVAL_SYNC: std::sync::LazyLock<
    std::sync::Mutex<BTreeMap<String, std::collections::BTreeSet<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

fn remember_removal_sync(source_id: &str, accounts: Vec<String>) -> Result<Vec<String>, String> {
    let mut pending = PENDING_REMOVAL_SYNC
        .lock()
        .map_err(|_| "CATALOG_PARTIAL_UNBIND")?;
    if accounts.is_empty() && !pending.contains_key(source_id) {
        return Ok(Vec::new());
    }
    let ids = pending.entry(source_id.to_owned()).or_default();
    ids.extend(accounts);
    Ok(ids.iter().cloned().collect())
}

fn finish_removal_sync(source_id: &str, account_id: &str) {
    if let Ok(mut pending) = PENDING_REMOVAL_SYNC.lock() {
        if let Some(ids) = pending.get_mut(source_id) {
            ids.remove(account_id);
            if ids.is_empty() {
                pending.remove(source_id);
            }
        }
    }
}

#[cfg(test)]
mod removal_retry_tests {
    use super::*;

    #[test]
    fn failed_sync_retries_ids_after_their_bindings_are_cleared() {
        let source = format!("retry-{}", uuid::Uuid::new_v4());
        let first = remember_removal_sync(&source, vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(first, vec!["a", "b"]);
        // First account completed; second failed. The next disk scan finds no
        // bindings, but the failed account must still be synchronized.
        finish_removal_sync(&source, "a");
        assert_eq!(remember_removal_sync(&source, vec![]).unwrap(), vec!["b"]);
        finish_removal_sync(&source, "b");
        assert!(!PENDING_REMOVAL_SYNC.lock().unwrap().contains_key(&source));
    }

    #[test]
    fn retry_deduplicates_new_matches_and_keeps_other_sources() {
        let source = format!("retry-{}", uuid::Uuid::new_v4());
        let other = format!("other-{}", uuid::Uuid::new_v4());
        remember_removal_sync(&source, vec!["a".into()]).unwrap();
        remember_removal_sync(&other, vec!["a".into()]).unwrap();
        assert_eq!(
            remember_removal_sync(&source, vec!["a".into(), "b".into()]).unwrap(),
            vec!["a", "b"]
        );
        finish_removal_sync(&source, "a");
        finish_removal_sync(&source, "b");
        assert_eq!(remember_removal_sync(&other, vec![]).unwrap(), vec!["a"]);
        finish_removal_sync(&other, "a");
    }
}

async fn remove_source(source_id: String) -> Result<CatalogView, String> {
    let guard = std::sync::Arc::new(catalog::SourceGuard::new(source_id.clone())?);
    catalog::ensure_exists(&source_id).await?;
    let matching_source = source_id.clone();
    let (account_ids, existing_ids) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tauri::async_runtime::spawn_blocking(move || {
            crate::modules::codex_account::list_accounts_for_proxy_removal().map(|accounts| {
                let existing = accounts
                    .iter()
                    .map(|account| account.id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                let matching = accounts
                    .into_iter()
                    .filter(|account| {
                        account.egress_proxy_url.as_deref().is_some_and(|raw| {
                            crate::modules::codex_proxy_catalog_binding::belongs_to_source(
                                raw,
                                &matching_source,
                            )
                        })
                    })
                    .map(|account| account.id)
                    .collect::<Vec<_>>();
                (matching, existing)
            })
        }),
    )
    .await
    .map_err(|_| "CATALOG_TIMEOUT")?
    .map_err(|_| "CATALOG_ACCOUNT_READ")??;
    // Remember before unbinding: cancelled or failed work can be retried even
    // though the account no longer references this source.
    let pending = remember_removal_sync(&source_id, account_ids)?;
    let had_updates = !pending.is_empty();
    for account_id in pending {
        // A separately deleted account no longer needs an auth-file update.
        // The checked account listing above distinguishes deletion from read failure.
        if !existing_ids.contains(&account_id) {
            finish_removal_sync(&source_id, &account_id);
            continue;
        }
        crate::modules::codex_proxy_runtime::clear_binding_if_source(
            account_id.clone(),
            source_id.clone(),
        )
        .await
        .map_err(|_| "CATALOG_PARTIAL_UNBIND")?;
        let lock = crate::modules::codex_account::codex_token_lock_for(&account_id);
        let token_guard =
            tokio::time::timeout(std::time::Duration::from_secs(10), lock.lock_owned())
                .await
                .map_err(|_| "CATALOG_PARTIAL_UNBIND")?;
        let source = source_id.clone();
        let worker_guard = guard.clone();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tauri::async_runtime::spawn_blocking(move || {
                // Keep both guards in the worker after a timeout. No retry can
                // race this write, and no old account snapshot is written back.
                let _source_guard = worker_guard;
                let _token_guard = token_guard;
                let account = crate::modules::codex_account::load_account(&account_id)
                    .ok_or("CATALOG_PARTIAL_UNBIND")?;
                crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)
                    .map_err(|_| "CATALOG_PARTIAL_UNBIND")?;
                if crate::modules::codex_local_access::collection_contains_account(&account_id) {
                    crate::modules::codex_local_access::trigger_gateway_reload_in_background(
                        "删除账号代理来源",
                    );
                }
                finish_removal_sync(&source, &account_id);
                Ok::<(), &str>(())
            }),
        )
        .await
        .map_err(|_| "CATALOG_PARTIAL_UNBIND")?
        .map_err(|_| "CATALOG_PARTIAL_UNBIND")??;
    }
    let removed = catalog::remove(source_id.clone()).await.map_err(|error| {
        if had_updates {
            "CATALOG_PARTIAL_UNBIND".into()
        } else {
            error
        }
    })?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, true).await;
    Ok(removed)
}
/// 保存自建多节点策略：带 id 时保留原来源 ID，新建时生成新 ID。
#[tauri::command]
pub async fn codex_proxy_strategy_save(
    id: Option<String>,
    name: String,
    kind: String,
    members: Vec<catalog::StrategyMember>,
    options: Option<catalog::StrategyOptions>,
) -> Result<CatalogView, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    let id = id.filter(|id| !id.trim().is_empty());
    let view = catalog::save_strategy(id.clone(), name, kind, members, options.unwrap_or_default())
        .await?;
    if let Some(id) = id {
        // 统一代理引用同一策略时必须同步快照，否则账号仍走旧成员。
        crate::modules::codex_unified_proxy::resync_source(&id, false).await;
    }
    Ok(view)
}
/// 删除自建策略：沿用来源删除路径与部分失败错误码，只额外确认目标确实是策略来源。
#[tauri::command]
pub async fn codex_proxy_strategy_remove(id: String) -> Result<CatalogView, String> {
    catalog::ensure_strategy(&id).await?;
    remove_source(id).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_set_auto_update(
    source_id: String,
    enabled: bool,
) -> Result<CatalogView, String> {
    catalog::set_auto_update(source_id, enabled).await
}
/// 保存来源默认项：只影响账号草稿预填，不写入任何账号绑定。
#[tauri::command]
pub async fn codex_proxy_catalog_set_default(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<CatalogView, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    let _guard = catalog::SourceGuard::new(source_id.clone())?;
    // set_default validates the current catalog and group membership locally.
    // A failed public-IP service must not prevent saving this preference.
    catalog::set_default(source_id, item_id, selections, group_id).await
}
/// 清除来源默认项，并确认“默认项已失效”提示。
#[tauri::command]
pub async fn codex_proxy_catalog_clear_default(source_id: String) -> Result<CatalogView, String> {
    catalog::clear_default(source_id).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_bind(
    account_id: String,
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<crate::models::codex::CodexAccount, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    let _guard = catalog::SourceGuard::new(source_id.clone())?;
    let snapshot = catalog::snapshot_with_group(source_id, item_id, selections, group_id).await?;
    // Same binding transaction and API-service synchronization as the existing account form.
    super::codex::update_codex_account_egress_proxy(account_id, Some(snapshot)).await
}
#[tauri::command]
pub async fn codex_proxy_catalog_probe(
    request_id: String,
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
) -> Result<crate::modules::codex_proxy_probe::ProxyProbeResult, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    catalog::probe(request_id, source_id, item_id, selections).await
}

#[tauri::command]
pub async fn codex_proxy_catalog_network(
    source_id: String,
    revision: String,
    network: crate::modules::codex_proxy_network::NetworkOptions,
) -> Result<CatalogView, String> {
    let view = catalog::set_network(source_id.clone(), revision, network).await?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, false).await;
    Ok(view)
}
#[tauri::command]
pub async fn codex_proxy_catalog_insecure(
    source_id: String,
    node_id: String,
    revision: String,
    enabled: bool,
) -> Result<CatalogView, String> {
    let view = catalog::set_insecure(source_id.clone(), node_id, revision, enabled).await?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, false).await;
    Ok(view)
}

#[tauri::command]
pub async fn codex_proxy_catalog_group_insecure(
    source_id: String,
    group_id: String,
    revision: String,
    enabled: bool,
) -> Result<CatalogView, String> {
    let view = catalog::set_group_insecure(source_id.clone(), group_id, revision, enabled).await?;
    crate::modules::codex_unified_proxy::resync_source(&source_id, false).await;
    Ok(view)
}
