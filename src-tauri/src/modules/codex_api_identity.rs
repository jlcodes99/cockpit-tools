//! Account installation profiles shared by the native engine and the Go manifest.
//! All storage operations must run on a blocking worker. Protocol rewriting is
//! deliberately limited to installation fields, never business data or history.
use std::{fs::OpenOptions, path::Path, time::Duration};

use axum::http::{HeaderMap, HeaderValue};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::models::codex::CodexAccount;

const STORE_FILE: &str = "codex_api_account_identity.sqlite";
const INSTALLATION_HEADER: &str = "x-codex-installation-id";
const METADATA_HEADER: &str = "x-codex-turn-metadata";
const INSTALLATION_FIELDS: [&str; 2] = ["installation_id", INSTALLATION_HEADER];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationProfile {
    pub version: u8,
    pub scope: String,
    pub installation_id: String,
    pub revision: u64,
}

fn scope(account: &CodexAccount) -> Result<String, String> {
    if account.id.trim().is_empty() {
        return Err("Account identity requires a local account ID".into());
    }
    // Local records remain independent even when the same principal is imported
    // twice. A changed principal/workspace/realm gets a new installation, while
    // refreshing an OAuth token, changing a proxy or creating a task does not.
    let grok = crate::modules::codex_account::is_grok_upstream_provider(account);
    let api_key = account.is_api_key_auth() && !grok;
    let realm = if grok {
        "grok".to_owned()
    } else if api_key {
        api_realm(account)
    } else {
        "chatgpt".to_owned()
    };
    let key_digest = api_key.then(|| {
        format!(
            "{:x}",
            Sha256::digest(account.openai_api_key.as_deref().unwrap_or_default())
        )
    });
    let (principal, workspace) = principal_ids(account);
    let material = serde_json::to_vec(&(
        1u8,
        &account.id,
        realm,
        if grok {
            "grok"
        } else if api_key {
            "api_key"
        } else if account.agent_identity.is_some() {
            "agent_identity"
        } else {
            "oauth"
        },
        (!grok).then_some(principal),
        (!grok).then_some(workspace),
        &account.upstream_grok_account_id,
        key_digest,
    ))
    .map_err(|_| "Account identity cannot be encoded".to_string())?;
    Ok(format!("{:x}", Sha256::digest(material)))
}

pub(crate) fn api_realm(account: &CodexAccount) -> String {
    let raw = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://api.openai.com/v1");
    url::Url::parse(raw)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| raw.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

pub(crate) fn principal_ids(account: &CodexAccount) -> (Option<&str>, Option<&str>) {
    if let Some(identity) = &account.agent_identity {
        (Some(&identity.chatgpt_user_id), Some(&identity.account_id))
    } else {
        (account.user_id.as_deref(), account.account_id.as_deref())
    }
}

pub fn resolve(account: &CodexAccount) -> Result<InstallationProfile, String> {
    resolve_at(
        &crate::modules::account::get_data_dir()?.join(STORE_FILE),
        account,
    )
}

pub(crate) fn resolve_at(
    path: &Path,
    account: &CodexAccount,
) -> Result<InstallationProfile, String> {
    let scope = scope(account)?;
    let parent = path
        .parent()
        .ok_or("Account identity store has no parent")?;
    std::fs::create_dir_all(parent).map_err(|_| "Account identity store is unavailable")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("Account identity store is unavailable".into()),
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Account identity store is unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Account identity store must be a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Account identity store permissions could not be secured")?;
        }
    }
    let failure = |_| "Account identity store is unavailable or invalid".to_string();
    let mut connection = Connection::open(path).map_err(failure)?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(failure)?;
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(failure)?;
    if version > 1 {
        return Err("Unsupported account identity store version".into());
    }
    if version == 1 {
        if let Some((id, revision)) = read_profile(&connection, &scope)? {
            // Normal request reads do not acquire a global SQLite writer lock.
            // Only a genuinely missing installation needs the creation path.
            return profile(scope, id, revision);
        }
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failure)?;
    let version: u32 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(failure)?;
    if version > 1 {
        return Err("Unsupported account identity store version".into());
    }
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS account_installation_v1 (
           scope TEXT PRIMARY KEY, installation_id TEXT NOT NULL UNIQUE,
           revision INTEGER NOT NULL CHECK(revision > 0)
         ); PRAGMA user_version=1;",
        )
        .map_err(failure)?;
    let existing = read_profile(&transaction, &scope)?;
    let (installation_id, revision) = if let Some((id, revision)) = existing {
        profile(scope.clone(), id.clone(), revision)?;
        (id, revision)
    } else {
        let id = Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO account_installation_v1(scope,installation_id,revision) VALUES(?1,?2,1)",
            params![scope, id],
        ).map_err(failure)?;
        (id, 1)
    };
    transaction.commit().map_err(failure)?;
    profile(scope, installation_id, revision)
}

