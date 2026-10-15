// Claude 账号模块统一入口。
// 业务分片按存储、OAuth/provider、桌面 profile 和认证导出组织，
// 通过 include! 保持原模块作用域，调用方和持久化行为不变。
include!("claude_account_core_storage.rs");
include!("claude_account_oauth_provider.rs");
include!("claude_account_cookie_snapshot.rs");
include!("claude_account_desktop_profile.rs");
include!("claude_account_desktop_auth.rs");
include!("claude_account_cli_sync.rs");
include!("claude_account_oauth_updates.rs");

#[cfg(test)]
mod tests {
    include!("claude_account_tests.rs");
}

#[cfg(test)]
mod cookie_snapshot_tests {
    include!("claude_account_cookie_snapshot_tests.rs");
}

#[cfg(test)]
mod cli_sync_tests {
    include!("claude_account_cli_sync_tests.rs");
}
