use crate::models::opencode_go::{
    OpenCodeGoConnectionSummary, OpenCodeGoQuotaError, OpenCodeGoQuotaSnapshot,
    OpenCodeGoQuotaWindowSnapshot,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const CONNECTIONS_FILE: &str = "opencode_go_connections.json";
const STORAGE_KIND: &str = "opencode_go";
pub const BASE_URL: &str = "https://opencode.ai/zen/go/v1";

static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConnection {
    id: String,
    name: String,
    api_key: String,
    #[serde(default)]
    email: Option<String>,
    created_at: i64,
    updated_at: i64,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default)]
    quota: Option<OpenCodeGoQuotaSnapshot>,
    #[serde(default)]
    quota_error: Option<OpenCodeGoQuotaError>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionStore {
    #[serde(default = "store_version")]
    version: u32,
    #[serde(default)]
    connections: Vec<StoredConnection>,
}

fn store_version() -> u32 {
    1
}

fn default_enabled() -> bool {
    true
}

fn default_provider() -> String {
    "go".to_string()
}

fn normalize_provider(provider: &str) -> Result<String, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "go" => Ok("go".to_string()),
        "zen" => Ok("zen".to_string()),
        _ => Err("OPENCODE_PROVIDER_INVALID".to_string()),
    }
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(CONNECTIONS_FILE))
}

fn normalize_name(name: &str) -> String {
    name.trim().to_string()
}

fn normalize_key(api_key: &str) -> Result<String, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("OPENCODE_GO_API_KEY_REQUIRED".to_string());
    }
    if key.chars().any(char::is_whitespace)
        || key.to_ascii_lowercase().starts_with("redacted:")
        || key.eq_ignore_ascii_case("[redacted]")
        || key.contains("****")
    {
        return Err("OPENCODE_GO_API_KEY_INVALID".to_string());
    }
    Ok(key.to_string())
}

fn normalize_email(email: Option<&str>) -> Result<Option<String>, String> {
    let Some(email) = email.map(str::trim).filter(|email| !email.is_empty()) else {
        return Ok(None);
    };
    if email.len() > 254 || email.chars().any(char::is_whitespace) || !email.contains('@') {
        return Err("OPENCODE_GO_EMAIL_INVALID".to_string());
    }
    let mut parts = email.rsplitn(2, '@');
    let domain = parts.next().unwrap_or_default();
    let local = parts.next().unwrap_or_default();
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err("OPENCODE_GO_EMAIL_INVALID".to_string());
    }
    Ok(Some(format!("{}@{}", local, domain.to_ascii_lowercase())))
}

fn email_hint(email: Option<&str>) -> Option<String> {
    let email = email?;
    let (local, domain) = email.split_once('@')?;
    let visible = local.chars().next()?;
    Some(format!("{}***@{}", visible, domain))
}

fn validate_connection_id(id: &str) -> Result<&str, String> {
    let id = id.trim();
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("OPENCODE_GO_CONNECTION_ID_INVALID".to_string());
    }
    Ok(id)
}

fn key_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    match chars.len() {
        0 => String::new(),
        1..=8 => format!("{}****", chars.iter().take(2).collect::<String>()),
        len => format!(
            "{}****{}",
            chars.iter().take(4).collect::<String>(),
            chars.iter().skip(len - 4).collect::<String>()
        ),
    }
}

fn to_summary(connection: &StoredConnection) -> OpenCodeGoConnectionSummary {
    OpenCodeGoConnectionSummary {
        id: connection.id.clone(),
        name: connection.name.clone(),
        key_hint: key_hint(&connection.api_key),
        email_hint: email_hint(connection.email.as_deref()),
        created_at: connection.created_at,
        updated_at: connection.updated_at,
        enabled: connection.enabled,
        provider: connection.provider.clone(),
        quota: connection.quota.clone(),
        quota_error: connection.quota_error.clone(),
    }
}