fn read_profile(connection: &Connection, scope: &str) -> Result<Option<(String, u64)>, String> {
    connection
        .query_row(
            "SELECT installation_id,revision FROM account_installation_v1 WHERE scope=?1",
            [scope],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| "Account identity store is unavailable or invalid".into())
}

fn profile(
    scope: String,
    installation_id: String,
    revision: u64,
) -> Result<InstallationProfile, String> {
    let parsed = Uuid::parse_str(&installation_id)
        .map_err(|_| "Invalid persisted account installation ID")?;
    if parsed.get_version_num() != 4
        || parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.to_string() != installation_id
        || revision == 0
    {
        return Err("Invalid persisted account installation profile".into());
    }
    Ok(InstallationProfile {
        version: 1,
        scope,
        installation_id,
        revision,
    })
}

pub fn apply_headers(profile: &InstallationProfile, headers: &mut HeaderMap) -> Result<(), String> {
    let value = HeaderValue::from_str(&profile.installation_id)
        .map_err(|_| "Invalid account installation ID".to_string())?;
    // insert replaces all duplicate values; a downstream installation cannot
    // override the account selected for this upstream attempt.
    headers.insert(INSTALLATION_HEADER, value);
    if let Some(raw) = headers.get(METADATA_HEADER) {
        let mut metadata: Value = serde_json::from_slice(raw.as_bytes())
            .map_err(|_| "Invalid turn metadata JSON".to_string())?;
        if replace_fields(profile, &mut metadata)? {
            let json = ascii_json(&metadata)?;
            headers.insert(
                METADATA_HEADER,
                HeaderValue::from_str(&json)
                    .map_err(|_| "Invalid account turn metadata".to_string())?,
            );
        }
    }
    Ok(())
}

pub fn apply_body(profile: &InstallationProfile, body: &mut Value) -> Result<(), String> {
    replace_fields(profile, body)?;
    if let Some(metadata) = body
        .get_mut("client_metadata")
        .filter(|value| value.is_object())
    {
        replace_fields(profile, metadata)?;
        if let Some(raw) = metadata.get(METADATA_HEADER) {
            let raw = raw.as_str().ok_or("Turn metadata must be a JSON string")?;
            let mut canonical: Value =
                serde_json::from_str(raw).map_err(|_| "Invalid turn metadata JSON")?;
            if replace_fields(profile, &mut canonical)? {
                metadata[METADATA_HEADER] = Value::String(
                    serde_json::to_string(&canonical)
                        .map_err(|_| "Turn metadata cannot be encoded")?,
                );
            }
        }
    }
    Ok(())
}

fn replace_fields(profile: &InstallationProfile, value: &mut Value) -> Result<bool, String> {
    let object = value
        .as_object_mut()
        .ok_or("Identity metadata must be an object")?;
    let mut changed = false;
    for field in INSTALLATION_FIELDS {
        if let Some(value) = object.get_mut(field) {
            *value = Value::String(profile.installation_id.clone());
            changed = true;
        }
    }
    Ok(changed)
}

fn ascii_json(value: &Value) -> Result<String, String> {
    let json = serde_json::to_string(value).map_err(|_| "Turn metadata cannot be encoded")?;
    let mut result = String::with_capacity(json.len());
    for ch in json.chars() {
        if ch.is_ascii() {
            result.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0u16; 2]) {
                use std::fmt::Write;
                write!(result, "\\u{unit:04x}").map_err(|_| "Turn metadata cannot be encoded")?;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "codex_api_identity_tests.rs"]
mod tests;
