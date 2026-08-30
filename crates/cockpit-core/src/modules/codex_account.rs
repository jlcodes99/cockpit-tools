// cockpit-core Codex 账号模块统一入口。
// 按 provider、Token/index、authority、runtime/import 和 mutations/quota 职责拆分，
// 通过 include! 保持原模块作用域与调用路径不变。
include!("codex_account_core_provider.rs");
include!("codex_account_core_tokens.rs");
include!("codex_account_core_authority.rs");
include!("codex_account_core_runtime_import.rs");

#[cfg(test)]
mod tests {
    include!("codex_account_core_tests.rs");
}

include!("codex_account_core_mutations_quota.rs");
