use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::models::{
    Account, AccountIndex, AccountSummary, DeviceProfile, DeviceProfileVersion, QuotaData,
    QuotaErrorInfo, TokenData,
};
use crate::modules;

static ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static LIST_ACCOUNTS_CACHE: std::sync::LazyLock<Mutex<Option<ListAccountsCacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
static LIST_ACCOUNTS_LOAD_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const LIST_ACCOUNTS_CACHE_TTL_MS: u64 = 800;

// 使用与 AntigravityCockpit 插件相同的数据目录
const DATA_DIR: &str = ".antigravity_cockpit";

/// 对邮箱地址进行脱敏，仅保留首字符和域名，例如 "u***@example.com"
#[inline]
fn mask_email(email: &str) -> String {
    if let Some(at_pos) = email.find('@') {
        let local = &email[..at_pos];
        let domain = &email[at_pos..];
        let first = local
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        format!("{}***{}", first, domain)
    } else {
        "***".to_string()
    }
}
const ACCOUNTS_INDEX: &str = "accounts.json";
const ACCOUNTS_DIR: &str = "accounts";
const DELETED_ACCOUNT_FP_BINDINGS: &str = "deleted_account_fingerprint_bindings.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeletedAccountFingerprintBindings {
    #[serde(default)]
    by_email: HashMap<String, String>,
}

#[derive(Clone)]
struct ListAccountsCacheEntry {
    cached_at: Instant,
    accounts: Vec<Account>,
}

fn invalidate_list_accounts_cache() {
    if let Ok(mut cache) = LIST_ACCOUNTS_CACHE.lock() {
        *cache = None;
    }
}

fn read_list_accounts_cache() -> Option<Vec<Account>> {
    let Ok(cache) = LIST_ACCOUNTS_CACHE.lock() else {
        return None;
    };

    let Some(entry) = cache.as_ref() else {
        return None;
    };

    if entry.cached_at.elapsed() > Duration::from_millis(LIST_ACCOUNTS_CACHE_TTL_MS) {
        return None;
    }

    Some(entry.accounts.clone())
}

fn write_list_accounts_cache(accounts: &[Account]) {
    if let Ok(mut cache) = LIST_ACCOUNTS_CACHE.lock() {
        *cache = Some(ListAccountsCacheEntry {
            cached_at: Instant::now(),
            accounts: accounts.to_vec(),
        });
    }
}

fn normalize_account_email_key(email: &str) -> String {
    email.trim().to_lowercase()
}

fn get_deleted_account_fp_bindings_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(DELETED_ACCOUNT_FP_BINDINGS))
}

fn load_deleted_account_fp_bindings() -> Result<DeletedAccountFingerprintBindings, String> {
    let path = get_deleted_account_fp_bindings_path()?;
    if !path.exists() {
        return Ok(DeletedAccountFingerprintBindings::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("读取指纹映射失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(DeletedAccountFingerprintBindings::default());
    }

    match serde_json::from_str::<DeletedAccountFingerprintBindings>(&content) {
        Ok(bindings) => Ok(bindings),
        Err(e) => {
            modules::logger::log_warn(&format!("指纹映射文件损坏，已重置为空: {}", e));
            Ok(DeletedAccountFingerprintBindings::default())
        }
    }
}

fn save_deleted_account_fp_bindings(
    bindings: &DeletedAccountFingerprintBindings,
) -> Result<(), String> {
    let path = get_deleted_account_fp_bindings_path()?;
    let content =
        serde_json::to_string_pretty(bindings).map_err(|e| format!("序列化指纹映射失败: {}", e))?;
    fs::write(path, content).map_err(|e| format!("保存指纹映射失败: {}", e))
}

fn remember_deleted_account_fingerprint(account: &Account) -> Result<(), String> {
    let key = normalize_account_email_key(&account.email);
    if key.is_empty() {
        return Ok(());
    }

    let Some(fp_id) = account.fingerprint_id.as_ref() else {
        return Ok(());
    };

    if crate::modules::fingerprint::get_fingerprint(fp_id).is_err() {
        modules::logger::log_warn(&format!(
            "删除账号时发现指纹不存在，跳过映射记录: email={}, fingerprint_id={}",
            mask_email(&account.email),
            fp_id
        ));
        return Ok(());
    }

    let mut bindings = load_deleted_account_fp_bindings()?;
    bindings.by_email.insert(key, fp_id.clone());
    save_deleted_account_fp_bindings(&bindings)?;
    Ok(())
}

fn lookup_deleted_account_fingerprint(email: &str) -> Result<Option<String>, String> {
    let key = normalize_account_email_key(email);
    if key.is_empty() {
        return Ok(None);
    }

    let bindings = load_deleted_account_fp_bindings()?;
    let Some(fp_id) = bindings.by_email.get(&key).cloned() else {
        return Ok(None);
    };

    if crate::modules::fingerprint::get_fingerprint(&fp_id).is_ok() {
        Ok(Some(fp_id))
    } else {
        modules::logger::log_warn(&format!(
            "账号重建时命中过期指纹映射，已忽略: email={}, fingerprint_id={}",
            mask_email(email),
            fp_id
        ));
        let _ = clear_deleted_account_fingerprint(email);
        Ok(None)
    }
}

fn clear_deleted_account_fingerprint(email: &str) -> Result<(), String> {
    let key = normalize_account_email_key(email);
    if key.is_empty() {
        return Ok(());
    }

    let mut bindings = load_deleted_account_fp_bindings()?;
    if bindings.by_email.remove(&key).is_some() {
        save_deleted_account_fp_bindings(&bindings)?;
    }
    Ok(())
}

/// 获取数据目录路径
pub fn get_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let data_dir = home.join(DATA_DIR);

    if !data_dir.exists() {
        fs::create_dir_all(&data_dir).map_err(|e| format!("创建数据目录失败: {}", e))?;
    }

    Ok(data_dir)
}

/// 获取账号目录路径
pub fn get_accounts_dir() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    let accounts_dir = data_dir.join(ACCOUNTS_DIR);

    if !accounts_dir.exists() {
        fs::create_dir_all(&accounts_dir).map_err(|e| format!("创建账号目录失败: {}", e))?;
    }

    Ok(accounts_dir)
}

fn repair_account_index_from_details(reason: &str) -> Result<Option<AccountIndex>, String> {
    let index_path = get_data_dir()?.join(ACCOUNTS_INDEX);
    let accounts_dir = get_accounts_dir()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id).ok(),
    )?;

    if accounts.is_empty() {
        return Ok(None);
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = AccountIndex::new();
    index.accounts = accounts
        .iter()
        .map(|account| AccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            name: account.name.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        })
        .collect();
    index.current_account_id = accounts.first().map(|account| account.id.clone());

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            modules::logger::log_warn(&format!(
                "自动修复账号索引前备份失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        modules::logger::log_warn(&format!(
            "自动修复账号索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    modules::logger::log_warn(&format!(
        "检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Ok(Some(index))
}

/// 加载账号索引
pub fn load_account_index() -> Result<AccountIndex, String> {
    let data_dir = get_data_dir()?;
    let index_path = data_dir.join(ACCOUNTS_INDEX);

    if !index_path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在")? {
            return Ok(index);
        }
        return Ok(AccountIndex::new());
    }

    let content =
        fs::read_to_string(&index_path).map_err(|e| format!("读取账号索引失败: {}", e))?;

    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空")? {
            return Ok(index);
        }
        return Ok(AccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<AccountIndex>(
        &index_path,
        &content,
    ) {
        Ok(index) => {
            if index.accounts.is_empty() {
                if let Some(repaired) = repair_account_index_from_details("索引账号列表为空")?
                {
                    return Ok(repaired);
                }
            }
            Ok(index)
        }
        Err(e) => {
            if let Some(index) = repair_account_index_from_details("索引文件损坏")? {
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                ACCOUNTS_INDEX,
                &index_path.to_string_lossy(),
                &e.to_string(),
            ))
        }
    }
}

/// 保存账号索引
pub fn save_account_index(index: &AccountIndex) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    let index_path = data_dir.join(ACCOUNTS_INDEX);

    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&index_path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))?;
    invalidate_list_accounts_cache();
    Ok(())
}

