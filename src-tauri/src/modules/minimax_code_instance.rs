//! MiniMax Code 应用多开实例存储。
//!
//! 与 WorkBuddy 的 `workbuddy_instance.rs` 同构，但实例的 `user_data_dir` 直接就是
//! MiniMax 客户端的 **data root**（对应客户端的 `MINIMAX_DATA_DIR` / `--data-dir`）。
//!
//! 客户端把登录态写在 `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json`
//! （见 `minimax_code_account::switch_minimax_code_account_to_data_dir`），因此每个实例
//! 只要拿到独立的 data root，就拥有互相隔离的登录态 —— 这是本模块多开的根基。
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{DefaultInstanceSettings, InstanceProfile, InstanceStore};
use crate::modules;
use crate::modules::instance::InstanceDefaults;
use crate::modules::instance_store;

pub use crate::modules::instance_store::{CreateInstanceParams, UpdateInstanceParams};

static MINIMAX_CODE_INSTANCE_STORE_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const MINIMAX_CODE_INSTANCES_FILE: &str = "minimax_code_instances.json";

/// 客户端 `MCODE_OAUTH_CLIENT_ID`（见 app.asar `contracts.js`）。
pub const MINIMAX_CODE_OAUTH_CLIENT_ID: &str = "mcode-public";
/// 客户端 `MCODE_OAUTH_SCOPES`。
pub const MINIMAX_CODE_OAUTH_SCOPES: &[&str] = &["agent.default"];
/// 客户端 `MCODE_OAUTH_AUDIENCE`。
pub const MINIMAX_CODE_OAUTH_AUDIENCE: &str = "agent-backend";
/// 客户端服务键前缀：`com.minimax.mcode.oauth.<buildEnv>.<region>`。
pub const MINIMAX_CODE_OAUTH_SERVICE_BASE: &str = "com.minimax.mcode.oauth";

fn instances_path() -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    Ok(instances_path_for(&data_dir))
}

/// 实例存储路径（便于测试注入独立的数据根）。
fn instances_path_for(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(MINIMAX_CODE_INSTANCES_FILE)
}

pub fn load_instance_store() -> Result<InstanceStore, String> {
    let path = instances_path()?;
    instance_store::load_instance_store(&path, MINIMAX_CODE_INSTANCES_FILE)
}

pub fn save_instance_store(store: &InstanceStore) -> Result<(), String> {
    let path = instances_path()?;
    instance_store::save_instance_store(&path, MINIMAX_CODE_INSTANCES_FILE, store)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    let store = load_instance_store()?;
    Ok(store.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    extra_args: Option<String>,
    follow_local_account: Option<bool>,
) -> Result<DefaultInstanceSettings, String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let settings = &mut store.default_settings;

    if follow_local_account == Some(true) {
        settings.follow_local_account = false;
    }

    if let Some(bind) = bind_account_id {
        settings.bind_account_id = bind;
        settings.follow_local_account = false;
    }

    if let Some(args) = extra_args {
        settings.extra_args = args.trim().to_string();
    }

    let updated = settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

/// 默认实例设置的读写（可注入数据根，供测试使用）。
#[cfg(test)]
fn update_default_settings_at(
    data_dir: &std::path::Path,
    bind_account_id: Option<Option<String>>,
    extra_args: Option<String>,
) -> Result<DefaultInstanceSettings, String> {
    let mut store =
        instance_store::load_instance_store(&instances_path_for(data_dir), MINIMAX_CODE_INSTANCES_FILE)?;
    let settings = &mut store.default_settings;
    if let Some(bind) = bind_account_id {
        settings.bind_account_id = bind;
    }
    if let Some(args) = extra_args {
        settings.extra_args = args.trim().to_string();
    }
    let updated = settings.clone();
    instance_store::save_instance_store(
        &instances_path_for(data_dir),
        MINIMAX_CODE_INSTANCES_FILE,
        &store,
    )?;
    Ok(updated)
}

/// MiniMax Code 客户端的默认 data root。
///
/// 客户端读取顺序：`--data-dir` CLI 参数 → `MINIMAX_DATA_DIR` → `MAVIS_DATA_DIR`
/// → `DATA_DIR` → `~/.minimax`（见 app.asar 中 `getExplicitDataDirArg`）。
/// 这里让显式设置的环境变量优先，保证外部已用 `MINIMAX_DATA_DIR` 启动过时行为一致。
pub fn get_default_minimax_code_data_dir() -> Result<PathBuf, String> {
    if let Some(explicit) = explicit_minimax_code_data_dir() {
        return Ok(explicit);
    }
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".minimax"))
}

