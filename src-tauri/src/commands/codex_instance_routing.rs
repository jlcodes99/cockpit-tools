use crate::models::{CodexInstanceModelRouting, InstanceLaunchMode};
use crate::modules;

pub(super) fn launch_mode_uses_desktop_runtime(launch_mode: &InstanceLaunchMode) -> bool {
    *launch_mode == InstanceLaunchMode::App
}

/// 绑定账号是否是可以直接登录的 OAuth 订阅账号（混合模型路由的底座账号）。
pub(super) fn routing_base_account_is_oauth(bind_account_id: Option<&str>) -> bool {
    let Some(bind_account_id) = bind_account_id.map(str::trim).filter(|value| !value.is_empty())
    else {
        return false;
    };
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        return false;
    }
    let account_id = modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
        .unwrap_or_else(|| bind_account_id.to_string());
    let Some(account) = modules::codex_account::load_account(&account_id) else {
        return false;
    };
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
}

pub(super) fn disabled_model_routing(
    routing: &CodexInstanceModelRouting,
) -> CodexInstanceModelRouting {
    CodexInstanceModelRouting {
        enabled: false,
        ..routing.clone()
    }
}

pub(super) fn validate_instance_model_routing(
    bind_account_id: Option<&str>,
    launch_mode: &InstanceLaunchMode,
    model_routing: Option<&CodexInstanceModelRouting>,
) -> Result<Option<CodexInstanceModelRouting>, String> {
    let Some(model_routing) = model_routing.filter(|routing| routing.enabled) else {
        return Ok(model_routing.cloned());
    };
    // 混合模型路由只在「桌面实例 + 绑定可直接登录的 OAuth 订阅账号」时生效。
    // 其他情况一律按关闭路由处理（渠道配置保留），不能因为路由拦住普通的账号切换、
    // 实例启动或其他功能。
    if !launch_mode_uses_desktop_runtime(launch_mode)
        || !routing_base_account_is_oauth(bind_account_id)
    {
        return Ok(Some(disabled_model_routing(model_routing)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CodexInstanceApiRoute;

    fn enabled_routing() -> CodexInstanceModelRouting {
        CodexInstanceModelRouting {
            enabled: true,
            version: 1,
            routes: vec![CodexInstanceApiRoute {
                id: "route-1".to_string(),
                namespace: "cpa".to_string(),
                provider_account_id: "cpa-account".to_string(),
                enabled: true,
                selected_models: None,
                extra_models: None,
            }],
        }
    }

    /// 绑定账号不是可直接登录的 OAuth 订阅账号时，路由必须自动关闭而不是拦住保存/启动，
    /// 且渠道配置要保留。
    #[test]
    fn routing_without_oauth_binding_is_disabled_instead_of_rejected() {
        let routing = enabled_routing();
        let normalized = validate_instance_model_routing(
            Some("api-key-account"),
            &InstanceLaunchMode::App,
            Some(&routing),
        )
        .expect("路由不应再拦截保存")
        .expect("路由配置需要保留");
        assert!(!normalized.enabled);
        assert_eq!(normalized.routes.len(), 1);
        assert_eq!(normalized.routes[0].namespace, "cpa");
    }

    #[test]
    fn routing_on_non_desktop_launch_mode_is_disabled_instead_of_rejected() {
        let routing = enabled_routing();
        let normalized = validate_instance_model_routing(
            None,
            &InstanceLaunchMode::Cli,
            Some(&routing),
        )
        .expect("路由不应再拦截 CLI 启动")
        .expect("路由配置需要保留");
        assert!(!normalized.enabled);
        assert_eq!(normalized.routes.len(), 1);
    }

    #[test]
    fn disabled_routing_is_returned_unchanged() {
        let routing = CodexInstanceModelRouting {
            enabled: false,
            ..CodexInstanceModelRouting::default()
        };
        let normalized =
            validate_instance_model_routing(None, &InstanceLaunchMode::App, Some(&routing))
                .expect("读取关闭状态的路由不应报错")
                .expect("配置需要保留");
        assert!(!normalized.enabled);
    }
}
