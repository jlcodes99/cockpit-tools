use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::models::DeviceProfile;
use crate::modules::{device, logger};

const FINGERPRINTS_FILE: &str = "fingerprints.json";

/// 指纹存储结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintStore {
    /// 原始指纹（只读，不可删除）
    pub original_baseline: Option<Fingerprint>,
    /// 当前应用到系统的指纹ID
    pub current_fingerprint_id: Option<String>,
    /// 用户创建的指纹列表
    pub fingerprints: Vec<Fingerprint>,
}

impl FingerprintStore {
    pub fn new() -> Self {
        Self {
            original_baseline: None,
            current_fingerprint_id: None,
            fingerprints: Vec::new(),
        }
    }
}

/// 单个指纹
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fingerprint {
    pub id: String,
    pub name: String,
    pub profile: DeviceProfile,
    pub created_at: i64,
}

impl Fingerprint {
    pub fn new(name: String, profile: DeviceProfile) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            profile,
            created_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// 获取指纹存储文件路径
fn get_fingerprints_path() -> Result<PathBuf, String> {
    let data_dir = crate::modules::account::get_data_dir()?;
    Ok(data_dir.join(FINGERPRINTS_FILE))
}

/// 加载指纹存储
pub fn load_fingerprint_store() -> Result<FingerprintStore, String> {
    let path = get_fingerprints_path()?;

    if !path.exists() {
        // 首次使用，尝试捕获原始指纹
        let mut store = FingerprintStore::new();
        if let Ok(storage_path) = device::get_storage_path() {
            if let Ok(profile) = device::read_profile(&storage_path) {
                let baseline = Fingerprint {
                    id: "original".to_string(),
                    name: "原始指纹".to_string(),
                    profile,
                    created_at: chrono::Utc::now().timestamp(),
                };
                store.original_baseline = Some(baseline.clone());
                store.current_fingerprint_id = Some("original".to_string());
                logger::log_info("已捕获原始设备指纹");
            }
        }
        save_fingerprint_store(&store)?;
        return Ok(store);
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("读取指纹存储失败: {}", e))?;

    if content.trim().is_empty() {
        return Ok(FingerprintStore::new());
    }

    match serde_json::from_str(&content) {
        Ok(store) => Ok(store),
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_warn(&format!(
                    "指纹存储解析失败，已隔离并使用空指纹列表: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_warn(&format!(
                    "指纹存储解析失败，文件已不存在，使用空指纹列表: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_warn(&format!(
                    "指纹存储解析失败，隔离失败，使用空指纹列表: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(FingerprintStore::new())
        }
    }
}

/// 保存指纹存储
pub fn save_fingerprint_store(store: &FingerprintStore) -> Result<(), String> {
    let path = get_fingerprints_path()?;
    let content =
        serde_json::to_string_pretty(store).map_err(|e| format!("序列化指纹存储失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存指纹存储失败: {}", e))
}

/// 获取指纹详情
pub fn get_fingerprint(fingerprint_id: &str) -> Result<Fingerprint, String> {
    let store = load_fingerprint_store()?;

    if fingerprint_id == "original" {
        return store.original_baseline.ok_or("原始指纹不存在".to_string());
    }

    store
        .fingerprints
        .iter()
        .find(|f| f.id == fingerprint_id)
        .cloned()
        .ok_or(format!("指纹不存在: {}", fingerprint_id))
}

/// 获取当前应用的指纹ID
pub fn get_current_fingerprint_id() -> Result<Option<String>, String> {
    let store = load_fingerprint_store()?;
    Ok(store.current_fingerprint_id)
}

/// 设置当前应用的指纹ID
pub fn set_current_fingerprint_id(fingerprint_id: &str) -> Result<(), String> {
    let mut store = load_fingerprint_store()?;
    store.current_fingerprint_id = Some(fingerprint_id.to_string());
    save_fingerprint_store(&store)
}

/// 生成新指纹
pub fn generate_fingerprint(name: String) -> Result<Fingerprint, String> {
    let profile = device::generate_profile();
    create_fingerprint_with_profile(name, profile)
}

/// 捕获当前系统指纹
pub fn capture_fingerprint(name: String) -> Result<Fingerprint, String> {
    let storage_path = device::get_storage_path()?;
    let profile = device::read_profile(&storage_path)?;
    create_fingerprint_with_profile(name, profile)
}

/// 使用指定 profile 创建指纹
pub fn create_fingerprint_with_profile(
    name: String,
    mut profile: DeviceProfile,
) -> Result<Fingerprint, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("指纹名称不能为空".to_string());
    }