/// 读取用户显式设置的 MiniMax data root。
///
/// 优先级与客户端 `getExplicitDataDirArg()` 对齐：
/// `--data-dir`（由启动器处理）→ `MINIMAX_DATA_DIR` → `MAVIS_DATA_DIR` → `DATA_DIR`。
/// 这里只能看到环境变量部分，`--data-dir` 不经过本进程。
pub fn explicit_minimax_code_data_dir() -> Option<PathBuf> {
    ["MINIMAX_DATA_DIR", "MAVIS_DATA_DIR", "DATA_DIR"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

/// 默认（非托管）实例的 data root。
pub fn get_default_minimax_code_user_data_dir() -> Result<PathBuf, String> {
    get_default_minimax_code_data_dir()
}

pub fn get_default_instances_root_dir() -> Result<PathBuf, String> {
    crate::modules::data_paths::managed_instances_root_dir("minimax_code")
}

/// 校验用户提供的实例 data root 是否可用。
///
/// - 空路径 → 拒绝；
/// - 路径已存在但不是目录 → 拒绝；
/// - 路径已存在且非空 → 仅在显式 `init_mode = existingdir` 时接受（由调用方判断）。
fn validate_instance_data_dir(user_data_dir: &str) -> Result<PathBuf, String> {
    let raw = user_data_dir.trim();
    if raw.is_empty() {
        return Err("实例目录不能为空".to_string());
    }
    let path = PathBuf::from(raw);
    if path.exists() && !path.is_dir() {
        return Err("所选路径不是目录".to_string());
    }
    Ok(path)
}

pub fn get_instance_defaults() -> Result<InstanceDefaults, String> {
    let root_dir = get_default_instances_root_dir()?;
    let default_user_data_dir = get_default_minimax_code_user_data_dir()?;
    Ok(InstanceDefaults {
        root_dir: root_dir.to_string_lossy().to_string(),
        default_user_data_dir: default_user_data_dir.to_string_lossy().to_string(),
    })
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let directory_preexisted = std::path::Path::new(params.user_data_dir.trim()).exists();
    let creation_mode = params.init_mode.clone().unwrap_or_else(|| "copy".to_string());
    let _creation_guard = crate::modules::instance_storage_cleanup::protect_instance_creation()?;
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;

    let name = instance_store::normalize_name(&params.name)?;
    let user_data_dir = params.user_data_dir.trim().to_string();
    if user_data_dir.is_empty() {
        return Err("实例目录不能为空".to_string());
    }

    instance_store::ensure_unique(&store, &name, &user_data_dir, None)?;

    let init_mode = params
        .init_mode
        .as_deref()
        .unwrap_or("copy")
        .to_ascii_lowercase();
    let create_empty = init_mode == "empty";
    let use_existing_dir = init_mode == "existingdir" || init_mode == "existing_dir";
    let user_dir_path = validate_instance_data_dir(&user_data_dir)?;

    if use_existing_dir {
        if !user_dir_path.exists() {
            let resolved = instance_store::display_path(&user_dir_path);
            return Err(format!("所选目录不存在：{}", resolved));
        }
    } else if create_empty {
        // 空白实例：只保证 data root 存在。登录态在首次启动时注入。
        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                return Err(format!("空白实例需要目标目录为空：{}", resolved_path));
            }
        }
        fs::create_dir_all(&user_dir_path)
            .map_err(|e| format!("创建实例目录失败：{}", e))?;
    } else {
        // 复制默认 data root（`~/.minimax`）到实例目录，
        // 这样实例自带一份 auth/config/session，多开后互不干扰。
        let source_dir = match params.copy_source_instance_id.as_deref() {
            Some("__default__") | None => get_default_minimax_code_data_dir()?,
            Some(source_id) => {
                let source_instance = store
                    .instances
                    .iter()
                    .find(|item| item.id == source_id)
                    .ok_or("复制来源实例不存在")?;
                PathBuf::from(source_instance.user_data_dir.trim())
            }
        };

        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                modules::logger::log_info(&format!(
                    "[MiniMax Code Instance] 复制来源实例需要空目录，但目标已存在：{}",
                    resolved_path
                ));
                return Err(format!("复制来源实例需要目标目录为空：{}", resolved_path));
            }
        }

        if !source_dir.exists() {
            return Err("未找到复制来源目录，请先确保来源实例已初始化".to_string());
        }

        instance_store::copy_dir_recursive(&source_dir, &user_dir_path)?;
    }

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir,
        working_dir: params.working_dir,
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: if create_empty {
            None
        } else {
            params.bind_account_id
        },
        model_routing: None,
        launch_mode: crate::models::InstanceLaunchMode::App,
        app_speed: crate::models::codex::CodexAppSpeed::Standard,
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };

    crate::modules::instance_directory_ownership::record_creation(
        std::path::Path::new(&instance.user_data_dir),
        &instance.id,
        &creation_mode,
        directory_preexisted,
    )?;
    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == params.instance_id)
        .ok_or("实例不存在")?;

    let current_id = store.instances[index].id.clone();
    let current_dir = store.instances[index].user_data_dir.clone();
    let next_name = params
        .name
        .as_ref()
        .map(|name| instance_store::normalize_name(name))
        .transpose()?;

    if let Some(ref normalized) = next_name {
        instance_store::ensure_unique(&store, normalized, &current_dir, Some(&current_id))?;
    }

    let instance = &mut store.instances[index];
    if let Some(normalized) = next_name {
        instance.name = normalized;
    }
    if let Some(ref extra_args) = params.extra_args {
        instance.extra_args = extra_args.trim().to_string();
    }
    if let Some(bind) = params.bind_account_id.clone() {
        instance.bind_account_id = bind;
    }

    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn delete_instance(instance_id: &str) -> Result<(), String> {
    let _creation_guard = crate::modules::instance_storage_cleanup::protect_instance_creation()?;
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == instance_id)
        .ok_or("实例不存在")?;
    let user_data_dir = store.instances[index].user_data_dir.clone();

    if !user_data_dir.trim().is_empty() {
        let dir_path = PathBuf::from(&user_data_dir);
        modules::instance::delete_instance_directory(&dir_path)?;
    }

    store.instances.remove(index);
    save_instance_store(&store)?;
    Ok(())
}

