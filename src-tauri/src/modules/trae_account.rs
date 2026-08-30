// Trae 账号模块统一入口。
// 业务分片按平台存储、导入 payload、产品路径和 Token 注入组织，
// 通过 include! 保持原模块作用域与调用路径不变。
include!("trae_account_platform_storage.rs");
include!("trae_account_import_payload.rs");
include!("trae_account_product_paths.rs");
include!("trae_account_token_injection.rs");

#[cfg(test)]
mod tests {
    include!("trae_account_tests.rs");
}
