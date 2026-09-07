use crate::models::{CodexInstanceModelRouting, InstanceLaunchMode};
use crate::modules;

pub(super) fn launch_mode_uses_desktop_runtime(launch_mode: &InstanceLaunchMode) -> bool {
    *launch_mode == InstanceLaunchMode::App
}

pub(super) fn validate_instance_model_routing(
    bind_account_id: Option<&str>,
    launch_mode: &InstanceLaunchMode,
    model_routing: Option<&CodexInstanceModelRouting>,
) -> Result<Option<CodexInstanceModelRouting>, String> {
    let Some(model_routing) = model_routing.filter(|routing| routing.enabled) else {
        return Ok(model_routing.cloned());
    };
    if !launch_mode_uses_desktop_runtime(launch_mode) {
        return Err("混合模型路由第一版仅支持桌面版实例".to_string());
    }
    let normalized = modules::codex_local_access::validate_mixed_model_routing_config(
        bind_account_id,
        model_routing,
    )?;
    Ok(Some(normalized))
}

pub(super) fn model_routing_update_error(error: String, rollback_errors: Vec<String>) -> String {
    if rollback_errors.is_empty() {
        return error;
    }
    format!(
        "{}；恢复原实例配置时仍有错误: {}",
        error,
        rollback_errors.join("；")
    )
}