pub fn update_instance_after_start(instance_id: &str, pid: u32) -> Result<InstanceProfile, String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut updated = None;
    for instance in &mut store.instances {
        if instance.id == instance_id {
            instance.last_launched_at = Some(Utc::now().timestamp_millis());
            instance.last_pid = Some(pid);
            updated = Some(instance.clone());
            break;
        }
    }
    let updated = updated.ok_or("实例不存在")?;
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_instance_pid(instance_id: &str, pid: Option<u32>) -> Result<InstanceProfile, String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut updated = None;
    for instance in &mut store.instances {
        if instance.id == instance_id {
            instance.last_pid = pid;
            updated = Some(instance.clone());
            break;
        }
    }
    let updated = updated.ok_or("实例不存在")?;
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_default_pid(pid: Option<u32>) -> Result<DefaultInstanceSettings, String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = pid;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn clear_all_pids() -> Result<(), String> {
    let _lock = MINIMAX_CODE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = None;
    for instance in &mut store.instances {
        instance.last_pid = None;
    }
    save_instance_store(&store)?;
    Ok(())
}

/// 客户端命名空间里的 buildEnv/region。
///
/// 客户端 `createAuthNamespace` 用 `MAVIS_BUILD_ENV`/`NEXT_PUBLIC_BUILD_ENV` 决定
/// buildEnv（默认 `prod`），用 `NEXT_PUBLIC_LOCALE`/区域配置决定 region（`cn`/`en`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinimaxCodeAuthScope {
    pub build_env: String,
    pub region: String,
}

