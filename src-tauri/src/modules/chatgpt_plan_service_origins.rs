static ORIGIN_BACKFILLS: LazyLock<Mutex<std::collections::HashSet<(String, String)>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

fn schedule_source_origin_backfill(summary: &chatgpt_plan::ChatgptPlanAccountSummary) {
    let key = (
        summary.id.clone(),
        summary.source_account_id.clone().unwrap_or_default(),
    );
    if !ORIGIN_BACKFILLS
        .lock()
        .map(|mut pending| pending.insert(key.clone()))
        .unwrap_or(false)
    {
        return;
    }
    tokio::spawn(async move {
        let (id, source) = key.clone();
        let projection = match codex_account::chatgpt_plan_projection_id(&id) {
            Ok(id) => id,
            Err(_) => return,
        };
        // Existence checks and historical metadata reads never block account-list
        // rendering. Unjoined legacy references must not write service config.
        let (_, members) = match codex_local_access::chatgpt_plan_pool_snapshot().await {
            Ok(pool) => pool,
            Err(_) => { if let Ok(mut pending) = ORIGIN_BACKFILLS.lock() { pending.remove(&key); } return; },
        };
        if !members.contains(&projection) {
            if let Ok(mut pending) = ORIGIN_BACKFILLS.lock() { pending.remove(&key); }
            return;
        }
        let result = if source.is_empty() {
            codex_local_access::pause_chatgpt_plan_service_endpoint(&projection)
                .await
                .map(|_| ())
        } else {
            let registration = id.clone();
            let original = source.clone();
            let remembered = tokio::task::spawn_blocking(move || {
                let projection = codex_account::chatgpt_plan_projection_id(&registration)?;
                if codex_account::load_account(&projection).is_none() {
                    return Ok::<_, String>(());
                }
                codex_account::set_chatgpt_plan_projection_source(&registration, &original)?;
                codex_local_access::remember_chatgpt_plan_service_origin(&registration, &original)
            })
            .await;
            if matches!(remembered, Ok(Ok(()))) {
                codex_local_access::migrate_legacy_official_member(&id, &source).await
            } else {
                Err("CHATGPT_PLAN_STORAGE_FAILED".into())
            }
        };
        if result.is_err() {
            if let Ok(mut pending) = ORIGIN_BACKFILLS.lock() {
                pending.remove(&key);
            }
        }
    });
}