    device::ensure_service_machine_id(&mut profile);
    let fingerprint = Fingerprint::new(trimmed.to_string(), profile);

    let mut store = load_fingerprint_store()?;
    store.fingerprints.push(fingerprint.clone());
    save_fingerprint_store(&store)?;

    logger::log_info(&format!("已保存指纹: {}", fingerprint.name));
    Ok(fingerprint)
}

/// 应用指纹到系统
pub fn apply_fingerprint(fingerprint_id: &str) -> Result<String, String> {
    let fingerprint = get_fingerprint(fingerprint_id)?;
    let storage_path = device::get_storage_path()?;

    device::write_profile(&storage_path, &fingerprint.profile)?;

    let mut store = load_fingerprint_store()?;
    store.current_fingerprint_id = Some(fingerprint_id.to_string());
    save_fingerprint_store(&store)?;

    logger::log_info(&format!("已应用指纹到系统: {}", fingerprint.name));
    Ok(format!("已应用指纹: {}", fingerprint.name))
}

/// 重命名指纹
pub fn rename_fingerprint(fingerprint_id: &str, name: String) -> Result<(), String> {
    if fingerprint_id == "original" {
        return Err("原始指纹不可重命名".to_string());
    }
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("指纹名称不能为空".to_string());
    }

    let mut store = load_fingerprint_store()?;
    let fp = store
        .fingerprints
        .iter_mut()
        .find(|f| f.id == fingerprint_id)
        .ok_or_else(|| "指纹不存在".to_string())?;

    fp.name = trimmed.to_string();
    save_fingerprint_store(&store)?;
    logger::log_info(&format!("指纹已重命名: {}", fingerprint_id));
    Ok(())
}

/// 删除指纹
pub fn delete_fingerprint(fingerprint_id: &str) -> Result<(), String> {
    if fingerprint_id == "original" {
        return Err("原始指纹不可删除".to_string());
    }

    let mut store = load_fingerprint_store()?;

    let before = store.fingerprints.len();
    store.fingerprints.retain(|f| f.id != fingerprint_id);

    if store.fingerprints.len() == before {
        return Err("指纹不存在".to_string());
    }

    // 如果删除的是当前应用的指纹，切换到原始指纹
    if store.current_fingerprint_id.as_deref() == Some(fingerprint_id) {
        store.current_fingerprint_id = Some("original".to_string());
    }

    save_fingerprint_store(&store)?;

    // 更新所有绑定此指纹的账号，改为绑定原始指纹
    update_accounts_fingerprint(fingerprint_id, "original")?;

    logger::log_info(&format!("已删除指纹: {}", fingerprint_id));
    Ok(())
}

/// 删除所有未绑定账号的指纹（排除原始指纹）
pub fn delete_unbound_fingerprints() -> Result<usize, String> {
    let accounts = crate::modules::account::list_accounts()?;
    let bound_ids: HashSet<String> = accounts
        .into_iter()
        .filter_map(|account| account.fingerprint_id)
        .collect();

    let mut store = load_fingerprint_store()?;
    let to_delete_ids: HashSet<String> = store
        .fingerprints
        .iter()
        .filter(|fp| !bound_ids.contains(&fp.id))
        .map(|fp| fp.id.clone())
        .collect();

    if to_delete_ids.is_empty() {
        return Ok(0);
    }

    let deleted_count = to_delete_ids.len();
    store
        .fingerprints
        .retain(|fp| !to_delete_ids.contains(&fp.id));

    if store
        .current_fingerprint_id
        .as_ref()
        .is_some_and(|fid| to_delete_ids.contains(fid))
    {
        store.current_fingerprint_id = Some("original".to_string());
    }

    save_fingerprint_store(&store)?;
    logger::log_info(&format!("已批量删除未绑定账号指纹: {} 个", deleted_count));
    Ok(deleted_count)
}

