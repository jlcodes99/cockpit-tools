// cockpit-core Trae 账号模块统一入口。
// 按平台存储、导入、产品路径、注入和刷新职责拆分，保持原模块作用域不变。
include!("trae_account_core_platform_storage.rs");
include!("trae_account_core_import.rs");
include!("trae_account_core_product_paths.rs");
include!("trae_account_core_injection.rs");
include!("trae_account_core_refresh.rs");

#[cfg(test)]
mod tests {
    include!("trae_account_core_tests.rs");
}
