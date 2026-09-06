// Codex Local Access 统一入口。
// 业务分片只在完整顶层 item 之间切开，通过 include! 保持同一模块作用域；
// 调用方、网关生命周期、请求转换和账号池行为均与拆分前一致。
include!("codex_local_access_foundation.rs");
include!("codex_local_access_quota_cooldown.rs");
include!("codex_local_access_request_transform.rs");
include!("codex_local_access_routing_pricing.rs");
include!("codex_local_access_request_logs.rs");
include!("codex_local_access_profile_takeover.rs");
include!("codex_local_access_sidecar_config.rs");
include!("codex_local_access_sidecar_runtime.rs");
include!("codex_local_access_collection.rs");
include!("codex_local_access_gateway_runtime.rs");
include!("codex_local_access_provider_gateway.rs");
include!("codex_local_access_probe_chat.rs");
include!("codex_local_access_commands.rs");
include!("codex_local_access_http.rs");
include!("codex_local_access_recovery.rs");

#[cfg(test)]
mod tests {
    include!("codex_local_access_tests_sidecar_gateway.rs");
    include!("codex_local_access_tests_pricing_profile.rs");
    include!("codex_local_access_tests_request_routing.rs");
    include!("codex_local_access_tests_takeover.rs");
}
