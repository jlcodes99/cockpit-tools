// Codex 账号模块统一入口。
// 各业务分片只在完整顶层 item 之间切开，并通过 include! 保持同一模块作用域，
// 因此调用方无需改变，启动、切号、刷新、重新授权和导入行为保持一致。
include!("codex_account_provider.rs");
include!("codex_account_check.rs");
include!("codex_account_model_catalog.rs");
include!("codex_account_context_management.rs");
include!("codex_account_storage_locks.rs");
include!("codex_account_token_refresh.rs");
include!("codex_account_index.rs");
include!("codex_account_lifecycle.rs");
include!("codex_account_authority_sync.rs");
include!("codex_account_projection.rs");
include!("codex_account_runtime_switch.rs");
include!("codex_account_import.rs");

#[cfg(test)]
mod tests {
    include!("codex_account_tests_identity_import_refresh.rs");
    include!("codex_account_tests_storage_provider.rs");
    include!("codex_account_tests_model_catalog.rs");
    include!("codex_account_tests_quick_config.rs");
}

include!("codex_account_mutations_quota.rs");
