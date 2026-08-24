use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_account_file_ids(accounts_dir: &Path) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(accounts_dir).map_err(|e| {
        format!(
            "读取账号详情目录失败: path={}, error={}",
            accounts_dir.display(),
            e
        )
    })?;

    let mut account_ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("遍历账号详情目录失败: {}", e))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let is_json = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if !is_json {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        account_ids.push(stem.to_string());
    }

    account_ids.sort();
    account_ids.dedup();
    Ok(account_ids)
}

pub fn load_accounts_from_details<TAccount, FLoad>(
    accounts_dir: &Path,
    mut load_account: FLoad,
) -> Result<Vec<TAccount>, String>
where
    FLoad: FnMut(&str) -> Option<TAccount>,
{
    let account_ids = collect_account_file_ids(accounts_dir)?;
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut accounts = Vec::new();
    for account_id in account_ids {
        if let Some(account) = load_account(&account_id) {
            accounts.push(account);
        }
    }

    Ok(accounts)
}

pub fn sort_accounts_by_recency<T, FLastUsed, FCreatedAt, FId>(
    accounts: &mut [T],
    last_used: FLastUsed,
    created_at: FCreatedAt,
    id: FId,
) where
    FLastUsed: Fn(&T) -> i64,
    FCreatedAt: Fn(&T) -> i64,
    FId: Fn(&T) -> &str,
{
    accounts.sort_by(|left, right| {
        last_used(right)
            .cmp(&last_used(left))
            .then_with(|| created_at(right).cmp(&created_at(left)))
            .then_with(|| id(left).cmp(id(right)))
    });
}

pub fn backup_existing_index(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(format!("索引文件名非法: {}", path.display()));
    };

    let backup_name = format!("{}.bak.{}", file_name, Utc::now().timestamp_millis());
    let backup_path = path.with_file_name(backup_name);
    if let Some(parent) = path.parent() {
        let prefix = format!("{}.bak.", file_name);
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let candidate = entry.path();
                let Some(name) = candidate.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                if name.starts_with(&prefix) && candidate != backup_path {
                    let _ = fs::remove_file(candidate);
                }
            }
        }
    }
    fs::rename(path, &backup_path).map_err(|e| {
        format!(
            "备份账号索引失败: from={}, to={}, error={}",
            path.display(),
            backup_path.display(),
            e
        )
    })?;
    Ok(Some(backup_path))
}
