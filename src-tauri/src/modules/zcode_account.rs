//! ZCode 账号存储与额度刷新。
//!
//! 账号来源：zcode-switch 的本地快照（`~/.zcode-switch/accounts/*.json`），
//! 无快照时兜底导入本机 ZCode 活跃凭证（`~/.zcode/v2/credentials.json`）。
//! 只读凭证做额度查询，不写回 ZCode 配置（不做切换）。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::zcode::{ZcodeAccount, ZcodeAccountIndex, ZcodeStoredAccount};
use crate::modules::zcode_credentials::{
    default_secret, home_dir, identity_with_secret, list_switch_snapshots, new_gen_provider_config,
    read_live_config, read_live_credentials,
};
use crate::modules::{account, logger, zcode_quota};

const ACCOUNTS_INDEX_FILE: &str = "zcode_accounts.json";
const ACCOUNTS_DIR: &str = "zcode_accounts";
/// 无 zcode-switch 快照时，本机活跃凭证导入后的账号 ID
const LOCAL_ACCOUNT_ID: &str = "local-live";

static ZCODE_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZcodeExportPayload {
    version: String,
    accounts: Vec<ZcodeStoredAccount>,
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 ZCode 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.');
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }
    Ok(trimmed.to_string())
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(get_accounts_dir()?.join(format!("{}.json", normalized)))
}

pub fn load_stored_account(account_id: &str) -> Option<ZcodeStoredAccount> {
    let account_path = resolve_account_file_path(account_id).ok()?;
    if !account_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&account_path).ok()?;
    crate::modules::atomic_write::parse_json_with_auto_restore(&account_path, &content).ok()
}

fn save_stored_account_file(account: &ZcodeStoredAccount) -> Result<(), String> {
    let path = resolve_account_file_path(account.public_account.id.as_str())?;
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存账号失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }
    Ok(())
}

fn load_account_index() -> ZcodeAccountIndex {
    let path = match get_accounts_index_path() {
        Ok(p) => p,
        Err(_) => return ZcodeAccountIndex::new(),
    };
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(ZcodeAccountIndex::new);
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(ZcodeAccountIndex::new)
        }
        Ok(content) => match crate::modules::atomic_write::parse_json_with_auto_restore::<
            ZcodeAccountIndex,
        >(&path, &content)
        {
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(ZcodeAccountIndex::new),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Zcode Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(ZcodeAccountIndex::new)
            }
        },
        Err(_) => ZcodeAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<ZcodeAccountIndex, String> {
    let path = get_accounts_index_path()?;
    if !path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            return Ok(index);
        }
        return Ok(ZcodeAccountIndex::new());
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件读取失败") {
                return Ok(index);
            }
            return Err(format!("读取账号索引失败: {}", err));
        }
    };

    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            return Ok(index);
        }
        return Ok(ZcodeAccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<ZcodeAccountIndex>(
        &path, &content,
    ) {
        Ok(index) if !index.accounts.is_empty() => Ok(index),
        Ok(index) => {
            if let Some(repaired) = repair_account_index_from_details("索引账号列表为空") {
                return Ok(repaired);
            }
            Ok(index)
        }
        Err(err) => {
            if let Some(index) = repair_account_index_from_details("索引文件损坏") {
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                ACCOUNTS_INDEX_FILE,
                &path.to_string_lossy(),
                &err.to_string(),
            ))
        }
    }
}

fn save_account_index(index: &ZcodeAccountIndex) -> Result<(), String> {
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))
}

