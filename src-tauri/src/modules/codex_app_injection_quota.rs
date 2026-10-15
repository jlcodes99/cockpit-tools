// API 服务注入的手动额度刷新：跨平台目标、有限并发与单飞。

#[derive(Default, Debug, PartialEq, Eq)]
struct ApiServiceQuotaRefreshTargets {
    existing_account_count: usize,
    codex_ids: Vec<String>,
    grok_ids: Vec<String>,
}

fn collect_api_service_quota_targets(accounts: &[CodexAccount]) -> ApiServiceQuotaRefreshTargets {
    let mut targets = ApiServiceQuotaRefreshTargets::default();
    let mut seen = HashSet::new();
    let mut grok_seen = HashSet::new();
    for account in accounts {
        if !seen.insert(&account.id) {
            continue;
        }
        targets.existing_account_count += 1;
        if codex_account::is_grok_upstream_provider(account) {
            if let Some(id) = account
                .upstream_grok_account_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                if grok_seen.insert(id) {
                    targets.grok_ids.push(id.to_string());
                }
            }
        } else if codex_quota::supports_quota_refresh(account) {
            targets.codex_ids.push(account.id.clone());
        }
    }
    targets
}

async fn api_service_quota_refresh_targets() -> Result<Option<ApiServiceQuotaRefreshTargets>, String>
{
    let Some(collection) = codex_local_access::get_local_access_state()
        .await?
        .collection
    else {
        return Ok(None);
    };
    if collection.account_ids.is_empty() {
        return Ok(Some(ApiServiceQuotaRefreshTargets::default()));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let accounts: Vec<_> = collection
            .account_ids
            .iter()
            .filter_map(|id| codex_account::load_account(id))
            .collect();
        let targets = collect_api_service_quota_targets(&accounts);
        // 原子替换时的暂时读取失败不能被当成真正的空池。
        (targets.existing_account_count > 0).then_some(targets)
    })
    .await
    .map_err(|error| format!("读取 API 服务额度刷新目标失败: {}", error))
}

async fn api_service_account_pool_is_empty() -> Result<Option<bool>, String> {
    let state = codex_local_access::get_local_access_state().await?;
    Ok(state
        .collection
        .map(|collection| collection.account_ids.is_empty()))
}

async fn refresh_grok_quota_batch<F, Fut>(
    ids: Vec<String>,
    request_timeout: Duration,
    refresh: F,
) -> i32
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut results = futures_util::stream::iter(ids.into_iter().map(|id| {
        let refresh = &refresh;
        async move {
            let result = timeout(request_timeout, refresh(id.clone())).await
                .unwrap_or_else(|_| Err("额度刷新超时".to_string()));
            match result {
                Ok(()) => 1,
                Err(error) => {
                    logger::log_warn(&format!("[Codex App Injection] Grok 额度刷新失败（保留缓存）: account_id={}, error={}", id, error));
                    0
                }
            }
        }
    })).buffer_unordered(4);
    let mut success_count = 0;
    while let Some(success) = results.next().await {
        success_count += success;
    }
    success_count
}

async fn refresh_api_service_quota_pool(app: &AppHandle) -> Result<Option<(i32, usize)>, String> {
    let Some(targets) = api_service_quota_refresh_targets().await? else {
        return Ok(None);
    };
    if targets.existing_account_count == 0 {
        return Ok(Some((0, 0)));
    }
    let total = targets.codex_ids.len() + targets.grok_ids.len();
    if total == 0 {
        return Err("API 服务账号池暂无可刷新的额度".to_string());
    }
    let codex_refresh = async {
        if targets.codex_ids.is_empty() {
            return 0;
        }
        match crate::commands::codex::refresh_codex_quotas_batch(
            app.clone(),
            targets.codex_ids,
            Some(true),
            Some(false),
        )
        .await
        {
            Ok(count) => count,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Codex App Injection] Codex 额度刷新失败: {}",
                    error
                ));
                0
            }
        }
    };
    let grok_refresh = refresh_grok_quota_batch(
        targets.grok_ids,
        Duration::from_secs(120),
        |id| async move {
            let result = crate::modules::grok_account::refresh_account(&id).await;
            // 即使配额查询软失败，也可能已经轮换了凭据，需写穿到运行中的网关。
            codex_local_access::sync_grok_upstream_auth_files_in_background(id);
            let account = result?;
            match account.quota_query_last_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        },
    );
    let (codex_count, grok_count) = tokio::join!(codex_refresh, grok_refresh);
    let success_count = codex_count + grok_count;
    if success_count == 0 {
        return Err("API 服务账号池额度刷新失败".to_string());
    }
    Ok(Some((success_count, total)))
}

async fn run_quota_refresh_singleflight(app: &AppHandle) -> Result<Option<(i32, usize)>, String> {
    let lock = quota_refresh_lock();
    match lock.try_lock() {
        Ok(_guard) => refresh_api_service_quota_pool(app).await,
        Err(_) => {
            let _guard = lock.lock().await;
            let Some(targets) = api_service_quota_refresh_targets().await? else {
                return Ok(None);
            };
            Ok((targets.existing_account_count == 0).then_some((0, 0)))
        }
    }
}

#[cfg(test)]
mod quota_refresh_tests {
    use super::*;
    use crate::models::codex::{CodexApiProviderMode, CodexTokens};

    #[test]
    fn mixed_pool_refreshes_source_grok_ids_once_and_keeps_codex_ids() {
        let codex = CodexAccount::new(
            "oauth".into(),
            "a@example.test".into(),
            CodexTokens {
                access_token: "test".into(),
                id_token: "test".into(),
                refresh_token: None,
            },
        );
        let mut grok = CodexAccount::new_api_key(
            "proxy".into(),
            "g@example.test".into(),
            String::new(),
            CodexApiProviderMode::Custom,
            None,
            Some("grok".into()),
            None,
            vec![],
        );
        grok.upstream_grok_account_id = Some("source".into());
        let mut duplicate = grok.clone();
        duplicate.id = "other-proxy".into();
        let targets = collect_api_service_quota_targets(&[codex, grok.clone(), duplicate]);
        assert_eq!(targets.codex_ids, vec!["oauth"]);
        assert_eq!(targets.grok_ids, vec!["source"]);
        assert_eq!(targets.existing_account_count, 3);
        let only_grok = collect_api_service_quota_targets(&[grok]);
        assert!(only_grok.codex_ids.is_empty());
        assert_eq!(only_grok.grok_ids, vec!["source"]);
    }

    #[tokio::test]
    async fn failed_and_stalled_grok_accounts_do_not_block_healthy_accounts_or_retry() {
        let ids = vec!["ok".into(), "failed".into(), "stalled".into()];
        let count = refresh_grok_quota_batch(ids, Duration::from_millis(20), |id| async move {
            match id.as_str() {
                "failed" => Err("offline".into()),
                "stalled" => std::future::pending().await,
                _ => Ok(()),
            }
        })
        .await;
        assert_eq!(count, 1);
        assert_eq!(
            refresh_grok_quota_batch(vec!["stalled".into()], Duration::from_secs(1), |_| async {
                Ok(())
            })
            .await,
            1
        );
    }

    #[tokio::test]
    async fn grok_quota_refresh_limits_simultaneous_requests() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let ids = (0..9).map(|id| id.to_string()).collect();
        let count = refresh_grok_quota_batch(ids, Duration::from_secs(1), |_| async {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(count, 9);
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
