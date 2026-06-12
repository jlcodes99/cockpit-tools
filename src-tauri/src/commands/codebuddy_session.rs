use crate::modules::codebuddy_session::{
    CodebuddySessionFilter, CodebuddySessionPlatform, CodebuddySessionRecord,
};

#[tauri::command]
pub fn codebuddy_list_sessions(
    platform: String,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<Vec<CodebuddySessionRecord>, String> {
    let p = parse_platform(&platform)?;
    let filter = CodebuddySessionFilter { keyword, status };
    crate::modules::codebuddy_session::list_sessions(&p, &filter)
}

fn parse_platform(platform: &str) -> Result<CodebuddySessionPlatform, String> {
    match platform {
        "cn" => Ok(CodebuddySessionPlatform::Cn),
        "intl" => Ok(CodebuddySessionPlatform::Intl),
        _ => Err(format!("Unknown platform: {}. Use 'cn' or 'intl'.", platform)),
    }
}
