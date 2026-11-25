use std::fs;
use std::path::PathBuf;

use super::{
    account_index_repair::collect_account_file_ids,
    qoder_channel::{credential_source, CredentialSource},
    qoder_identity::normalize_account_id,
    secure_account_storage::{read_at, write_at},
};
use crate::models::qoder::QoderAccount;

const STORE_KIND: &str = "qoder_cn";
const ACCOUNTS_DIR: &str = "qoder_cn_accounts";

/// Encrypted account details for the public Qoder CN client.
pub struct QoderCnAccountStore {
    root: PathBuf,
}

impl QoderCnAccountStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { root: data_dir }
    }

    fn directory(&self) -> PathBuf {
        self.root.join(ACCOUNTS_DIR)
    }

    fn path(&self, id: &str) -> Result<PathBuf, String> {
        Ok(self
            .directory()
            .join(format!("{}.json", normalize_account_id(id)?)))
    }

    pub fn load_account(&self, id: &str) -> Result<Option<QoderAccount>, String> {
        let path = self.path(id)?;
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Ok(None);
        }
        let account: QoderAccount = read_at(&self.root, &path)?;
        if account.id != id.trim() {
            return Err("QODER_SESSION_CHANGED".into());
        }
        require_public_cn(&account)?;
        Ok(Some(account))
    }

    pub fn save_account(&self, account: &QoderAccount) -> Result<(), String> {
        require_public_cn(account)?;
        if account.id.trim() != account.id {
            return Err("Qoder 账号 ID 包含首尾空白".into());
        }
        let path = self.path(&account.id)?;
        fs::create_dir_all(self.directory()).map_err(|error| error.to_string())?;
        write_at(&self.root, &path, STORE_KIND, account, &[])
    }

    pub fn list_account_ids(&self) -> Result<Vec<String>, String> {
        let directory = self.directory();
        if !directory.try_exists().map_err(|error| error.to_string())? {
            return Ok(Vec::new());
        }
        collect_account_file_ids(&directory)
    }
}

fn require_public_cn(account: &QoderAccount) -> Result<(), String> {
    if credential_source(account)? != CredentialSource::PublicCn {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::qoder_channel::account_fixture;

    #[test]
    fn public_cn_details_are_separate_and_encrypted() {
        let root = tempfile::tempdir().unwrap();
        let account = account_fixture(None, Some("qoder_cn"));
        let ide_directory = root.path().join("qoder_accounts");
        fs::create_dir(&ide_directory).unwrap();
        let ide_path = ide_directory.join(format!("{}.json", account.id));
        let ide = account_fixture(Some("qodercn"), None);
        write_at(root.path(), &ide_path, "qoder", &ide, &[]).unwrap();
        let ide_before = fs::read(&ide_path).unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());

        assert!(store.load_account(&account.id).unwrap().is_none());
        assert!(store.list_account_ids().unwrap().is_empty());
        store.save_account(&account).unwrap();
        let saved = store.load_account(&account.id).unwrap().unwrap();
        assert_eq!(saved.auth_user_info_raw, account.auth_user_info_raw);
        assert_eq!(saved.auth_user_plan_raw, account.auth_user_plan_raw);
        assert_eq!(store.list_account_ids().unwrap(), [account.id.clone()]);
        assert_eq!(fs::read(&ide_path).unwrap(), ide_before);
        let envelope: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path(&account.id).unwrap()).unwrap()).unwrap();
        assert_eq!(envelope["kind"], "qoder_cn");
        assert!(envelope["ciphertext"].is_string());
        assert!(!envelope.to_string().contains("synthetic-access-token"));
    }

    #[test]
    fn public_cn_store_rejects_other_sources_and_misfiled_details() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        for source in [
            None,
            Some("qoder"),
            Some("qodercn"),
            Some("dedicated_qodercn"),
        ] {
            let account = account_fixture(source, None);
            assert_eq!(
                store.save_account(&account),
                Err("QODER_PLATFORM_MISMATCH".into())
            );
        }
        assert!(!store.directory().exists());

        let legacy = account_fixture(Some("qodercn"), None);
        fs::create_dir(store.directory()).unwrap();
        write_at(
            root.path(),
            &store.path(&legacy.id).unwrap(),
            "qoder",
            &legacy,
            &[],
        )
        .unwrap();
        assert_eq!(
            store.load_account(&legacy.id).unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
    }
}
