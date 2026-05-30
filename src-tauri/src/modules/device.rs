use crate::models::DeviceProfile;
use crate::modules::logger;
use rand::{distributions::Alphanumeric, Rng};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const GLOBAL_BASELINE: &str = "device_original.json";

fn get_data_dir() -> Result<PathBuf, String> {
    crate::modules::account::get_data_dir()
}

/// 寻找 storage.json 路径
pub fn get_storage_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path = home
            .join("Library/Application Support/Antigravity IDE/User/globalStorage/storage.json");
        if path.exists() {
            return Ok(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let path = crate::modules::antigravity_paths::storage_json_path()?;
        if path.exists() {
            return Ok(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        let path = home.join(".config/Antigravity IDE/User/globalStorage/storage.json");
        if path.exists() {
            return Ok(path);
        }
    }

    Err("未找到 storage.json，请确认 Antigravity IDE 已运行过".to_string())
}

/// 获取 storage.json 所在目录
pub fn get_storage_dir() -> Result<PathBuf, String> {
    let path = get_storage_path()?;
    path.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法获取 storage.json 所在目录".to_string())
}

/// 获取 state.vscdb 路径
pub fn get_state_db_path() -> Result<PathBuf, String> {
    let dir = get_storage_dir()?;
    Ok(dir.join("state.vscdb"))
}

/// 获取 machineid 文件路径
fn get_machine_id_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join("Library/Application Support/Antigravity IDE/machineid"));
    }

    #[cfg(target_os = "windows")]
    {
        return crate::modules::antigravity_paths::machine_id_path();
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join(".config/Antigravity IDE/machineid"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 machineid 路径".to_string())
}

fn is_valid_uuid(value: &str) -> bool {
    use regex::Regex;
    use std::sync::LazyLock;
    static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap()
    });
    UUID_RE.is_match(value.trim())
}

fn validate_service_machine_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if is_valid_uuid(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn generate_service_machine_id() -> String {
    Uuid::new_v4().to_string()
}

fn read_machine_id_file() -> Option<String> {
    let path = get_machine_id_path().ok()?;
    let content = fs::read_to_string(&path).ok()?;
    validate_service_machine_id(&content)
}

fn write_machine_id_file(service_id: &str) -> Result<(), String> {
    let path = get_machine_id_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 machineid 目录失败: {}", e))?;
    }
    fs::write(&path, service_id).map_err(|e| format!("写入 machineid 失败: {}", e))?;
    Ok(())
}

fn read_state_service_machine_id_value() -> Option<String> {
    let db_path = get_state_db_path().ok()?;
    if !db_path.exists() {
        return None;
    }
    let conn = Connection::open(&db_path).ok()?;
    let value: Result<String, _> = conn.query_row(
        "SELECT value FROM ItemTable WHERE key = 'storage.serviceMachineId'",
        [],
        |row| row.get(0),
    );
    value.ok().and_then(|v| validate_service_machine_id(&v))
}

fn sync_state_service_machine_id_value(service_id: &str) -> Result<(), String> {
    let db_path = get_state_db_path()?;
    if !db_path.exists() {
        logger::log_warn(&format!(
            "state.vscdb 不存在，跳过 serviceMachineId 同步: {:?}",
            db_path
        ));
        return Ok(());
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("打开 state.vscdb 失败: {}", e))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT);",
        [],
    )
    .map_err(|e| format!("创建 ItemTable 失败: {}", e))?;
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('storage.serviceMachineId', ?1);",
        [service_id],
    )
    .map_err(|e| format!("写入 storage.serviceMachineId 失败: {}", e))?;
    logger::log_info("已同步 storage.serviceMachineId 至 state.vscdb");
    Ok(())
}

/// 获取 serviceMachineId（官方优先级：数据库 → 文件 → 生成）
pub fn get_service_machine_id() -> String {
    // 1. 优先从数据库读取
    if let Some(id) = read_state_service_machine_id_value() {
        return id;
    }
    // 2. 其次从 machineid 文件读取
    if let Some(id) = read_machine_id_file() {
        let _ = sync_state_service_machine_id_value(&id);
        return id;
    }
    // 3. 生成新的并写入
    let new_id = generate_service_machine_id();
    if let Err(e) = write_machine_id_file(&new_id) {
        logger::log_warn(&format!("写入 machineid 失败: {}", e));
    }
    let _ = sync_state_service_machine_id_value(&new_id);
    new_id
}