fn read_store_from(path: &Path) -> Result<ConnectionStore, String> {
    if !path.exists() {
        return Ok(ConnectionStore {
            version: store_version(),
            connections: Vec::new(),
        });
    }
    let content =
        fs::read_to_string(path).map_err(|_| "OPENCODE_GO_STORE_READ_FAILED".to_string())?;
    let (store, needs_rotation) =
        crate::modules::secure_account_storage::deserialize_account_file::<ConnectionStore>(
            path, &content,
        )
        .map_err(|_| "OPENCODE_GO_STORE_DECRYPT_FAILED".to_string())?;
    validate_store(&store)?;
    if needs_rotation {
        write_store_to(path, &store)?;
    }
    Ok(store)
}

fn validate_store(store: &ConnectionStore) -> Result<(), String> {
    for connection in &store.connections {
        validate_connection_id(&connection.id)?;
        normalize_key(&connection.api_key)?;
        normalize_email(connection.email.as_deref())?;
    }
    Ok(())
}

fn write_store_to(path: &Path, store: &ConnectionStore) -> Result<(), String> {
    validate_store(store)?;
    let content =
        crate::modules::secure_account_storage::serialize_account_file(STORAGE_KIND, store)
            .map_err(|_| "OPENCODE_GO_STORE_ENCRYPT_FAILED".to_string())?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|_| "OPENCODE_GO_STORE_WRITE_FAILED".to_string())
}

fn with_store<T>(
    operation: impl FnOnce(&mut ConnectionStore) -> Result<(T, bool), String>,
) -> Result<T, String> {
    let _guard = STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = store_path()?;
    let mut store = read_store_from(&path)?;
    let (result, changed) = operation(&mut store)?;
    if changed {
        write_store_to(&path, &store)?;
    }
    Ok(result)
}

fn ordered_summaries(connections: &[StoredConnection]) -> Vec<OpenCodeGoConnectionSummary> {
    let mut ordered = connections.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    ordered.into_iter().map(to_summary).collect()
}

pub fn list_connections() -> Result<Vec<OpenCodeGoConnectionSummary>, String> {
    with_store(|store| Ok((ordered_summaries(&store.connections), false)))
}

/// The account-transfer boundary carries the existing encrypted store envelope,
/// never decoded connection data. It can only be restored by the same local
/// secure-storage key, so a bundle cannot disclose or transplant credentials.
pub fn export_encrypted_transfer(connection_ids: Vec<String>) -> Result<String, String> {
    let selected = connection_ids
        .into_iter()
        .map(|id| validate_connection_id(&id).map(str::to_string))
        .collect::<Result<std::collections::HashSet<_>, _>>()?;
    let _guard = STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = store_path()?;
    let store = read_store_from(&path)?;
    if selected.len() != store.connections.len()
        || store
            .connections
            .iter()
            .any(|connection| !selected.contains(&connection.id))
    {
        return Err("OPENCODE_GO_TRANSFER_SELECTION_INVALID".to_string());
    }
    fs::read_to_string(path).map_err(|_| "OPENCODE_GO_STORE_READ_FAILED".to_string())
}

pub fn import_encrypted_transfer(
    content: String,
) -> Result<Vec<OpenCodeGoConnectionSummary>, String> {
    let imported = crate::modules::secure_account_storage::deserialize_encrypted_account_file::<
        ConnectionStore,
    >(STORAGE_KIND, &content)
    .map_err(|_| "OPENCODE_GO_TRANSFER_UNAVAILABLE".to_string())?;
    validate_store(&imported)?;
    with_store(move |store| {
        *store = imported;
        Ok((ordered_summaries(&store.connections), true))
    })
}