impl MinimaxCodeAuthScope {
    pub const DEFAULT_BUILD_ENV: &'static str = "prod";
    pub const DEFAULT_REGION: &'static str = "cn";

    pub fn default_scope() -> Self {
        Self {
            build_env: Self::DEFAULT_BUILD_ENV.to_string(),
            region: Self::DEFAULT_REGION.to_string(),
        }
    }

    pub fn service_prefix(&self) -> String {
        format!(
            "{}.{}.{}",
            MINIMAX_CODE_OAUTH_SERVICE_BASE, self.build_env, self.region
        )
    }

    pub fn is_valid_build_env(value: &str) -> bool {
        matches!(value, "dev" | "test" | "staging" | "prod")
    }

    pub fn is_valid_region(value: &str) -> bool {
        matches!(value, "cn" | "en")
    }
}

/// 把客户端的 buildEnv/region 字符串归一到枚举值；非法值返回 `None`。
fn normalize_build_env(value: &str) -> Option<&'static str> {
    match value {
        "dev" => Some("dev"),
        "test" => Some("test"),
        "staging" => Some("staging"),
        "prod" => Some("prod"),
        _ => None,
    }
}

fn normalize_region(value: &str) -> Option<&'static str> {
    match value {
        "cn" => Some("cn"),
        "en" => Some("en"),
        _ => None,
    }
}

/// 客户端命名空间路径：`<data root>/auth/<buildEnv>/<region>/mcode-public`。
pub fn minimax_code_namespace_home(data_dir: &std::path::Path, scope: &MinimaxCodeAuthScope) -> PathBuf {
    data_dir
        .join("auth")
        .join(&scope.build_env)
        .join(&scope.region)
        .join(MINIMAX_CODE_OAUTH_CLIENT_ID)
}

/// 解析某个实例 data root 下应当写入的 `auth.json` 路径。
///
/// buildEnv/region 的解析顺序（与客户端布局保持一致）：
/// 1. data root 下已有 `auth-state.json` → 读它的 `buildEnv`/`region`；
/// 2. 否则已有 `auth.json` → 从记录键前缀 `com.minimax.mcode.oauth.<buildEnv>.<region>` 取；
/// 3. 否则回退到 `prod`/`cn`（当前正式版的默认值）。
///
/// `preferred_region` 用于账号自带 region（例如导入的 `en` 账号）覆盖默认值。
#[cfg(test)]
pub fn resolve_minimax_code_auth_file_path_with_scope(
    data_dir: &std::path::Path,
    preferred_region: Option<&str>,
) -> PathBuf {
    // 已有 auth.json 时直接复用其所在命名空间（可能与默认 prod/cn 不同，
    // 例如 staging/en）。这是最可靠的信号，避免写到一个客户端不会读的新目录。
    if let Some(existing) = find_auth_file_under(data_dir) {
        return existing;
    }
    let scope = resolve_minimax_code_auth_scope(data_dir, preferred_region);
    minimax_code_namespace_home(data_dir, &scope).join("auth.json")
}

#[cfg(test)]
pub fn resolve_minimax_code_auth_file_path(data_dir: &std::path::Path) -> PathBuf {
    resolve_minimax_code_auth_file_path_with_scope(data_dir, None)
}

