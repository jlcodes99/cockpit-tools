use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

use serde_json::Value;

use super::{
    account_index_repair::{
        collect_account_file_ids, load_accounts_from_details, sort_accounts_by_recency,
    },
    logger, provider_account_index,
    qoder_channel::{credential_source, CredentialSource},
    qoder_identity::normalize_account_id,
    secure_account_storage::{read_at, write_at},
};
use crate::models::qoder::{QoderAccount, QoderCnAccountIndex};

const STORE_KIND: &str = "qoder_cn";
const ACCOUNTS_DIR: &str = "qoder_cn_accounts";
/// Legacy JSON index path. The store's SQLite database is derived from this name,
/// so it must stay in sync with `QoderCnAccountIndex::LEGACY_FILE`.
const ACCOUNTS_INDEX_FILE: &str = "qoder_cn_accounts.json";
const INDEX_BUSY: &str = "QODER_CN_ACCOUNT_BUSY";
/// Marker every persisted CN session carries; see `qoder_channel::Channel`.
const LOGIN_SOURCE_MARKER: &str = "qoder_cn";

/// Serializes index mutations. Detail files are written outside this lock, but
/// every write that also touches the index goes through it.
static CN_ACCOUNT_INDEX_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn index_guard() -> Result<MutexGuard<'static, ()>, String> {
    CN_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| INDEX_BUSY.to_string())
}

/// Encrypted account details for the public Qoder CN client.
pub struct QoderCnAccountStore {
    root: PathBuf,
}