/// 加载账号数据
pub fn load_account(account_id: &str) -> Result<Account, String> {
    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account_id));

    if !account_path.exists() {
        return Err(format!("账号不存在: {}", account_id));
    }

    let content =
        fs::read_to_string(&account_path).map_err(|e| format!("读取账号数据失败: {}", e))?;

    crate::modules::atomic_write::parse_json_with_auto_restore::<Account>(&account_path, &content)
        .map_err(|e| format!("解析账号数据失败: {}", e))
}

/// 保存账号数据
pub fn save_account(account: &Account) -> Result<(), String> {
    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account.id));

    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号数据失败: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&account_path, &content)
        .map_err(|e| format!("保存账号数据失败: {}", e))?;
    invalidate_list_accounts_cache();
    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    let mut result: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("标签不能为空".to_string());
        }
        if trimmed.chars().count() > 20 {
            return Err("标签长度不能超过 20 个字符".to_string());
        }
        let normalized = trimmed.to_lowercase();
        if seen.insert(normalized.clone()) {
            result.push(normalized);
        }
    }

    if result.len() > 10 {
        return Err("标签数量不能超过 10 个".to_string());
    }

    Ok(result)
}

/// 更新账号标签
pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<Account, String> {
    let mut account = load_account(account_id)?;
    let normalized = normalize_tags(tags)?;
    account.tags = normalized;
    save_account(&account)?;
    Ok(account)
}

/// 更新账号备注
pub fn update_account_notes(account_id: &str, notes: String) -> Result<Account, String> {
    let mut account = load_account(account_id)?;
    let trimmed = notes.trim().to_string();
    account.notes = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    save_account(&account)?;
    Ok(account)
}

/// 列出所有账号
pub fn list_accounts() -> Result<Vec<Account>, String> {
    if let Some(accounts) = read_list_accounts_cache() {
        return Ok(accounts);
    }

    let _load_guard = LIST_ACCOUNTS_LOAD_LOCK
        .lock()
        .map_err(|e| format!("获取账号列表锁失败: {}", e))?;

    if let Some(accounts) = read_list_accounts_cache() {
        return Ok(accounts);
    }

    modules::logger::log_info("开始列出账号...");
    let index = load_account_index()?;
    let mut accounts = Vec::new();

    for summary in &index.accounts {
        match load_account(&summary.id) {
            Ok(mut account) => {
                let _ = modules::quota_cache::apply_cached_quota(&mut account, "authorized");
                accounts.push(account);
            }
            Err(e) => {
                modules::logger::log_error(&format!("加载账号失败: {}", e));
            }
        }
    }

    write_list_accounts_cache(&accounts);
    Ok(accounts)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn is_strict_account_identity_match(existing: &Account, email: &str, token: &TokenData) -> bool {
    if let Some(session_id) = non_empty(token.session_id.as_deref()) {
        if non_empty(existing.token.session_id.as_deref()) == Some(session_id) {
            return true;
        }
    }

    if let Some(refresh_token) = non_empty(Some(token.refresh_token.as_str())) {
        if non_empty(Some(existing.token.refresh_token.as_str())) == Some(refresh_token) {
            return true;
        }
    }

    if existing.email == email {
        if let Some(project_id) = non_empty(token.project_id.as_deref()) {
            if non_empty(existing.token.project_id.as_deref()) == Some(project_id) {
                return true;
            }
        }
    }

    false
}

fn find_matching_account_id(
    index: &AccountIndex,
    email: &str,
    token: &TokenData,
) -> Result<Option<String>, String> {
    for summary in &index.accounts {
        let existing = match load_account(&summary.id) {
            Ok(account) => account,
            Err(err) => {
                modules::logger::log_warn(&format!(
                    "账号匹配时跳过损坏账号文件: id={}, error={}",
                    summary.id, err
                ));
                continue;
            }
        };

        if is_strict_account_identity_match(&existing, email, token) {
            return Ok(Some(existing.id));
        }
    }

    Ok(None)
}

/// 添加账号
pub fn add_account(
    email: String,
    name: Option<String>,
    token: TokenData,
) -> Result<Account, String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    if find_matching_account_id(&index, &email, &token)?.is_some() {
        return Err(format!("账号已存在: {}", email));
    }

    let account_id = Uuid::new_v4().to_string();
    let mut account = Account::new(account_id.clone(), email.clone(), token);
    account.name = name.clone();

    let reused_fp_id = match lookup_deleted_account_fingerprint(&email) {
        Ok(fp_id) => fp_id,
        Err(e) => {
            modules::logger::log_warn(&format!(
                "读取已删除账号指纹映射失败，回退为新建指纹: email={}, error={}",
                mask_email(&email),
                e
            ));
            None
        }
    };

    if let Some(ref fp_id) = reused_fp_id {
        account.fingerprint_id = Some(fp_id.clone());
        modules::logger::log_info(&format!(
            "账号复用已删除映射指纹: email={}, fingerprint_id={}",
            mask_email(&email),
            fp_id
        ));
    } else {
        let fingerprint = crate::modules::fingerprint::generate_fingerprint(email.clone())?;
        account.fingerprint_id = Some(fingerprint.id.clone());
    }

    save_account(&account)?;

    index.accounts.push(AccountSummary {
        id: account_id.clone(),
        email: email.clone(),
        name: name.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    });

    if index.current_account_id.is_none() {
        index.current_account_id = Some(account_id);
    }

    save_account_index(&index)?;

    if reused_fp_id.is_some() {
        if let Err(e) = clear_deleted_account_fingerprint(&email) {
            modules::logger::log_warn(&format!(
                "清理已删除账号指纹映射失败: email={}, error={}",
                mask_email(&email),
                e
            ));
        }
    }

    Ok(account)
}

/// 添加或更新账号
pub fn upsert_account(
    email: String,
    name: Option<String>,
    token: TokenData,
) -> Result<Account, String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let existing_account_id = find_matching_account_id(&index, &email, &token)?;

    if let Some(account_id) = existing_account_id {
        match load_account(&account_id) {
            Ok(mut account) => {
                account.token = token;
                account.name = name.clone();
                if account.disabled {
                    account.disabled = false;
                    account.disabled_reason = None;
                    account.disabled_at = None;
                }
                account.update_last_used();
                save_account(&account)?;

                if let Some(idx_summary) = index.accounts.iter_mut().find(|s| s.id == account_id) {
                    idx_summary.name = name;
                    save_account_index(&index)?;
                }

                return Ok(account);
            }
            Err(e) => {
                modules::logger::log_warn(&format!("账号文件缺失，正在重建: {}", e));
                let mut account = Account::new(account_id.clone(), email.clone(), token);
                account.name = name.clone();
                let fingerprint = crate::modules::fingerprint::generate_fingerprint(email.clone())?;
                account.fingerprint_id = Some(fingerprint.id.clone());
                save_account(&account)?;

                if let Some(idx_summary) = index.accounts.iter_mut().find(|s| s.id == account_id) {
                    idx_summary.name = name;
                    save_account_index(&index)?;
                }

                return Ok(account);
            }
        }
    }

    drop(_lock);
    add_account(email, name, token)
}

/// 删除账号
pub fn delete_account(account_id: &str) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    if let Ok(account) = load_account(account_id) {
        if let Err(e) = remember_deleted_account_fingerprint(&account) {
            modules::logger::log_warn(&format!(
                "记录删除账号指纹映射失败: account_id={}, email={}, error={}",
                account_id,
                mask_email(&account.email),
                e
            ));
        }
    }

    let original_len = index.accounts.len();
    index.accounts.retain(|s| s.id != account_id);

    if index.accounts.len() == original_len {
        return Err(format!("找不到账号 ID: {}", account_id));
    }

    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = index.accounts.first().map(|s| s.id.clone());
    }

    save_account_index(&index)?;

    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account_id));

    if account_path.exists() {
        fs::remove_file(&account_path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }

    Ok(())
}