/// 解析 data root 对应的 buildEnv/region（见 `resolve_minimax_code_auth_file_path`）。
///
/// 解析顺序：
/// 1. `auth-state.json` 里的 `buildEnv`/`region`（客户端自己写的，最权威）；
/// 2. `auth.json` 记录键前缀 `com.minimax.mcode.oauth.<buildEnv>.<region>`；
/// 3. 命名空间目录结构 `<data root>/auth/<buildEnv>/<region>/mcode-public`；
/// 4. 账号自带的 `preferred_region`，否则 `prod`/`cn`。
pub fn resolve_minimax_code_auth_scope(
    data_dir: &std::path::Path,
    preferred_region: Option<&str>,
) -> MinimaxCodeAuthScope {
    if let Some(scope) = read_scope_from_auth_state(data_dir) {
        return scope;
    }
    if let Some(scope) = read_scope_from_auth_file(data_dir) {
        return scope;
    }
    let fallback_region = preferred_region
        .map(str::trim)
        .and_then(normalize_region)
        .unwrap_or(MinimaxCodeAuthScope::DEFAULT_REGION);
    if let Some(scope) = read_scope_from_namespace_dirs(data_dir) {
        // 账号显式指定的 region 优先于从目录推断的结果。
        if preferred_region
            .map(str::trim)
            .and_then(normalize_region)
            .is_some_and(|value| value != scope.region)
        {
            return MinimaxCodeAuthScope {
                build_env: scope.build_env,
                region: fallback_region.to_string(),
            };
        }
        return scope;
    }
    MinimaxCodeAuthScope {
        build_env: MinimaxCodeAuthScope::DEFAULT_BUILD_ENV.to_string(),
        region: fallback_region.to_string(),
    }
}

/// 扫描 `<data root>/auth/*/*/<clientId>` 目录，取第一个合法的 buildEnv/region。
fn read_scope_from_namespace_dirs(data_dir: &std::path::Path) -> Option<MinimaxCodeAuthScope> {
    let auth_root = data_dir.join("auth");
    let Ok(envs) = fs::read_dir(&auth_root) else {
        return None;
    };
    for env_entry in envs.flatten() {
        let Ok(regions) = fs::read_dir(env_entry.path()) else {
            continue;
        };
        for region_entry in regions.flatten() {
            let client_dir = region_entry.path();
            if client_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| name != MINIMAX_CODE_OAUTH_CLIENT_ID)
            {
                continue;
            }
            let env_name = env_entry.file_name();
            let region_name = region_entry.file_name();
            let build_env = env_name.to_str();
            let region = region_name.to_str();
            if let (Some(build_env), Some(region)) = (build_env, region) {
                if let (Some(build_env), Some(region)) =
                    (normalize_build_env(build_env), normalize_region(region))
                {
                    return Some(MinimaxCodeAuthScope {
                        build_env: build_env.to_string(),
                        region: region.to_string(),
                    });
                }
            }
        }
    }
    None
}

/// 从 data root 下任意命名空间的 `auth-state.json` 读取 buildEnv/region。
fn read_scope_from_auth_state(data_dir: &std::path::Path) -> Option<MinimaxCodeAuthScope> {
    let auth_root = data_dir.join("auth");
    let mut stack = vec![auth_root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) != Some("auth-state.json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            let build_env = parsed
                .get("buildEnv")
                .and_then(|value| value.as_str())
                .and_then(normalize_build_env);
            let region = parsed
                .get("region")
                .and_then(|value| value.as_str())
                .and_then(normalize_region);
            // 客户端允许两者同时缺省（isOptionalAuthScope），此时用默认值。
            if build_env.is_none() && region.is_none() {
                return Some(MinimaxCodeAuthScope::default_scope());
            }
            if let (Some(build_env), Some(region)) = (build_env, region) {
                return Some(MinimaxCodeAuthScope {
                    build_env: build_env.to_string(),
                    region: region.to_string(),
                });
            }
        }
    }
    None
}

/// 从 data root 下任意命名空间的 `auth.json` 记录键前缀读取 buildEnv/region。
fn read_scope_from_auth_file(data_dir: &std::path::Path) -> Option<MinimaxCodeAuthScope> {
    let auth_file = find_auth_file_under(data_dir)?;
    let raw = fs::read_to_string(&auth_file).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let key = parsed.get("records")?.as_object()?.keys().next()?;
    let service = key.split('\u{0}').next()?;
    let mut parts = service.rsplit('.');
    let region = parts.next().and_then(normalize_region)?;
    let build_env = parts.next().and_then(normalize_build_env)?;
    Some(MinimaxCodeAuthScope {
        build_env: build_env.to_string(),
        region: region.to_string(),
    })
}

