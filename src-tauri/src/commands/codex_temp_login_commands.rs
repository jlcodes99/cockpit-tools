// Codex 官方客户端临时登录命令。
//
// 本片段由 `commands/codex.rs` 通过 include! 纳入 `commands::codex` 模块，
// 对外调用仍使用 commands::codex 下的同名命令，不改变既有命令路径。

/// 打开官方客户端完成一次登录：使用一次性的空白临时 profile。
///
/// 关闭临时客户端后读取最终凭据，导入成功后清理临时 profile 和对应认证条目。
/// 失败保留凭据供重试，明确取消则丢弃；进度通过 codex:temp-login-progress 回传。
#[tauri::command]
pub fn start_codex_temp_login(
    app: AppHandle,
) -> Result<crate::modules::codex_temp_login::CodexTempLoginSession, String> {
    crate::modules::codex_temp_login::start(app)
}

/// 取消官方客户端登录（仍会关闭客户端并清理临时 profile）。
#[tauri::command]
pub fn cancel_codex_temp_login(session_id: String) -> Result<(), String> {
    crate::modules::codex_temp_login::cancel(session_id.as_str())
}

#[tauri::command]
pub fn retry_codex_temp_login_import(
    app: AppHandle,
    session_id: String,
) -> Result<crate::modules::codex_temp_login::CodexTempLoginSession, String> {
    crate::modules::codex_temp_login::retry_import(app, &session_id)
}

/// 手动触发一次残留清理（正常由启动巡检与定期巡检自动完成）。
#[tauri::command]
pub fn cleanup_codex_temp_login_artifacts(
) -> Result<crate::modules::codex_temp_login::CodexTempLoginCleanupReport, String> {
    Ok(crate::modules::codex_temp_login::cleanup_stale_sessions())
}
