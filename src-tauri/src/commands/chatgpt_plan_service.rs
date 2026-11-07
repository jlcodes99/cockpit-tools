#[tauri::command]
pub async fn chatgpt_plan_join(
    registration_id: String,
    model: Option<String>,
) -> Result<(), String> {
    crate::modules::chatgpt_plan_service::join(
        &registration_id,
        model.as_deref().unwrap_or_default(),
    )
    .await
}

#[tauri::command]
pub async fn chatgpt_plan_remove_from_service(registration_id: String) -> Result<(), String> {
    crate::modules::chatgpt_plan_service::remove_from_service(&registration_id).await
}
#[tauri::command]
pub async fn chatgpt_plan_set_service_mode(
    source_account_id: String,
    registration_id: Option<String>,
    mode: String,
    model: Option<String>,
    intent: Option<String>,
) -> Result<crate::models::codex_local_access::CodexLocalAccessState, String> {
    crate::modules::chatgpt_plan_service::set_service_mode_with_intent(
        &source_account_id,
        registration_id.as_deref(),
        &mode,
        model.as_deref(),
        intent.as_deref().unwrap_or("join"),
    )
    .await
}

#[tauri::command]
pub async fn chatgpt_plan_unbind_account(
    registration_id: String,
    disposition: Option<String>,
) -> Result<crate::modules::chatgpt_plan_service::ChatgptPlanAuthorizationActionResult, String> {
    crate::modules::chatgpt_plan_service::unbind_with_disposition(
        &registration_id,
        disposition.as_deref().unwrap_or("pause"),
    )
    .await
}

#[tauri::command]
pub async fn chatgpt_plan_release_authorization(
    registration_id: String,
    disposition: String,
) -> Result<crate::modules::chatgpt_plan_service::ChatgptPlanAuthorizationActionResult, String> {
    crate::modules::chatgpt_plan_service::release_authorization(&registration_id, &disposition)
        .await
}