impl QoderCnAccountStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { root: data_dir }
    }

    /// Store rooted at the desktop data directory.
    pub fn default_store() -> Result<Self, String> {
        Ok(Self::new(super::account::get_data_dir()?))
    }

    fn directory(&self) -> PathBuf {
        self.root.join(ACCOUNTS_DIR)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(ACCOUNTS_INDEX_FILE)
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
        let _guard = index_guard()?;
        self.write_details(account)
    }

    pub fn list_account_ids(&self) -> Result<Vec<String>, String> {
        let directory = self.directory();
        if !directory.try_exists().map_err(|error| error.to_string())? {
            return Ok(Vec::new());
        }
        collect_account_file_ids(&directory)
    }

    /// Accounts ordered by last use, resolved through the index.
    pub fn list_accounts(&self) -> Result<Vec<QoderAccount>, String> {
        let index = self.load_index_checked()?;
        let mut accounts = Vec::with_capacity(index.accounts.len());
        for summary in &index.accounts {
            if let Some(account) = self.load_account(&summary.id)? {
                accounts.push(account);
            }
        }
        accounts.sort_by(|left, right| right.last_used.cmp(&left.last_used));
        Ok(accounts)
    }

    pub fn remove_account(&self, id: &str) -> Result<(), String> {
        let _guard = index_guard()?;
        let mut index = self.load_index_checked()?;
        index.accounts.retain(|item| item.id != id.trim());
        self.save_index(&index)?;
        self.delete_details(id)
    }

    pub fn remove_accounts(&self, ids: &[String]) -> Result<(), String> {
        let targets: HashSet<String> = ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        if targets.is_empty() {
            return Ok(());
        }
        let _guard = index_guard()?;
        let mut index = self.load_index_checked()?;
        index
            .accounts
            .retain(|item| !targets.contains(item.id.as_str()));
        self.save_index(&index)?;
        for id in targets {
            self.delete_details(&id)?;
        }
        Ok(())
    }

    /// Records a failed quota query on the account, but only when the credentials
    /// are still the ones the caller queried with.
    pub fn mark_quota_query_error(
        &self,
        expected: &QoderAccount,
        error: String,
    ) -> Result<(), String> {
        let _guard = index_guard()?;
        let Some(mut account) = self.load_account(&expected.id)? else {
            return Ok(());
        };
        if account.auth_user_info_raw != expected.auth_user_info_raw
            || account.auth_user_plan_raw != expected.auth_user_plan_raw
        {
            return Ok(());
        }
        account.quota_query_last_error = Some(error);
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        self.write_details(&account)
    }

    /// Merges a freshly logged-in snapshot into the CN store. The account is
    /// matched by id, then by user id, so a re-login updates in place instead of
    /// creating a second entry for the same person.
    pub fn upsert_from_snapshot(
        &self,
        user_info_raw: Value,
        user_plan_raw: Option<Value>,
        credit_usage_raw: Option<Value>,
    ) -> Result<QoderAccount, String> {
        let _guard = index_guard()?;
        let (user_info_raw, user_plan_raw) = stamp_public_cn(user_info_raw, user_plan_raw)?;
        let draft = super::qoder_account::build_account_from_snapshot(
            user_info_raw.clone(),
            user_plan_raw.clone(),
            None,
            None,
        );
        let existing = self.find_existing(&draft)?;
        let account = super::qoder_account::build_account_from_snapshot(
            user_info_raw,
            user_plan_raw,
            credit_usage_raw,
            existing.as_ref(),
        );
        self.write_details(&account)?;
        Ok(account)
    }

    /// Persists a refreshed snapshot, but only while the stored credentials are
    /// still the ones the caller refreshed with.
    pub fn save_refresh(
        &self,
        expected: &QoderAccount,
        user_info: Value,
        plan: Option<Value>,
        usage: Option<Value>,
        quota_error: Option<String>,
    ) -> Result<QoderAccount, String> {
        let _guard = index_guard()?;
        let current = self
            .load_account(&expected.id)?
            .ok_or_else(|| "QODER_SESSION_CHANGED".to_string())?;
        if current.auth_user_info_raw != expected.auth_user_info_raw
            || current.auth_user_plan_raw != expected.auth_user_plan_raw
        {
            return Err("QODER_SESSION_CHANGED".into());
        }
        let mut updated = super::qoder_account::build_account_from_snapshot(
            user_info,
            plan,
            usage,
            Some(&current),
        );
        updated.quota_query_last_error = quota_error;
        updated.quota_query_last_error_at = updated
            .quota_query_last_error
            .as_ref()
            .map(|_| chrono::Utc::now().timestamp_millis());
        self.write_details(&updated)?;
        Ok(updated)
    }

    /// Path of the live index file, for diagnostics in the UI.
    pub fn accounts_index_path_string(&self) -> Result<String, String> {
        Ok(
            provider_account_index::storage_path::<QoderCnAccountIndex>(&self.index_path())?
                .to_string_lossy()
                .into_owned(),
        )
    }

    pub fn update_account_tags(&self, id: &str, tags: Vec<String>) -> Result<QoderAccount, String> {
        let _guard = index_guard()?;
        let mut account = self
            .load_account(id)?
            .ok_or_else(|| "QODER_ACCOUNT_NOT_FOUND".to_string())?;
        account.tags = (!tags.is_empty()).then_some(tags);
        self.write_details(&account)?;
        Ok(account)
    }

    /// Records that the credentials were just written to the client, so the list
    /// can show which account the client is currently signed in as.
    pub fn mark_switched(&self, id: &str) -> Result<(), String> {
        let _guard = index_guard()?;
        let Some(mut account) = self.load_account(id)? else {
            return Err("QODER_ACCOUNT_NOT_FOUND".into());
        };
        account.last_switched_at = Some(chrono::Utc::now().timestamp_millis());
        self.write_details(&account)
    }

    /// Imports accounts from an export file. Every item is validated before the
    /// first write, so a mixed-region payload leaves the store untouched.
    pub fn import_json(&self, json_content: &str) -> Result<Vec<QoderAccount>, String> {
        let parsed: Value = serde_json::from_str(json_content)
            .map_err(|error| format!("JSON 解析失败: {error}"))?;
        let items: Vec<Value> = match parsed {
            Value::Array(items) => items,
            Value::Object(map) => match map.get("accounts") {
                Some(Value::Array(items)) => items.clone(),
                _ => vec![Value::Object(map)],
            },
            _ => return Err("仅支持对象或数组格式的 Qoder JSON".to_string()),
        };
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut snapshots = Vec::with_capacity(items.len());
        for item in &items {
            let info = item
                .get("auth_user_info_raw")
                .or_else(|| item.get("userInfo"))
                .cloned()
                .ok_or_else(|| "QODER_IMPORT_IDENTITY_MISSING".to_string())?;
            let plan = item
                .get("auth_user_plan_raw")
                .or_else(|| item.get("userPlan"))
                .cloned();
            let usage = item
                .get("auth_credit_usage_raw")
                .or_else(|| item.get("creditUsage"))
                .cloned();
            // Validating the whole payload first keeps a bad item from leaving
            // half of the file imported.
            stamp_public_cn(info.clone(), plan.clone())?;
            snapshots.push((info, plan, usage));
        }
        let mut imported = Vec::with_capacity(snapshots.len());
        for (info, plan, usage) in snapshots {
            imported.push(self.upsert_from_snapshot(info, plan, usage)?);
        }
        Ok(imported)
    }

    /// Serializes the selected accounts, credentials included, for a backup file.
    pub fn export_accounts(&self, account_ids: &[String]) -> Result<String, String> {
        let mut accounts = self.list_accounts()?;
        if !account_ids.is_empty() {
            let targets: HashSet<String> = account_ids
                .iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect();
            accounts.retain(|account| targets.contains(&account.id));
        }
        serde_json::to_string_pretty(&accounts)
            .map_err(|error| format!("序列化导出 JSON 失败: {error}"))
    }

    fn find_existing(&self, draft: &QoderAccount) -> Result<Option<QoderAccount>, String> {
        if let Some(account) = self.load_account(&draft.id)? {
            return Ok(Some(account));
        }
        let Some(user_id) = draft.user_id.as_deref() else {
            return Ok(None);
        };
        let index = self.load_index_checked()?;
        let Some(summary) = index
            .accounts
            .iter()
            .find(|summary| summary.user_id.as_deref() == Some(user_id))
        else {
            return Ok(None);
        };
        self.load_account(&summary.id)
    }

    fn delete_details(&self, id: &str) -> Result<(), String> {
        let path = self.path(id)?;
        if path.try_exists().map_err(|error| error.to_string())? {
            fs::remove_file(&path).map_err(|error| format!("删除账号文件失败: {error}"))?;
        }
        Ok(())
    }

    /// Writes one detail file and refreshes its index summary. Callers hold the lock.
    fn write_details(&self, account: &QoderAccount) -> Result<(), String> {
        require_public_cn(account)?;
        if account.id.trim() != account.id {
            return Err("Qoder 账号 ID 包含首尾空白".into());
        }
        let path = self.path(&account.id)?;
        fs::create_dir_all(self.directory()).map_err(|error| error.to_string())?;
        write_at(&self.root, &path, STORE_KIND, account, &[])?;
        let mut index = self.load_index_checked()?;
        refresh_summary(&mut index, account);
        self.save_index(&index)
    }

    fn load_index_checked(&self) -> Result<QoderCnAccountIndex, String> {
        provider_account_index::checked(
            &self.index_path(),
            super::account::schedule_index_store,
            |reason| self.repair_index_from_details(reason),
            |index: &QoderCnAccountIndex| index.accounts.is_empty(),
        )
    }

    fn save_index(&self, index: &QoderCnAccountIndex) -> Result<(), String> {
        provider_account_index::save(&self.index_path(), index)
    }

    fn repair_index_from_details(&self, reason: &str) -> Option<QoderCnAccountIndex> {
        let directory = self.directory();
        if !directory.try_exists().unwrap_or(false) {
            return None;
        }
        let mut accounts =
            load_accounts_from_details(&directory, |id| self.load_account(id).ok().flatten())
                .ok()?;
        if accounts.is_empty() {
            return None;
        }
        sort_accounts_by_recency(
            &mut accounts,
            |account| account.last_used,
            |account| account.created_at,
            |account| account.id.as_str(),
        );
        let mut index = QoderCnAccountIndex::new();
        index.accounts = accounts.iter().map(|account| account.summary()).collect();
        if let Err(error) = self.save_index(&index) {
            logger::log_warn(&format!(
                "[Qoder CN Account] 保存恢复索引失败: reason={reason}, error={error}"
            ));
            return None;
        }
        logger::log_info(&format!(
            "[Qoder CN Account] 已恢复账号索引: reason={reason}, accounts={}",
            index.accounts.len()
        ));
        Some(index)
    }
}

