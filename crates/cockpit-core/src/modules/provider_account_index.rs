//! Typed provider summaries and their shared desktop/CLI SQLite representation.
use super::{
    antigravity_account_index_store::{IndexDocument, IndexRead, IndexStore, StoredSummary},
    legacy_json_input::{Input, ReadError},
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub(crate) struct Revision(AtomicU64);

impl Default for Revision {
    fn default() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
}

impl Clone for Revision {
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

impl Revision {
    pub(crate) fn get(&self) -> Option<u64> {
        let value = self.0.load(Ordering::Relaxed);
        (value != u64::MAX).then_some(value)
    }

    pub(crate) fn set(&self, revision: Option<u64>) {
        self.0
            .store(revision.unwrap_or(u64::MAX), Ordering::Relaxed);
    }
}

pub(crate) fn store<T: IndexDocument>(path: &Path) -> Result<IndexStore<T>, String> {
    let root = path.parent().ok_or("无法获取账号索引目录")?;
    Ok(IndexStore::new(root.to_path_buf()).with_legacy_path(path))
}

pub(crate) fn storage_path<T: IndexDocument>(path: &Path) -> Result<PathBuf, String> {
    store::<T>(path)?.active_path()
}

pub(crate) fn save<T: IndexDocument>(path: &Path, index: &T) -> Result<(), String> {
    store(path)?.replace(index)
}

pub(crate) fn load<T: IndexDocument>(
    path: &Path,
    schedule: impl FnOnce(IndexStore<T>),
) -> Result<IndexRead<T>, String> {
    let store = store(path)?;
    let index = store.load()?;
    if matches!(&index, IndexRead::Legacy(_)) {
        schedule(store);
    }
    Ok(index)
}

pub(crate) fn checked<T: IndexDocument + Default>(
    path: &Path,
    schedule: impl FnOnce(IndexStore<T>),
    repair: impl Fn(&str) -> Option<T>,
    is_legacy_empty: impl FnOnce(&T) -> bool,
) -> Result<T, String> {
    match load(path, schedule)? {
        IndexRead::Active(index) => Ok(index),
        IndexRead::Legacy(index) if is_legacy_empty(&index) => {
            Ok(repair("旧索引账号列表为空").unwrap_or(index))
        }
        IndexRead::Legacy(index) => Ok(index),
        IndexRead::Missing => Ok(repair("旧索引不存在").unwrap_or_default()),
        IndexRead::InvalidLegacy(error) => repair("旧索引损坏").ok_or_else(|| {
            crate::error::file_corrupted_error(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(""),
                &path.to_string_lossy(),
                &error,
            )
        }),
    }
}

pub(crate) fn to_summary(row: &impl Serialize) -> Result<StoredSummary, String> {
    let value = serde_json::to_value(row).map_err(|error| error.to_string())?;
    let text = |key: &str| value[key].as_str().map(str::to_owned);
    let time = |key: &str| value[key].as_i64().unwrap_or_default();
    Ok(StoredSummary {
        id: text("id").ok_or("账号摘要缺少 ID")?,
        email: text("email")
            .or_else(|| text("github_email"))
            .unwrap_or_default(),
        name: text("display_name"),
        plan_type: [
            "plan_type",
            "copilot_plan",
            "plan_name",
            "plan_raw",
            "membership_type",
        ]
        .into_iter()
        .find_map(text),
        provider_login: text("github_login"),
        provider_user_id: text("user_id")
            .or_else(|| text("auth_id"))
            .or_else(|| text("account_uuid")),
        created_at: time("created_at"),
        last_used: time("last_used"),
        payload: Some(serde_json::to_string(&value).map_err(|error| error.to_string())?),
        ..StoredSummary::default()
    })
}

pub(crate) fn from_rows<T: serde::de::DeserializeOwned>(
    rows: Vec<StoredSummary>,
) -> Result<Vec<T>, String> {
    rows.into_iter()
        .map(|row| {
            let payload = row.payload.ok_or("账号摘要数据缺失")?;
            serde_json::from_str(&payload).map_err(|error| error.to_string())
        })
        .collect()
}

macro_rules! provider_index {
    ($index:ty, $summary:ty, $name:literal, $import_empty:expr, $default_version:expr $(, $current:ident)? $(; $metadata:ident)?) => {
        impl crate::modules::antigravity_account_index_store::IndexDocument for $index {
            const LEGACY_FILE: &'static str = concat!($name, "_accounts.json");
            const DATABASE: &'static str = concat!($name, "_accounts.sqlite");
            const KEY: &'static str = concat!($name, "-account-index");
            const CHECK_REVISION: bool = true;
            const IMPORT_EMPTY: bool = $import_empty;
            const RECOVER_DETAILS: bool = crate::modules::provider_account_index::provider_index!(@recover $($metadata)?);
            const DEFAULT_LEGACY_VERSION: Option<&'static str> = $default_version;

            fn from_storage(
                state: crate::modules::antigravity_account_index_store::State,
                rows: Vec<crate::modules::antigravity_account_index_store::StoredSummary>,
            ) -> Result<Self, String> {
                let mut index = Self::default();
                index.version = state.version.ok_or("账号索引缺少版本")?;
                index.accounts = crate::modules::provider_account_index::from_rows(rows)?;
                $(index.$current = state.current_id;)?
                $(crate::modules::provider_account_index::provider_index!(@decode index, state, $metadata);)?
                index.storage_revision.set(state.revision);
                Ok(index)
            }

            fn storage_state(&self) -> crate::modules::antigravity_account_index_store::State {
                let state = crate::modules::antigravity_account_index_store::State::for_document(
                    self.version.clone(),
                    crate::modules::provider_account_index::provider_index!(@current self $(, $current)?),
                    0,
                    self.storage_revision.get(),
                );
                crate::modules::provider_account_index::provider_index!(@encode state, self $(, $metadata)?)

            }

            fn storage_rows(&self) -> impl Iterator<Item = Result<crate::modules::antigravity_account_index_store::StoredSummary, String>> {
                self.accounts.iter().map(crate::modules::provider_account_index::to_summary)
            }

            fn mark_legacy(&mut self) {
                self.storage_revision.set(Some(0));
            }

            fn mark_saved(&self, revision: Option<u64>) {
                self.storage_revision.set(revision);
            }

            fn read_legacy_summary(
                input: &mut crate::modules::legacy_json_input::Input,
            ) -> Result<crate::modules::antigravity_account_index_store::StoredSummary, crate::modules::legacy_json_input::ReadError> {
                crate::modules::provider_account_index::read_summary::<$summary>(input)
            }
        }
    };
    (@current $self:ident, $current:ident) => { $self.$current.clone() };
    (@current $self:ident) => { None };
    (@recover recovery) => { true };
    (@recover $($other:ident)?) => { false };
    (@decode $index:ident, $state:ident, recovery) => {
        $index.detail_recovery_complete = $state.detail_recovery_complete;
        $index.detail_recovery_requires_account = $state.detail_recovery_requires_account;
    };
    (@decode $index:ident, $state:ident, identity) => {
        $index.identity_revision = $state.identity_revision;
        $index.identity_maintenance_version = $state.identity_maintenance_version;
    };
    (@encode $state:ident, $self:ident, identity) => {{
        let mut state = $state;
        state.identity_revision = $self.identity_revision;
        state.identity_maintenance_version = $self.identity_maintenance_version;
        state
    }};
    (@encode $state:ident, $self:ident $(, $other:ident)?) => { $state };

}

pub(crate) fn read_summary<T: serde::de::DeserializeOwned + Serialize>(
    input: &mut Input,
) -> Result<StoredSummary, ReadError> {
    let row = input.value::<T>()?;
    to_summary(&row).map_err(|_| ReadError::Format)
}

provider_index!(
    crate::models::codebuddy::CodebuddyAccountIndex,
    crate::models::codebuddy::CodebuddyAccountSummary,
    "codebuddy",
    false,
    None
);

provider_index!(
    crate::models::github_copilot::GitHubCopilotAccountIndex,
    crate::models::github_copilot::GitHubCopilotAccountSummary,
    "github_copilot",
    false,
    None
);

provider_index!(
    crate::models::qoder::QoderAccountIndex,
    crate::models::qoder::QoderAccountSummary,
    "qoder",
    false,
    None
);

provider_index!(
    crate::models::workbuddy::WorkbuddyAccountIndex,
    crate::models::workbuddy::WorkbuddyAccountSummary,
    "workbuddy",
    false,
    None
);

provider_index!(
    crate::models::zed::ZedAccountIndex,
    crate::models::zed::ZedAccountSummary,
    "zed",
    false,
    None,
    current_account_id
);

provider_index!(
    crate::models::cursor::CursorAccountIndex,
    crate::models::cursor::CursorAccountSummary,
    "cursor",
    true,
    Some("1.0");
    recovery
);

pub(crate) use provider_index;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{codebuddy::CodebuddyAccountIndex, zed::ZedAccountIndex};
    use serde_json::json;
    use std::fs;

    fn fixture(count: usize) -> serde_json::Value {
        json!({
            "version": "1.0",
            "current_account_id": format!("account-{}", count - 1),
            "accounts": (0..count).map(|id| json!({
                "id": format!("account-{id}"),
                "user_id": format!("provider-{id}"),
                "github_login": format!("login-{id}"),
                "display_name": "Display",
                "plan_raw": "pro",
                "tags": ["work"],
                "created_at": 1,
                "last_used": id,
                "access_token": "fixture-credential"
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn provider_index_resumes_and_queries_exact_identity_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("zed_accounts.json");
        let bytes = serde_json::to_vec(&fixture(67)).unwrap();
        fs::write(&path, &bytes).unwrap();
        let first = store::<ZedAccountIndex>(&path).unwrap();
        assert!(!first.migrate_batch().unwrap());
        assert!(
            matches!(first.load().unwrap(), IndexRead::Legacy(index) if index.accounts.len() == 67)
        );
        let reopened = store::<ZedAccountIndex>(&path).unwrap();
        for _ in 0..10 {
            if reopened.migrate_batch().unwrap() {
                break;
            }
        }
        let IndexRead::Active(index) = reopened.load().unwrap() else {
            panic!("expected committed index");
        };
        assert_eq!(index.current_account_id.as_deref(), Some("account-66"));
        assert_eq!(index.accounts[66].tags.as_deref().unwrap(), ["work"]);
        assert_eq!(index.accounts[66].plan_raw.as_deref(), Some("pro"));
        assert_eq!(
            reopened.active_ids_by_provider_user("provider-6").unwrap(),
            Some(vec!["account-6".into()])
        );
        assert_eq!(
            reopened.active_ids_by_provider_user("provider-").unwrap(),
            Some(Vec::new())
        );
        assert!(!fs::read(reopened.database_path())
            .unwrap()
            .windows(b"fixture-credential".len())
            .any(|bytes| bytes == b"fixture-credential"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn provider_index_removal_wins_over_in_progress_migration() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("zed_accounts.json");
        fs::write(&path, serde_json::to_vec(&fixture(67)).unwrap()).unwrap();
        let store = store::<ZedAccountIndex>(&path).unwrap();
        assert!(!store.migrate_batch().unwrap());
        let IndexRead::Legacy(mut edited) = store.load().unwrap() else {
            panic!("expected old index during migration");
        };
        let stale = edited.clone();
        edited.accounts.clear();
        edited.current_account_id = None;
        store.replace(&edited).unwrap();
        assert!(store.migrate_batch().unwrap());
        assert!(store.replace(&stale).is_err());
        assert!(
            matches!(store.load().unwrap(), IndexRead::Active(index) if index.accounts.is_empty())
        );
    }

    #[test]
    fn provider_index_keeps_codebuddy_editions_separate() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("codebuddy_accounts.json");
        let china = root.path().join("codebuddy_cn_accounts.json");
        for (path, id) in [(&global, "global"), (&china, "china")] {
            fs::write(path, json!({
                "version": "1.0",
                "accounts": [{"id": id, "email": "test@example.test", "created_at": 1, "last_used": 1}]
            }).to_string()).unwrap();
            let store = store::<CodebuddyAccountIndex>(path).unwrap();
            assert!(store.migrate_batch().unwrap());
        }
        assert_ne!(
            storage_path::<CodebuddyAccountIndex>(&global).unwrap(),
            storage_path::<CodebuddyAccountIndex>(&china).unwrap()
        );
        for (path, id) in [(&global, "global"), (&china, "china")] {
            assert!(
                matches!(store::<CodebuddyAccountIndex>(path).unwrap().load().unwrap(),
                IndexRead::Active(index) if index.accounts[0].id == id)
            );
        }
    }
}

provider_index!(crate::models::kiro::KiroAccountIndex, crate::models::kiro::KiroAccountSummary, "kiro", false, None; identity);

provider_index!(crate::models::windsurf::WindsurfAccountIndex, crate::models::windsurf::WindsurfAccountSummary, "windsurf", false, None; identity);

provider_index!(
    crate::models::trae::TraeAccountIndex,
    crate::models::trae::TraeAccountSummary,
    "trae",
    false,
    None
);