/// 确保 profile 有有效的 serviceMachineId
pub fn ensure_service_machine_id(profile: &mut DeviceProfile) -> bool {
    match validate_service_machine_id(&profile.service_machine_id) {
        Some(value) => {
            if value != profile.service_machine_id {
                profile.service_machine_id = value;
                return true;
            }
            false
        }
        None => {
            profile.service_machine_id = generate_service_machine_id();
            true
        }
    }
}

/// 从 storage.json 读取当前设备指纹
pub fn read_profile(storage_path: &Path) -> Result<DeviceProfile, String> {
    let content =
        fs::read_to_string(storage_path).map_err(|e| format!("读取 storage.json 失败: {}", e))?;
    let json: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 storage.json 失败: {}", e))?;

    let get_field = |key: &str| -> Option<String> {
        if let Some(obj) = json.get("telemetry").and_then(|v| v.as_object()) {
            if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
        if let Some(v) = json
            .get(format!("telemetry.{key}"))
            .and_then(|v| v.as_str())
        {
            return Some(v.to_string());
        }
        None
    };

    // serviceMachineId 使用官方优先级
    let service_machine_id = get_service_machine_id();

    Ok(DeviceProfile {
        machine_id: get_field("machineId").ok_or("缺少 telemetry.machineId")?,
        mac_machine_id: get_field("macMachineId").ok_or("缺少 telemetry.macMachineId")?,
        dev_device_id: get_field("devDeviceId").ok_or("缺少 telemetry.devDeviceId")?,
        sqm_id: get_field("sqmId").ok_or("缺少 telemetry.sqmId")?,
        service_machine_id,
    })
}

pub struct ReadProfileWithAutofillResult {
    pub profile: DeviceProfile,
    pub auto_generated_fields: Vec<String>,
}

/// 从 storage.json 读取当前设备指纹；缺失字段时自动补齐并回写
pub fn read_profile_with_autofill(
    storage_path: &Path,
) -> Result<ReadProfileWithAutofillResult, String> {
    let content =
        fs::read_to_string(storage_path).map_err(|e| format!("读取 storage.json 失败: {}", e))?;
    let json: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 storage.json 失败: {}", e))?;

    let get_field = |key: &str| -> Option<String> {
        if let Some(obj) = json.get("telemetry").and_then(|v| v.as_object()) {
            if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
        if let Some(v) = json
            .get(format!("telemetry.{key}"))
            .and_then(|v| v.as_str())
        {
            return Some(v.to_string());
        }
        None
    };

    let mut auto_generated_fields = Vec::new();
    let machine_id = match get_field("machineId") {
        Some(value) => value,
        None => {
            auto_generated_fields.push("machineId".to_string());
            format!("auth0|user_{}", random_hex(32))
        }
    };
    let mac_machine_id = match get_field("macMachineId") {
        Some(value) => value,
        None => {
            auto_generated_fields.push("macMachineId".to_string());
            new_standard_machine_id()
        }
    };
    let dev_device_id = match get_field("devDeviceId") {
        Some(value) => value,
        None => {
            auto_generated_fields.push("devDeviceId".to_string());
            Uuid::new_v4().to_string()
        }
    };
    let sqm_id = match get_field("sqmId") {
        Some(value) => value,
        None => {
            auto_generated_fields.push("sqmId".to_string());
            format!("{{{}}}", Uuid::new_v4().to_string().to_uppercase())
        }
    };
    let service_machine_id = get_service_machine_id();

    let profile = DeviceProfile {
        machine_id,
        mac_machine_id,
        dev_device_id,
        sqm_id,
        service_machine_id,
    };

    if !auto_generated_fields.is_empty() {
        if let Err(err) = write_profile(storage_path, &profile) {
            logger::log_warn(&format!(
                "storage.json 缺失字段自动补齐后回写失败（仅影响持久化，不影响预览）: {}",
                err
            ));
        }
    }

    Ok(ReadProfileWithAutofillResult {
        profile,
        auto_generated_fields,
    })
}

/// 将设备指纹写入 storage.json
pub fn write_profile(storage_path: &Path, profile: &DeviceProfile) -> Result<(), String> {
    if !storage_path.exists() {
        return Err(format!("storage.json 不存在: {:?}", storage_path));
    }

    let content =
        fs::read_to_string(storage_path).map_err(|e| format!("读取 storage.json 失败: {}", e))?;
    let mut json: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 storage.json 失败: {}", e))?;

    // 确保 telemetry 是对象
    if !json.get("telemetry").map_or(false, |v| v.is_object()) {
        if json.as_object_mut().is_some() {
            json["telemetry"] = serde_json::json!({});
        } else {
            return Err("storage.json 顶层不是对象".to_string());
        }
    }

    if let Some(telemetry) = json.get_mut("telemetry").and_then(|v| v.as_object_mut()) {
        telemetry.insert(
            "machineId".to_string(),
            Value::String(profile.machine_id.clone()),
        );
        telemetry.insert(
            "macMachineId".to_string(),
            Value::String(profile.mac_machine_id.clone()),
        );
        telemetry.insert(
            "devDeviceId".to_string(),
            Value::String(profile.dev_device_id.clone()),
        );
        telemetry.insert("sqmId".to_string(), Value::String(profile.sqm_id.clone()));
    }

    // 同时写入扁平键，兼容旧格式
    if let Some(map) = json.as_object_mut() {
        map.insert(
            "telemetry.machineId".to_string(),
            Value::String(profile.machine_id.clone()),
        );
        map.insert(
            "telemetry.macMachineId".to_string(),
            Value::String(profile.mac_machine_id.clone()),
        );
        map.insert(
            "telemetry.devDeviceId".to_string(),
            Value::String(profile.dev_device_id.clone()),
        );
        map.insert(
            "telemetry.sqmId".to_string(),
            Value::String(profile.sqm_id.clone()),
        );
    }

    // serviceMachineId 验证
    let service_machine_id = match validate_service_machine_id(&profile.service_machine_id) {
        Some(value) => value,
        None => {
            let generated = get_service_machine_id();
            logger::log_warn("serviceMachineId 无效，已从官方来源获取或生成新值");
            generated
        }
    };

    let updated = serde_json::to_string_pretty(&json).map_err(|e| format!("序列化失败: {}", e))?;
    fs::write(storage_path, updated).map_err(|e| format!("写入失败: {}", e))?;
    logger::log_info("已写入设备指纹到 storage.json");

    // 同步 machineid 文件
    if let Err(e) = write_machine_id_file(&service_machine_id) {
        logger::log_warn(&format!("写入 machineid 失败: {}", e));
    }

    // 同步 state.vscdb
    let _ = sync_state_service_machine_id_value(&service_machine_id);
    Ok(())
}

/// 全局原始指纹的加载
pub fn load_global_original() -> Option<DeviceProfile> {
    if let Ok(dir) = get_data_dir() {
        let path = dir.join(GLOBAL_BASELINE);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(mut profile) = serde_json::from_str::<DeviceProfile>(&content) {
                    // 如果原始备份里没有 serviceMachineId，补充
                    if !is_valid_uuid(&profile.service_machine_id) {
                        let sys_id = get_service_machine_id();
                        logger::log_info(&format!(
                            "原始备份缺少 serviceMachineId，已从系统补充: {}",
                            sys_id
                        ));
                        profile.service_machine_id = sys_id;
                        let _ = save_global_original_force(&profile);
                    }
                    return Some(profile);
                }
            }
        }
    }
    None
}