/// 批量删除账号
pub fn delete_accounts(account_ids: &[String]) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let accounts_dir = get_accounts_dir()?;

    for account_id in account_ids {
        if let Ok(account) = load_account(account_id) {
            if let Err(e) = remember_deleted_account_fingerprint(&account) {
                modules::logger::log_warn(&format!(
                    "批量删除时记录账号指纹映射失败: account_id={}, email={}, error={}",
                    account_id,
                    mask_email(&account.email),
                    e
                ));
            }
        }

        index.accounts.retain(|s| &s.id != account_id);

        if index.current_account_id.as_deref() == Some(account_id) {
            index.current_account_id = None;
        }

        let account_path = accounts_dir.join(format!("{}.json", account_id));
        if account_path.exists() {
            let _ = fs::remove_file(&account_path);
        }
    }

    if index.current_account_id.is_none() {
        index.current_account_id = index.accounts.first().map(|s| s.id.clone());
    }

    save_account_index(&index)
}

/// 重新排序账号列表
pub fn reorder_accounts(account_ids: &[String]) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let id_to_summary: std::collections::HashMap<_, _> = index
        .accounts
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut new_accounts = Vec::new();
    for id in account_ids {
        if let Some(summary) = id_to_summary.get(id) {
            new_accounts.push(summary.clone());
        }
    }

    for summary in &index.accounts {
        if !account_ids.contains(&summary.id) {
            new_accounts.push(summary.clone());
        }
    }

    index.accounts = new_accounts;

    save_account_index(&index)
}

/// 获取当前账号 ID
pub fn get_current_account_id() -> Result<Option<String>, String> {
    let index = load_account_index()?;
    Ok(index.current_account_id)
}

/// 获取当前激活账号
pub fn get_current_account() -> Result<Option<Account>, String> {
    if let Some(id) = get_current_account_id()? {
        let mut account = load_account(&id)?;
        let _ = modules::quota_cache::apply_cached_quota(&mut account, "authorized");
        Ok(Some(account))
    } else {
        Ok(None)
    }
}

/// 设置当前激活账号 ID
pub fn set_current_account_id(account_id: &str) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;
    index.current_account_id = Some(account_id.to_string());
    save_account_index(&index)?;

    // 同时写入 current_account.json 供扩展读取
    if let Ok(account) = load_account(account_id) {
        let _ = save_current_account_file(&account.email);
    }

    Ok(())
}

/// 保存当前账号信息到共享文件（供扩展启动时读取）
fn save_current_account_file(email: &str) -> Result<(), String> {
    use std::fs;
    use std::io::Write;

    let data_dir = get_data_dir()?;
    let file_path = data_dir.join("current_account.json");

    let content = serde_json::json!({
        "email": email,
        "updated_at": chrono::Utc::now().timestamp()
    });

    let json = serde_json::to_string_pretty(&content).map_err(|e| format!("序列化失败: {}", e))?;

    let mut file = fs::File::create(&file_path).map_err(|e| format!("创建文件失败: {}", e))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入文件失败: {}", e))?;

    modules::logger::log_info("已保存当前账号");
    Ok(())
}

/// 更新账号配额
pub fn update_account_quota(account_id: &str, quota: QuotaData) -> Result<(), String> {
    let mut account = load_account(account_id)?;

    // 容错：如果新获取的 models 为空，但之前有数据，保留原来的 models
    if quota.models.is_empty() {
        if let Some(ref existing_quota) = account.quota {
            if !existing_quota.models.is_empty() {
                modules::logger::log_warn(&format!(
                    "⚠️ 新配额 models 为空，保留原有 {} 个模型数据",
                    existing_quota.models.len()
                ));
                // 只更新非 models 字段（subscription_tier, is_forbidden 等）
                let mut merged_quota = existing_quota.clone();
                merged_quota.subscription_tier = quota.subscription_tier.clone();
                merged_quota.is_forbidden = quota.is_forbidden;
                merged_quota.last_updated = quota.last_updated;
                account.update_quota(merged_quota);
                account.usage_updated_at = Some(chrono::Utc::now().timestamp());
                save_account(&account)?;
                return Ok(());
            }
        }
    }

    account.update_quota(quota);
    account.usage_updated_at = Some(chrono::Utc::now().timestamp());
    save_account(&account)?;
    if let Some(ref quota) = account.quota {
        let _ = modules::quota_cache::write_quota_cache("authorized", &account.email, quota);
    }
    Ok(())
}

/// 设备指纹信息（兼容旧 API）
#[derive(Debug, Serialize)]
pub struct DeviceProfiles {
    pub current_storage: Option<DeviceProfile>,
    pub bound_profile: Option<DeviceProfile>,
    pub history: Vec<DeviceProfileVersion>,
    pub baseline: Option<DeviceProfile>,
}

pub fn get_device_profiles(account_id: &str) -> Result<DeviceProfiles, String> {
    let storage_path = crate::modules::device::get_storage_path()?;
    let current = crate::modules::device::read_profile(&storage_path).ok();
    let account = load_account(account_id)?;

    // 获取账号绑定的指纹
    let bound = account
        .fingerprint_id
        .as_ref()
        .and_then(|fp_id| crate::modules::fingerprint::get_fingerprint(fp_id).ok())
        .map(|fp| fp.profile);

    // 获取原始指纹
    let baseline = crate::modules::fingerprint::load_fingerprint_store()
        .ok()
        .and_then(|store| store.original_baseline)
        .map(|fp| fp.profile);

    Ok(DeviceProfiles {
        current_storage: current,
        bound_profile: bound,
        history: Vec::new(), // 历史功能已移除
        baseline,
    })
}

/// 绑定设备指纹（兼容旧 API，现在会创建新指纹并绑定）
pub fn bind_device_profile(account_id: &str, mode: &str) -> Result<DeviceProfile, String> {
    let name = format!("自动生成 {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"));

    let fingerprint = match mode {
        "capture" => crate::modules::fingerprint::capture_fingerprint(name)?,
        "generate" => crate::modules::fingerprint::generate_fingerprint(name)?,
        _ => return Err("mode 只能是 capture 或 generate".to_string()),
    };

    // 绑定到账号
    let mut account = load_account(account_id)?;
    account.fingerprint_id = Some(fingerprint.id.clone());
    save_account(&account)?;

    Ok(fingerprint.profile)
}

/// 使用指定的 profile 绑定（创建新指纹并绑定）
pub fn bind_device_profile_with_profile(
    account_id: &str,
    profile: DeviceProfile,
) -> Result<DeviceProfile, String> {
    use crate::modules::fingerprint;

    let name = format!("自动生成 {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"));

    // 创建新指纹
    let mut store = fingerprint::load_fingerprint_store()?;
    let fp = fingerprint::Fingerprint::new(name, profile.clone());
    store.fingerprints.push(fp.clone());
    fingerprint::save_fingerprint_store(&store)?;

    // 绑定到账号
    let mut account = load_account(account_id)?;
    account.fingerprint_id = Some(fp.id.clone());
    save_account(&account)?;

    // 应用到系统
    if let Ok(storage_path) = crate::modules::device::get_storage_path() {
        let _ = crate::modules::device::write_profile(&storage_path, &fp.profile);
    }

    Ok(fp.profile)
}

/// 列出指纹版本（兼容旧 API）
pub fn list_device_versions(account_id: &str) -> Result<DeviceProfiles, String> {
    get_device_profiles(account_id)
}

/// 恢复指纹版本（兼容旧 API）
pub fn restore_device_version(
    _account_id: &str,
    version_id: &str,
) -> Result<DeviceProfile, String> {
    // 直接应用指定的指纹
    let fingerprint = crate::modules::fingerprint::get_fingerprint(version_id)?;
    let _ = crate::modules::fingerprint::apply_fingerprint(version_id);
    Ok(fingerprint.profile)
}

/// 删除历史指纹（兼容旧 API - 已废弃）

pub fn delete_device_version(_account_id: &str, version_id: &str) -> Result<(), String> {
    crate::modules::fingerprint::delete_fingerprint(version_id)
}