fn repair_account_index_from_details(reason: &str) -> Option<ZcodeAccountIndex> {
    let index_path = get_accounts_index_path().ok()?;
    let accounts_dir = get_accounts_dir().ok()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_stored_account(account_id),
    )
    .ok()?;

    if accounts.is_empty() {
        return None;
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.public_account.last_used,
        |account| account.public_account.created_at,
        |account| account.public_account.id.as_str(),
    );

    let mut index = ZcodeAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Zcode Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Zcode Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_warn(&format!(
        "[Zcode Account] 检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn refresh_summary(index: &mut ZcodeAccountIndex, account: &ZcodeStoredAccount) {
    if let Some(summary) = index
        .accounts
        .iter_mut()
        .find(|item| item.id == account.public_account.id)
    {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn list_stored_accounts_from_index(index: &ZcodeAccountIndex) -> Vec<ZcodeStoredAccount> {
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_stored_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| {
        b.public_account
            .last_used
            .cmp(&a.public_account.last_used)
            .then_with(|| a.public_account.name.cmp(&b.public_account.name))
    });
    accounts
}

pub fn list_accounts() -> Vec<ZcodeAccount> {
    let index = load_account_index();
    list_stored_accounts_from_index(&index)
        .into_iter()
        .map(|account| account.to_public())
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<ZcodeAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(list_stored_accounts_from_index(&index)
        .into_iter()
        .map(|account| account.to_public())
        .collect())
}

fn upsert_account_record(mut account: ZcodeStoredAccount) -> Result<ZcodeAccount, String> {
    let _lock = ZCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    let mut index = load_account_index();

    if let Some(existing) = load_stored_account(&account.public_account.id) {
        account.public_account.created_at = existing.public_account.created_at;
        if account.public_account.tags.is_none() {
            account.public_account.tags = existing.public_account.tags;
        }
    }

    save_stored_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account.to_public())
}

fn update_quota_query_error(
    account_id: &str,
    message: Option<String>,
) -> Result<Option<ZcodeAccount>, String> {
    let Some(mut stored) = load_stored_account(account_id) else {
        return Ok(None);
    };
    stored.public_account.quota_query_last_error = message;
    stored.public_account.quota_query_last_error_at = stored
        .public_account
        .quota_query_last_error
        .as_ref()
        .map(|_| Utc::now().timestamp_millis());
    let updated = upsert_account_record(stored)?;
    Ok(Some(updated))
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = ZCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    delete_account_file(account_id)?;
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let targets: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if targets.is_empty() {
        return Ok(());
    }

    let _lock = ZCODE_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 ZCode 账号锁失败".to_string())?;
    let mut index = load_account_index();
    index.accounts.retain(|item| !targets.contains(&item.id));
    save_account_index(&index)?;

    for account_id in targets {
        delete_account_file(&account_id)?;
    }

    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Option<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            normalized.push(trimmed);
        }
    }
    (!normalized.is_empty()).then_some(normalized)
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<ZcodeAccount, String> {
    let Some(mut stored) = load_stored_account(account_id) else {
        return Err(format!("账号不存在: {account_id}"));
    };
    stored.public_account.tags = normalize_tags(tags);
    upsert_account_record(stored)
}

