use crate::modules::codebuddy_session::{
    CodebuddySessionFilter, CodebuddySessionOverview, CodebuddySessionPlatform,
    CodebuddySessionRecord,
};

#[tauri::command]
pub fn codebuddy_list_sessions(
    platform: String,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<Vec<CodebuddySessionRecord>, String> {
    let overview = codebuddy_list_session_overview(platform, keyword, status)?;
    if overview.sessions.is_empty() && !overview.warnings.is_empty() {
        return Err(overview.warnings.join("\n"));
    }
    Ok(overview.sessions)
}

#[tauri::command]
pub fn codebuddy_list_session_overview(
    platform: String,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<CodebuddySessionOverview, String> {
    let p = parse_platform(&platform)?;
    let filter = CodebuddySessionFilter { keyword, status };
    crate::modules::codebuddy_session::list_sessions(&p, &filter)
}

fn parse_platform(platform: &str) -> Result<CodebuddySessionPlatform, String> {
    match platform {
        "cn" => Ok(CodebuddySessionPlatform::Cn),
        "intl" => Ok(CodebuddySessionPlatform::Intl),
        "workbuddy" => Ok(CodebuddySessionPlatform::Workbuddy),
        _ => Err(format!(
            "Unknown platform: {}. Use 'cn', 'intl' or 'workbuddy'.",
            platform
        )),
    }
}

#[tauri::command]
pub async fn workbuddy_rebind_workspace(
    instance_id: String,
    old_cwd: String,
    new_cwd: String,
) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::modules::codebuddy_session::rebind_workbuddy_workspace(
            &instance_id,
            &old_cwd,
            &new_cwd,
        )
    })
    .await
    .map_err(|error| format!("WorkBuddy workspace task failed: {error}"))?
}