/// 更新账号的指纹绑定（当指纹被删除时）
fn update_accounts_fingerprint(old_id: &str, new_id: &str) -> Result<(), String> {
    let accounts = crate::modules::account::list_accounts()?;

    for mut account in accounts {
        if account.fingerprint_id.as_deref() == Some(old_id) {
            account.fingerprint_id = Some(new_id.to_string());
            crate::modules::account::save_account(&account)?;
        }
    }

    Ok(())
}

/// 获取绑定某指纹的所有账号
pub fn get_bound_accounts(fingerprint_id: &str) -> Result<Vec<crate::models::Account>, String> {
    let accounts = crate::modules::account::list_accounts()?;
    Ok(accounts
        .into_iter()
        .filter(|a| a.fingerprint_id.as_deref() == Some(fingerprint_id))
        .collect())
}

/// 指纹列表响应（包含绑定账号数）
#[derive(Debug, Serialize)]
pub struct FingerprintWithStats {
    pub id: String,
    pub name: String,
    pub profile: DeviceProfile,
    pub created_at: i64,
    pub is_original: bool,
    pub is_current: bool,
    pub bound_account_count: usize,
}

/// 列出所有指纹（带统计信息）
pub fn list_fingerprints_with_stats() -> Result<Vec<FingerprintWithStats>, String> {
    let store = load_fingerprint_store()?;
    let accounts = crate::modules::account::list_accounts()?;

    // 读取系统实际的 storage.json 获取真正的当前 machineId
    let actual_current_machine_id = if let Ok(storage_path) = device::get_storage_path() {
        device::read_profile(&storage_path)
            .ok()
            .map(|p| p.machine_id)
    } else {
        None
    };

    let count_bound = |fid: &str| -> usize {
        accounts
            .iter()
            .filter(|a| a.fingerprint_id.as_deref() == Some(fid))
            .count()
    };

    // 判断指纹是否是当前系统应用的
    let is_current_fp = |profile: &DeviceProfile| -> bool {
        actual_current_machine_id.as_ref() == Some(&profile.machine_id)
    };

    let mut result = Vec::new();

    // 原始指纹
    if let Some(baseline) = store.original_baseline {
        let is_current = is_current_fp(&baseline.profile);
        result.push(FingerprintWithStats {
            id: baseline.id.clone(),
            name: baseline.name,
            profile: baseline.profile,
            created_at: baseline.created_at,
            is_original: true,
            is_current,
            bound_account_count: count_bound(&baseline.id),
        });
    }

    // 其余按时间倒序
    let mut others = store.fingerprints.clone();
    others.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    // 当前应用的放第二位
    let current_pos = others.iter().position(|f| is_current_fp(&f.profile));
    if let Some(pos) = current_pos {
        let fp = others.remove(pos);
        result.push(FingerprintWithStats {
            id: fp.id.clone(),
            name: fp.name,
            profile: fp.profile,
            created_at: fp.created_at,
            is_original: false,
            is_current: true,
            bound_account_count: count_bound(&fp.id),
        });
    }

    // 其余指纹
    for fp in others {
        result.push(FingerprintWithStats {
            id: fp.id.clone(),
            name: fp.name,
            profile: fp.profile,
            created_at: fp.created_at,
            is_original: false,
            is_current: false,
            bound_account_count: count_bound(&fp.id),
        });
    }

    Ok(result)
}
