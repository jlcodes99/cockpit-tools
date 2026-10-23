use crate::modules::chatgpt_plan::{
    self, ChatgptPlanAccountSummary, ChatgptPlanLogin, ChatgptPlanLoginStatus, ChatgptPlanModel,
};

#[tauri::command]
pub async fn chatgpt_plan_start_login(
    registration_id: Option<String>,
) -> Result<ChatgptPlanLogin, String> {
    chatgpt_plan::start_login(registration_id.as_deref()).await
}

#[tauri::command]
pub async fn chatgpt_plan_poll_login(login_id: String) -> Result<ChatgptPlanLoginStatus, String> {
    chatgpt_plan::poll_login(&login_id).await
}

#[tauri::command]
pub async fn chatgpt_plan_cancel_login(login_id: String) -> Result<(), String> {
    chatgpt_plan::cancel_login(&login_id).await
}

#[tauri::command]
pub async fn chatgpt_plan_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    chatgpt_plan::submit_callback_url(&login_id, &callback_url).await
}

#[tauri::command]
pub async fn chatgpt_plan_open_incognito_window(
    app_handle: tauri::AppHandle,
    login_id: String,
) -> Result<(), String> {
    chatgpt_plan::open_incognito_window(&app_handle, &login_id)
}

#[tauri::command]
pub async fn chatgpt_plan_list_accounts(
) -> Result<Vec<crate::modules::chatgpt_plan_service::ChatgptPlanServiceAccount>, String> {
    crate::modules::chatgpt_plan_service::list_accounts().await
}

#[tauri::command]
pub async fn chatgpt_plan_list_models(
    registration_id: String,
) -> Result<Vec<ChatgptPlanModel>, String> {
    crate::modules::chatgpt_plan_service::list_models(&registration_id).await
}

#[tauri::command]
pub async fn chatgpt_plan_verify(
    registration_id: String,
    model: String,
) -> Result<ChatgptPlanAccountSummary, String> {
    chatgpt_plan::verify(&registration_id, &model).await
}

#[tauri::command]
pub async fn chatgpt_plan_check_authorization(
    registration_id: String,
) -> Result<ChatgptPlanAccountSummary, String> {
    crate::modules::chatgpt_plan_service::check_authorization(&registration_id).await
}

#[tauri::command]
pub async fn chatgpt_plan_bind_account(
    registration_id: String,
    source_account_id: String,
) -> Result<ChatgptPlanAccountSummary, String> {
    chatgpt_plan::bind_account(&registration_id, &source_account_id).await
}

#[tauri::command]
pub async fn chatgpt_plan_disconnect(
    registration_id: String,
    disposition: Option<String>,
) -> Result<crate::modules::chatgpt_plan_service::ChatgptPlanAuthorizationActionResult, String> {
    crate::modules::chatgpt_plan_service::disconnect_with_disposition(
        &registration_id,
        disposition.as_deref().unwrap_or("pause"),
    )
    .await
}