pub fn create_connection(
    name: String,
    api_key: String,
    email: Option<String>,
    provider: String,
) -> Result<OpenCodeGoConnectionSummary, String> {
    let name = normalize_name(&name);
    let api_key = normalize_key(&api_key)?;
    let email = normalize_email(email.as_deref())?;
    let provider = normalize_provider(&provider)?;
    with_store(move |store| {
        if store.connections.iter().any(|item| item.api_key == api_key) {
            return Err("OPENCODE_GO_API_KEY_EXISTS".to_string());
        }
        let now = chrono::Utc::now().timestamp();
        let connection = StoredConnection {
            id: format!("ocg_{}", uuid::Uuid::new_v4().simple()),
            name,
            api_key,
            email,
            created_at: now,
            updated_at: now,
            enabled: true,
            provider,
            quota: None,
            quota_error: None,
        };
        let summary = to_summary(&connection);
        store.connections.push(connection);
        Ok((summary, true))
    })
}

pub fn update_connection(
    connection_id: String,
    name: Option<String>,
    api_key: Option<String>,
    email: Option<String>,
) -> Result<OpenCodeGoConnectionSummary, String> {
    let connection_id = validate_connection_id(&connection_id)?.to_string();
    let name = name.map(|value| normalize_name(&value));
    let api_key = api_key.map(|value| normalize_key(&value)).transpose()?;
    let email = email.map(|value| normalize_email(Some(&value))).transpose()?;
    with_store(move |store| {
        if let Some(ref key) = api_key {
            if store
                .connections
                .iter()
                .any(|item| item.id != connection_id && item.api_key == *key)
            {
                return Err("OPENCODE_GO_API_KEY_EXISTS".to_string());
            }
        }
        let connection = store
            .connections
            .iter_mut()
            .find(|item| item.id == connection_id)
            .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())?;
        if let Some(name) = name {
            connection.name = name;
        }
        if let Some(api_key) = api_key {
            connection.api_key = api_key;
            connection.quota = None;
            connection.quota_error = None;
        }
        if let Some(email) = email {
            connection.email = email;
        }
        connection.updated_at = chrono::Utc::now().timestamp();
        Ok((to_summary(connection), true))
    })
}

pub fn set_connection_enabled(
    connection_id: String,
    enabled: bool,
) -> Result<OpenCodeGoConnectionSummary, String> {
    let connection_id = validate_connection_id(&connection_id)?.to_string();
    with_store(move |store| {
        let connection = store
            .connections
            .iter_mut()
            .find(|item| item.id == connection_id)
            .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())?;
        connection.enabled = enabled;
        connection.updated_at = chrono::Utc::now().timestamp();
        Ok((to_summary(connection), true))
    })
}

pub fn delete_connection(connection_id: String) -> Result<(), String> {
    let connection_id = validate_connection_id(&connection_id)?.to_string();
    with_store(move |store| {
        let original_len = store.connections.len();
        store.connections.retain(|item| item.id != connection_id);
        if store.connections.len() == original_len {
            return Err("OPENCODE_GO_CONNECTION_NOT_FOUND".to_string());
        }
        Ok(((), true))
    })
}

pub fn api_key(connection_id: &str) -> Result<String, String> {
    let connection_id = validate_connection_id(connection_id)?.to_string();
    with_store(move |store| {
        store
            .connections
            .iter()
            .find(|item| item.id == connection_id)
            .map(|item| (item.api_key.clone(), false))
            .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())
    })
}

pub fn save_quota(
    connection_id: &str,
    quota: Result<OpenCodeGoQuotaSnapshot, String>,
) -> Result<OpenCodeGoConnectionSummary, String> {
    let connection_id = validate_connection_id(connection_id)?.to_string();
    with_store(move |store| {
        let connection = store
            .connections
            .iter_mut()
            .find(|item| item.id == connection_id)
            .ok_or_else(|| "OPENCODE_GO_CONNECTION_NOT_FOUND".to_string())?;
        match quota {
            Ok(snapshot) => {
                connection.quota = Some(snapshot);
                connection.quota_error = None;
            }
            Err(kind) => {
                connection.quota_error = Some(OpenCodeGoQuotaError {
                    kind,
                    occurred_at: chrono::Utc::now().timestamp(),
                });
            }
        }
        connection.updated_at = chrono::Utc::now().timestamp();
        Ok((to_summary(connection), true))
    })
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn json_timestamp(value: &serde_json::Value) -> Option<i64> {
    if let Some(number) = json_number(value) {
        if !number.is_finite() || number <= 0.0 {
            return None;
        }
        return Some(
            (if number > 10_000_000_000.0 {
                number / 1000.0
            } else {
                number
            })
            .floor() as i64,
        );
    }
    value
        .as_str()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw.trim()).ok())
        .map(|timestamp| timestamp.timestamp())
}