#[derive(Serialize)]
pub struct RefreshStats {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaRefreshTrigger {
    ManualBatch,
    Auto,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuotaAlertPayload {
    pub platform: String,
    pub current_account_id: String,
    pub current_email: String,
    pub threshold: i32,
    pub threshold_display: Option<String>,
    pub lowest_percentage: i32,
    pub low_models: Vec<String>,
    pub recommended_account_id: Option<String>,
    pub recommended_email: Option<String>,
    pub triggered_at: i64,
}

fn normalize_auto_switch_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

const AUTO_SWITCH_SCOPE_ANY_GROUP: &str = "any_group";
const AUTO_SWITCH_SCOPE_SELECTED_GROUPS: &str = "selected_groups";
const AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
const AUTO_SWITCH_POLICY_AVG_QUOTA_DESC_LAST_USED_ASC: &str = "avg_quota_desc_then_last_used_asc";
const AUTO_SWITCH_RULE_CURRENT_DISABLED: &str = "current_disabled";
const AUTO_SWITCH_RULE_CURRENT_QUOTA_FORBIDDEN: &str = "current_quota_forbidden";
const AUTO_SWITCH_RULE_GROUP_BELOW_THRESHOLD: &str = "group_below_threshold";

#[derive(Debug, Clone)]
struct AutoSwitchGroupDefinition {
    id: String,
    name: String,
    models: Vec<String>,
}

#[derive(Debug, Clone)]
struct AutoSwitchGroupQuota {
    id: String,
    name: String,
    percentage: i32,
}

#[derive(Debug, Clone)]
struct AutoSwitchTriggerContext {
    rule: String,
    threshold: i32,
    scope_mode: String,
    selected_group_ids: Vec<String>,
    selected_group_names: Vec<String>,
    hit_groups: Vec<modules::antigravity_switch_history::AntigravityAutoSwitchHitGroup>,
}

fn default_auto_switch_groups() -> Vec<AutoSwitchGroupDefinition> {
    vec![
        AutoSwitchGroupDefinition {
            id: "claude_45".to_string(),
            name: "Claude".to_string(),
            models: vec![
                "claude-opus-4-6-thinking".to_string(),
                "claude-opus-4-6".to_string(),
                "claude-opus-4-5-thinking".to_string(),
                "claude-sonnet-4-6".to_string(),
                "claude-sonnet-4-6-thinking".to_string(),
                "claude-sonnet-4-5".to_string(),
                "claude-sonnet-4-5-thinking".to_string(),
                "gpt-oss-120b-medium".to_string(),
                "MODEL_PLACEHOLDER_M12".to_string(),
                "MODEL_PLACEHOLDER_M26".to_string(),
                "MODEL_PLACEHOLDER_M35".to_string(),
                "MODEL_CLAUDE_4_5_SONNET".to_string(),
                "MODEL_CLAUDE_4_5_SONNET_THINKING".to_string(),
                "MODEL_OPENAI_GPT_OSS_120B_MEDIUM".to_string(),
            ],
        },
        AutoSwitchGroupDefinition {
            id: "g3_pro".to_string(),
            name: "Gemini Pro".to_string(),
            models: vec![
                "gemini-3.1-pro-high".to_string(),
                "gemini-3.1-pro-low".to_string(),
                "gemini-3-pro-high".to_string(),
                "gemini-3-pro-low".to_string(),
                "gemini-3-pro-image".to_string(),
                "MODEL_PLACEHOLDER_M7".to_string(),
                "MODEL_PLACEHOLDER_M8".to_string(),
                "MODEL_PLACEHOLDER_M9".to_string(),
                "MODEL_PLACEHOLDER_M36".to_string(),
                "MODEL_PLACEHOLDER_M37".to_string(),
            ],
        },
        AutoSwitchGroupDefinition {
            id: "g3_flash".to_string(),
            name: "Gemini Flash".to_string(),
            models: vec![
                "gemini-3-flash".to_string(),
                "gemini-3.1-flash".to_string(),
                "gemini-3-flash-image".to_string(),
                "gemini-3.1-flash-image".to_string(),
                "gemini-3-flash-lite".to_string(),
                "gemini-3.1-flash-lite".to_string(),
                "MODEL_PLACEHOLDER_M18".to_string(),
            ],
        },
    ]
}

fn normalize_auto_switch_scope_mode(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase();
    if normalized == AUTO_SWITCH_SCOPE_SELECTED_GROUPS {
        AUTO_SWITCH_SCOPE_SELECTED_GROUPS.to_string()
    } else {
        AUTO_SWITCH_SCOPE_ANY_GROUP.to_string()
    }
}

fn normalize_auto_switch_selected_group_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn normalize_auto_switch_account_scope_mode(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase();
    if normalized == AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED.to_string()
    } else {
        AUTO_SWITCH_ACCOUNT_SCOPE_ALL.to_string()
    }
}

fn normalize_auto_switch_selected_account_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn resolve_monitored_auto_switch_account_ids(
    scope_mode: &str,
    selected_account_ids: &[String],
    accounts: &[Account],
) -> HashSet<String> {
    if scope_mode != AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        return accounts.iter().map(|account| account.id.clone()).collect();
    }

    let selected = normalize_auto_switch_selected_account_ids(selected_account_ids);
    if selected.is_empty() {
        return HashSet::new();
    }

    let existing: HashSet<&str> = accounts.iter().map(|account| account.id.as_str()).collect();
    selected
        .into_iter()
        .filter(|account_id| existing.contains(account_id.as_str()))
        .collect()
}

fn resolve_monitored_auto_switch_groups(
    scope_mode: &str,
    selected_group_ids: &[String],
    groups: &[AutoSwitchGroupDefinition],
) -> Vec<AutoSwitchGroupDefinition> {
    if groups.is_empty() {
        return Vec::new();
    }

    if scope_mode != AUTO_SWITCH_SCOPE_SELECTED_GROUPS {
        return groups.to_vec();
    }

    let selected = normalize_auto_switch_selected_group_ids(selected_group_ids);
    if selected.is_empty() {
        return groups.to_vec();
    }

    let selected_set: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let resolved: Vec<AutoSwitchGroupDefinition> = groups
        .iter()
        .filter(|group| selected_set.contains(group.id.as_str()))
        .cloned()
        .collect();
    if resolved.is_empty() {
        groups.to_vec()
    } else {
        resolved
    }
}

fn normalize_model_for_group_match(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return normalized;
    }
    match normalized.as_str() {
        "gemini-3-pro-high" => "gemini-3.1-pro-high".to_string(),
        "gemini-3-pro-low" => "gemini-3.1-pro-low".to_string(),
        "claude-sonnet-4-5" => "claude-sonnet-4-6".to_string(),
        "claude-sonnet-4-5-thinking" => "claude-sonnet-4-6".to_string(),
        "claude-opus-4-5-thinking" => "claude-opus-4-6-thinking".to_string(),
        _ => normalized,
    }
}

fn model_matches_group_model(model_name: &str, group_model_id: &str) -> bool {
    let left = normalize_model_for_group_match(model_name);
    let right = normalize_model_for_group_match(group_model_id);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    left.starts_with(&(right.clone() + "-")) || right.starts_with(&(left + "-"))
}

fn collect_group_quotas(
    account: &Account,
    groups: &[AutoSwitchGroupDefinition],
) -> Vec<AutoSwitchGroupQuota> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };
    if quota.models.is_empty() {
        return Vec::new();
    }

    groups
        .iter()
        .filter_map(|group| {
            let mut sum = 0i32;
            let mut count = 0usize;
            for model in &quota.models {
                if group
                    .models
                    .iter()
                    .any(|candidate| model_matches_group_model(&model.name, candidate))
                {
                    sum += model.percentage;
                    count += 1;
                }
            }
            if count == 0 {
                return None;
            }
            let average = ((sum as f64) / (count as f64)).round() as i32;
            Some(AutoSwitchGroupQuota {
                id: group.id.clone(),
                name: group.name.clone(),
                percentage: average,
            })
        })
        .collect()
}