/// 在 data root 下查找 `auth.json`。
///
/// 优先返回 buildEnv/region 与目标一致的命名空间，避免任意 DFS 命中到旧版本/其它环境的目录。
fn find_auth_file_under(data_dir: &std::path::Path) -> Option<PathBuf> {
    find_auth_file_under_with_scope(data_dir, None)
}

fn find_auth_file_under_with_scope(
    data_dir: &std::path::Path,
    preferred: Option<&MinimaxCodeAuthScope>,
) -> Option<PathBuf> {
    let auth_root = data_dir.join("auth");
    let mut stack = vec![auth_root];
    let mut fallback: Option<PathBuf> = None;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) != Some("auth.json") {
                continue;
            }
            if let Some(scope) = preferred {
                if namespace_matches_scope(&path, scope) {
                    return Some(path);
                }
                if fallback.is_none() {
                    fallback = Some(path);
                }
                continue;
            }
            return Some(path);
        }
    }
    fallback
}

/// 判断 `auth.json` 所在命名空间是否与目标 buildEnv/region 匹配。
fn namespace_matches_scope(auth_file: &std::path::Path, scope: &MinimaxCodeAuthScope) -> bool {
    let Some(namespace) = auth_file.parent() else {
        return false;
    };
    let Some(region_dir) = namespace.parent() else {
        return false;
    };
    let Some(env_dir) = region_dir.parent() else {
        return false;
    };
    env_dir.file_name().and_then(|name| name.to_str()) == Some(scope.build_env.as_str())
        && region_dir.file_name().and_then(|name| name.to_str()) == Some(scope.region.as_str())
}

#[cfg(test)]
mod minimax_code_instance_tests {
    use super::*;

