use super::*;

pub(super) fn provider_binding_for_switch(bind_account_id: &str) -> Option<String> {
    modules::codex_account::load_account(bind_account_id)
        .filter(modules::codex_local_access::account_requires_provider_gateway)
        .and_then(|account| modules::codex_instance::provider_gateway_bind_account_id(&account.id))
}

pub(crate) async fn prepare_project_state_after_switch(instance_id: String, data_dir: PathBuf) -> bool {
    let result = tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_visibility::prepare_desktop_project_state_after_switch(
            &instance_id, &data_dir,
        )
        .map_err(|error| format!("instance_id={instance_id}, error={error}"))
    })
    .await;
    match result {
        Ok(Ok(_)) => false,
        Ok(Err(error)) => {
            modules::logger::log_warn(&format!("[Codex Local Projects] switch preparation deferred: {error}"));
            true
        }
        Err(error) => {
            modules::logger::log_warn(&format!("[Codex Local Projects] switch preparation task failed: {error}"));
            true
        }
    }
}

#[tauri::command]
pub async fn codex_switch_instance_account(
    instance_id: String,
    bind_account_id: String,
    app_speed: Option<CodexAppSpeed>,
) -> Result<bool, String> {
    let target = resolve_codex_instance_start_target(&instance_id)?;
    let profile = &target.user_data_dir;
    let _lease = modules::codex_account::try_acquire_profile_mutation_lease(profile, "instance-switch")?;
    let bind_account_id = effective_provider_bind_account_id(&bind_account_id);
    let binding = provider_binding_for_switch(&bind_account_id)
        .unwrap_or_else(|| bind_account_id.into_owned());
    validate_instance_model_routing(Some(&binding), &target.launch_mode, target.model_routing.as_ref())?;

    if let Some(oauth_id) = modules::codex_account::oauth_account_id_for_runtime_binding(Some(&binding)) {
        modules::codex_account::prepare_account_for_instance_launch_preflight(&oauth_id)
            .await
            .map_err(|error| modules::codex_account::format_account_switch_error(&oauth_id, error))?;
    }
    modules::codex_app_injection::stop_for_profile(profile);
    if launch_mode_uses_desktop_runtime(&target.launch_mode) {
        let is_default = target.is_default;
        let profile = profile.to_string_lossy().into_owned();
        stop_runtime_for_start(StartRuntimeState::NeedsStop, move || {
            if is_default {
                modules::process::close_codex_default(20)
            } else {
                modules::process::close_codex_instances(&[profile], 20)
            }
        })
        .await?;
    }
    restore_mixed_model_gateway_when_disabled(profile, None).await?;
    inject_preflighted_bound_account_to_profile(profile, &binding).await?;
    if target.is_default {
        modules::codex_instance::update_default_pid(None)?;
    } else {
        modules::codex_instance::update_instance_pid(&instance_id, None)?;
    }
    let gateway_account_id = modules::codex_instance::parse_provider_gateway_bind_account_id(&binding);
    codex_update_instance(
        instance_id.clone(), None, None, None, Some(Some(binding)), None,
        Some(false), None, app_speed, None, Some(true),
    )
    .await?;
    if let Some(account_id) = gateway_account_id {
        modules::codex_cli_daemon::after_provider_gateway_profile_write(profile, &account_id);
    }
    if launch_mode_uses_desktop_runtime(&target.launch_mode) {
        return Ok(prepare_project_state_after_switch(instance_id, target.user_data_dir).await);
    }
    Ok(false)
}
