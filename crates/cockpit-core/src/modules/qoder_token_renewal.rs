use aes_gcm::{Aes256Gcm, KeyInit};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
};

use super::{
    account_file_crypto::{
        decrypt_envelope, lock_account_file, read_key, read_or_create_key_at,
        serialize_with_cipher, SecureAccountEnvelope, KEY_FILE,
    },
    atomic_write::write_secret_string_atomic,
};
use crate::models::qoder::QoderAccount;

const KIND: &str = "qoder-token-renewal";

pub(crate) struct TokenRenewal {
    path: PathBuf,
    root: PathBuf,
    _lock: File,
}

#[derive(Deserialize, Serialize)]
struct PendingToken {
    previous: [u8; 32],
    user_info: Value,
    verified_origin: Option<String>,
}

fn fingerprint(account: &QoderAccount) -> Result<[u8; 32], String> {
    let previous = serde_json::to_vec(&(
        &account.id,
        account.created_at,
        &account.auth_user_info_raw,
        &account.auth_user_plan_raw,
        &account.verified_openapi_origin,
    ))
    .map_err(|error| error.to_string())?;
    Ok(Sha256::digest(previous).into())
}

fn persistence_error(error: impl std::fmt::Display) -> String {
    format!("QODER_RENEWAL_PERSIST_FAILED: {error}")
}

impl TokenRenewal {
    pub(super) fn open(root: PathBuf, account_path: &Path) -> Result<Self, String> {
        let path = account_path.with_extension("renewal");
        let lock = lock_account_file(&path).map_err(|_| "QODER_ACCOUNT_BUSY")?;
        Ok(Self {
            path,
            root,
            _lock: lock,
        })
    }

    pub(super) fn check_current(
        expected: &QoderAccount,
        value: Option<&Value>,
    ) -> Result<(), String> {
        let current: QoderAccount =
            serde_json::from_value(value.ok_or("QODER_SESSION_CHANGED")?.clone())
                .map_err(|_| "QODER_SESSION_CHANGED")?;
        let expected = serde_json::to_value(expected).map_err(|error| error.to_string())?;
        let current = serde_json::to_value(current).map_err(|error| error.to_string())?;
        if expected != current {
            return Err("QODER_SESSION_CHANGED".into());
        }
        Ok(())
    }

    pub(super) fn discard(account_path: &Path) -> Result<(), String> {
        remove_pending(&account_path.with_extension("renewal")).map_err(persistence_error)
    }

    pub(super) fn record(
        &self,
        expected: &QoderAccount,
        user_info: Value,
        verified_origin: Option<String>,
    ) -> Result<(), String> {
        let write = || {
            let pending = PendingToken {
                previous: fingerprint(expected)?,
                user_info,
                verified_origin,
            };
            let key = read_or_create_key_at(&self.root.join(KEY_FILE))?;
            let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
            let data = serde_json::to_vec(&pending).map_err(|error| error.to_string())?;
            let envelope = serialize_with_cipher(KIND, &data, cipher)?;
            write_secret_string_atomic(&self.path, &envelope)
        };
        write().map_err(persistence_error)
    }

    pub(super) fn commit(
        &self,
        expected: &QoderAccount,
        user_info: Value,
        verified_origin: Option<String>,
        load: impl FnOnce() -> Result<QoderAccount, String>,
        save: impl FnOnce(&QoderAccount, &QoderAccount) -> Result<(), String>,
    ) -> Result<QoderAccount, String> {
        // Stage the rotated token before any account detail read or index access.
        self.record(expected, user_info, verified_origin)?;
        let current = load().map_err(persistence_error)?;
        let pending = self
            .read()
            .map_err(persistence_error)?
            .ok_or("QODER_SESSION_CHANGED")?;
        if fingerprint(&current)? != pending.previous {
            remove_pending(&self.path).map_err(persistence_error)?;
            return Err("QODER_SESSION_CHANGED".into());
        }
        self.apply(current, pending, save)
    }

    pub(super) fn recover(
        &self,
        current: QoderAccount,
        save: impl FnOnce(&QoderAccount, &QoderAccount) -> Result<(), String>,
    ) -> Result<QoderAccount, String> {
        let Some(pending) = self.read().map_err(persistence_error)? else {
            return Ok(current);
        };

        // A later authorization or an already committed write wins over the record.
        if fingerprint(&current)? != pending.previous {
            remove_pending(&self.path).map_err(persistence_error)?;
            return Ok(current);
        }
        self.apply(current, pending, save)
    }