/// 从快照/凭证构造待入库账号（不落盘）。
fn build_stored_account(
    id: &str,
    name: &str,
    credentials: Value,
    config: Option<Value>,
) -> Option<ZcodeStoredAccount> {
    if credentials
        .as_object()
        .map(|m| m.is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    let home = home_dir();
    let secret = default_secret(&home);
    let identity = identity_with_secret(&credentials, &secret);
    let now = Utc::now().timestamp();
    Some(ZcodeStoredAccount {
        public_account: ZcodeAccount {
            id: id.to_string(),
            name: name.trim().to_string(),
            provider: Some(identity.provider),
            email: identity.email,
            display_name: identity.display_name.or(identity.username),
            plan_tier: None,
            plan_expire: None,
            quota_total: None,
            quota_used: None,
            quota_remaining: None,
            quota_percent_used: None,
            quota_items: vec![],
            quota_source: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            tags: None,
            created_at: now,
            last_used: 0,
        },
        credentials,
        config,
    })
}

/// 导入本机已有的 ZCode 账号：优先 zcode-switch 快照，其次本机活跃凭证。
/// 返回本次导入/更新的账号列表。
pub fn import_from_local() -> Result<Vec<ZcodeAccount>, String> {
    let mut imported = vec![];

    for snap in list_switch_snapshots() {
        if let Some(stored) =
            build_stored_account(&snap.id, &snap.name, snap.credentials, snap.config)
        {
            let public = upsert_account_record(stored)?;
            imported.push(public);
        }
    }

    if imported.is_empty() {
        if let Some(live) = read_live_credentials() {
            let config = (!new_gen_provider_config())
                .then(read_live_config)
                .flatten();
            if let Some(stored) = build_stored_account(LOCAL_ACCOUNT_ID, "本机账号", live, config)
            {
                let public = upsert_account_record(stored)?;
                imported.push(public);
            }
        }
    }

    if imported.is_empty() {
        return Err(
            "未找到 ZCode 账号：需要本机存在 zcode-switch 快照（~/.zcode-switch/accounts）或已登录的 ZCode 凭证（~/.zcode/v2/credentials.json）"
                .to_string(),
        );
    }

    Ok(imported)
}

fn apply_quota_overview(account: &mut ZcodeAccount, ov: zcode_quota::QuotaOverview) {
    account.plan_tier = ov.plan_tier;
    account.plan_expire = ov.plan_expire;
    account.quota_total = ov.total;
    account.quota_used = ov.used;
    account.quota_remaining = ov.remaining;
    account.quota_percent_used = ov.percent_used;
    account.quota_items = ov.items;
    account.quota_source = (!ov.source.is_empty()).then_some(ov.source);
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.usage_updated_at = Some(Utc::now().timestamp_millis());
    account.last_used = Utc::now().timestamp();
}

pub async fn refresh_account(account_id: &str) -> Result<ZcodeAccount, String> {
    let stored =
        load_stored_account(account_id).ok_or_else(|| format!("账号不存在: {account_id}"))?;
    match zcode_quota::query_credentials_quota(&stored.credentials, stored.config.as_ref()).await {
        Ok(ov) => {
            if ov.is_empty && ov.items.is_empty() {
                // 无套餐：保留提示但不覆盖历史额度
                let mut updated = stored.public_account.clone();
                updated.quota_query_last_error =
                    Some("该凭证当前没有可用的 Coding/Start Plan 套餐".to_string());
                updated.quota_query_last_error_at = Some(Utc::now().timestamp_millis());
                updated.usage_updated_at = Some(Utc::now().timestamp_millis());
                updated.last_used = Utc::now().timestamp();
                return upsert_account_record(ZcodeStoredAccount {
                    public_account: updated,
                    credentials: stored.credentials,
                    config: stored.config,
                });
            }
            let mut updated = stored.public_account.clone();
            apply_quota_overview(&mut updated, ov);
            upsert_account_record(ZcodeStoredAccount {
                public_account: updated,
                credentials: stored.credentials,
                config: stored.config,
            })
        }
        Err(err) => {
            update_quota_query_error(account_id, Some(err.clone()))?;
            Err(err)
        }
    }
}

pub async fn refresh_all_accounts() -> Result<Vec<ZcodeAccount>, String> {
    let mut refreshed = Vec::new();
    for account in load_account_index().accounts.clone() {
        match refresh_account(&account.id).await {
            Ok(updated) => refreshed.push(updated),
            Err(err) => {
                logger::log_warn(&format!(
                    "[Zcode Account] 刷新账号额度失败: account_id={}, err={}",
                    account.id, err
                ));
            }
        }
    }
    Ok(refreshed)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<ZcodeAccount>, String> {
    let payload: ZcodeExportPayload =
        serde_json::from_str(json_content).map_err(|e| format!("解析 ZCode 导出文件失败: {e}"))?;
    let mut imported = vec![];
    for account in payload.accounts {
        let public = upsert_account_record(account)?;
        imported.push(public);
    }
    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let mut accounts = vec![];
    for id in account_ids {
        if let Some(stored) = load_stored_account(id) {
            accounts.push(stored);
        }
    }
    if accounts.is_empty() {
        return Err("没有可导出的 ZCode 账号".to_string());
    }
    let payload = ZcodeExportPayload {
        version: "1.0".to_string(),
        accounts,
    };
    serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化导出数据失败: {e}"))
}