fn refresh_summary(index: &mut QoderCnAccountIndex, account: &QoderAccount) {
    let summary = account.summary();
    if let Some(slot) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *slot = summary;
        return;
    }
    index.accounts.push(summary);
}

/// Marks a fresh login snapshot as a Cockpit public CN session. A snapshot that
/// already carries another channel's marker is rejected instead of relabelled.
fn stamp_public_cn(
    mut user_info: Value,
    mut plan: Option<Value>,
) -> Result<(Value, Option<Value>), String> {
    if !user_info.is_object() {
        return Err("QODER_LOGIN_PAYLOAD_INVALID".into());
    }
    stamp_marker(&mut user_info)?;
    if let Some(payload) = plan.as_mut().filter(|value| value.is_object()) {
        stamp_marker(payload)?;
    }
    Ok((user_info, plan))
}

fn stamp_marker(payload: &mut Value) -> Result<(), String> {
    match payload.get("login_source") {
        None | Some(Value::Null) => {
            payload["login_source"] = Value::String(LOGIN_SOURCE_MARKER.to_string());
            Ok(())
        }
        Some(value) if value.as_str() == Some(LOGIN_SOURCE_MARKER) => Ok(()),
        Some(_) => Err("QODER_PLATFORM_MISMATCH".into()),
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

    fn cn_account(id: &str, last_used: i64) -> QoderAccount {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "email": format!("{id}@example.invalid"),
            "created_at": 1,
            "last_used": last_used,
            "auth_user_info_raw": {
                "id": format!("user-{id}"),
                "token": "synthetic-access-token",
                "login_source": "qoder_cn",
            },
        }))
        .expect("Qoder CN fixture")
    }

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

    #[test]
    fn index_lists_accounts_by_last_use_and_tracks_removals() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        store.save_account(&cn_account("older", 10)).unwrap();
        store.save_account(&cn_account("newer", 20)).unwrap();

        let listed: Vec<String> = store
            .list_accounts()
            .unwrap()
            .into_iter()
            .map(|account| account.id)
            .collect();
        assert_eq!(listed, ["newer", "older"]);

        store.remove_account("newer").unwrap();
        let listed: Vec<String> = store
            .list_accounts()
            .unwrap()
            .into_iter()
            .map(|account| account.id)
            .collect();
        assert_eq!(listed, ["older"]);
        assert!(store.load_account("newer").unwrap().is_none());
    }

    #[test]
    fn index_is_rebuilt_from_details_when_missing() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        fs::create_dir(store.directory()).unwrap();
        for (id, last_used) in [("recovered-a", 5), ("recovered-b", 7)] {
            let account = cn_account(id, last_used);
            write_at(
                root.path(),
                &store.path(id).unwrap(),
                STORE_KIND,
                &account,
                &[],
            )
            .unwrap();
        }

        let listed: Vec<String> = store
            .list_accounts()
            .unwrap()
            .into_iter()
            .map(|account| account.id)
            .collect();
        assert_eq!(listed, ["recovered-b", "recovered-a"]);
    }

    #[test]
    fn quota_errors_are_recorded_only_for_matching_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let account = cn_account("quota", 1);
        store.save_account(&account).unwrap();

        let mut stale = account.clone();
        stale.auth_user_info_raw = Some(serde_json::json!({ "token": "rotated" }));
        store
            .mark_quota_query_error(&stale, "stale".into())
            .unwrap();
        assert!(store
            .load_account("quota")
            .unwrap()
            .unwrap()
            .quota_query_last_error
            .is_none());

        store
            .mark_quota_query_error(&account, "quota-unavailable".into())
            .unwrap();
        let stored = store.load_account("quota").unwrap().unwrap();
        assert_eq!(
            stored.quota_query_last_error.as_deref(),
            Some("quota-unavailable")
        );
        assert!(stored.quota_query_last_error_at.is_some());
    }

    #[test]
    fn refresh_updates_details_and_rejects_stale_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let account = cn_account("refresh", 1);
        store.save_account(&account).unwrap();

        let mut refreshed_info = account.auth_user_info_raw.clone().unwrap();
        refreshed_info["token"] = serde_json::json!("rotated-token");
        let refreshed = store
            .save_refresh(
                &account,
                refreshed_info.clone(),
                None,
                None,
                Some("quota-down".into()),
            )
            .unwrap();
        assert_eq!(
            refreshed.auth_user_info_raw.as_ref().unwrap()["token"],
            "rotated-token"
        );
        assert_eq!(
            refreshed.quota_query_last_error.as_deref(),
            Some("quota-down")
        );

        // The caller refreshed with credentials that are no longer stored.
        assert_eq!(
            store
                .save_refresh(&account, refreshed_info, None, None, None)
                .unwrap_err(),
            "QODER_SESSION_CHANGED"
        );
    }

    #[test]
    fn login_snapshot_is_stamped_and_merged_by_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let snapshot = || {
            serde_json::json!({
                "id": "cn-user-1",
                "email": "cn-login@example.invalid",
                "token": "synthetic-token",
                "expireTime": 9_999_999_999_000_i64,
            })
        };

        let first = store.upsert_from_snapshot(snapshot(), None, None).unwrap();
        assert_eq!(
            first.auth_user_info_raw.as_ref().unwrap()["login_source"],
            "qoder_cn"
        );

        let second = store.upsert_from_snapshot(snapshot(), None, None).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.list_accounts().unwrap().len(), 1);
    }

    #[test]
    fn login_snapshot_from_another_channel_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let snapshot = serde_json::json!({
            "id": "cn-user-2",
            "token": "synthetic-token",
            "login_source": "qoder",
        });
        assert_eq!(
            store
                .upsert_from_snapshot(snapshot, None, None)
                .unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
    }

    #[test]
    fn json_import_validates_every_item_before_writing_any() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let payload = serde_json::json!({
            "accounts": [
                { "auth_user_info_raw": { "id": "cn-one", "token": "t", "login_source": "qoder_cn" } },
                { "auth_user_info_raw": { "id": "cn-two", "token": "t", "login_source": "qoder" } },
            ]
        });
        assert_eq!(
            store.import_json(&payload.to_string()).unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
        // The valid first item must not survive a rejected payload.
        assert!(store.list_accounts().unwrap().is_empty());
    }

    #[test]
    fn json_import_and_export_round_trip_selected_accounts() {
        let root = tempfile::tempdir().unwrap();
        let store = QoderCnAccountStore::new(root.path().to_path_buf());
        let payload = serde_json::json!([
            { "userInfo": { "id": "cn-one", "token": "t1" } },
            { "userInfo": { "id": "cn-two", "token": "t2" } },
        ]);
        let imported = store.import_json(&payload.to_string()).unwrap();
        assert_eq!(imported.len(), 2);
        assert!(imported
            .iter()
            .all(|item| item.auth_user_info_raw.as_ref().unwrap()["login_source"] == "qoder_cn"));

        let exported = store
            .export_accounts(&[imported[0].id.clone()])
            .expect("export");
        let restored: Vec<QoderAccount> = serde_json::from_str(&exported).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, imported[0].id);
    }
}
