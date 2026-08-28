use crate::models::opencode_go::{
    OpenCodeGoConnectionSummary, OpenCodeGoQuotaBatchResult, OpenCodeGoQuotaQueryResult,
};
use crate::modules::opencode_go;

#[tauri::command]
pub async fn list_opencode_go_connections() -> Result<Vec<OpenCodeGoConnectionSummary>, String> {
    tauri::async_runtime::spawn_blocking(opencode_go::list_connections)
        .await
        .map_err(|_| "OPENCODE_GO_STORE_TASK_FAILED".to_string())?
}

// Keep API-key command names explicit for frontend callers while the connection-named
// commands remain available for compatibility with the initial integration.
#[tauri::command]
pub async fn list_opencode_go_api_keys() -> Result<Vec<OpenCodeGoConnectionSummary>, String> {
    list_opencode_go_connections().await
}

#[tauri::command]
pub async fn create_opencode_go_connection(
    name: String,
    api_key: String,
    email: Option<String>,
    provider: Option<String>,
) -> Result<OpenCodeGoConnectionSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        opencode_go::create_connection(
            name,
            api_key,
            email,
            provider.unwrap_or_else(|| "go".to_string()),
        )
    })
        .await
        .map_err(|_| "OPENCODE_GO_STORE_TASK_FAILED".to_string())?
}

#[tauri::command]
pub async fn add_opencode_go_api_key(
    name: String,
    api_key: String,
) -> Result<OpenCodeGoConnectionSummary, String> {
    create_opencode_go_connection(name, api_key, None, Some("go".to_string())).await
}

#[tauri::command]
pub async fn update_opencode_go_connection(
    connection_id: String,
    name: Option<String>,
    api_key: Option<String>,
    email: Option<String>,
) -> Result<OpenCodeGoConnectionSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        opencode_go::update_connection(connection_id, name, api_key, email)
    })
    .await
    .map_err(|_| "OPENCODE_GO_STORE_TASK_FAILED".to_string())?
}

#[tauri::command]
pub async fn update_opencode_go_api_key(
    connection_id: String,
    name: Option<String>,
    api_key: Option<String>,
) -> Result<OpenCodeGoConnectionSummary, String> {
    update_opencode_go_connection(connection_id, name, api_key, None).await
}

#[tauri::command]
pub async fn set_opencode_go_connection_enabled(
    connection_id: String,
    enabled: bool,
) -> Result<OpenCodeGoConnectionSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        opencode_go::set_connection_enabled(connection_id, enabled)
    })
    .await
    .map_err(|_| "OPENCODE_GO_STORE_TASK_FAILED".to_string())?
}

#[tauri::command]
pub async fn delete_opencode_go_connection(connection_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || opencode_go::delete_connection(connection_id))
        .await
        .map_err(|_| "OPENCODE_GO_STORE_TASK_FAILED".to_string())?
}

#[tauri::command]
pub async fn set_opencode_go_api_key_enabled(
    connection_id: String,
    enabled: bool,
) -> Result<OpenCodeGoConnectionSummary, String> {
    set_opencode_go_connection_enabled(connection_id, enabled).await
}

#[tauri::command]
pub async fn delete_opencode_go_api_key(connection_id: String) -> Result<(), String> {
    delete_opencode_go_connection(connection_id).await
}

#[tauri::command]
async fn fetch_quota(
    connection_id: &str,
) -> Result<crate::models::opencode_go::OpenCodeGoQuotaSnapshot, String> {
    let key = opencode_go::api_key(connection_id)?;
    let client = crate::modules::opencode_go_http::OpenCodeGoHttpClient::new(opencode_go::BASE_URL)
        .map_err(|error| quota_error_kind(&error))?;
    let body = client
        .fetch_usage(&key)
        .await
        .map_err(|error| quota_error_kind(&error))?;
    opencode_go::sanitize_quota_response(&body).map_err(|_| "unavailable".to_string())
}

async fn test_zen_connection(api_key: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "unavailable".to_string())?;
    let response = client
        .get("https://opencode.ai/zen/v1/models")
        .bearer_auth(api_key.trim())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| if error.is_timeout() { "network" } else { "network" }.to_string())?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err("authentication".to_string()),
        429 => Err("rate_limit".to_string()),
        _ => Err("unavailable".to_string()),
    }
}