fn evaluate_auto_switch_trigger(
    account: &Account,
    threshold: i32,
    scope_mode: &str,
    monitored_groups: &[AutoSwitchGroupDefinition],
) -> Option<AutoSwitchTriggerContext> {
    if monitored_groups.is_empty() {
        return None;
    }

    let selected_group_ids: Vec<String> = monitored_groups
        .iter()
        .map(|group| group.id.clone())
        .collect();
    let selected_group_names: Vec<String> = monitored_groups
        .iter()
        .map(|group| group.name.clone())
        .collect();

    if account.disabled {
        return Some(AutoSwitchTriggerContext {
            rule: AUTO_SWITCH_RULE_CURRENT_DISABLED.to_string(),
            threshold,
            scope_mode: scope_mode.to_string(),
            selected_group_ids,
            selected_group_names,
            hit_groups: Vec::new(),
        });
    }

    let Some(quota) = account.quota.as_ref() else {
        return None;
    };

    if quota.is_forbidden {
        return Some(AutoSwitchTriggerContext {
            rule: AUTO_SWITCH_RULE_CURRENT_QUOTA_FORBIDDEN.to_string(),
            threshold,
            scope_mode: scope_mode.to_string(),
            selected_group_ids,
            selected_group_names,
            hit_groups: Vec::new(),
        });
    }

    let group_quotas = collect_group_quotas(account, monitored_groups);
    let hit_groups: Vec<modules::antigravity_switch_history::AntigravityAutoSwitchHitGroup> =
        group_quotas
            .into_iter()
            .filter(|group| group.percentage <= threshold)
            .map(
                |group| modules::antigravity_switch_history::AntigravityAutoSwitchHitGroup {
                    group_id: group.id,
                    group_name: group.name,
                    percentage: group.percentage,
                },
            )
            .collect();
    if hit_groups.is_empty() {
        return None;
    }

    Some(AutoSwitchTriggerContext {
        rule: AUTO_SWITCH_RULE_GROUP_BELOW_THRESHOLD.to_string(),
        threshold,
        scope_mode: scope_mode.to_string(),
        selected_group_ids,
        selected_group_names,
        hit_groups,
    })
}

fn can_be_auto_switch_candidate(
    account: &Account,
    current_id: &str,
    threshold: i32,
    monitored_groups: &[AutoSwitchGroupDefinition],
) -> bool {
    if account.id == current_id || account.disabled {
        return false;
    }

    let Some(quota) = account.quota.as_ref() else {
        return false;
    };

    if quota.is_forbidden || quota.models.is_empty() {
        return false;
    }

    let group_quotas = collect_group_quotas(account, monitored_groups);
    if group_quotas.len() < monitored_groups.len() {
        return false;
    }
    group_quotas
        .iter()
        .all(|group| group.percentage >= threshold)
}

fn can_be_quota_alert_candidate(account: &Account, current_id: &str) -> bool {
    if account.id == current_id || account.disabled {
        return false;
    }

    let Some(quota) = account.quota.as_ref() else {
        return false;
    };

    if quota.is_forbidden || quota.models.is_empty() {
        return false;
    }

    true
}

fn average_quota_percentage(account: &Account) -> f64 {
    let Some(quota) = account.quota.as_ref() else {
        return 0.0;
    };
    if quota.models.is_empty() {
        return 0.0;
    }
    let sum: i32 = quota.models.iter().map(|m| m.percentage).sum();
    sum as f64 / quota.models.len() as f64
}

fn build_auto_switch_reason(
    context: &AutoSwitchTriggerContext,
    candidate_count: usize,
) -> modules::antigravity_switch_history::AntigravityAutoSwitchReason {
    modules::antigravity_switch_history::AntigravityAutoSwitchReason {
        rule: context.rule.clone(),
        threshold: context.threshold,
        scope_mode: context.scope_mode.clone(),
        selected_group_ids: context.selected_group_ids.clone(),
        selected_group_names: context.selected_group_names.clone(),
        hit_groups: context.hit_groups.clone(),
        candidate_count,
        selected_policy: AUTO_SWITCH_POLICY_AVG_QUOTA_DESC_LAST_USED_ASC.to_string(),
    }
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

pub(crate) fn pick_quota_alert_recommendation(
    accounts: &[Account],
    current_id: &str,
) -> Option<Account> {
    let mut candidates: Vec<Account> = accounts
        .iter()
        .filter(|a| can_be_quota_alert_candidate(a, current_id))
        .cloned()
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let avg_a = average_quota_percentage(a);
        let avg_b = average_quota_percentage(b);
        avg_b
            .partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.last_used.cmp(&b.last_used))
    });

    candidates.into_iter().next()
}

fn build_quota_alert_notification_text(payload: &QuotaAlertPayload) -> (String, String) {
    let locale = crate::modules::config::get_user_config().language;
    let threshold_text = payload.threshold.to_string();
    let lowest_text = payload.lowest_percentage.to_string();
    let model_text = if payload.low_models.is_empty() {
        modules::i18n::translate(&locale, "quotaAlert.modal.unknownModel", &[])
    } else {
        payload.low_models.join(", ")
    };

    let platform_label = match payload.platform.as_str() {
        "codex" => "Codex",
        "github_copilot" => "GitHub Copilot",
        "windsurf" => "Windsurf",
        "kiro" => "Kiro",
        "cursor" => "Cursor",
        "gemini" => "Gemini Cli",
        "codebuddy" => "CodeBuddy",
        "zed" => "Zed",
        _ => "Antigravity",
    };
    let title = format!(
        "{} {}",
        platform_label,
        modules::i18n::translate(&locale, "quotaAlert.modal.title", &[])
    );
    let mut body = modules::i18n::translate(
        &locale,
        "quotaAlert.bannerText",
        &[
            ("email", payload.current_email.as_str()),
            ("threshold", threshold_text.as_str()),
            ("lowest", lowest_text.as_str()),
            ("models", model_text.as_str()),
        ],
    );
    if let Some(email) = payload.recommended_email.as_ref() {
        let recommended_label =
            modules::i18n::translate(&locale, "quotaAlert.modal.recommended", &[]);
        body.push_str(" · ");
        body.push_str(&format!("{}: {}", recommended_label, email));
    }
    (title, body)
}

pub fn emit_quota_alert(app_handle: &tauri::AppHandle, payload: &QuotaAlertPayload) {
    use tauri::Emitter;
    let _ = app_handle.emit("quota:alert", payload);
}

#[cfg(not(target_os = "macos"))]
pub fn send_quota_alert_native_notification(payload: &QuotaAlertPayload) {
    let Some(app_handle) = crate::get_app_handle() else {
        return;
    };

    use tauri_plugin_notification::NotificationExt;

    let (title, body) = build_quota_alert_notification_text(payload);

    if let Err(e) = app_handle
        .notification()
        .builder()
        .title(&title)
        .body(body)
        .show()
    {
        modules::logger::log_warn(&format!("[QuotaAlert] 原生通知发送失败: {}", e));
    }
}

#[cfg(target_os = "macos")]
pub fn send_quota_alert_native_notification(payload: &QuotaAlertPayload) {
    let Some(app_handle) = crate::get_app_handle() else {
        return;
    };
    let bundle_identifier = app_handle.config().identifier.to_string();
    let (title, body) = build_quota_alert_notification_text(payload);

    std::thread::spawn(move || {
        let mut notification = mac_notification_sys::Notification::new();
        // Fire-and-forget on macOS. Waiting for clicks keeps a dedicated run loop alive
        // inside mac-notification-sys, which can cause persistent background energy usage.
        notification
            .title(title.as_str())
            .message(body.as_str())
            .wait_for_click(false)
            .asynchronous(true);

        if let Err(e) = mac_notification_sys::set_application(&bundle_identifier) {
            modules::logger::log_warn(&format!("[QuotaAlert] 设置通知应用标识失败: {}", e));
        }

        if let Err(e) = notification.send() {
            modules::logger::log_warn(&format!("[QuotaAlert] 原生通知发送失败: {}", e));
        }
    });
}

pub fn dispatch_quota_alert(payload: &QuotaAlertPayload) {
    modules::logger::log_warn(&format!(
        "[QuotaAlert] 触发配额预警: platform={}, current_id={}, threshold={}%, lowest={}%",
        payload.platform, payload.current_account_id, payload.threshold, payload.lowest_percentage
    ));

    if let Some(app_handle) = crate::get_app_handle() {
        emit_quota_alert(app_handle, payload);
    }
    send_quota_alert_native_notification(payload);
}

