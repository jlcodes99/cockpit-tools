// Manual session repair command; included in codex_instance to retain the registered IPC path.
#[tauri::command]
pub async fn codex_repair_session_visibility_across_instances(
    app: AppHandle,
    mode: Option<modules::codex_session_visibility::CodexSessionVisibilityRepairMode>,
    run_id: Option<String>,
    target_provider: Option<String>,
    target_instance_id: Option<String>,
    repair_instance_ids: Option<Vec<String>>,
    session_ids: Option<Vec<String>>,
    dry_run: Option<bool>,
) -> Result<modules::codex_session_visibility::CodexSessionVisibilityRepairSummary, String> {
    let mode =
        mode.unwrap_or(modules::codex_session_visibility::CodexSessionVisibilityRepairMode::Quick);
    let resolved_target_provider = if mode
        == modules::codex_session_visibility::CodexSessionVisibilityRepairMode::History
    {
        None
    } else {
        match target_provider
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(provider) => Some(provider),
        None => match target_instance_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(instance_id) => Some(
                modules::codex_session_visibility::resolve_session_visibility_target_provider_from_instance_id(
                    instance_id,
                )?,
            ),
            None => None,
        },
    }
    };
    let progress_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let reporter =
            |progress: modules::codex_session_visibility::CodexSessionVisibilityRepairProgress| {
                let _ = progress_app.emit(
                    modules::codex_session_visibility::SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT,
                    progress,
                );
            };
        modules::codex_session_visibility::repair_session_visibility_across_instances_with_target(
            mode,
            run_id,
            Some(&reporter),
            resolved_target_provider,
            session_ids,
            repair_instance_ids,
            dry_run.unwrap_or(false),
        )
    })
    .await
    .map_err(|error| format!("修复 Codex 会话可见性任务失败: {}", error))?
}
