use super::antigravity_account_index_store::{IndexDocument, IndexStore, State, StoredSummary};
use crate::models::codex::{CodexAccountIndex, CodexAccountSummary};

pub(crate) type CodexIndexStore = IndexStore<CodexAccountIndex>;

impl IndexDocument for CodexAccountIndex {
    const LEGACY_FILE: &'static str = "codex_accounts.json";
    const DATABASE: &'static str = "codex_accounts.sqlite";
    const KEY: &'static str = "codex-account-index";
    const MIN_DETAIL_SCHEMA: u32 = 2;
    const CHECK_REVISION: bool = true;

    fn from_storage(state: State, rows: Vec<StoredSummary>) -> Result<Self, String> {
        Ok(Self {
            version: state.version.ok_or("Codex 账号索引缺少版本")?,
            detail_schema_version: state.detail_schema.unwrap_or_default(),
            current_account_id: state.current_id,
            storage_revision: state.revision,
            accounts: rows
                .into_iter()
                .map(|row| CodexAccountSummary {
                    id: row.id,
                    email: row.email,
                    plan_type: row.plan_type,
                    subscription_active_until: row.subscription_active_until,
                    created_at: row.created_at,
                    last_used: row.last_used,
                })
                .collect(),
        })
    }

    fn storage_state(&self) -> State {
        State::for_document(
            self.version.clone(),
            self.current_account_id.clone(),
            self.detail_schema_version,
            self.storage_revision,
        )
    }

    fn storage_rows(&self) -> impl Iterator<Item = Result<StoredSummary, String>> {
        self.accounts.iter().map(|row| {
            Ok(StoredSummary {
                id: row.id.clone(),
                email: row.email.clone(),
                name: None,
                plan_type: row.plan_type.clone(),
                subscription_active_until: row.subscription_active_until.clone(),
                created_at: row.created_at,
                last_used: row.last_used,
                ..StoredSummary::default()
            })
        })
    }

    fn mark_legacy(&mut self) {
        self.storage_revision = Some(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::antigravity_account_index_store::IndexRead;
    use std::fs;

    fn fixture(rows: usize) -> CodexAccountIndex {
        let mut index = CodexAccountIndex::new();
        index.current_account_id = Some(format!("account-{}", rows - 1));
        index.accounts = (0..rows)
            .map(|i| CodexAccountSummary {
                id: format!("account-{i}"),
                email: format!("user-{i}@example.test"),
                plan_type: Some("team".into()),
                subscription_active_until: Some("2026-12-01T00:00:00Z".into()),
                created_at: i as i64,
                last_used: i as i64,
            })
            .collect();
        index
    }

    fn active(store: &CodexIndexStore) -> CodexAccountIndex {
        match store.load().unwrap() {
            IndexRead::Active(index) => index,
            _ => panic!("expected a committed index"),
        }
    }

    #[test]
    fn codex_index_resumes_from_older_root_and_preserves_summary_fields() {
        let dir = tempfile::tempdir().unwrap();
        let older = tempfile::tempdir().unwrap();
        let open = || CodexIndexStore::new(dir.path().into()).with_old_root(older.path().into());
        let store = open();
        let bytes = serde_json::to_vec(&fixture(81)).unwrap();
        let original = older.path().join("codex_accounts.json");
        fs::write(&original, &bytes).unwrap();
        assert!(
            matches!(store.load().unwrap(), IndexRead::Legacy(index) if index.accounts.len() == 81)
        );
        assert!(!dir.path().join("codex_accounts.sqlite").exists());
        assert!(!store.migrate_batch().unwrap());
        assert!(!store.migrate_batch().unwrap());
        assert!(
            matches!(store.load().unwrap(), IndexRead::Legacy(index) if index.accounts.len() == 81)
        );
        drop(store);
        let store = open();
        for _ in 0..10 {
            if store.migrate_batch().unwrap() {
                break;
            }
        }
        let index = active(&store);
        assert_eq!(index.current_account_id.as_deref(), Some("account-80"));
        assert_eq!(index.accounts.len(), 81);
        assert_eq!(index.accounts[0].plan_type.as_deref(), Some("team"));
        assert_eq!(
            index.accounts[80].subscription_active_until.as_deref(),
            Some("2026-12-01T00:00:00Z")
        );
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }

    #[test]
    fn codex_index_concurrent_save_wins_over_migration_and_stale_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodexIndexStore::new(dir.path().into());
        fs::write(
            store.legacy_path(),
            serde_json::to_vec(&fixture(81)).unwrap(),
        )
        .unwrap();
        assert!(!store.migrate_batch().unwrap());
        let IndexRead::Legacy(mut next) = store.load().unwrap() else {
            panic!("expected the legacy index");
        };
        let stale = next.clone();
        next.accounts.clear();
        next.current_account_id = None;
        store.replace(&next).unwrap();
        assert!(store.migrate_batch().unwrap());
        assert!(store.replace(&stale).is_err());
        assert!(store.replace(&CodexAccountIndex::new()).is_err());
        assert!(active(&store).accounts.is_empty());
        assert!(store.legacy_path().exists());
    }

    #[test]
    fn codex_index_keeps_old_detail_schema_available_for_repair() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodexIndexStore::new(dir.path().into());
        let mut index = fixture(1);
        index.detail_schema_version = 0;
        fs::write(store.legacy_path(), serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(store.migrate_batch().unwrap());
        assert!(
            matches!(store.load().unwrap(), IndexRead::Legacy(index) if index.detail_schema_version == 0)
        );
    }
}