pub fn run_quota_alert_if_needed() -> Result<Option<QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.quota_alert_enabled {
        return Ok(None);
    }

    let threshold = normalize_quota_alert_threshold(cfg.quota_alert_threshold);
    let current_id = match get_current_account_id()? {
        Some(id) => id,
        None => return Ok(None),
    };

    let accounts = list_accounts()?;
    let current = match accounts.iter().find(|a| a.id == current_id) {
        Some(acc) => acc,
        None => return Ok(None),
    };

    if current.disabled {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

    let Some(quota) = current.quota.as_ref() else {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    };

    let low_models: Vec<(String, i32)> = if quota.is_forbidden {
        vec![("all".to_string(), 0)]
    } else {
        quota
            .models
            .iter()
            .filter(|model| model.percentage <= threshold)
            .map(|model| (model.name.clone(), model.percentage))
            .collect()
    };

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key = build_quota_alert_cooldown_key(&current_id, threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(&accounts, &current_id);
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = QuotaAlertPayload {
        platform: "antigravity".to_string(),
        current_account_id: current_id.clone(),
        current_email: current.email.clone(),
        threshold,
        threshold_display: None,
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|acc| acc.id.clone()),
        recommended_email: recommendation.as_ref().map(|acc| acc.email.clone()),
        triggered_at: now,
    };
    dispatch_quota_alert(&payload);
    Ok(Some(payload))
}

async fn run_auto_switch_if_needed_inner() -> Result<Option<Account>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.auto_switch_enabled {
        return Ok(None);
    }

    let threshold = normalize_auto_switch_threshold(cfg.auto_switch_threshold);
    let scope_mode = normalize_auto_switch_scope_mode(&cfg.auto_switch_scope_mode);
    let account_scope_mode =
        normalize_auto_switch_account_scope_mode(&cfg.auto_switch_account_scope_mode);
    let all_groups = default_auto_switch_groups();
    let monitored_groups = resolve_monitored_auto_switch_groups(
        &scope_mode,
        &cfg.auto_switch_selected_group_ids,
        &all_groups,
    );
    if monitored_groups.is_empty() {
        modules::logger::log_warn("[AutoSwitch] 可监控模型分组为空，跳过自动切号");
        return Ok(None);
    }
    let current_id = match get_current_account_id()? {
        Some(id) => id,
        None => return Ok(None),
    };

    let accounts = list_accounts()?;
    let monitored_account_ids = resolve_monitored_auto_switch_account_ids(
        &account_scope_mode,
        &cfg.auto_switch_selected_account_ids,
        &accounts,
    );
    if monitored_account_ids.is_empty() {
        modules::logger::log_warn(&format!(
            "[AutoSwitch] 可监控账号范围为空(scope={})，跳过自动切号",
            account_scope_mode
        ));
        return Ok(None);
    }
    if !monitored_account_ids.contains(&current_id) {
        modules::logger::log_info(&format!(
            "[AutoSwitch] 当前账号不在监控范围内(current_id={}, scope={})，跳过自动切号",
            current_id, account_scope_mode
        ));
        return Ok(None);
    }

    let current = match accounts.iter().find(|a| a.id == current_id) {
        Some(acc) => acc,
        None => return Ok(None),
    };

    let Some(trigger_context) =
        evaluate_auto_switch_trigger(current, threshold, &scope_mode, &monitored_groups)
    else {
        return Ok(None);
    };

    let mut candidates: Vec<Account> = accounts
        .into_iter()
        .filter(|a| monitored_account_ids.contains(&a.id))
        .filter(|a| can_be_auto_switch_candidate(a, &current_id, threshold, &monitored_groups))
        .collect();

    if candidates.is_empty() {
        modules::logger::log_warn(&format!(
            "[AutoSwitch] 命中自动切号条件(rule={}, threshold={}%, scope={})，但没有可切换候选账号",
            trigger_context.rule, threshold, trigger_context.scope_mode
        ));
        return Ok(None);
    }

    let reason_snapshot = build_auto_switch_reason(&trigger_context, candidates.len());

    candidates.sort_by(|a, b| {
        let avg_a = average_quota_percentage(a);
        let avg_b = average_quota_percentage(b);
        avg_b
            .partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.last_used.cmp(&b.last_used))
    });

    let target = &candidates[0];
    modules::logger::log_info(&format!(
        "[AutoSwitch] 触发自动切号: current_id={}, target_id={}, threshold={}%, scope={}, rule={}",
        current_id, target.id, threshold, scope_mode, trigger_context.rule
    ));

    let switched = if cfg.antigravity_dual_switch_no_restart_enabled {
        switch_account_dual_no_restart(
            &target.id,
            "auto",
            "tools.account.auto_switch",
            "auto_switch",
            Some(reason_snapshot),
        )
        .await?
    } else {
        let switched = switch_account_internal(&target.id).await?;
        modules::websocket::broadcast_account_switched(&switched.id, &switched.email);
        switched
    };
    modules::websocket::broadcast_data_changed("auto_switch");
    Ok(Some(switched))
}

pub async fn run_auto_switch_if_needed() -> Result<Option<Account>, String> {
    if AUTO_SWITCH_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        modules::logger::log_info("[AutoSwitch] 自动切号进行中，跳过本次检查");
        return Ok(None);
    }

    let result = run_auto_switch_if_needed_inner().await;
    AUTO_SWITCH_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

/// 批量刷新所有账号配额
pub async fn refresh_all_quotas_logic(
    trigger: QuotaRefreshTrigger,
) -> Result<RefreshStats, String> {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let start = std::time::Instant::now();
    let trigger_label = match trigger {
        QuotaRefreshTrigger::ManualBatch => "manual_batch",
        QuotaRefreshTrigger::Auto => "auto",
    };

    modules::logger::log_info(&format!(
        "开始批量刷新所有账号配额 (trigger={}, 并发模式, 最大并发: {})",
        trigger_label, MAX_CONCURRENT
    ));
    let accounts = list_accounts()?;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));

    let tasks: Vec<_> = accounts
        .into_iter()
        .filter(|account| {
            if trigger == QuotaRefreshTrigger::Auto {
                if account.disabled {
                    modules::logger::log_info("  - Skipping Disabled account (auto)");
                    return false;
                }
                if let Some(ref q) = account.quota {
                    if q.is_forbidden {
                        modules::logger::log_info("  - Skipping Forbidden account (auto)");
                        return false;
                    }
                }
            }
            true
        })
        .map(|mut account| {
            let email = account.email.clone();
            let account_id = account.id.clone();
            let permit = semaphore.clone();
            async move {
                let _guard = permit.acquire().await.unwrap();
                match fetch_quota_with_delayed_retry(&mut account, false).await {
                    Ok(quota) => {
                        if let Err(e) = update_account_quota(&account_id, quota) {
                            let msg = format!("Account {}: Save quota failed - {}", email, e);
                            Err(msg)
                        } else {
                            Ok(())
                        }
                    }
                    Err(e) => {
                        let msg = format!("Account {}: Fetch quota failed - {}", email, e);
                        Err(msg)
                    }
                }
            }
        })
        .collect();

    let total = tasks.len();
    let results = join_all(tasks).await;

    let mut success = 0;
    let mut failed = 0;
    let mut details = Vec::new();

    for result in results {
        match result {
            Ok(()) => success += 1,
            Err(msg) => {
                failed += 1;
                details.push(msg);
            }
        }
    }

    let elapsed = start.elapsed();
    modules::logger::log_info(&format!(
        "批量刷新完成: {} 成功, {} 失败, 耗时: {}ms",
        success,
        failed,
        elapsed.as_millis()
    ));

    Ok(RefreshStats {
        total,
        success,
        failed,
        details,
    })
}

pub async fn fetch_quota_with_delayed_retry(
    account: &mut Account,
    skip_cache: bool,
) -> crate::error::AppResult<QuotaData> {
    match fetch_quota_with_retry(account, skip_cache).await {
        Ok(quota) => Ok(quota),
        Err(first_error) => {
            modules::logger::log_warn(&format!(
                "[Antigravity Refresh] 首次刷新失败，{} 秒后重试: account_id={}, email={}, error={}",
                crate::modules::refresh_retry::ACCOUNT_REFRESH_RETRY_DELAY_SECS,
                account.id,
                account.email,
                first_error
            ));

            tokio::time::sleep(Duration::from_secs(
                crate::modules::refresh_retry::ACCOUNT_REFRESH_RETRY_DELAY_SECS,
            ))
            .await;

            match fetch_quota_with_retry(account, skip_cache).await {
                Ok(quota) => {
                    modules::logger::log_info(&format!(
                        "[Antigravity Refresh] 重试刷新成功: account_id={}, email={}",
                        account.id, account.email
                    ));
                    Ok(quota)
                }
                Err(second_error) => {
                    modules::logger::log_warn(&format!(
                        "[Antigravity Refresh] 重试后仍失败: account_id={}, email={}, first_error={}, second_error={}",
                        account.id,
                        account.email,
                        first_error,
                        second_error
                    ));
                    Err(second_error)
                }
            }
        }
    }
}