    fn apply(
        &self,
        current: QoderAccount,
        pending: PendingToken,
        save: impl FnOnce(&QoderAccount, &QoderAccount) -> Result<(), String>,
    ) -> Result<QoderAccount, String> {
        let mut updated = current.clone();
        updated.auth_user_info_raw = Some(pending.user_info);
        if pending.verified_origin.is_some() {
            updated.verified_openapi_origin = pending.verified_origin;
        }
        save(&current, &updated).map_err(persistence_error)?;
        remove_pending(&self.path).map_err(persistence_error)?;
        Ok(updated)
    }

    fn read(&self) -> Result<Option<PendingToken>, String> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let envelope: SecureAccountEnvelope =
            serde_json::from_str(&content).map_err(|error| error.to_string())?;
        if envelope.kind != KIND {
            return Err("Unexpected renewal record type".into());
        }
        let key = read_key(&self.root.join(KEY_FILE))?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
        decrypt_envelope(&envelope, cipher).map(Some)
    }
}

fn remove_pending(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account() -> QoderAccount {
        serde_json::from_value(json!({
            "id": "fixture-account",
            "email": "fixture@example.invalid",
            "created_at": 1,
            "last_used": 1,
            "auth_user_info_raw": { "id": "fixture-user", "token": "old-synthetic-token" },
        }))
        .unwrap()
    }

    fn rotated() -> Value {
        json!({ "id": "fixture-user", "token": "new-synthetic-token" })
    }

    #[test]
    fn retries_detail_write_after_reopening_the_encrypted_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.json");
        let root = dir.path().to_path_buf();
        let old = account();
        let origin = "https://fixture.invalid";
        {
            let renewal = TokenRenewal::open(root.clone(), &path).unwrap();
            renewal
                .record(&old, rotated(), Some(origin.into()))
                .unwrap();
            let error = renewal
                .recover(old.clone(), |_, _| {
                    Err("simulated account write failure".into())
                })
                .unwrap_err();
            assert_eq!(
                error,
                "QODER_RENEWAL_PERSIST_FAILED: simulated account write failure"
            );
            let stored = fs::read_to_string(&renewal.path).unwrap();
            assert!(!stored.contains("new-synthetic-token"));
            assert!(!stored.contains("old-synthetic-token"));
        }

        let renewal = TokenRenewal::open(root, &path).unwrap();
        // No summary index exists. Credential recovery only writes account details.
        let saved = renewal
            .recover(old, |_, next| {
                assert_eq!(next.auth_user_info_raw, Some(rotated()));
                assert_eq!(next.verified_openapi_origin.as_deref(), Some(origin));
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.auth_user_info_raw, Some(rotated()));
        assert!(!renewal.path.exists());
    }

    #[test]
    fn stale_record_cannot_overwrite_newer_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.json");
        let root = dir.path().to_path_buf();
        for current_info in [
            rotated(),
            json!({ "token": "reauthorized-synthetic-token" }),
        ] {
            let renewal = TokenRenewal::open(root.clone(), &path).unwrap();
            let old = account();
            renewal.record(&old, rotated(), None).unwrap();
            let mut current = old;
            current.auth_user_info_raw = Some(current_info.clone());
            let current = renewal
                .recover(current, |_, _| {
                    panic!("stale journal rewrote current details")
                })
                .unwrap();
            assert_eq!(current.auth_user_info_raw, Some(current_info));
            assert!(!renewal.path.exists());
        }
    }

    #[test]
    fn detail_compare_prevents_overwriting_concurrent_updates_or_deletion() {
        let expected = account();
        let mut current = serde_json::to_value(&expected).unwrap();
        current["verified_openapi_origin"] = Value::Null;
        assert!(TokenRenewal::check_current(&expected, Some(&current)).is_ok());
        current["tags"] = json!(["changed"]);
        assert_eq!(
            TokenRenewal::check_current(&expected, Some(&current)),
            Err("QODER_SESSION_CHANGED".into())
        );
        assert_eq!(
            TokenRenewal::check_current(&expected, None),
            Err("QODER_SESSION_CHANGED".into())
        );
    }

    #[test]
    fn record_write_failure_and_concurrent_renewal_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.json");
        let root = dir.path().to_path_buf();
        let renewal = TokenRenewal::open(root.clone(), &path).unwrap();
        assert!(matches!(
            TokenRenewal::open(root, &path),
            Err(error) if error == "QODER_ACCOUNT_BUSY"
        ));
        fs::create_dir(&renewal.path).unwrap();
        let error = renewal.record(&account(), rotated(), None).unwrap_err();
        assert!(error.starts_with("QODER_RENEWAL_PERSIST_FAILED: "));
    }
}