fn unavailable_quota_window(status: &str, error: &str) -> OpenCodeGoQuotaWindowSnapshot {
    OpenCodeGoQuotaWindowSnapshot {
        status: status.to_string(),
        used_percent: None,
        remaining_percent: None,
        resets_at: None,
        error: Some(error.to_string()),
    }
}

fn quota_window(body: &serde_json::Value, key: &str) -> OpenCodeGoQuotaWindowSnapshot {
    let Some(window) = body
        .get("usage")
        .and_then(|usage| usage.get(key))
        .filter(|value| value.is_object())
    else {
        return unavailable_quota_window("unavailable", "window missing");
    };
    if window.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return unavailable_quota_window("error", "window unavailable");
    }
    let used_percent = window
        .get("percent")
        .and_then(json_number)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
    let resets_at = window
        .get("resetsAt")
        .or_else(|| window.get("resets_at"))
        .and_then(json_timestamp);
    let error = if used_percent.is_none() {
        Some("percent invalid".to_string())
    } else if resets_at.is_none() {
        Some("reset missing".to_string())
    } else {
        None
    };
    OpenCodeGoQuotaWindowSnapshot {
        status: if error.is_some() {
            "error"
        } else {
            "available"
        }
        .to_string(),
        used_percent,
        remaining_percent: used_percent.map(|used| 100.0 - used),
        resets_at,
        error,
    }
}

pub fn sanitize_quota_response(
    body: &serde_json::Value,
) -> Result<OpenCodeGoQuotaSnapshot, String> {
    let rolling = quota_window(body, "rolling");
    let weekly = quota_window(body, "weekly");
    let monthly = quota_window(body, "monthly");
    let available = [&rolling, &weekly, &monthly]
        .iter()
        .filter(|window| window.used_percent.is_some())
        .count();
    let exhausted = [&rolling, &weekly, &monthly]
        .iter()
        .any(|window| window.remaining_percent == Some(0.0));
    Ok(OpenCodeGoQuotaSnapshot {
        rolling,
        weekly,
        monthly,
        status: if available == 0 {
            "unavailable"
        } else if available < 3 {
            "partial"
        } else if exhausted {
            "exhausted"
        } else {
            "available"
        }
        .to_string(),
        queried_at: chrono::Utc::now().timestamp(),
    })
}