/// 带重试的配额查询
/// skip_cache: 是否跳过缓存，单个账号刷新应传 true
pub async fn fetch_quota_with_retry(
    account: &mut Account,
    skip_cache: bool,
) -> crate::error::AppResult<QuotaData> {
    use crate::error::AppError;
    use crate::modules::oauth;

    let token = match oauth::ensure_fresh_token(&account.token).await {
        Ok(t) => t,
        Err(e) => {
            if e.contains("invalid_grant") {
                account.disabled = true;
                account.disabled_at = Some(chrono::Utc::now().timestamp());
                account.disabled_reason = Some(format!("invalid_grant: {}", e));
                let _ = save_account(account);
            }
            account.quota_error = Some(QuotaErrorInfo {
                code: None,
                message: format!("OAuth error: {}", e),
                timestamp: chrono::Utc::now().timestamp(),
            });
            let _ = save_account(account);
            return Err(AppError::OAuth(e));
        }
    };

    if token.access_token != account.token.access_token {
        account.token = token.clone();
        let _ = upsert_account(account.email.clone(), account.name.clone(), token.clone());
    }

    let result =
        modules::quota::fetch_quota_for_token(&account.token, &account.email, skip_cache).await;
    match result {
        Ok(payload) => {
            // 配额获取成功，Token 有效
            // 只解除 invalid_grant 类禁用，其他（verification_required / tos_violation / unknown）不解除
            if account.is_invalid_grant_disabled() {
                modules::logger::log_info(&format!(
                    "账号配额获取成功，自动解除禁用状态: {}",
                    account.email
                ));
                account.clear_disabled();
            }
            account.quota_error = payload.error.map(|err| QuotaErrorInfo {
                code: err.code,
                message: err.message,
                timestamp: chrono::Utc::now().timestamp(),
            });
            let _ = save_account(account);
            Ok(payload.quota)
        }
        Err(err) => {
            account.quota_error = Some(QuotaErrorInfo {
                code: None,
                message: err.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
            });
            let _ = save_account(account);
            Err(err)
        }
    }
}

/// 内部切换账号函数（供 WebSocket 调用）
/// 完整流程：Token刷新 + 关闭程序 + 注入 + 指纹同步 + 重启
pub async fn switch_account_internal(account_id: &str) -> Result<Account, String> {
    modules::logger::log_info("[Switch] 开始切换账号");

    // 路径缺失时不执行关闭/注入，避免破坏当前运行态。
    modules::process::ensure_antigravity_launch_path_configured()?;

    // 1. 加载并验证账号存在
    let mut account = prepare_account_for_injection(account_id).await?;
    modules::logger::log_info("[Switch] 正在切换到账号");

    // 3. 写入设备指纹到 storage.json
    if let Ok(storage_path) = modules::device::get_storage_path() {
        if let Some(ref fp_id) = account.fingerprint_id {
            // 优先使用绑定的指纹
            if let Ok(fingerprint) = modules::fingerprint::get_fingerprint(fp_id) {
                modules::logger::log_info("[Switch] 写入设备指纹");
                let _ = modules::device::write_profile(&storage_path, &fingerprint.profile);
                let _ =
                    modules::db::write_service_machine_id(&fingerprint.profile.service_machine_id);
            }
        }
    }

    // 4. 更新工具内部状态
    set_current_account_id(account_id)?;
    account.update_last_used();
    save_account(&account)?;

    // 5. 同步更新默认实例绑定账号，确保默认实例注入目标明确
    if let Err(e) = modules::instance::update_default_settings(
        Some(Some(account_id.to_string())),
        None,
        Some(false),
    ) {
        modules::logger::log_warn(&format!("[Switch] 更新默认实例绑定账号失败: {}", e));
    }

    // 6. 对齐默认实例启动逻辑：按默认实例目录关闭受管进程，再注入默认实例目录
    let default_dir = modules::instance::get_default_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();
    modules::process::close_antigravity_instances(&[default_dir_str], 20)?;
    let _ = modules::instance::update_default_pid(None);
    modules::instance::inject_account_to_profile(&default_dir, account_id)?;

    // 7. 启动 Antigravity（带默认实例自定义启动参数；启动失败不阻断切号，保持原行为）
    modules::logger::log_info("[Switch] 正在启动 Antigravity 默认实例...");
    let default_settings = modules::instance::load_default_settings()?;
    let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
    let launch_result = if extra_args.is_empty() {
        modules::process::start_antigravity()
    } else {
        modules::process::start_antigravity_with_args("", &extra_args)
    };
    match launch_result {
        Ok(pid) => {
            let _ = modules::instance::update_default_pid(Some(pid));
        }
        Err(e) => {
            modules::logger::log_warn(&format!("[Switch] Antigravity 启动失败: {}", e));
            // 不中断流程，允许用户手动启动
        }
    }

    modules::logger::log_info("[Switch] 账号切换完成");
    Ok(account)
}

fn persist_switch_history(item: modules::antigravity_switch_history::AntigravitySwitchHistoryItem) {
    if let Err(err) = modules::antigravity_switch_history::add_history_item(item) {
        modules::logger::log_warn(&format!("写入 Antigravity 切号记录失败: {}", err));
    }
}

fn ensure_antigravity_running_for_no_restart_switch() -> Result<bool, String> {
    let default_settings = modules::instance::load_default_settings()?;
    let resolved_pid = modules::process::resolve_antigravity_pid(default_settings.last_pid, None);
    if let Some(pid) = resolved_pid {
        if default_settings.last_pid != Some(pid) {
            let _ = modules::instance::update_default_pid(Some(pid));
        }
        modules::logger::log_info(&format!(
            "[Switch][NoRestart] 检测到 Antigravity 已运行: pid={}",
            pid
        ));
        return Ok(false);
    }

    modules::logger::log_info("[Switch][NoRestart] Antigravity 未运行，尝试自动启动");
    let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
    let launch_result = if extra_args.is_empty() {
        modules::process::start_antigravity()
    } else {
        modules::process::start_antigravity_with_args("", &extra_args)
    };
    match launch_result {
        Ok(pid) => {
            if let Err(err) = modules::instance::update_default_pid(Some(pid)) {
                modules::logger::log_warn(&format!(
                    "[Switch][NoRestart] 更新默认实例 PID 失败: {}",
                    err
                ));
            }
            modules::logger::log_info(&format!(
                "[Switch][NoRestart] Antigravity 自动启动成功: pid={}",
                pid
            ));
            Ok(true)
        }
        Err(err) => Err(err),
    }
}

