//! 统一代理出口命令层：读取 / 预览 / 启用 / 关闭，以及代理资源变更后的重新同步。
//!
//! 统一代理只保存「引用 + 快照」，账号文件不改写：没有独立绑定的可用账号
//! 使用统一出口；账号自己的绑定始终优先。
use crate::modules::codex_proxy_catalog as catalog;
use crate::modules::codex_unified_proxy::{self as unified, Reference};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static APPLY_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedProxyView {
    mode: String,
    binding: Option<serde_json::Value>,
    eligible_account_ids: Vec<String>,
    independent_account_ids: Vec<String>,
    stale_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedProxyPreview {
    binding: serde_json::Value,
    eligible_account_ids: Vec<String>,
    independent_account_ids: Vec<String>,
}

/// 可用账号与「已有独立绑定」的账号清单；账号读取是磁盘操作，必须离开异步线程。
async fn account_totals() -> Result<(Vec<String>, Vec<String>), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut eligible = Vec::new();
        let mut independent = Vec::new();
        for account in crate::modules::codex_account::list_accounts() {
            if !crate::modules::codex_account_proxy::eligible(&account) {
                continue;
            }
            if account
                .egress_proxy_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                independent.push(account.id.clone());
            }
            eligible.push(account.id);
        }
        (eligible, independent)
    })
    .await
    .map_err(|_| "UNIFIED_PROXY_FAILED".to_string())
}

async fn current_view() -> Result<UnifiedProxyView, String> {
    let state = unified::ensure_loaded().await?;
    let (eligible_account_ids, independent_account_ids) = account_totals().await?;
    Ok(UnifiedProxyView {
        mode: state.mode,
        binding: state.snapshot.as_deref().and_then(crate::modules::codex_proxy_catalog_binding::summary),
        eligible_account_ids,
        independent_account_ids,
        stale_error: state.stale_error,
    })
}

#[tauri::command]
pub async fn codex_unified_proxy_get() -> Result<UnifiedProxyView, String> {
    current_view().await
}

#[tauri::command]
pub async fn codex_unified_proxy_preview(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<UnifiedProxyPreview, String> {
    let _guard = catalog::SourceGuard::new(source_id.clone())?;
    let snapshot =
        catalog::snapshot_with_group(source_id, item_id, selections, group_id).await?;
    let binding = crate::modules::codex_proxy_catalog_binding::summary(&snapshot)
        .ok_or_else(|| "UNIFIED_PROXY_STALE".to_string())?;
    let (eligible_account_ids, independent_account_ids) = account_totals().await?;
    Ok(UnifiedProxyPreview {
        binding,
        eligible_account_ids,
        independent_account_ids,
    })
}

#[tauri::command]
pub async fn codex_unified_proxy_apply(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<UnifiedProxyView, String> {
    crate::modules::codex_proxy_engine_preflight::require().await?;
    let _apply = APPLY_LOCK.try_lock().map_err(|_| "UNIFIED_PROXY_BUSY")?;
    let _guard = catalog::SourceGuard::new(source_id.clone())?;
    let snapshot = catalog::snapshot_with_group(
        source_id.clone(),
        item_id.clone(),
        selections.clone(),
        group_id.clone(),
    )
    .await?;
    // Snapshot validation is local. Egress probes remain an explicit action and
    // cannot veto an otherwise valid shared-exit configuration.
    let decoded = crate::modules::codex_proxy_catalog_binding::decode(&snapshot)
        .map_err(|_| "UNIFIED_PROXY_STALE".to_string())?;
    let reference = Reference {
        source_id: decoded.source_id,
        source_name: decoded.source_name,
        item_id,
        group_id,
        name: decoded.name,
        selections,
    };
    tauri::async_runtime::spawn_blocking(move || unified::enable(reference, snapshot))
        .await
        .map_err(|_| "UNIFIED_PROXY_FAILED".to_string())??;
    crate::modules::codex_proxy_runtime::spawn_reload_after_unified_change();
    current_view().await
}

#[tauri::command]
pub async fn codex_unified_proxy_disable() -> Result<UnifiedProxyView, String> {
    let _apply = APPLY_LOCK.try_lock().map_err(|_| "UNIFIED_PROXY_BUSY")?;
    tauri::async_runtime::spawn_blocking(unified::disable)
        .await
        .map_err(|_| "UNIFIED_PROXY_FAILED".to_string())??;
    crate::modules::codex_proxy_runtime::spawn_reload_after_unified_change();
    current_view().await
}