/// 强制保存原始指纹（覆盖）
fn save_global_original_force(profile: &DeviceProfile) -> Result<(), String> {
    let dir = get_data_dir()?;
    let path = dir.join(GLOBAL_BASELINE);
    let content =
        serde_json::to_string_pretty(profile).map_err(|e| format!("序列化失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入失败: {}", e))
}

/// 恢复原始设备指纹到 storage.json
pub fn restore_original_device() -> Result<String, String> {
    let baseline = load_global_original().ok_or("未找到原始指纹备份")?;
    let storage_path = get_storage_path()?;
    write_profile(&storage_path, &baseline)?;
    Ok("已恢复原始设备指纹".to_string())
}

/// 生成一组新的设备指纹
pub fn generate_profile() -> DeviceProfile {
    DeviceProfile {
        machine_id: format!("auth0|user_{}", random_hex(32)),
        mac_machine_id: new_standard_machine_id(),
        dev_device_id: Uuid::new_v4().to_string(),
        sqm_id: format!("{{{}}}", Uuid::new_v4().to_string().to_uppercase()),
        service_machine_id: generate_service_machine_id(),
    }
}

fn random_hex(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect::<String>()
        .to_lowercase()
}

fn new_standard_machine_id() -> String {
    let mut rng = rand::thread_rng();
    let mut id = String::with_capacity(36);
    for ch in "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".chars() {
        if ch == '-' || ch == '4' {
            id.push(ch);
        } else if ch == 'x' {
            id.push_str(&format!("{:x}", rng.gen_range(0..16)));
        } else if ch == 'y' {
            id.push_str(&format!("{:x}", rng.gen_range(8..12)));
        }
    }
    id
}

/// 打开设备存储目录
pub fn open_device_folder() -> Result<(), String> {
    let storage_dir = get_storage_dir()?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&storage_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&storage_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&storage_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}
