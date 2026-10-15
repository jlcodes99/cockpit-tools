// Account snapshots are independent of the live account index. Credentials use
// the same encrypted envelope as active accounts; listing never decrypts them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodexRecycledAccount {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub account_name: Option<String>,
    pub plan_type: Option<String>,
    pub deleted_at: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CodexRecycledAccountFile {
    summary: CodexRecycledAccount,
    encrypted_account: String,
}

fn codex_recycle_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join("codex_account_recycle_bin"))
}

fn validate_recycle_account_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid Codex account ID".into());
    }
    Ok(())
}

fn codex_recycle_path(id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid recycle entry ID".to_string())?;
    Ok(codex_recycle_dir()?.join(format!("{id}.json")))
}

fn archive_codex_account(account: &CodexAccount) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let entry = CodexRecycledAccountFile {
        summary: CodexRecycledAccount {
            id: id.clone(),
            account_id: account.id.clone(),
            email: account.email.clone(),
            account_name: account.account_name.clone(),
            plan_type: account.plan_type.clone(),
            deleted_at: now_timestamp(),
        },
        encrypted_account: crate::modules::secure_account_storage::serialize_account_file(
            "codex",
            &crate::modules::codex_account_proxy::storage_value(account)?,
        )?,
    };
    let content = serde_json::to_string(&entry).map_err(|error| error.to_string())?;
    crate::modules::atomic_write::write_secret_string_atomic(&codex_recycle_path(&id)?, &content)
        .map_err(|error| format!("Cannot save account to recycle bin: {error}"))
}

fn read_codex_recycled_account(id: &str) -> Result<CodexRecycledAccountFile, String> {
    let content = fs::read_to_string(codex_recycle_path(id)?)
        .map_err(|error| format!("Cannot read recycled account: {error}"))?;
    parse_codex_recycled_account(id, &content)
}

fn parse_codex_recycled_account(
    id: &str,
    content: &str,
) -> Result<CodexRecycledAccountFile, String> {
    let entry: CodexRecycledAccountFile = serde_json::from_str(content)
        .map_err(|error| format!("Invalid recycled account: {error}"))?;
    if entry.summary.id != id {
        return Err("Recycle entry identity mismatch".into());
    }
    validate_recycle_account_id(&entry.summary.account_id)?;
    Ok(entry)
}

pub fn list_recycled_accounts() -> Result<Vec<CodexRecycledAccount>, String> {
    // Immutable snapshots are atomically published. Listing must never hold
    // the live-account mutation lock across an unbounded directory traversal.
    let entries = match fs::read_dir(codex_recycle_dir()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot list recycled accounts: {error}")),
    };
    let mut result = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or("Invalid recycle entry filename")?;
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot read recycled account: {error}")),
        };
        result.push(parse_codex_recycled_account(id, &content)?.summary);
    }
    result.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then(left.id.cmp(&right.id))
    });
    Ok(result)
}

pub fn export_recycled_accounts(recycle_ids: &[String]) -> Result<String, String> {
    if recycle_ids.is_empty() {
        return Err("No recycled accounts selected".into());
    }
    let mut accounts = Vec::with_capacity(recycle_ids.len());
    let mut seen = std::collections::HashSet::new();
    // Snapshots are immutable. Read them without holding the live-account lock
    // or consulting a newly imported account that shares the same account ID.
    for id in recycle_ids {
        if !seen.insert(id) {
            continue;
        }
        let entry = read_codex_recycled_account(id)?;
        let account: CodexAccount =
            crate::modules::secure_account_storage::deserialize_encrypted_snapshot(
                &entry.encrypted_account,
            )?;
        if account.id != entry.summary.account_id {
            return Err("Recycled account identity mismatch".into());
        }
        accounts.push(account);
    }
    serialize_accounts_for_export(&accounts)
}

pub fn export_recycled_accounts_to_file(recycle_ids: &[String], path: &str) -> Result<(), String> {
    let path = std::path::Path::new(path);
    let filename = path.file_name().ok_or("Invalid export filename")?;
    let parent = path
        .parent()
        .ok_or("Invalid export directory")?
        .canonicalize()
        .map_err(|error| format!("Cannot access export directory: {error}"))?;
    let data_dir = account::get_data_dir()?
        .canonicalize()
        .map_err(|error| format!("Cannot resolve account storage directory: {error}"))?;
    if parent.starts_with(&data_dir) {
        return Err("Choose an export location outside the application data directory".into());
    }
    // Materialize every selected snapshot before touching the destination.
    // Export errors must never remove a snapshot or produce a partial backup.
    let content = export_recycled_accounts(recycle_ids)?;
    crate::modules::atomic_write::write_secret_string_atomic(&parent.join(filename), &content)
}

pub fn restore_recycled_account(id: &str) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex account mutation lock poisoned".to_string())?;
    let entry = read_codex_recycled_account(id)?;
    let account_id = &entry.summary.account_id;
    if load_account_with_summary(account_id, None)?.is_some() {
        return Err("This account already exists; its recycled copy was kept".into());
    }
    let (mut account, _) = crate::modules::secure_account_storage::deserialize_account_file::<
        CodexAccount,
    >(&codex_recycle_path(id)?, &entry.encrypted_account)?;
    if account.id != *account_id {
        return Err("Recycled account identity mismatch".into());
    }
    let tombstone = read_account_tombstone(account_id);
    account.token_generation = account
        .token_generation
        .max(
            tombstone
                .as_ref()
                .map(|value| value.generation)
                .unwrap_or(0),
        )
        .checked_add(1)
        .ok_or("Account generation exhausted")?;
    let mut index = load_account_index_checked()?;
    index.accounts.retain(|summary| summary.id != *account_id);
    index.accounts.push(CodexAccountSummary {
        id: account.id.clone(),
        email: account.email.clone(),
        plan_type: account.plan_type.clone(),
        subscription_active_until: account.subscription_active_until.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    });
    // Publish the new generation only after both the detail and index exist.
    // On any earlier failure the snapshot remains recoverable and tombstoned.
    save_account_unchecked(&account)?;
    save_account_index(&index)?;
    write_account_tombstone(
        account_id,
        false,
        account.token_generation,
        account_credential_hash(&account),
    )?;
    // The live detail/index/generation are committed. Cleanup is best effort:
    // reporting restore failure here would make retry conflict with the live
    // account. A leftover encrypted copy remains visible for explicit cleanup.
    if let Err(error) = crate::modules::atomic_write::remove_file_locked(&codex_recycle_path(id)?) {
        logger::log_warn(&format!("[Codex Recycle Bin] Account restored; snapshot cleanup failed: entry={id}, error={error}"));
    }
    Ok(())
}

pub fn delete_recycled_account(id: &str) -> Result<(), String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex account mutation lock poisoned".to_string())?;
    let path = codex_recycle_path(id)?;
    if !path.exists() {
        return Ok(());
    }
    let entry = read_codex_recycled_account(id)?;
    // Remove leftovers from an interrupted soft deletion, but never remove a
    // newly imported live account with the same identity. Keep the tombstone.
    if account_is_tombstoned(&entry.summary.account_id) {
        delete_account_file_unlocked(&entry.summary.account_id)?;
        let backup = get_accounts_dir().join(format!("{}.json.bak", entry.summary.account_id));
        crate::modules::atomic_write::remove_file_locked(&backup)?;
    }
    crate::modules::atomic_write::remove_file_locked(&path)?;
    Ok(())
}