pub fn classify_query_error(status: Option<u16>) -> String {
    match status {
        Some(401 | 403) => "authentication",
        Some(429) => "rate_limit",
        Some(_) => "unavailable",
        None => "network",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("opencode-go-store-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn store_encrypts_keys_and_exposes_only_hints() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let summary = create_connection(
            "Primary".into(),
            "ocg-super-secret-key".into(),
            Some("primary@example.com".into()),
            "go".into(),
        ).unwrap();
        assert_eq!(summary.key_hint, "ocg-****-key");
        assert_eq!(summary.email_hint.as_deref(), Some("p***@example.com"));
        let raw = fs::read_to_string(dir.join(CONNECTIONS_FILE)).unwrap();
        assert!(raw.contains("AES-256-GCM"));
        assert!(!raw.contains("ocg-super-secret-key"));
        assert!(!raw.contains("primary@example.com"));
        assert_eq!(api_key(&summary.id).unwrap(), "ocg-super-secret-key");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn connection_count_is_unbounded_and_association_stays_encrypted() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        for index in 0..5 {
            let provider = if index % 2 == 0 { "go" } else { "zen" };
            create_connection(
                format!("Connection {}", index + 1),
                format!("key-{index}"),
                Some(format!("owner{index}@example.com")),
                provider.into(),
            ).unwrap();
        }
        let connections = list_connections().unwrap();
        assert_eq!(connections.len(), 5);
        assert_eq!(connections[4].email_hint.as_deref(), Some("o***@example.com"));
        let raw = fs::read_to_string(dir.join(CONNECTIONS_FILE)).unwrap();
        assert!(!raw.contains("owner4@example.com"));
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn malformed_email_is_rejected_before_persistence() {
        assert_eq!(
            create_connection(
                "Primary".into(),
                "valid-key".into(),
                Some("not-an-email".into()),
                "go".into(),
            )
            .unwrap_err(),
            "OPENCODE_GO_EMAIL_INVALID"
        );
    }

    #[test]
    fn updating_email_preserves_key_and_returns_only_a_masked_identity() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let created = create_connection("Primary".into(), "secure-key".into(), None, "go".into())
            .expect("create connection");
        let updated = update_connection(
            created.id.clone(),
            None,
            None,
            Some("owner@example.com".into()),
        )
        .expect("update association");
        assert_eq!(updated.email_hint.as_deref(), Some("o***@example.com"));
        assert_eq!(api_key(&created.id).expect("key remains available"), "secure-key");
        let raw = fs::read_to_string(dir.join(CONNECTIONS_FILE)).expect("read encrypted store");
        assert!(!raw.contains("owner@example.com"));
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn listing_has_stable_creation_order_even_when_storage_is_scrambled() {
        let first = StoredConnection {
            id: "ocg_z-last-id".into(),
            name: "First created".into(),
            api_key: "key-first".into(),
            email: None,
            created_at: 10,
            updated_at: 30,
            enabled: true,
            provider: "go".into(),
            quota: None,
            quota_error: None,
        };
        let second = StoredConnection {
            id: "ocg_a-first-id".into(),
            name: "Second created".into(),
            api_key: "key-second".into(),
            email: None,
            created_at: 20,
            updated_at: 20,
            enabled: true,
            provider: "go".into(),
            quota: None,
            quota_error: None,
        };
        let tied = StoredConnection {
            id: "ocg_b-tied-id".into(),
            name: "Tied creation".into(),
            api_key: "key-tied".into(),
            email: None,
            created_at: 20,
            updated_at: 10,
            enabled: true,
            provider: "go".into(),
            quota: None,
            quota_error: None,
        };

        let summaries = ordered_summaries(&[tied, second, first]);
        assert_eq!(
            summaries
                .iter()
                .map(|connection| connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ocg_z-last-id", "ocg_a-first-id", "ocg_b-tied-id"]
        );
    }

    #[test]
    fn manual_add_only_never_imports_legacy_codex_provider_keys() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        fs::create_dir_all(&dir).expect("create temp data dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        fs::write(
            dir.join("codex_model_providers.json"),
            r#"[{"id":"opencode_go","apiKeys":[{"apiKey":"must-not-import"}]}]"#,
        )
        .expect("write legacy provider store");

        assert!(list_connections().expect("list manual store").is_empty());
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn encrypted_transfer_round_trips_without_serializing_credentials() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let created = create_connection(
            "Transfer".into(),
            "transfer-secret-key".into(),
            Some("transfer@example.com".into()),
            "go".into(),
        )
        .expect("create encrypted connection");
        let transfer = export_encrypted_transfer(vec![created.id.clone()])
            .expect("export encrypted envelope");
        assert!(transfer.contains("AES-256-GCM"));
        assert!(!transfer.contains("transfer-secret-key"));
        assert!(!transfer.contains("transfer@example.com"));
        delete_connection(created.id).expect("delete preimage");
        let restored = import_encrypted_transfer(transfer).expect("restore local envelope");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].email_hint.as_deref(), Some("t***@example.com"));
        assert_eq!(api_key(&restored[0].id).expect("key restored locally"), "transfer-secret-key");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn encrypted_transfer_rejects_plaintext_credentials() {
        assert_eq!(
            import_encrypted_transfer(r#"{"connections":[{"apiKey":"must-not-import"}]}"#.into())
                .unwrap_err(),
            "OPENCODE_GO_TRANSFER_UNAVAILABLE"
        );
    }

    #[test]
    fn enabled_state_is_persisted_and_publicly_visible() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_dir();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let added = create_connection("Manual".into(), "manual-test-key".into(), None, "go".into()).unwrap();
        assert!(added.enabled);
        let disabled = set_connection_enabled(added.id, false).unwrap();
        assert!(!disabled.enabled);
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn quota_snapshot_is_strict_and_sanitized() {
        let raw = serde_json::json!({
            "apiKey": "must-not-escape",
            "usage": {
                "rolling": {"status": "ok", "percent": 25.5, "resetsAt": 1_900_000_000},
                "weekly": {"status": "ok", "percent": "10", "resets_at": 1_900_100_000_000_i64},
                "monthly": {"status": "ok", "percent": 0, "resetsAt": "2030-03-17T17:46:40Z"}
            }
        });
        let snapshot = sanitize_quota_response(&raw).unwrap();
        assert_eq!(snapshot.rolling.remaining_percent, Some(74.5));
        assert_eq!(snapshot.weekly.resets_at, Some(1_900_100_000));
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("apiKey"));
        assert!(!serialized.contains("must-not-escape"));
        assert_eq!(snapshot.status, "available");
    }

    #[test]
    fn quota_snapshot_preserves_valid_windows_when_monthly_is_unavailable() {
        let raw = serde_json::json!({
            "usage": {
                "rolling": {"status": "ok", "percent": 25, "resetsAt": 1_900_000_000},
                "weekly": {"status": "ok", "percent": 60, "resetsAt": 1_900_100_000},
                "monthly": {
                    "status": "error",
                    "error": "upstream secret detail",
                    "token": "response-secret"
                }
            }
        });

        let snapshot = sanitize_quota_response(&raw).expect("partial response");
        assert_eq!(snapshot.status, "partial");
        assert_eq!(snapshot.rolling.remaining_percent, Some(75.0));
        assert_eq!(snapshot.weekly.used_percent, Some(60.0));
        assert_eq!(snapshot.monthly.status, "error");
        assert_eq!(
            snapshot.monthly.error.as_deref(),
            Some("window unavailable")
        );
        let serialized = serde_json::to_string(&snapshot).expect("serialize snapshot");
        assert!(!serialized.contains("response-secret"));
        assert!(!serialized.contains("upstream secret detail"));
    }

    #[test]
    fn quota_snapshot_models_missing_fields_independently() {
        let snapshot = sanitize_quota_response(&serde_json::json!({
            "usage": {
                "rolling": {"status": "ok", "percent": 10},
                "weekly": {"status": "ok", "resetsAt": 1_900_100_000}
            }
        }))
        .expect("independent windows");

        assert_eq!(snapshot.status, "partial");
        assert_eq!(snapshot.rolling.used_percent, Some(10.0));
        assert_eq!(snapshot.rolling.error.as_deref(), Some("reset missing"));
        assert_eq!(snapshot.weekly.error.as_deref(), Some("percent invalid"));
        assert_eq!(snapshot.monthly.status, "unavailable");
        assert_eq!(snapshot.monthly.error.as_deref(), Some("window missing"));
    }

    #[test]
    fn quota_transport_classifications_cover_auth_rate_limit_and_network() {
        assert_eq!(classify_query_error(Some(401)), "authentication");
        assert_eq!(classify_query_error(Some(429)), "rate_limit");
        assert_eq!(classify_query_error(None), "network");
    }
}