pub async fn switch_account_dual_no_restart(
    account_id: &str,
    trigger_type: &str,
    trigger_source: &str,
    reason: &str,
    auto_switch_reason: Option<modules::antigravity_switch_history::AntigravityAutoSwitchReason>,
) -> Result<Account, String> {
    let started = Instant::now();
    let history_id = Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let trigger_type = if trigger_type.trim().is_empty() {
        "manual".to_string()
    } else {
        trigger_type.trim().to_string()
    };
    let trigger_source = if trigger_source.trim().is_empty() {
        "tools.account.switch".to_string()
    } else {
        trigger_source.trim().to_string()
    };
    let target_email_fallback = load_account(account_id)
        .map(|account| account.email)
        .unwrap_or_else(|_| account_id.to_string());

    let local_started = Instant::now();
    let local_result = switch_account_local_no_restart(account_id).await;
    let local_duration_ms = local_started.elapsed().as_millis() as u64;

    let account = match local_result {
        Ok(account) => account,
        Err(error) => {
            persist_switch_history(
                modules::antigravity_switch_history::AntigravitySwitchHistoryItem {
                    id: history_id,
                    timestamp,
                    account_id: account_id.to_string(),
                    target_email: target_email_fallback,
                    trigger_type: trigger_type.clone(),
                    trigger_source: trigger_source.clone(),
                    local_ok: false,
                    seamless_ok: false,
                    success: false,
                    local_duration_ms,
                    seamless_duration_ms: None,
                    total_duration_ms: started.elapsed().as_millis() as u64,
                    error_stage: Some("local".to_string()),
                    error_code: None,
                    error_message: Some(error.clone()),
                    seamless_effective_mode: None,
                    seamless_from_email: None,
                    seamless_to_email: None,
                    seamless_execution_id: None,
                    seamless_finished_at: None,
                    auto_switch_reason: auto_switch_reason.clone(),
                },
            );
            return Err(error);
        }
    };

    let was_started = match ensure_antigravity_running_for_no_restart_switch() {
        Ok(started_now) => started_now,
        Err(error) => {
            modules::websocket::broadcast_account_switched(&account.id, &account.email);
            persist_switch_history(
                modules::antigravity_switch_history::AntigravitySwitchHistoryItem {
                    id: history_id,
                    timestamp,
                    account_id: account.id.clone(),
                    target_email: account.email.clone(),
                    trigger_type: trigger_type.clone(),
                    trigger_source: trigger_source.clone(),
                    local_ok: true,
                    seamless_ok: false,
                    success: false,
                    local_duration_ms,
                    seamless_duration_ms: None,
                    total_duration_ms: started.elapsed().as_millis() as u64,
                    error_stage: Some("client_start".to_string()),
                    error_code: None,
                    error_message: Some(error.clone()),
                    seamless_effective_mode: None,
                    seamless_from_email: None,
                    seamless_to_email: None,
                    seamless_execution_id: None,
                    seamless_finished_at: None,
                    auto_switch_reason: auto_switch_reason.clone(),
                },
            );
            if error.starts_with("APP_PATH_NOT_FOUND:") {
                return Err(error);
            }
            return Err(format!("本地切换已完成，但客户端启动失败: {}", error));
        }
    };

    let wait_timeout_ms = if was_started { 12_000 } else { 1_500 };
    if let Err(wait_error) = modules::websocket::wait_for_connected_clients(wait_timeout_ms).await {
        modules::logger::log_warn(&format!(
            "[Switch][NoRestart] 扩展连接等待结束: {}",
            wait_error
        ));
    }

    let seamless_started = Instant::now();
    let seamless_result = modules::websocket::request_plugin_switch_account(
        &account.email,
        "seamless",
        &trigger_type,
        &trigger_source,
        reason,
        12_000,
    )
    .await;
    let seamless_duration_ms = seamless_started.elapsed().as_millis() as u64;

    match seamless_result {
        Ok(response) if response.success => {
            modules::websocket::broadcast_account_switched(&account.id, &account.email);
            persist_switch_history(
                modules::antigravity_switch_history::AntigravitySwitchHistoryItem {
                    id: history_id,
                    timestamp,
                    account_id: account.id.clone(),
                    target_email: account.email.clone(),
                    trigger_type: trigger_type.clone(),
                    trigger_source: trigger_source.clone(),
                    local_ok: true,
                    seamless_ok: true,
                    success: true,
                    local_duration_ms,
                    seamless_duration_ms: Some(seamless_duration_ms),
                    total_duration_ms: started.elapsed().as_millis() as u64,
                    error_stage: None,
                    error_code: None,
                    error_message: None,
                    seamless_effective_mode: Some(response.effective_mode),
                    seamless_from_email: response.from_email,
                    seamless_to_email: Some(response.to_email),
                    seamless_execution_id: Some(response.execution_id),
                    seamless_finished_at: Some(response.finished_at),
                    auto_switch_reason: auto_switch_reason.clone(),
                },
            );
            Ok(account)
        }
        Ok(response) => {
            modules::websocket::broadcast_account_switched(&account.id, &account.email);
            persist_switch_history(
                modules::antigravity_switch_history::AntigravitySwitchHistoryItem {
                    id: history_id,
                    timestamp,
                    account_id: account.id.clone(),
                    target_email: account.email.clone(),
                    trigger_type: trigger_type.clone(),
                    trigger_source: trigger_source.clone(),
                    local_ok: true,
                    seamless_ok: false,
                    success: false,
                    local_duration_ms,
                    seamless_duration_ms: Some(seamless_duration_ms),
                    total_duration_ms: started.elapsed().as_millis() as u64,
                    error_stage: Some("seamless".to_string()),
                    error_code: response.error_code.clone(),
                    error_message: response.error_message.clone(),
                    seamless_effective_mode: Some(response.effective_mode),
                    seamless_from_email: response.from_email,
                    seamless_to_email: Some(response.to_email),
                    seamless_execution_id: Some(response.execution_id),
                    seamless_finished_at: Some(response.finished_at),
                    auto_switch_reason: auto_switch_reason.clone(),
                },
            );
            Err(format!(
                "本地切换已完成，但扩展无感切号失败: {}",
                response
                    .error_message
                    .unwrap_or_else(|| "扩展未返回具体原因".to_string())
            ))
        }
        Err(error) => {
            modules::websocket::broadcast_account_switched(&account.id, &account.email);
            persist_switch_history(
                modules::antigravity_switch_history::AntigravitySwitchHistoryItem {
                    id: history_id,
                    timestamp,
                    account_id: account.id.clone(),
                    target_email: account.email.clone(),
                    trigger_type,
                    trigger_source,
                    local_ok: true,
                    seamless_ok: false,
                    success: false,
                    local_duration_ms,
                    seamless_duration_ms: Some(seamless_duration_ms),
                    total_duration_ms: started.elapsed().as_millis() as u64,
                    error_stage: Some("seamless".to_string()),
                    error_code: None,
                    error_message: Some(error.clone()),
                    seamless_effective_mode: None,
                    seamless_from_email: None,
                    seamless_to_email: None,
                    seamless_execution_id: None,
                    seamless_finished_at: None,
                    auto_switch_reason,
                },
            );
            Err(format!("本地切换已完成，但扩展无感切号失败: {}", error))
        }
    }
}

fn apply_bound_fingerprint_for_switch(account: &Account) {
    if let Ok(storage_path) = modules::device::get_storage_path() {
        if let Some(ref fp_id) = account.fingerprint_id {
            if let Ok(fingerprint) = modules::fingerprint::get_fingerprint(fp_id) {
                modules::logger::log_info("[Switch] 写入设备指纹");
                let _ = modules::device::write_profile(&storage_path, &fingerprint.profile);
                let _ =
                    modules::db::write_service_machine_id(&fingerprint.profile.service_machine_id);
                let _ = modules::fingerprint::set_current_fingerprint_id(fp_id);
            }
        }
    }
}

/// 本地切号（不关闭/不重启 Antigravity）
/// 流程：Token刷新 + 本地状态更新 + 指纹同步 + 默认实例注入
pub async fn switch_account_local_no_restart(account_id: &str) -> Result<Account, String> {
    modules::logger::log_info(&format!(
        "[Switch][NoRestart] 开始本地切号: account_id={}",
        account_id
    ));

    let mut account = prepare_account_for_injection(account_id).await?;
    account.update_last_used();
    save_account(&account)?;

    set_current_account_id(account_id)?;

    if let Err(e) = modules::instance::update_default_settings(
        Some(Some(account_id.to_string())),
        None,
        Some(false),
    ) {
        modules::logger::log_warn(&format!(
            "[Switch][NoRestart] 更新默认实例绑定账号失败: {}",
            e
        ));
    }

    apply_bound_fingerprint_for_switch(&account);

    let default_dir = modules::instance::get_default_user_data_dir()?;
    modules::instance::inject_account_to_profile(&default_dir, account_id)?;

    modules::logger::log_info(&format!(
        "[Switch][NoRestart] 本地切号完成: {}",
        account.email
    ));
    Ok(account)
}

/// 准备账号注入：确保 Token 新鲜并落盘
pub async fn prepare_account_for_injection(account_id: &str) -> Result<Account, String> {
    let mut account = load_account(account_id)?;
    let fresh_token = modules::oauth::ensure_fresh_token(&account.token)
        .await
        .map_err(|e| format!("Token 刷新失败: {}", e))?;
    if fresh_token.access_token != account.token.access_token {
        modules::logger::log_info("[Account] Token 已刷新");
        account.token = fresh_token.clone();
        save_account(&account)?;
    }
    Ok(account)
}