#[tauri::command]
pub async fn test_opencode_connection(connection_id: String) -> Result<(), String> {
    let connection = opencode_go::list_connections()?
        .into_iter()
        .find(|item| item.id == connection_id)
        .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())?;
    if !connection.enabled {
        return Err("OPENCODE_GO_CONNECTION_DISABLED".to_string());
    }
    let result = if connection.provider == "zen" {
        let key = opencode_go::api_key(&connection_id)?;
        test_zen_connection(&key).await
    } else {
        fetch_quota(&connection_id).await.map(|_| ())
    };
    result.map_err(|kind| format!("OPENCODE_CONNECTION_TEST_{}", kind.to_ascii_uppercase()))
}

fn quota_error_kind(error: &crate::modules::opencode_go_http::OpenCodeGoHttpError) -> String {
    use crate::modules::opencode_go_http::OpenCodeGoHttpError;
    match error {
        OpenCodeGoHttpError::Authentication => "authentication",
        OpenCodeGoHttpError::RateLimited { .. } => "rate_limit",
        OpenCodeGoHttpError::Timeout | OpenCodeGoHttpError::Network => "network",
        _ => "unavailable",
    }
    .to_string()
}

#[tauri::command]
pub async fn query_opencode_go_quota(
    connection_id: String,
) -> Result<OpenCodeGoQuotaQueryResult, String> {
    let connection = opencode_go::list_connections()?
        .into_iter()
        .find(|item| item.id == connection_id)
        .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())?;
    if !connection.enabled {
        return Err("OPENCODE_GO_CONNECTION_DISABLED".to_string());
    }
    if connection.provider == "zen" {
        return Err("OPENCODE_ZEN_USAGE_UNAVAILABLE".to_string());
    }
    let quota = match fetch_quota(&connection_id).await {
        Ok(quota) => quota,
        // Preserve local CRUD errors; only provider failures become usage errors.
        Err(kind) if kind.starts_with("OPENCODE_GO_") => return Err(kind),
        Err(kind) => {
            let _ = opencode_go::save_quota(&connection_id, Err(kind.clone()));
            return Err(format!("OPENCODE_GO_USAGE_{}", kind.to_ascii_uppercase()));
        }
    };
    let connection = opencode_go::save_quota(&connection_id, Ok(quota.clone()))?;
    Ok(OpenCodeGoQuotaQueryResult { connection, quota })
}

#[tauri::command]
pub async fn refresh_opencode_go_api_key(
    connection_id: String,
) -> Result<OpenCodeGoQuotaQueryResult, String> {
    query_opencode_go_quota(connection_id).await
}

#[tauri::command]
pub async fn query_all_opencode_go_quotas() -> Result<OpenCodeGoQuotaBatchResult, String> {
    let connections = opencode_go::list_connections()?;
    let mut updated = Vec::with_capacity(connections.len());
    for connection in connections {
        if !connection.enabled || connection.provider == "zen" {
            updated.push(connection);
            continue;
        }
        match fetch_quota(&connection.id).await {
            Ok(quota) => updated.push(opencode_go::save_quota(&connection.id, Ok(quota))?),
            Err(kind) => updated.push(opencode_go::save_quota(&connection.id, Err(kind))?),
        }
    }
    Ok(OpenCodeGoQuotaBatchResult {
        connections: updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("opencode-go-command-test-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn api_key_commands_cover_crud_without_an_artificial_key_limit() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let mut created = Vec::new();
        for index in 0..5 {
            created.push(
                add_opencode_go_api_key(format!("Key {}", index + 1), format!("test-key-{index}"))
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(list_opencode_go_api_keys().await.unwrap().len(), 5);

        let updated = update_opencode_go_connection(
            created[0].id.clone(),
            Some("Renamed".into()),
            Some("replacement-test-key".into()),
            Some("owner@example.com".into()),
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.key_hint, "repl****-key");
        assert_eq!(updated.email_hint.as_deref(), Some("o***@example.com"));

        delete_opencode_go_api_key(created[1].id.clone())
            .await
            .unwrap();
        assert_eq!(list_opencode_go_api_keys().await.unwrap().len(), 4);

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn refresh_command_rejects_an_unknown_key_before_network_access() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        assert_eq!(
            refresh_opencode_go_api_key("ocg_missing".into())
                .await
                .unwrap_err(),
            "OPENCODE_GO_CONNECTION_NOT_FOUND"
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn query_command_source_never_logs_key_material() {
        // Literals are assembled at runtime so this test body does not trip itself.
        let forbidden = [
            concat!("log", "_info"),
            concat!("log", "_warn"),
            concat!("log", "_error"),
            concat!("format!", "(\"{}\", key)"),
        ];
        for needle in forbidden {
            assert!(!include_str!("opencode_go.rs").contains(needle));
        }
    }
}
