#[tauri::command]
pub async fn apply_apikey_fun_gemini_cli_env(api_key: String, model: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::apikey_fun_gemini_cli::apply(&api_key, &model)
    })
    .await
    .map_err(|error| format!("写入 Gemini CLI 配置任务失败: {error}"))?
}