    fn temp_data_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "cockpit-minimax-code-instance-{}-{}",
            tag,
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&base).expect("create temp data dir");
        base
    }

    /// data root 必须尊重用户显式设置的 `MINIMAX_DATA_DIR`。
    #[test]
    fn default_data_dir_follows_explicit_env() {
        let previous = std::env::var("MINIMAX_DATA_DIR").ok();
        let temp = temp_data_dir("env");
        std::env::set_var("MINIMAX_DATA_DIR", &temp);
        let resolved = get_default_minimax_code_data_dir().expect("resolve data dir");
        match previous {
            Some(value) => std::env::set_var("MINIMAX_DATA_DIR", value),
            None => std::env::remove_var("MINIMAX_DATA_DIR"),
        }
        assert_eq!(resolved, temp);
    }

    /// 未设置环境变量时回落到 `~/.minimax`。
    #[test]
    fn default_data_dir_falls_back_to_home() {
        let previous = std::env::var("MINIMAX_DATA_DIR").ok();
        let previous_mavis = std::env::var("MAVIS_DATA_DIR").ok();
        std::env::remove_var("MINIMAX_DATA_DIR");
        std::env::remove_var("MAVIS_DATA_DIR");
        let resolved = get_default_minimax_code_data_dir().expect("resolve data dir");
        if let Some(value) = previous {
            std::env::set_var("MINIMAX_DATA_DIR", value);
        }
        if let Some(value) = previous_mavis {
            std::env::set_var("MAVIS_DATA_DIR", value);
        }
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(resolved, home.join(".minimax"));
    }

    /// `auth.json` 解析：已有文件时复用其命名空间，否则回落到默认命名空间。
    #[test]
    fn auth_file_path_reuses_existing_namespace() {
        let data_dir = temp_data_dir("authpath");
        let custom = data_dir
            .join("auth")
            .join("staging")
            .join("en")
            .join("mcode-public");
        fs::create_dir_all(&custom).expect("create namespace dir");
        let existing = custom.join("auth.json");
        fs::write(&existing, b"{\"schemaVersion\":1,\"records\":{}}").expect("write auth.json");

        assert_eq!(
            resolve_minimax_code_auth_file_path(&data_dir),
            existing,
            "已有 auth.json 时应复用其所在命名空间目录"
        );

        let fresh = temp_data_dir("authpath-fresh");
        let fallback = resolve_minimax_code_auth_file_path(&fresh);
        assert_eq!(
            fallback,
            fresh.join("auth").join("prod").join("cn").join("mcode-public").join("auth.json")
        );
    }

    /// 已有 `auth-state.json` 时，buildEnv/region 必须从它读取（而不是硬编码）。
    #[test]
    fn auth_file_path_prefers_scope_from_auth_state() {
        let data_dir = temp_data_dir("authstate");
        // 一个非默认的命名空间，只有 auth-state.json，没有 auth.json。
        let namespace = data_dir.join("auth").join("staging").join("en").join("mcode-public");
        fs::create_dir_all(&namespace).expect("create namespace dir");
        fs::write(
            namespace.join("auth-state.json"),
            b"{\"schemaVersion\":2,\"status\":\"authenticated\",\"storeKind\":\"file\",\
              \"clientId\":\"mcode-public\",\"scopes\":[\"agent.default\"],\
              \"audience\":\"agent-backend\",\"buildEnv\":\"staging\",\"region\":\"en\",\
              \"generation\":1,\"expiresAtMs\":1792767948636}",
        )
        .expect("write auth-state.json");

        assert_eq!(
            resolve_minimax_code_auth_file_path(&data_dir),
            namespace.join("auth.json"),
            "buildEnv/region 应从 auth-state.json 解析"
        );
    }

    /// 实例存储 create/list/update/delete 往返。
    ///
    /// 直接走 `*_at(data_dir)` 内部变体，避开 `get_data_dir()` 的进程级 `OnceLock` 缓存
    /// （该缓存在单测进程里可能已被其它测试解析成真实用户目录）。
    #[test]
    fn instance_store_round_trip() {
        let base = temp_data_dir("store");
        let store_path = instances_path_for(&base);
        let load = || {
            instance_store::load_instance_store(&store_path, MINIMAX_CODE_INSTANCES_FILE)
                .expect("load store")
        };
        let save = |store: &InstanceStore| {
            instance_store::save_instance_store(
                &store_path,
                MINIMAX_CODE_INSTANCES_FILE,
                store,
            )
            .expect("save store")
        };

        let instance_dir = base.join("instance-roundtrip");
        let name = instance_store::normalize_name("round-trip").expect("normalize name");
        instance_store::ensure_unique(&load(), &name, &instance_dir.to_string_lossy(), None)
            .expect("ensure unique");

        let instance = InstanceProfile {
            id: Uuid::new_v4().to_string(),
            name: name.clone(),
            user_data_dir: instance_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: crate::models::InstanceLaunchMode::App,
            app_speed: crate::models::codex::CodexAppSpeed::Standard,
            created_at: Utc::now().timestamp_millis(),
            last_launched_at: None,
            last_pid: None,
        };

        // 空白实例：目录必须被创建，且不绑定账号。
        fs::create_dir_all(&instance_dir).expect("create instance dir");
        let mut store = load();
        store.instances.push(instance.clone());
        save(&store);

        let loaded = load();
        assert_eq!(loaded.instances.len(), 1);
        assert_eq!(loaded.instances[0].id, instance.id);
        assert_eq!(loaded.instances[0].name, "round-trip");

        // update
        let index = load()
            .instances
            .iter()
            .position(|item| item.id == instance.id)
            .expect("instance present");
        let mut store = load();
        store.instances[index].name =
            instance_store::normalize_name("renamed").expect("normalize renamed");
        store.instances[index].extra_args = "--foo".to_string();
        store.instances[index].bind_account_id = Some("acc-2".to_string());
        save(&store);

        let updated = load().instances.into_iter().next().expect("instance");
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.extra_args, "--foo");
        assert_eq!(updated.bind_account_id.as_deref(), Some("acc-2"));

        // after start
        let mut store = load();
        store.instances[0].last_pid = Some(4242);
        store.instances[0].last_launched_at = Some(Utc::now().timestamp_millis());
        save(&store);
        assert_eq!(load().instances[0].last_pid, Some(4242));

        // delete：实例记录必须从存储中移除。
        // 目录本身由 `delete_instance_directory` 的 fail-closed 守卫保护 —— 位于受管实例
        // 存储之外（例如临时目录）时只移除注册、保留目录，避免误删用户数据。
        let mut store = load();
        let removed = store.instances.remove(0);
        save(&store);
        assert!(load().instances.is_empty(), "删除实例应清空实例列表");
        assert_eq!(removed.id, instance.id);
        // 清理临时目录（真实托管实例目录会由 delete_instance_directory 移入回收站）。
        let _ = fs::remove_dir_all(&instance_dir);
        assert!(!instance_dir.exists());
    }

    /// `create_instance` / `delete_instance` 真实往返（空实例模式）。
    ///
    /// 布局与生产一致：`<data root>/instances/minimax_code/<instance_id>`。
    /// 这样 `record_creation` 推导出的根正好落在受管实例存储下，删除守卫才会放行。
    ///
    /// 通过 `COCKPIT_TOOLS_DATA_DIR` 把数据根指向临时目录：单测构建里
    /// `account::get_data_dir()` 走 `test_support::fallback_data_dir()`，而
    /// `data_paths::managed_instances_root_dir()` 仍解析 `%APPDATA%\.cockpit_tools`，
    /// 两者不一致会让删除守卫找不到受管根。生产构建里两者同源，不存在该差异。
    #[test]
    fn create_and_delete_empty_instance_uses_managed_root() {
        let temp_root = temp_data_dir("managed-round-trip");
        let previous = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
        let previous_test = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").ok();
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &temp_root);
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp_root);

        let root = get_default_instances_root_dir().expect("managed root");
        assert_eq!(
            root,
            temp_root.join("instances").join("minimax_code"),
            "受管实例根应跟随 COCKPIT_TOOLS_DATA_DIR"
        );
        let instance_dir = root.join(format!("test-{}", Uuid::new_v4().simple()));

        // 先清理可能残留的同名目录，避免守卫误判。
        let _ = fs::remove_dir_all(&instance_dir);

        let instance = create_instance(CreateInstanceParams {
            name: "managed-round-trip".to_string(),
            user_data_dir: instance_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            copy_source_instance_id: None,
            init_mode: Some("empty".to_string()),
        })
        .expect("create instance");

        assert!(instance_dir.is_dir(), "empty 模式应创建实例 data root");
        assert!(instance.bind_account_id.is_none());
        assert!(
            load_instance_store()
                .expect("load store")
                .instances
                .iter()
                .any(|item| item.id == instance.id),
            "新实例应写入实例存储"
        );

        delete_instance(&instance.id).expect("delete instance");
        assert!(!instance_dir.exists(), "删除实例应移除其 data root");
        assert!(
            !load_instance_store()
                .expect("reload")
                .instances
                .iter()
                .any(|item| item.id == instance.id),
            "删除后实例不应仍存在于存储中"
        );

        // 恢复环境变量，避免影响同进程内的其它测试。
        match previous {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
        }
        match previous_test {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        let _ = fs::remove_dir_all(&temp_root);
    }

    /// 默认实例设置往返（同样走 `*_at` 内部变体）。
    #[test]
    fn default_settings_round_trip_at_injected_root() {
        let base = temp_data_dir("default-settings");
        let updated = update_default_settings_at(
            &base,
            Some(Some("acc-9".to_string())),
            Some("  --new-window  ".to_string()),
        )
        .expect("update default settings");
        assert_eq!(updated.bind_account_id.as_deref(), Some("acc-9"));
        assert_eq!(updated.extra_args, "--new-window");
        assert!(instances_path_for(&base).is_file());
    }

    /// 每个实例的 `auth.json` 都落在自己的 data root 下（多开隔离的根本保证）。
    #[test]
    fn each_instance_resolves_its_own_auth_file() {
        let first = temp_data_dir("isolation-a");
        let second = temp_data_dir("isolation-b");
        let first_auth = resolve_minimax_code_auth_file_path(&first);
        let second_auth = resolve_minimax_code_auth_file_path(&second);

        assert!(first_auth.starts_with(&first));
        assert!(second_auth.starts_with(&second));
        assert_ne!(first_auth, second_auth);
        assert!(first_auth.ends_with("auth.json"));
    }
}
