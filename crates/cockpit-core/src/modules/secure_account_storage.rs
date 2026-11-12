use aes_gcm::{Aes256Gcm, KeyInit};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use super::{
    account,
    account_file_crypto::{
        decrypt_envelope, lock_account_file_for_write, read_key, read_or_create_key_at,
        restore_legacy_antigravity_token, serialize_with_cipher, SecureAccountEnvelope, KEY_FILE,
    },
    atomic_write,
};

fn cipher(key: &[u8; 32]) -> Result<Aes256Gcm, String> {
    Aes256Gcm::new_from_slice(key).map_err(|error| format!("初始化账号详情加密失败: {error}"))
}

fn decode<T: DeserializeOwned>(root: &Path, content: &str) -> Result<T, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut value: Value =
        serde_json::from_str(content).map_err(|error| format!("解析账号详情失败: {error}"))?;
    if value.get("ciphertext").is_some() {
        let envelope: SecureAccountEnvelope = serde_json::from_value(value)
            .map_err(|error| format!("解析账号详情密文失败: {error}"))?;
        return decrypt_envelope(&envelope, cipher(&read_key(&root.join(KEY_FILE))?)?);
    }
    if value.get("token_encrypted").is_some() {
        restore_legacy_antigravity_token(root, &mut value)?;
    }
    serde_json::from_value(value).map_err(|error| format!("解析账号详情失败: {error}"))
}

pub(crate) fn decode_account_snapshot<T: DeserializeOwned>(content: &str) -> Result<T, String> {
    decode(&account::get_data_dir()?, content)
}

fn backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    backup.into()
}

fn read_at<T: DeserializeOwned>(root: &Path, path: &Path) -> Result<T, String> {
    let content = fs::read_to_string(path).map_err(|error| format!("读取账号详情失败: {error}"))?;
    decode(root, &content).or_else(|error| {
        // Keep recovery reads non-mutating. A missing primary is an absence, not an old backup.
        let recovered = || {
            let backup = fs::read_to_string(backup_path(path)).ok()?;
            let value: Value = decode(root, &backup).ok()?;
            let id = path.file_stem()?.to_str()?;
            if value["id"].as_str() != Some(id) {
                return None;
            }
            serde_json::from_value(value).ok()
        };
        recovered().ok_or(error)
    })
}

pub(crate) fn read_account_file<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    read_at(&account::get_data_dir()?, path)
}

fn write_at<T: Serialize>(
    root: &Path,
    path: &Path,
    kind: &str,
    account: &T,
    desktop_fields: &[(&str, &[&str])],
) -> Result<(), String> {
    let _lock = lock_account_file_for_write(path)?;
    let mut next = serde_json::to_value(account).map_err(|error| error.to_string())?;
    let current: Option<Value> = match fs::read_to_string(path) {
        Ok(content) => Some(decode(root, &content)?),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("读取待更新账号详情失败: {error}")),
    };
    if let Some(previous) = &current {
        if previous["id"] != next["id"] {
            return Err("账号 ID 与原详情不一致".into());
        }
        if let Some(timestamp) = previous["last_switched_at"]
            .as_i64()
            .max(next["last_switched_at"].as_i64())
        {
            next["last_switched_at"] = timestamp.into();
        }
        for (parent, fields) in desktop_fields {
            let Some(previous) = previous.pointer(parent).and_then(Value::as_object) else {
                continue;
            };
            let Some(next) = next.pointer_mut(parent).and_then(Value::as_object_mut) else {
                continue;
            };
            for field in *fields {
                if let Some(value) = previous.get(*field) {
                    next.entry(*field).or_insert_with(|| value.clone());
                }
            }
        }
    }
    let key = read_or_create_key_at(&root.join(KEY_FILE))?;
    let seal = |value: &Value| {
        let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        serialize_with_cipher(kind, &bytes, cipher(&key)?)
    };
    if let Some(previous) = current {
        atomic_write::write_secret_string_atomic(&backup_path(path), &seal(&previous)?)?;
    }
    atomic_write::write_secret_string_atomic(path, &seal(&next)?)
}

pub(crate) fn write_account_file<T: Serialize>(
    path: &Path,
    kind: &str,
    account: &T,
) -> Result<(), String> {
    write_account_file_preserving_fields(path, kind, account, &[])
}

pub(crate) fn write_account_file_preserving_fields<T: Serialize>(
    path: &Path,
    kind: &str,
    account: &T,
    desktop_fields: &[(&str, &[&str])],
) -> Result<(), String> {
    write_at(
        &account::get_data_dir()?,
        path,
        kind,
        account,
        desktop_fields,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE_KEY: &str = "KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=";
    // AES-GCM reference fixture generated independently of the Rust codec.
    const ENCRYPTED: &str = r#"{
        "version":1,"kind":"cursor","algorithm":"AES-256-GCM",
        "key_id":"local-secure-account-storage-v1","nonce":"ERERERERERERERER",
        "ciphertext":"Ee8SylGp0pAdzoUFu0SlRGlQrG8NUFkfpwNT6ZwdC2yCnDwmOJyqiiMl4+R511FV3FGOEFAAFD/TZf78K1eGnaUalJI25GFsa8ErtdoIof9OsJc8",
        "encrypted_at":1
    }"#;

    #[test]
    fn reads_desktop_format_and_writes_encrypted_detail_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-fixture.json");
        fs::write(dir.path().join(KEY_FILE), FIXTURE_KEY).unwrap();
        fs::write(&path, ENCRYPTED).unwrap();
        let original: Value = decode(dir.path(), ENCRYPTED).unwrap();
        assert_eq!(original["secret"], "synthetic");
        let edited = json!({"id":"desktop-fixture", "last_switched_at":100, "secret":"updated"});
        write_at(dir.path(), &path, "cursor", &edited, &[]).unwrap();
        let current = fs::read_to_string(&path).unwrap();
        let backup = fs::read_to_string(backup_path(&path)).unwrap();
        assert!(!current.contains("updated"));
        assert!(!backup.contains("synthetic"));
        let current: Value = decode(dir.path(), &current).unwrap();
        assert_eq!(current["secret"], "updated");
        assert_eq!(current["last_switched_at"], 200);
        assert_eq!(decode::<Value>(dir.path(), &backup).unwrap(), original);
    }

    #[test]
    fn a_missing_key_or_concurrent_write_does_not_replace_the_existing_detail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-fixture.json");
        fs::write(&path, ENCRYPTED).unwrap();
        let edited = json!({"id":"desktop-fixture", "secret":"updated"});
        assert!(write_at(dir.path(), &path, "cursor", &edited, &[]).is_err());
        assert!(!dir.path().join(KEY_FILE).exists());
        assert!(!backup_path(&path).exists());
        fs::write(dir.path().join(KEY_FILE), FIXTURE_KEY).unwrap();
        let _lock = super::super::account_file_crypto::lock_account_file(&path).unwrap();
        assert!(write_at(dir.path(), &path, "cursor", &edited, &[]).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), ENCRYPTED);
    }

    #[test]
    fn saving_a_legacy_plaintext_detail_keeps_an_encrypted_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.json");
        let original = json!({"id":"legacy", "secret":"legacy-fixture"});
        fs::write(&path, original.to_string()).unwrap();
        write_at(
            dir.path(),
            &path,
            "github_copilot",
            &json!({"id":"legacy", "secret":"new-fixture"}),
            &[],
        )
        .unwrap();
        let backup = fs::read_to_string(backup_path(&path)).unwrap();
        assert!(!backup.contains("legacy-fixture"));
        assert_eq!(decode::<Value>(dir.path(), &backup).unwrap(), original);
    }

    #[test]
    fn recovery_requires_the_backup_to_belong_to_the_requested_account() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("fixture.json");
        let backup = backup_path(&path);
        fs::write(&path, "corrupt").unwrap();
        fs::write(&backup, r#"{"id":"other"}"#).unwrap();
        assert!(read_at::<Value>(root, &path).is_err());
        fs::write(&backup, r#"{"id":"fixture"}"#).unwrap();
        assert_eq!(read_at::<Value>(root, &path).unwrap()["id"], "fixture");
        assert_eq!(fs::read_to_string(&path).unwrap(), "corrupt");
        fs::remove_file(&path).unwrap();
        assert!(read_at::<Value>(root, &path).is_err());
    }

    #[test]
    fn credential_roundtrip_through_provider_entrypoints() {
        const FIXTURE_ROOT: &str = "COCKPIT_PROVIDER_CRYPTO_FIXTURE_ROOT";
        if let Some(root) = std::env::var_os(FIXTURE_ROOT) {
            let root = PathBuf::from(root);
            let (recovered, warning) =
                crate::modules::cursor_account::list_accounts_for_cli().unwrap();
            assert_eq!(recovered[0].id, "cursor_fixture");
            assert!(warning.is_none());
            crate::modules::cursor_account::inject_to_cursor_at_path(
                &root.join("cursor-fixture.db"),
                "cursor_fixture",
            )
            .unwrap();
            crate::modules::github_copilot_account::update_account_tags(
                "copilot_fixture",
                vec!["edited".into()],
            )
            .unwrap();
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("cursor_accounts")).unwrap();
        fs::create_dir(root.join("github_copilot_accounts")).unwrap();
        let cursor = root.join("cursor_accounts/cursor_fixture.json");
        let copilot = root.join("github_copilot_accounts/copilot_fixture.json");
        write_at(root, &cursor, "cursor", &json!({
            "id":"cursor_fixture", "email":"cursor@example.test", "access_token":"synthetic-cursor",
            "created_at":1, "last_used":2,
        }), &[]).unwrap();
        write_at(root, &copilot, "github_copilot", &json!({
            "id":"copilot_fixture", "github_id":1, "github_login":"synthetic-user",
            "github_access_token":"synthetic-github", "copilot_token":"synthetic-copilot",
            "copilot_token_based_billing":true, "last_switched_at":200, "created_at":1, "last_used":2,
        }), &[]).unwrap();
        let connection = rusqlite::Connection::open(root.join("cursor-fixture.db")).unwrap();
        connection
            .execute(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "modules::secure_account_storage::tests::credential_roundtrip_through_provider_entrypoints"])
            .env(FIXTURE_ROOT, root)
            .env("COCKPIT_TOOLS_DATA_DIR", root)
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let switched: String = connection
            .query_row(
                "SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(switched, "synthetic-cursor");
        let content = fs::read_to_string(copilot).unwrap();
        assert!(!content.contains("synthetic-copilot"));
        let edited: Value = decode(root, &content).unwrap();
        assert_eq!(edited["tags"], json!(["edited"]));
        assert_eq!(edited["copilot_token_based_billing"], true);
        assert_eq!(edited["last_switched_at"], 200);
    }

    #[test]
    fn shared_provider_editors_keep_details_encrypted_and_preserve_desktop_fields() {
        const FIXTURE: &str = "COCKPIT_MULTI_PROVIDER_CRYPTO_FIXTURE";
        const PLATFORMS: &[(&str, &str)] = &[
            ("accounts", "antigravity"),
            ("codex_accounts", "codex"),
            ("qoder_accounts", "qoder"),
            ("codebuddy_accounts", "codebuddy"),
            ("codebuddy_cn_accounts", "codebuddy_cn"),
            ("zed_accounts", "zed"),
            ("workbuddy_accounts", "workbuddy"),
            ("windsurf_accounts", "windsurf"),
            ("kiro_accounts", "kiro"),
            ("trae_accounts", "trae"),
        ];
        if std::env::var_os(FIXTURE).is_some() {
            use crate::modules::*;
            let tags = || vec!["edited".into()];
            account::update_account_tags("fixture", tags()).unwrap();
            codex_account::update_account_tags("fixture", tags()).unwrap();
            qoder_account::update_account_tags("fixture", tags()).unwrap();
            codebuddy_account::update_account_tags("fixture", tags()).unwrap();
            codebuddy_cn_account::update_account_tags("fixture", tags()).unwrap();
            zed_account::update_account_tags("fixture", tags()).unwrap();
            workbuddy_account::update_account_tags("fixture", tags()).unwrap();
            windsurf_account::update_account_tags("fixture", tags()).unwrap();
            kiro_account::update_account_tags("fixture", tags()).unwrap();
            trae_account::update_account_tags("fixture", tags()).unwrap();
            return;
        }

        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path()
                .join(super::super::account_file_crypto::LEGACY_TOKEN_KEY_FILE),
            FIXTURE_KEY,
        )
        .unwrap();
        for (directory, kind) in PLATFORMS {
            let directory = root.path().join(directory);
            fs::create_dir(&directory).unwrap();
            let mut fixture = json!({
                "id":"fixture", "email":"fixture@example.test", "user_id":"test-user",
                "github_id":123, "github_login":"fixture-user", "github_access_token":"synthetic-auth",
                "copilot_token":"synthetic-auth", "access_token":"synthetic-auth", "checkin_streak":0,
                "created_at":1, "last_used":2, "last_switched_at":100, "tags":["old"],
                "auth_user_info_raw":{"fixture":"synthetic-auth"},
            });
            match *kind {
                "antigravity" => {
                    use aes_gcm::{aead::Aead, Nonce};
                    use base64::{engine::general_purpose::STANDARD, Engine as _};
                    let token = json!({"access_token":"synthetic-auth", "refresh_token":"synthetic-refresh", "expires_in":60, "expiry_timestamp":60, "token_type":"Bearer"});
                    let nonce = [23; 12];
                    let encrypted = cipher(&[42; 32])
                        .unwrap()
                        .encrypt(Nonce::from_slice(&nonce), token.to_string().as_bytes())
                        .unwrap();
                    fixture["token_encrypted"] = json!({"version":1, "algorithm":"AES-256-GCM", "key_id":"local-account-token-v1", "nonce":STANDARD.encode(nonce), "ciphertext":STANDARD.encode(encrypted), "encrypted_at":1});
                    fixture["two_factor_secret"] = json!("desktop-fixture-secret");
                    fixture["quota"] = json!({"models":[], "last_updated":10, "quota_summary_stale":true, "quota_summary_updated_at":8, "is_gcp_tos":true, "project_id":"fixture-project"});
                }
                "codex" => {
                    fixture["tokens"] =
                        json!({"access_token":"synthetic-auth", "id_token":"synthetic-id"});
                    fixture["api_model_catalog"] = json!(["desktop-model"]);
                    fixture["quota"] = json!({"hourly_percentage":50, "weekly_percentage":75, "reset_credits_available":2});
                }
                "workbuddy" => fixture["web_session_enabled"] = json!(true),
                _ => {}
            }
            let path = directory.join("fixture.json");
            if matches!(*kind, "antigravity" | "kiro") {
                fs::write(path, fixture.to_string()).unwrap();
            } else {
                write_at(root.path(), &path, kind, &fixture, &[]).unwrap();
            }
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "modules::secure_account_storage::tests::shared_provider_editors_keep_details_encrypted_and_preserve_desktop_fields"])
            .env(FIXTURE, "1")
            .env("COCKPIT_TOOLS_DATA_DIR", root.path())
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        for (directory, kind) in PLATFORMS {
            let path = root.path().join(directory).join("fixture.json");
            let raw = fs::read_to_string(&path).unwrap();
            assert!(!raw.contains("synthetic-auth"));
            assert!(!fs::read_to_string(backup_path(&path))
                .unwrap()
                .contains("synthetic-auth"));
            let account: Value = decode(root.path(), &raw).unwrap();
            assert_eq!(account["tags"], json!(["edited"]));
            assert_eq!(account["last_switched_at"], 100);
            match *kind {
                "antigravity" => {
                    assert_eq!(account["token"]["refresh_token"], "synthetic-refresh");
                    assert!(account.get("token_encrypted").is_none());
                    assert_eq!(account["two_factor_secret"], "desktop-fixture-secret");
                    assert_eq!(account["quota"]["quota_summary_stale"], true);
                    assert_eq!(account["quota"]["quota_summary_updated_at"], 8);
                    assert_eq!(account["quota"]["is_gcp_tos"], true);
                    assert_eq!(account["quota"]["project_id"], "fixture-project");
                }
                "codex" => {
                    assert_eq!(account["api_model_catalog"], json!(["desktop-model"]));
                    assert_eq!(account["quota"]["reset_credits_available"], 2);
                }
                "workbuddy" => assert_eq!(account["web_session_enabled"], true),
                _ => {}
            }
        }
    }
}
