use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt as _;
use url::Url;

use crate::modules;
use crate::modules::config::{
    self, CloseWindowBehavior, MinimizeWindowBehavior, TrayIconStyle, UserConfig,
    DEFAULT_REPORT_PORT, DEFAULT_WS_PORT,
};
use crate::modules::web_report;
use crate::modules::websocket;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const AUTO_BACKUP_DIR_NAME: &str = "backups";

static GENERAL_CONFIG_SAVE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn lock_general_config_transaction() -> Result<std::sync::MutexGuard<'static, ()>, String>
{
    GENERAL_CONFIG_SAVE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "通用配置保存锁已损坏".to_string())
}

/// 网络服务配置（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// WebSocket 是否启用
    pub ws_enabled: bool,
    /// 配置的端口
    pub ws_port: u16,
    /// 实际运行的端口（可能与配置不同）
    pub actual_port: Option<u16>,
    /// 默认端口
    pub default_port: u16,
    /// 网页查询服务是否启用
    pub report_enabled: bool,
    /// 网页查询服务配置端口
    pub report_port: u16,
    /// 网页查询服务实际运行端口（可能与配置不同）
    pub report_actual_port: Option<u16>,
    /// 网页查询服务默认端口
    pub report_default_port: u16,
    /// 网页查询服务访问令牌
    pub report_token: String,
    /// 全局代理开关
    pub global_proxy_enabled: bool,
    /// 全局代理地址（如 http://127.0.0.1:7890）
    pub global_proxy_url: String,
    /// NO_PROXY 白名单（逗号分隔）
    pub global_proxy_no_proxy: String,
}

/// 通用设置配置（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// 界面语言
    pub language: String,
    /// 默认终端
    pub default_terminal: String,
    /// 应用主题: "light", "dark", "system"
    pub theme: String,
    /// 主题色套件 id
    pub theme_color: String,
    /// 是否允许外连
    pub external_network_enabled: bool,
    /// WebDAV 允许域名（逗号分隔）
    pub webdav_allowed_domains: String,
    /// 是否减少界面动画
    pub reduced_motion_enabled: bool,
    /// 界面缩放比例（WebView Zoom）
    pub ui_scale: f64,
    /// 自动刷新间隔（分钟），-1 表示禁用
    pub auto_refresh_minutes: i32,
    /// Codex 自动刷新间隔（分钟），-1 表示禁用
    pub codex_auto_refresh_minutes: i32,
    /// Codex 切号时是否同步覆盖 WSL 配置 (Windows Only)
    pub codex_sync_wsl: bool,
    /// 是否启用 Codex 客户端中的 API 服务额度显示注入
    pub codex_app_ui_injection_enabled: bool,
    /// Codex WSL 配置目录 (Windows Only)
    pub codex_wsl_config_dir: String,
    /// Zed 自动刷新间隔（分钟），-1 表示禁用
    pub zed_auto_refresh_minutes: i32,
    /// GitHub Copilot 自动刷新间隔（分钟），-1 表示禁用
    pub ghcp_auto_refresh_minutes: i32,
    /// Windsurf 自动刷新间隔（分钟），-1 表示禁用
    pub windsurf_auto_refresh_minutes: i32,
    /// Kiro 自动刷新间隔（分钟），-1 表示禁用
    pub kiro_auto_refresh_minutes: i32,
    /// Cursor 自动刷新间隔（分钟），-1 表示禁用
    pub cursor_auto_refresh_minutes: i32,
    /// Grok CLI 自动刷新间隔（分钟），-1 表示禁用
    pub grok_auto_refresh_minutes: i32,
    /// 默认实例切号时是否同步写入官方 ~/.grok/auth.json
    pub grok_sync_official_auth_on_switch: bool,
    /// Claude 自动刷新间隔（分钟），-1 表示禁用
    pub claude_auto_refresh_minutes: i32,
    /// CodeBuddy 自动刷新间隔（分钟），-1 表示禁用
    pub codebuddy_auto_refresh_minutes: i32,
    /// CodeBuddy CN 自动刷新间隔（分钟），-1 表示禁用
    pub codebuddy_cn_auto_refresh_minutes: i32,
    /// WorkBuddy 自动刷新间隔（分钟），-1 表示禁用
    pub workbuddy_auto_refresh_minutes: i32,
    /// Qoder 自动刷新间隔（分钟），-1 表示禁用
    pub qoder_auto_refresh_minutes: i32,
    /// ZCode 自动刷新间隔（分钟），-1 表示禁用
    pub zcode_auto_refresh_minutes: i32,
    /// Trae 自动刷新间隔（分钟），-1 表示禁用
    pub trae_auto_refresh_minutes: i32,
    pub trae_solo_auto_refresh_minutes: i32,
    pub trae_cn_auto_refresh_minutes: i32,
    pub trae_solo_cn_auto_refresh_minutes: i32,
    /// 窗口关闭行为: "ask", "minimize", "quit"
    pub close_behavior: String,
    /// 窗口最小化行为（macOS）: "dock_and_tray", "tray_only"
    pub minimize_behavior: String,
    /// 是否隐藏 Dock 图标（macOS）
    pub hide_dock_icon: bool,
    /// 菜单栏图标样式（macOS）: "template", "color"
    pub tray_icon_style: String,
    /// 是否在启动时显示悬浮卡片
    pub floating_card_show_on_startup: bool,
    /// 是否在启动后自动最小化主窗口
    pub startup_minimized: bool,
    /// 是否记住主窗口尺寸和位置
    pub remember_main_window_state: bool,
    /// 启动默认页面：`last` 或具体页面 id
    pub startup_page: String,
    /// 悬浮卡片是否默认置顶
    pub floating_card_always_on_top: bool,
    /// 是否启用应用开机自启动
    pub app_auto_launch_enabled: bool,
    /// 是否启用后台账号授权保活
    pub token_keeper_enabled: bool,
    /// 是否启用本机账号变更后自动导入
    pub auto_import_from_local_enabled: bool,
    /// 是否在应用启动后触发 Antigravity IDE 唤醒
    pub antigravity_startup_wakeup_enabled: bool,
    /// Antigravity IDE 启动后唤醒延时（秒）
    pub antigravity_startup_wakeup_delay_seconds: i32,
    /// 是否在应用启动后触发 Codex 唤醒
    pub codex_startup_wakeup_enabled: bool,
    /// Codex 启动后唤醒延时（秒）
    pub codex_startup_wakeup_delay_seconds: i32,
    /// 关闭悬浮卡片前是否显示确认弹框
    pub floating_card_confirm_on_close: bool,
    /// OpenCode 启动路径（为空则使用默认路径）
    pub opencode_app_path: String,
    /// Antigravity IDE 启动路径（为空则使用默认路径）
    pub antigravity_app_path: String,
    /// Codex 启动路径（为空则使用默认路径）
    pub codex_app_path: String,
    /// Claude 桌面应用启动路径（为空则使用默认路径）
    pub claude_app_path: String,
    /// Claude 桌面应用扫描范围（每行一个目录）
    pub claude_app_scan_roots: String,
    /// 切换 Codex 后需联动重启的指定应用路径
    pub codex_specified_app_path: String,
    /// Zed 启动路径（为空则使用默认路径）
    pub zed_app_path: String,
    /// VS Code 启动路径（为空则使用默认路径）
    pub vscode_app_path: String,
    /// Windsurf 启动路径（为空则使用默认路径）
    pub windsurf_app_path: String,
    /// Kiro 启动路径（为空则使用默认路径）
    pub kiro_app_path: String,
    /// Cursor 启动路径（为空则使用默认路径）
    pub cursor_app_path: String,
    /// CodeBuddy 启动路径（为空则使用默认路径）
    pub codebuddy_app_path: String,
    /// 切换 CodeBuddy 账号时是否在本机账号间合并本地会话
    pub codebuddy_share_sessions_on_switch: bool,
    /// CodeBuddy CN 启动路径（为空则使用默认路径）
    pub codebuddy_cn_app_path: String,
    /// Qoder 启动路径（为空则使用默认路径）
    pub qoder_app_path: String,
    /// ZCode 启动路径（为空则使用默认路径）
    pub zcode_app_path: String,
    /// Trae 启动路径（为空则使用默认路径）
    pub trae_app_path: String,
    /// Trae Windows 应用扫描范围（每行一个目录）
    pub trae_solo_app_path: String,
    pub trae_cn_app_path: String,
    pub trae_solo_cn_app_path: String,
    pub trae_app_scan_roots: String,
    pub trae_solo_app_scan_roots: String,
    pub trae_cn_app_scan_roots: String,
    pub trae_solo_cn_app_scan_roots: String,
    /// WorkBuddy 启动路径（为空则使用默认路径）
    pub workbuddy_app_path: String,
    /// 切换 WorkBuddy 账号时是否在本机账号间合并本地会话
    pub workbuddy_share_sessions_on_switch: bool,
    /// 切换 Codex 时是否自动重启 OpenCode
    pub opencode_sync_on_switch: bool,
    /// 切换 Codex 时是否覆盖 OpenCode 登录信息
    pub opencode_auth_overwrite_on_switch: bool,
    /// 切换 GitHub Copilot 时是否自动重启 OpenCode
    pub ghcp_opencode_sync_on_switch: bool,
    /// 切换 GitHub Copilot 时是否覆盖 OpenCode 登录信息
    pub ghcp_opencode_auth_overwrite_on_switch: bool,
    /// 切换 GitHub Copilot 时是否自动启动 GitHub Copilot
    pub ghcp_launch_on_switch: bool,
    /// 切换 Codex 时是否覆盖 OpenClaw 登录信息
    pub openclaw_auth_overwrite_on_switch: bool,
    pub hermes_auth_overwrite_on_switch: bool,
    /// 切换 Codex 时是否自动启动/重启 Codex App
    pub codex_launch_on_switch: bool,
    /// 切换 Antigravity IDE 时是否自动启动/重启应用
    pub antigravity_launch_on_switch: bool,
    /// 切换 Codex 时是否自动重启指定应用
    pub codex_restart_specified_app_on_switch: bool,
    /// 是否在 Codex 总览中显示 API 服务入口
    pub codex_local_access_entry_visible: bool,
    /// 是否显示顶部推广位
    pub top_right_ad_visible: bool,
    /// Antigravity 切号是否启用“本地落盘 + 扩展无感”且不重启
    pub antigravity_dual_switch_no_restart_enabled: bool,
    /// 是否启用自动切号
    pub auto_switch_enabled: bool,
    /// 自动切号阈值（百分比）
    pub auto_switch_threshold: i32,
    /// 是否启用 Credits 阈值自动切号
    pub auto_switch_credits_enabled: bool,
    /// Credits 自动切号阈值（剩余值）
    pub auto_switch_credits_threshold: i32,
    /// 自动切号触发模式：any_group | selected_groups
    pub auto_switch_scope_mode: String,
    /// 自动切号指定模型分组（分组 ID）
    pub auto_switch_selected_group_ids: Vec<String>,
    /// 自动切号账号范围模式：all_accounts | selected_accounts
    pub auto_switch_account_scope_mode: String,
    /// 自动切号指定账号（账号 ID）
    pub auto_switch_selected_account_ids: Vec<String>,
    /// 是否启用 Codex 自动切号
    pub codex_auto_switch_enabled: bool,
    /// Codex primary_window 自动切号阈值（百分比）
    pub codex_auto_switch_primary_threshold: i32,
    /// Codex secondary_window 自动切号阈值（百分比）
    pub codex_auto_switch_secondary_threshold: i32,
    /// Codex 自动切号账号范围模式：all_accounts | selected_accounts
    pub codex_auto_switch_account_scope_mode: String,
    /// Codex 自动切号指定账号（账号 ID）
    pub codex_auto_switch_selected_account_ids: Vec<String>,
    /// 是否启用配额预警通知
    pub quota_alert_enabled: bool,
    /// 配额预警阈值（百分比）
    pub quota_alert_threshold: i32,
    /// 是否启用 Codex 配额预警通知
    pub codex_quota_alert_enabled: bool,
    /// Codex 配额预警阈值（百分比）
    pub codex_quota_alert_threshold: i32,
    /// 是否启用 Zed 配额预警通知
    pub zed_quota_alert_enabled: bool,
    /// Zed 配额预警阈值（百分比）
    pub zed_quota_alert_threshold: i32,
    /// Codex primary_window 配额预警阈值（百分比）
    pub codex_quota_alert_primary_threshold: i32,
    /// Codex secondary_window 配额预警阈值（百分比）
    pub codex_quota_alert_secondary_threshold: i32,
    /// 是否启用 GitHub Copilot 配额预警通知
    pub ghcp_quota_alert_enabled: bool,
    /// GitHub Copilot 配额预警阈值（百分比）
    pub ghcp_quota_alert_threshold: i32,
    /// 是否启用 Windsurf 配额预警通知
    pub windsurf_quota_alert_enabled: bool,
    /// Windsurf 配额预警阈值（百分比）
    pub windsurf_quota_alert_threshold: i32,
    /// 是否启用 Kiro 配额预警通知
    pub kiro_quota_alert_enabled: bool,
    /// Kiro 配额预警阈值（百分比）
    pub kiro_quota_alert_threshold: i32,
    /// 是否启用 Cursor 配额预警通知
    pub cursor_quota_alert_enabled: bool,
    /// Cursor 配额预警阈值（百分比）
    pub cursor_quota_alert_threshold: i32,
    /// 是否启用 Grok CLI 配额预警通知
    pub grok_quota_alert_enabled: bool,
    /// Grok CLI 配额预警阈值（百分比）
    pub grok_quota_alert_threshold: i32,
    /// 是否启用 Claude 配额预警通知
    pub claude_quota_alert_enabled: bool,
    /// Claude 配额预警阈值（百分比）
    pub claude_quota_alert_threshold: i32,
    /// 是否启用 CodeBuddy 配额预警通知
    pub codebuddy_quota_alert_enabled: bool,
    /// CodeBuddy 配额预警阈值（百分比）
    pub codebuddy_quota_alert_threshold: i32,
    /// 是否启用 CodeBuddy CN 配额预警通知
    pub codebuddy_cn_quota_alert_enabled: bool,
    /// CodeBuddy CN 配额预警阈值（百分比）
    pub codebuddy_cn_quota_alert_threshold: i32,
    /// 是否启用 Qoder 配额预警通知
    pub qoder_quota_alert_enabled: bool,
    /// Qoder 配额预警阈值（百分比）
    pub qoder_quota_alert_threshold: i32,
    /// 是否启用 Trae 配额预警通知
    pub trae_quota_alert_enabled: bool,
    /// Trae 配额预警阈值（百分比）
    pub trae_quota_alert_threshold: i32,
    pub trae_solo_quota_alert_enabled: bool,
    pub trae_solo_quota_alert_threshold: i32,
    pub trae_cn_quota_alert_enabled: bool,
    pub trae_cn_quota_alert_threshold: i32,
    pub trae_solo_cn_quota_alert_enabled: bool,
    pub trae_solo_cn_quota_alert_threshold: i32,
    /// 是否启用 WorkBuddy 配额预警通知
    pub workbuddy_quota_alert_enabled: bool,
    /// WorkBuddy 配额预警阈值（百分比）
    pub workbuddy_quota_alert_threshold: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityInstalledVersionInfo {
    pub product_name: String,
    pub version: String,
    pub app_path: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AntigravityVersionScanMode {
    Quick,
    Full,
}

/// 自动备份设置（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupSettings {
    /// 是否启用自动备份
    pub enabled: bool,
    /// 是否包含账号数据
    pub include_accounts: bool,
    /// 是否包含配置数据
    pub include_config: bool,
    /// 备份保留天数
    pub retention_days: i32,
    /// 最近一次备份时间（ISO 8601）
    pub last_backup_at: Option<String>,
    /// 备份目录绝对路径
    pub directory_path: String,
}

/// 自动备份文件条目（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupFileEntry {
    /// 文件名
    pub file_name: String,
    /// 文件绝对路径
    pub path: String,
    /// 文件类型：json / zip
    pub file_kind: String,
    /// 文件大小（字节）
    pub size_bytes: u64,
    /// 最后修改时间（毫秒时间戳）
    pub modified_at_ms: Option<i64>,
    /// 同名 ZIP 备份文件名
    pub archive_file_name: Option<String>,
    /// 同名 ZIP 备份绝对路径
    pub archive_path: Option<String>,
    /// 同名 ZIP 备份大小（字节）
    pub archive_size_bytes: Option<u64>,
    /// 备份内包含账号的平台摘要
    pub platforms: Vec<AutoBackupPlatformEntry>,
}

/// 自动备份内的平台摘要（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupPlatformEntry {
    /// 平台 ID
    pub platform: String,
    /// 账号数量
    pub account_count: u64,
}

/// WebDAV 备份同步设置（前端使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavSyncSettings {
    /// 是否启用自动同步
    pub enabled: bool,
    /// WebDAV 服务地址
    pub url: String,
    /// WebDAV 用户名
    pub username: String,
    /// 本地配置中是否已保存密码
    pub has_password: bool,
    /// WebDAV 远端备份目录
    pub remote_dir: String,
    /// 最近一次上传时间
    pub last_upload_at: Option<String>,
    /// 最近一次上传文件名
    pub last_upload_file_name: Option<String>,
    /// 最近一次下载时间
    pub last_download_at: Option<String>,
    /// 最近一次下载文件名
    pub last_download_file_name: Option<String>,
    /// 备份保留天数
    pub retention_days: i32,
}

const DEFAULT_UI_SCALE: f64 = 1.0;
const MIN_UI_SCALE: f64 = 0.8;
const MAX_UI_SCALE: f64 = 2.0;
const MAX_STARTUP_WAKEUP_DELAY_SECONDS: i32 = 24 * 60 * 60;
const ANTIGRAVITY_VERSION_BADGE_TIMEOUT_MS: u64 = 1200;
const ANTIGRAVITY_VERSION_FULL_SCAN_TIMEOUT_MS: u64 = 30_000;
const AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
static ANTIGRAVITY_VERSION_INFO_CACHE: OnceLock<
    Mutex<HashMap<String, AntigravityInstalledVersionInfo>>,
> = OnceLock::new();

fn trim_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn json_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(trim_non_empty)
    })
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root_for_metadata(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_string_lossy();
    let app_idx = path_str.find(".app")?;
    let root = PathBuf::from(&path_str[..app_idx + 4]);
    root.exists().then_some(root)
}

#[cfg(target_os = "macos")]
fn read_macos_plist_string(path: &Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("plutil")
        .arg("-p")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let prefix = format!("\"{}\"", key);
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with(&prefix) {
            continue;
        }
        let value = line.split("=>").nth(1)?.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn antigravity_product_json_candidates(root: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("product.json"),
            root.join("resources").join("app").join("product.json"),
            root.join("app").join("product.json"),
        ]
    }

    #[cfg(not(target_os = "macos"))]
    {
        vec![
            root.join("resources").join("app").join("product.json"),
            root.join("app").join("product.json"),
        ]
    }
}

fn read_antigravity_product_json_metadata(root: &Path) -> Option<AntigravityInstalledVersionInfo> {
    for path in antigravity_product_json_candidates(root) {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(version) = json_string_field(&value, &["ideVersion", "version"]) else {
            continue;
        };
        let product_name = json_string_field(
            &value,
            &["nameShort", "nameLong", "productName", "applicationName"],
        )
        .unwrap_or_else(|| "Antigravity".to_string());
        return Some(AntigravityInstalledVersionInfo {
            product_name,
            version,
            app_path: root.to_string_lossy().to_string(),
            source: "product.json".to_string(),
        });
    }
    None
}

#[cfg(target_os = "macos")]
fn read_antigravity_macos_bundle_metadata(root: &Path) -> Option<AntigravityInstalledVersionInfo> {
    let plist_path = root.join("Contents").join("Info.plist");
    if !plist_path.exists() {
        return None;
    }

    let version = read_macos_plist_string(&plist_path, "CFBundleShortVersionString")
        .or_else(|| read_macos_plist_string(&plist_path, "CFBundleVersion"))?;
    let product_name = read_macos_plist_string(&plist_path, "CFBundleDisplayName")
        .or_else(|| read_macos_plist_string(&plist_path, "CFBundleName"))
        .unwrap_or_else(|| "Antigravity".to_string());

    Some(AntigravityInstalledVersionInfo {
        product_name,
        version,
        app_path: root.to_string_lossy().to_string(),
        source: "Info.plist".to_string(),
    })
}

#[cfg(target_os = "windows")]
fn find_antigravity_windows_exe(root: &Path) -> Option<PathBuf> {
    if root.is_file() {
        return Some(root.to_path_buf());
    }

    let candidates = [
        root.join("Antigravity.exe"),
        root.join("Antigravity IDE.exe"),
        root.join("antigravity.exe"),
        root.join("antigravity-ide.exe"),
        root.join("Electron.exe"),
    ];
    candidates.into_iter().find(|path| path.exists())
}

#[cfg(target_os = "windows")]
fn read_powershell_json_for_antigravity_exe(
    exe_path: &Path,
    script: &str,
) -> Option<serde_json::Value> {
    let _spawn_guard = modules::app_lifecycle::acquire_process_spawn_guard("PowerShell").ok()?;
    let mut command = std::process::Command::new("powershell");
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("COCKPIT_ANTIGRAVITY_EXE_PATH", exe_path.as_os_str())
        .output()
        .ok()?;
    if !output.status.success() {
        modules::logger::log_warn(&format!(
            "[Antigravity] Windows version metadata PowerShell probe failed: status={}",
            output.status
        ));
        return None;
    }

    serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
}

#[cfg(target_os = "windows")]
fn build_antigravity_windows_version_info(
    value: serde_json::Value,
    exe_path: &Path,
    source: &str,
) -> Option<AntigravityInstalledVersionInfo> {
    let version = json_string_field(&value, &["ProductVersion", "FileVersion", "DisplayVersion"])?;
    let product_name = json_string_field(&value, &["ProductName", "DisplayName"])
        .unwrap_or_else(|| "Antigravity".to_string());

    Some(AntigravityInstalledVersionInfo {
        product_name,
        version,
        app_path: exe_path.to_string_lossy().to_string(),
        source: source.to_string(),
    })
}

#[cfg(target_os = "windows")]
fn read_antigravity_windows_uninstall_metadata(
    exe_path: &Path,
) -> Option<AntigravityInstalledVersionInfo> {
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

function Normalize-RegistryPath([string]$value) {
  if ([string]::IsNullOrWhiteSpace($value)) { return $null }
  $clean = $value.Trim().Trim('"')
  $clean = $clean -replace ',\d+$',''
  try { return [System.IO.Path]::GetFullPath($clean) } catch { return $clean }
}

$exe = [Environment]::GetEnvironmentVariable('COCKPIT_ANTIGRAVITY_EXE_PATH', 'Process')
if ([string]::IsNullOrWhiteSpace($exe)) { exit 3 }
$exe = [System.IO.Path]::GetFullPath($exe)

$roots = @(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
)

$match = Get-ItemProperty -Path $roots -ErrorAction SilentlyContinue |
  Where-Object {
    $_.DisplayName -like 'Antigravity*' -and (
      ((Normalize-RegistryPath $_.DisplayIcon) -ieq $exe) -or
      ($_.InstallLocation -and $exe.StartsWith(
        (Normalize-RegistryPath $_.InstallLocation).TrimEnd('\') + '\',
        [System.StringComparison]::OrdinalIgnoreCase
      ))
    )
  } |
  Select-Object -First 1

if (-not $match) { exit 4 }

[pscustomobject]@{
  DisplayName = $match.DisplayName
  DisplayVersion = $match.DisplayVersion
} | ConvertTo-Json -Compress
"#;

    let value = read_powershell_json_for_antigravity_exe(exe_path, script)?;
    build_antigravity_windows_version_info(value, exe_path, "UninstallRegistry")
}

#[cfg(target_os = "windows")]
fn read_antigravity_windows_exe_metadata(root: &Path) -> Option<AntigravityInstalledVersionInfo> {
    let exe_path = find_antigravity_windows_exe(root)?;
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$p = [Environment]::GetEnvironmentVariable('COCKPIT_ANTIGRAVITY_EXE_PATH', 'Process')
if ([string]::IsNullOrWhiteSpace($p)) { exit 3 }
if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { exit 2 }
$v = (Get-Item -LiteralPath $p).VersionInfo
if ([string]::IsNullOrWhiteSpace($v.ProductVersion) -and [string]::IsNullOrWhiteSpace($v.FileVersion)) { exit 4 }
[pscustomobject]@{
  ProductName = $v.ProductName
  ProductVersion = $v.ProductVersion
  FileVersion = $v.FileVersion
} | ConvertTo-Json -Compress
"#;

    read_powershell_json_for_antigravity_exe(&exe_path, script)
        .and_then(|value| build_antigravity_windows_version_info(value, &exe_path, "VersionInfo"))
        .or_else(|| read_antigravity_windows_uninstall_metadata(&exe_path))
}

fn normalize_antigravity_metadata_root(path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Some(root) = normalize_macos_app_root_for_metadata(path) {
            return Some(root);
        }
    }

    if path.is_file() {
        return path.parent().map(Path::to_path_buf);
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    None
}

fn push_unique_antigravity_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    let normalized_key = path.to_string_lossy().to_ascii_lowercase();
    let exists = candidates
        .iter()
        .any(|item| item.to_string_lossy().to_ascii_lowercase() == normalized_key);
    if !exists {
        candidates.push(path);
    }
}

fn normalize_antigravity_metadata_target(target: Option<&str>) -> Option<&'static str> {
    match target.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "antigravity" => Some("antigravity"),
        "antigravity_ide" | "antigravity-ide" | "ide" => Some("antigravity_ide"),
        _ => None,
    }
}

fn normalize_antigravity_version_scan_mode(raw: Option<&str>) -> AntigravityVersionScanMode {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "full" | "complete" => AntigravityVersionScanMode::Full,
        _ => AntigravityVersionScanMode::Quick,
    }
}

fn antigravity_version_cache() -> &'static Mutex<HashMap<String, AntigravityInstalledVersionInfo>> {
    ANTIGRAVITY_VERSION_INFO_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn antigravity_version_cache_key(target: Option<&str>) -> String {
    normalize_antigravity_metadata_target(target)
        .unwrap_or("all")
        .to_string()
}

fn cache_antigravity_installed_version_info(
    target: Option<&str>,
    info: &AntigravityInstalledVersionInfo,
) {
    if let Ok(mut cache) = antigravity_version_cache().lock() {
        cache.insert(antigravity_version_cache_key(target), info.clone());
    }
}

pub fn get_cached_antigravity_installed_version_info_for_target(
    target: Option<&str>,
) -> Option<AntigravityInstalledVersionInfo> {
    antigravity_version_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&antigravity_version_cache_key(target)).cloned())
}

fn antigravity_metadata_root_matches_target(root: &Path, target: Option<&str>) -> bool {
    let Some(target) = normalize_antigravity_metadata_target(target) else {
        return true;
    };
    let value = root.to_string_lossy().to_ascii_lowercase();
    match target {
        "antigravity" => {
            value.contains("antigravity.app")
                || value.ends_with("antigravity")
                || value.ends_with("antigravity.exe")
                || (root.is_dir()
                    && (root.join("Antigravity.exe").exists()
                        || root.join("antigravity.exe").exists()))
        }
        "antigravity_ide" => {
            value.contains("antigravity ide.app")
                || value.contains("antigravity ide")
                || value.contains("antigravity-ide")
                || (root.is_dir()
                    && (root.join("Antigravity IDE.exe").exists()
                        || root.join("antigravity-ide.exe").exists()))
        }
        _ => true,
    }
}

fn antigravity_metadata_candidates(
    target: Option<&str>,
    scan_mode: AntigravityVersionScanMode,
) -> Vec<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    let _ = scan_mode;

    let mut candidates = Vec::new();
    let config_path = config::get_user_config().antigravity_app_path;
    let config_path = config_path.trim();
    if !config_path.is_empty() {
        let config_path = Path::new(config_path);
        if let Some(root) = normalize_antigravity_metadata_root(config_path) {
            if antigravity_metadata_root_matches_target(&root, target) {
                push_unique_antigravity_candidate(&mut candidates, root);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let paths: &[&str] = match normalize_antigravity_metadata_target(target) {
            Some("antigravity") => &["/Applications/Antigravity.app"],
            Some("antigravity_ide") => &["/Applications/Antigravity IDE.app"],
            _ => &[
                "/Applications/Antigravity.app",
                "/Applications/Antigravity IDE.app",
            ],
        };
        for path in paths {
            let path = PathBuf::from(path);
            if path.exists() {
                push_unique_antigravity_candidate(&mut candidates, path);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let paths: &[&str] = match normalize_antigravity_metadata_target(target) {
            Some("antigravity") => &["/usr/share/antigravity", "/opt/antigravity"],
            Some("antigravity_ide") => &["/usr/share/antigravity-ide", "/opt/antigravity-ide"],
            _ => &[
                "/usr/share/antigravity",
                "/usr/share/antigravity-ide",
                "/opt/antigravity",
                "/opt/antigravity-ide",
            ],
        };
        for path in paths {
            let path = PathBuf::from(path);
            if path.exists() {
                push_unique_antigravity_candidate(&mut candidates, path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(local_appdata).join("Programs");
            match normalize_antigravity_metadata_target(target) {
                Some("antigravity") => roots.push(base.join("Antigravity")),
                Some("antigravity_ide") => roots.push(base.join("Antigravity IDE")),
                _ => {
                    roots.push(base.join("Antigravity"));
                    roots.push(base.join("Antigravity IDE"));
                }
            }
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            let base = PathBuf::from(program_files);
            match normalize_antigravity_metadata_target(target) {
                Some("antigravity") => roots.push(base.join("Antigravity")),
                Some("antigravity_ide") => roots.push(base.join("Antigravity IDE")),
                _ => {
                    roots.push(base.join("Antigravity"));
                    roots.push(base.join("Antigravity IDE"));
                }
            }
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            let base = PathBuf::from(program_files_x86);
            match normalize_antigravity_metadata_target(target) {
                Some("antigravity") => roots.push(base.join("Antigravity")),
                Some("antigravity_ide") => roots.push(base.join("Antigravity IDE")),
                _ => {
                    roots.push(base.join("Antigravity"));
                    roots.push(base.join("Antigravity IDE"));
                }
            }
        }
        for path in roots {
            if path.exists() {
                push_unique_antigravity_candidate(&mut candidates, path);
            }
        }

        if scan_mode == AntigravityVersionScanMode::Full {
            let push_detected_candidate = |candidates: &mut Vec<PathBuf>, path: PathBuf| {
                if let Some(root) = normalize_antigravity_metadata_root(&path) {
                    if antigravity_metadata_root_matches_target(&root, target) {
                        push_unique_antigravity_candidate(candidates, root);
                    }
                }
            };

            match normalize_antigravity_metadata_target(target) {
                Some("antigravity") => {
                    if let Some(path) =
                        crate::modules::process::detect_antigravity_legacy_exec_path()
                    {
                        push_detected_candidate(&mut candidates, path);
                    }
                }
                Some("antigravity_ide") => {
                    if let Some(path) = crate::modules::process::detect_antigravity_exec_path() {
                        push_detected_candidate(&mut candidates, path);
                    }
                }
                _ => {
                    if let Some(path) =
                        crate::modules::process::detect_antigravity_legacy_exec_path()
                    {
                        push_detected_candidate(&mut candidates, path);
                    }
                    if let Some(path) = crate::modules::process::detect_antigravity_exec_path() {
                        push_detected_candidate(&mut candidates, path);
                    }
                }
            }
        }
    }

    candidates
}

fn resolve_antigravity_installed_version_info_for_target_with_mode(
    target: Option<&str>,
    scan_mode: AntigravityVersionScanMode,
) -> Option<AntigravityInstalledVersionInfo> {
    for root in antigravity_metadata_candidates(target, scan_mode) {
        if let Some(info) = read_antigravity_product_json_metadata(&root) {
            return Some(info);
        }

        #[cfg(target_os = "macos")]
        if let Some(info) = read_antigravity_macos_bundle_metadata(&root) {
            return Some(info);
        }

        #[cfg(target_os = "windows")]
        if scan_mode == AntigravityVersionScanMode::Full {
            if let Some(info) = read_antigravity_windows_exe_metadata(&root) {
                return Some(info);
            }
        }
    }

    None
}

fn detect_and_cache_antigravity_installed_version_info_for_target(
    target: Option<&str>,
    scan_mode: AntigravityVersionScanMode,
) -> Option<AntigravityInstalledVersionInfo> {
    let info = resolve_antigravity_installed_version_info_for_target_with_mode(target, scan_mode);
    if let Some(ref value) = info {
        cache_antigravity_installed_version_info(target, value);
    }
    info
}

pub fn resolve_antigravity_installed_version_info_for_target(
    target: Option<&str>,
) -> Option<AntigravityInstalledVersionInfo> {
    detect_and_cache_antigravity_installed_version_info_for_target(
        target,
        AntigravityVersionScanMode::Full,
    )
}

fn resolve_antigravity_installed_version_info_quick_for_target(
    target: Option<&str>,
) -> Option<AntigravityInstalledVersionInfo> {
    detect_and_cache_antigravity_installed_version_info_for_target(
        target,
        AntigravityVersionScanMode::Quick,
    )
}

fn sanitize_startup_wakeup_delay_seconds(raw: i32) -> i32 {
    raw.clamp(0, MAX_STARTUP_WAKEUP_DELAY_SECONDS)
}

fn normalize_auto_switch_account_scope_mode(raw: &str) -> String {
    if raw.trim().to_lowercase() == AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED.to_string()
    } else {
        AUTO_SWITCH_ACCOUNT_SCOPE_ALL.to_string()
    }
}

fn normalize_auto_switch_selected_account_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn apply_codex_quota_alert_thresholds(
    config: &mut UserConfig,
    threshold: Option<i32>,
    primary_threshold: Option<i32>,
    secondary_threshold: Option<i32>,
) {
    let inherited_threshold = threshold.and_then(|value| {
        let changed = config.codex_quota_alert_threshold != value;
        config.codex_quota_alert_threshold = value;
        changed.then_some(value)
    });

    if let Some(value) = primary_threshold.or(inherited_threshold) {
        config.codex_quota_alert_primary_threshold = value;
    }
    if let Some(value) = secondary_threshold.or(inherited_threshold) {
        config.codex_quota_alert_secondary_threshold = value;
    }
}

fn is_general_config_patch_field(key: &str) -> bool {
    matches!(
        key,
        "language"
            | "default_terminal"
            | "theme"
            | "theme_color"
            | "external_network_enabled"
            | "webdav_allowed_domains"
            | "reduced_motion_enabled"
            | "ui_scale"
            | "auto_refresh_minutes"
            | "codex_auto_refresh_minutes"
            | "codex_sync_wsl"
            | "codex_app_ui_injection_enabled"
            | "codex_wsl_config_dir"
            | "zed_auto_refresh_minutes"
            | "ghcp_auto_refresh_minutes"
            | "windsurf_auto_refresh_minutes"
            | "kiro_auto_refresh_minutes"
            | "cursor_auto_refresh_minutes"
            | "grok_auto_refresh_minutes"
            | "grok_sync_official_auth_on_switch"
            | "claude_auto_refresh_minutes"
            | "codebuddy_auto_refresh_minutes"
            | "codebuddy_cn_auto_refresh_minutes"
            | "workbuddy_auto_refresh_minutes"
            | "qoder_auto_refresh_minutes"
            | "zcode_auto_refresh_minutes"
            | "trae_auto_refresh_minutes"
            | "trae_solo_auto_refresh_minutes"
            | "trae_cn_auto_refresh_minutes"
            | "trae_solo_cn_auto_refresh_minutes"
            | "close_behavior"
            | "minimize_behavior"
            | "hide_dock_icon"
            | "tray_icon_style"
            | "floating_card_show_on_startup"
            | "startup_minimized"
            | "remember_main_window_state"
            | "startup_page"
            | "floating_card_always_on_top"
            | "app_auto_launch_enabled"
            | "token_keeper_enabled"
            | "auto_import_from_local_enabled"
            | "antigravity_startup_wakeup_enabled"
            | "antigravity_startup_wakeup_delay_seconds"
            | "codex_startup_wakeup_enabled"
            | "codex_startup_wakeup_delay_seconds"
            | "floating_card_confirm_on_close"
            | "opencode_app_path"
            | "antigravity_app_path"
            | "codex_app_path"
            | "claude_app_path"
            | "claude_app_scan_roots"
            | "codex_specified_app_path"
            | "zed_app_path"
            | "vscode_app_path"
            | "windsurf_app_path"
            | "kiro_app_path"
            | "cursor_app_path"
            | "codebuddy_app_path"
            | "codebuddy_share_sessions_on_switch"
            | "codebuddy_cn_app_path"
            | "qoder_app_path"
            | "zcode_app_path"
            | "trae_app_path"
            | "trae_solo_app_path"
            | "trae_cn_app_path"
            | "trae_solo_cn_app_path"
            | "trae_app_scan_roots"
            | "trae_solo_app_scan_roots"
            | "trae_cn_app_scan_roots"
            | "trae_solo_cn_app_scan_roots"
            | "workbuddy_app_path"
            | "workbuddy_share_sessions_on_switch"
            | "opencode_sync_on_switch"
            | "opencode_auth_overwrite_on_switch"
            | "ghcp_opencode_sync_on_switch"
            | "ghcp_opencode_auth_overwrite_on_switch"
            | "ghcp_launch_on_switch"
            | "openclaw_auth_overwrite_on_switch"
            | "hermes_auth_overwrite_on_switch"
            | "codex_launch_on_switch"
            | "antigravity_launch_on_switch"
            | "codex_restart_specified_app_on_switch"
            | "codex_local_access_entry_visible"
            | "top_right_ad_visible"
            | "antigravity_dual_switch_no_restart_enabled"
            | "auto_switch_enabled"
            | "auto_switch_threshold"
            | "auto_switch_credits_enabled"
            | "auto_switch_credits_threshold"
            | "auto_switch_scope_mode"
            | "auto_switch_selected_group_ids"
            | "auto_switch_account_scope_mode"
            | "auto_switch_selected_account_ids"
            | "codex_auto_switch_enabled"
            | "codex_auto_switch_primary_threshold"
            | "codex_auto_switch_secondary_threshold"
            | "codex_auto_switch_account_scope_mode"
            | "codex_auto_switch_selected_account_ids"
            | "quota_alert_enabled"
            | "quota_alert_threshold"
            | "codex_quota_alert_enabled"
            | "codex_quota_alert_threshold"
            | "zed_quota_alert_enabled"
            | "zed_quota_alert_threshold"
            | "codex_quota_alert_primary_threshold"
            | "codex_quota_alert_secondary_threshold"
            | "ghcp_quota_alert_enabled"
            | "ghcp_quota_alert_threshold"
            | "windsurf_quota_alert_enabled"
            | "windsurf_quota_alert_threshold"
            | "kiro_quota_alert_enabled"
            | "kiro_quota_alert_threshold"
            | "cursor_quota_alert_enabled"
            | "cursor_quota_alert_threshold"
            | "grok_quota_alert_enabled"
            | "grok_quota_alert_threshold"
            | "claude_quota_alert_enabled"
            | "claude_quota_alert_threshold"
            | "codebuddy_quota_alert_enabled"
            | "codebuddy_quota_alert_threshold"
            | "codebuddy_cn_quota_alert_enabled"
            | "codebuddy_cn_quota_alert_threshold"
            | "qoder_quota_alert_enabled"
            | "qoder_quota_alert_threshold"
            | "trae_quota_alert_enabled"
            | "trae_quota_alert_threshold"
            | "trae_solo_quota_alert_enabled"
            | "trae_solo_quota_alert_threshold"
            | "trae_cn_quota_alert_enabled"
            | "trae_cn_quota_alert_threshold"
            | "trae_solo_cn_quota_alert_enabled"
            | "trae_solo_cn_quota_alert_threshold"
            | "workbuddy_quota_alert_enabled"
            | "workbuddy_quota_alert_threshold"
    )
}

fn json_i32(updates: &JsonMap<String, JsonValue>, key: &str) -> Result<Option<i32>, String> {
    let Some(value) = updates.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("配置字段 {} 必须为整数", key))?;
    Ok(Some(value))
}

fn apply_general_config_updates(
    current: &mut UserConfig,
    updates: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    for key in updates.keys() {
        if !is_general_config_patch_field(key) {
            return Err(format!("不支持的通用配置字段: {}", key));
        }
    }

    let mut value = serde_json::to_value(&*current)
        .map_err(|error| format!("序列化当前通用配置失败: {}", error))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "当前通用配置结构无效".to_string())?;
    for (key, value) in updates {
        object.insert(key.clone(), value.clone());
    }
    let mut next: UserConfig = serde_json::from_value(value)
        .map_err(|error| format!("通用配置字段类型无效: {}", error))?;

    if updates.contains_key("language") {
        next.language = next.language.to_lowercase();
    }
    if updates.contains_key("ui_scale") {
        next.ui_scale = sanitize_ui_scale(next.ui_scale);
    }
    if updates.contains_key("antigravity_startup_wakeup_delay_seconds") {
        next.antigravity_startup_wakeup_delay_seconds =
            sanitize_startup_wakeup_delay_seconds(next.antigravity_startup_wakeup_delay_seconds);
    }
    if updates.contains_key("codex_startup_wakeup_delay_seconds") {
        next.codex_startup_wakeup_delay_seconds =
            sanitize_startup_wakeup_delay_seconds(next.codex_startup_wakeup_delay_seconds);
    }
    if updates.contains_key("startup_page") {
        next.startup_page = config::normalize_startup_page(&next.startup_page);
    }
    if updates.contains_key("theme_color") {
        next.theme_color = config::normalize_theme_color(&next.theme_color);
    }
    if updates.contains_key("webdav_allowed_domains") {
        next.webdav_allowed_domains = next
            .webdav_allowed_domains
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
    }

    macro_rules! trim_string_field {
        ($key:literal, $field:ident) => {
            if updates.contains_key($key) {
                next.$field = next.$field.trim().to_string();
            }
        };
    }
    macro_rules! normalize_app_path_field {
        ($key:literal, $field:ident) => {
            if updates.contains_key($key) {
                next.$field = modules::process::normalize_windows_user_facing_path(&next.$field);
            }
        };
    }
    normalize_app_path_field!("opencode_app_path", opencode_app_path);
    normalize_app_path_field!("antigravity_app_path", antigravity_app_path);
    normalize_app_path_field!("codex_app_path", codex_app_path);
    normalize_app_path_field!("claude_app_path", claude_app_path);
    trim_string_field!("claude_app_scan_roots", claude_app_scan_roots);
    normalize_app_path_field!("codex_specified_app_path", codex_specified_app_path);
    normalize_app_path_field!("zed_app_path", zed_app_path);
    normalize_app_path_field!("vscode_app_path", vscode_app_path);
    normalize_app_path_field!("windsurf_app_path", windsurf_app_path);
    normalize_app_path_field!("kiro_app_path", kiro_app_path);
    normalize_app_path_field!("cursor_app_path", cursor_app_path);
    normalize_app_path_field!("codebuddy_app_path", codebuddy_app_path);
    normalize_app_path_field!("codebuddy_cn_app_path", codebuddy_cn_app_path);
    normalize_app_path_field!("qoder_app_path", qoder_app_path);
    normalize_app_path_field!("zcode_app_path", zcode_app_path);
    normalize_app_path_field!("trae_app_path", trae_app_path);
    normalize_app_path_field!("trae_solo_app_path", trae_solo_app_path);
    normalize_app_path_field!("trae_cn_app_path", trae_cn_app_path);
    normalize_app_path_field!("trae_solo_cn_app_path", trae_solo_cn_app_path);
    trim_string_field!("trae_app_scan_roots", trae_app_scan_roots);
    trim_string_field!("trae_solo_app_scan_roots", trae_solo_app_scan_roots);
    trim_string_field!("trae_cn_app_scan_roots", trae_cn_app_scan_roots);
    trim_string_field!("trae_solo_cn_app_scan_roots", trae_solo_cn_app_scan_roots);
    normalize_app_path_field!("workbuddy_app_path", workbuddy_app_path);

    if updates.contains_key("auto_switch_scope_mode") {
        let normalized = next.auto_switch_scope_mode.trim();
        next.auto_switch_scope_mode = if normalized.is_empty() {
            current.auto_switch_scope_mode.clone()
        } else {
            normalized.to_string()
        };
    }
    if updates.contains_key("auto_switch_account_scope_mode") {
        next.auto_switch_account_scope_mode =
            normalize_auto_switch_account_scope_mode(&next.auto_switch_account_scope_mode);
    }
    if updates.contains_key("auto_switch_selected_account_ids") {
        next.auto_switch_selected_account_ids =
            normalize_auto_switch_selected_account_ids(&next.auto_switch_selected_account_ids);
    }
    if updates.contains_key("codex_auto_switch_account_scope_mode") {
        next.codex_auto_switch_account_scope_mode =
            normalize_auto_switch_account_scope_mode(&next.codex_auto_switch_account_scope_mode);
    }
    if updates.contains_key("codex_auto_switch_selected_account_ids") {
        next.codex_auto_switch_selected_account_ids = normalize_auto_switch_selected_account_ids(
            &next.codex_auto_switch_selected_account_ids,
        );
    }

    if updates.contains_key("opencode_sync_on_switch")
        || updates.contains_key("opencode_auth_overwrite_on_switch")
    {
        next.opencode_sync_on_switch =
            next.opencode_auth_overwrite_on_switch && next.opencode_sync_on_switch;
    }
    if updates.contains_key("ghcp_opencode_sync_on_switch")
        || updates.contains_key("ghcp_opencode_auth_overwrite_on_switch")
    {
        next.ghcp_opencode_sync_on_switch =
            next.ghcp_opencode_auth_overwrite_on_switch && next.ghcp_opencode_sync_on_switch;
    }

    let codex_threshold = json_i32(updates, "codex_quota_alert_threshold")?;
    let codex_primary = json_i32(updates, "codex_quota_alert_primary_threshold")?;
    let codex_secondary = json_i32(updates, "codex_quota_alert_secondary_threshold")?;
    let mut thresholds = current.clone();
    apply_codex_quota_alert_thresholds(
        &mut thresholds,
        codex_threshold,
        codex_primary,
        codex_secondary,
    );
    next.codex_quota_alert_threshold = thresholds.codex_quota_alert_threshold;
    next.codex_quota_alert_primary_threshold = thresholds.codex_quota_alert_primary_threshold;
    next.codex_quota_alert_secondary_threshold = thresholds.codex_quota_alert_secondary_threshold;

    *current = next;
    Ok(())
}

fn get_app_auto_launch_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("读取应用自启动状态失败: {}", err))
}

fn apply_app_auto_launch_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|err| format!("启用应用自启动失败: {}", err))
    } else {
        app.autolaunch()
            .disable()
            .map_err(|err| format!("停用应用自启动失败: {}", err))
    }
}

fn sanitize_ui_scale(raw: f64) -> f64 {
    if !raw.is_finite() {
        return DEFAULT_UI_SCALE;
    }
    raw.clamp(MIN_UI_SCALE, MAX_UI_SCALE)
}

fn resolve_downloads_dir() -> Result<PathBuf, String> {
    if let Some(dir) = dirs::download_dir() {
        return Ok(dir);
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home.join("Downloads"));
    }
    Err("无法获取下载目录".to_string())
}

fn get_auto_backup_dir_path() -> Result<PathBuf, String> {
    Ok(modules::account::get_data_dir()?.join(AUTO_BACKUP_DIR_NAME))
}

fn ensure_auto_backup_dir_path() -> Result<PathBuf, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|err| format!("创建自动备份目录失败: {}", err))?;
    }
    Ok(dir)
}

fn build_auto_backup_settings(config: &UserConfig) -> Result<AutoBackupSettings, String> {
    let (include_accounts, include_config) = config::normalize_auto_backup_selection(
        config.auto_backup_include_accounts,
        config.auto_backup_include_config,
    );
    Ok(AutoBackupSettings {
        enabled: config.auto_backup_enabled,
        include_accounts,
        include_config,
        retention_days: config::sanitize_auto_backup_retention_days(
            config.auto_backup_retention_days,
        ),
        last_backup_at: config.auto_backup_last_backup_at.clone(),
        directory_path: get_auto_backup_dir_path()?.to_string_lossy().to_string(),
    })
}

fn build_webdav_sync_settings(config: &UserConfig) -> WebdavSyncSettings {
    let url = modules::webdav_sync::normalize_base_url(&config.webdav_sync_url)
        .unwrap_or_else(|_| config::default_webdav_sync_url());
    let remote_dir = modules::webdav_sync::normalize_remote_dir(&config.webdav_sync_remote_dir)
        .unwrap_or_else(|_| config::default_webdav_sync_remote_dir());

    WebdavSyncSettings {
        enabled: config.webdav_sync_enabled,
        url,
        username: config.webdav_sync_username.clone(),
        has_password: !config.webdav_sync_password.is_empty(),
        remote_dir,
        last_upload_at: config.webdav_sync_last_upload_at.clone(),
        last_upload_file_name: config.webdav_sync_last_upload_file_name.clone(),
        last_download_at: config.webdav_sync_last_download_at.clone(),
        last_download_file_name: config.webdav_sync_last_download_file_name.clone(),
        retention_days: config.webdav_sync_retention_days,
    }
}

fn resolve_webdav_password_update(
    current_password: &str,
    password: Option<String>,
    clear_password: Option<bool>,
) -> String {
    if clear_password.unwrap_or(false) {
        return String::new();
    }
    password
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| current_password.to_string())
}

fn validate_webdav_sync_config(
    enabled: bool,
    url: &str,
    username: &str,
    password: &str,
    remote_dir: &str,
) -> Result<(String, String, String), String> {
    let normalized_url = modules::webdav_sync::normalize_base_url(url)?;
    let normalized_remote_dir = modules::webdav_sync::normalize_remote_dir(remote_dir)?;
    let normalized_username = username.trim().to_string();

    if enabled {
        if normalized_username.is_empty() {
            return Err("启用 WebDAV 同步时账号不能为空".to_string());
        }
        if password.is_empty() {
            return Err("启用 WebDAV 同步时应用密码不能为空".to_string());
        }
    }

    Ok((normalized_url, normalized_username, normalized_remote_dir))
}

fn sanitize_auto_backup_file_name(file_name: &str) -> Result<String, String> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return Err("备份文件名不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("备份文件名不合法".to_string());
    }
    if !trimmed.ends_with(".json") && !trimmed.ends_with(".zip") {
        return Err("自动备份文件必须为 JSON 或 ZIP".to_string());
    }
    Ok(trimmed.to_string())
}

fn resolve_auto_backup_file_path(file_name: &str) -> Result<PathBuf, String> {
    let safe_name = sanitize_auto_backup_file_name(file_name)?;
    Ok(get_auto_backup_dir_path()?.join(safe_name))
}

fn auto_backup_archive_file_name(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".json")
        .map(|stem| format!("{}.zip", stem))
}

fn auto_backup_json_file_name(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".zip")
        .map(|stem| format!("{}.json", stem))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn push_u16_le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_zip_entry(
    out: &mut Vec<u8>,
    central: &mut Vec<(String, u32, u32, u32)>,
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let name_len =
        u16::try_from(name_bytes.len()).map_err(|_| format!("ZIP 条目名过长: {}", name))?;
    let size = u32::try_from(content.len()).map_err(|_| format!("ZIP 条目过大: {}", name))?;
    let offset = u32::try_from(out.len()).map_err(|_| "ZIP 文件过大".to_string())?;
    let crc = crc32(content);
    let dos_time = 0u16;
    let dos_date = 33u16;

    push_u32_le(out, 0x0403_4b50);
    push_u16_le(out, 20);
    push_u16_le(out, 0);
    push_u16_le(out, 0);
    push_u16_le(out, dos_time);
    push_u16_le(out, dos_date);
    push_u32_le(out, crc);
    push_u32_le(out, size);
    push_u32_le(out, size);
    push_u16_le(out, name_len);
    push_u16_le(out, 0);
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(content);

    central.push((name.to_string(), crc, size, offset));
    Ok(())
}

fn build_stored_zip(entries: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    if entries.is_empty() {
        return Err("ZIP 条目不能为空".to_string());
    }

    let mut out = Vec::new();
    let mut central_entries = Vec::new();
    for (name, content) in entries {
        push_zip_entry(&mut out, &mut central_entries, &name, &content)?;
    }

    let central_offset = u32::try_from(out.len()).map_err(|_| "ZIP 文件过大".to_string())?;
    let dos_time = 0u16;
    let dos_date = 33u16;

    for (name, crc, size, offset) in &central_entries {
        let name_bytes = name.as_bytes();
        let name_len =
            u16::try_from(name_bytes.len()).map_err(|_| format!("ZIP 条目名过长: {}", name))?;
        push_u32_le(&mut out, 0x0201_4b50);
        push_u16_le(&mut out, 20);
        push_u16_le(&mut out, 20);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, dos_time);
        push_u16_le(&mut out, dos_date);
        push_u32_le(&mut out, *crc);
        push_u32_le(&mut out, *size);
        push_u32_le(&mut out, *size);
        push_u16_le(&mut out, name_len);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u32_le(&mut out, 0);
        push_u32_le(&mut out, *offset);
        out.extend_from_slice(name_bytes);
    }

    let central_size = u32::try_from(out.len())
        .ok()
        .and_then(|len| len.checked_sub(central_offset))
        .ok_or_else(|| "ZIP 中央目录过大".to_string())?;
    let entry_count =
        u16::try_from(central_entries.len()).map_err(|_| "ZIP 条目过多".to_string())?;

    push_u32_le(&mut out, 0x0605_4b50);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, entry_count);
    push_u16_le(&mut out, entry_count);
    push_u32_le(&mut out, central_size);
    push_u32_le(&mut out, central_offset);
    push_u16_le(&mut out, 0);

    Ok(out)
}

fn backup_json_from_zip_bytes(bytes: &[u8]) -> Result<String, String> {
    let mut offset = 0usize;
    while offset + 30 <= bytes.len() {
        let Some(signature) = read_u32_le(bytes, offset) else {
            break;
        };
        if signature != 0x0403_4b50 {
            break;
        }
        let compression =
            read_u16_le(bytes, offset + 8).ok_or_else(|| "ZIP 本地文件头不完整".to_string())?;
        let compressed_size =
            read_u32_le(bytes, offset + 18).ok_or_else(|| "ZIP 条目大小缺失".to_string())? as usize;
        let name_len = read_u16_le(bytes, offset + 26)
            .ok_or_else(|| "ZIP 条目名长度缺失".to_string())? as usize;
        let extra_len =
            read_u16_le(bytes, offset + 28).ok_or_else(|| "ZIP 扩展长度缺失".to_string())? as usize;
        let name_start = offset + 30;
        let name_end = name_start + name_len;
        let data_start = name_end + extra_len;
        let data_end = data_start + compressed_size;
        if data_end > bytes.len() {
            return Err("ZIP 条目内容不完整".to_string());
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_end]);
        if name == "backup.json" {
            if compression != 0 {
                return Err("暂不支持压缩过的 ZIP 备份条目".to_string());
            }
            return String::from_utf8(bytes[data_start..data_end].to_vec())
                .map_err(|_| "ZIP 备份中的 backup.json 不是 UTF-8".to_string());
        }
        offset = data_end;
    }

    Err("ZIP 备份中未找到 backup.json".to_string())
}

fn backup_json_from_path(path: &Path) -> Result<String, String> {
    match path.extension().and_then(|item| item.to_str()) {
        Some("json") => match fs::read_to_string(path) {
            Ok(content) => {
                if serde_json::from_str::<serde_json::Value>(&content).is_ok() {
                    return Ok(content);
                }
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if let Some(archive_name) = auto_backup_archive_file_name(file_name) {
                        let archive_path = path.with_file_name(archive_name);
                        if archive_path.exists() {
                            return backup_json_from_path(&archive_path);
                        }
                    }
                }
                Ok(content)
            }
            Err(err) => {
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if let Some(archive_name) = auto_backup_archive_file_name(file_name) {
                        let archive_path = path.with_file_name(archive_name);
                        if archive_path.exists() {
                            return backup_json_from_path(&archive_path);
                        }
                    }
                }
                Err(format!("读取自动备份文件失败: {}", err))
            }
        },
        Some("zip") => {
            let bytes = fs::read(path).map_err(|err| format!("读取自动备份压缩包失败: {}", err))?;
            backup_json_from_zip_bytes(&bytes)
        }
        _ => Err("不支持的自动备份文件类型".to_string()),
    }
}

fn collect_auto_backup_platforms_from_value(
    value: &serde_json::Value,
) -> Vec<AutoBackupPlatformEntry> {
    let accounts = value
        .get("accounts")
        .filter(|item| item.is_object())
        .unwrap_or(value);
    let Some(platforms) = accounts.get("platforms").and_then(|item| item.as_object()) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for (platform, payload) in platforms {
        let exported_data = payload
            .get("exported_data")
            .or_else(|| payload.get("data"))
            .or_else(|| payload.get("accounts"));
        let account_count = payload
            .get("account_count")
            .and_then(|item| item.as_u64())
            .or_else(|| {
                exported_data
                    .and_then(|item| item.as_array())
                    .map(|items| items.len() as u64)
            })
            .unwrap_or(0);
        if account_count == 0 {
            continue;
        }
        result.push(AutoBackupPlatformEntry {
            platform: platform.clone(),
            account_count,
        });
    }
    result.sort_by(|left, right| left.platform.cmp(&right.platform));
    result
}

fn collect_auto_backup_platforms(json_content: &str) -> Vec<AutoBackupPlatformEntry> {
    serde_json::from_str::<serde_json::Value>(json_content)
        .ok()
        .map(|value| collect_auto_backup_platforms_from_value(&value))
        .unwrap_or_default()
}

fn build_auto_backup_zip_bytes(file_name: &str, content: &str) -> Result<Vec<u8>, String> {
    let root = serde_json::from_str::<serde_json::Value>(content)
        .map_err(|err| format!("自动备份 JSON 解析失败，无法生成 ZIP: {}", err))?;
    let platforms = collect_auto_backup_platforms_from_value(&root);
    let manifest = serde_json::json!({
        "schema": "cockpit-tools.auto-backup-archive",
        "version": 1,
        "source_file_name": file_name,
        "platforms": &platforms,
        "sections": root.get("sections").cloned().unwrap_or(serde_json::Value::Null),
        "exported_at": root.get("exported_at").cloned().unwrap_or(serde_json::Value::Null),
    });

    let mut entries = vec![
        ("backup.json".to_string(), content.as_bytes().to_vec()),
        (
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|err| format!("序列化 ZIP 清单失败: {}", err))?,
        ),
    ];

    if let Some(accounts) = root.get("accounts").filter(|item| item.is_object()) {
        if let Some(platforms_map) = accounts.get("platforms").and_then(|item| item.as_object()) {
            for platform in &platforms {
                if let Some(payload) = platforms_map.get(&platform.platform) {
                    let exported_data = payload
                        .get("exported_data")
                        .or_else(|| payload.get("data"))
                        .or_else(|| payload.get("accounts"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([]));
                    entries.push((
                        format!("accounts/{}.json", platform.platform),
                        serde_json::to_vec_pretty(&exported_data)
                            .map_err(|err| format!("序列化平台备份失败: {}", err))?,
                    ));
                }
            }
        }
    } else if let Some(platforms_map) = root.get("platforms").and_then(|item| item.as_object()) {
        for platform in &platforms {
            if let Some(payload) = platforms_map.get(&platform.platform) {
                let exported_data = payload
                    .get("exported_data")
                    .or_else(|| payload.get("data"))
                    .or_else(|| payload.get("accounts"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                entries.push((
                    format!("accounts/{}.json", platform.platform),
                    serde_json::to_vec_pretty(&exported_data)
                        .map_err(|err| format!("序列化平台备份失败: {}", err))?,
                ));
            }
        }
    }

    build_stored_zip(entries)
}

fn system_time_to_unix_ms(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn open_path_in_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn open_data_folder() -> Result<(), String> {
    let path = modules::account::get_data_dir()?;
    open_path_in_system(path.as_path())
}

#[tauri::command]
pub fn open_local_path(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("路径不存在: {}", p.display()));
    }
    open_path_in_system(p.as_path())
}

/// 保存文本文件
#[tauri::command]
pub async fn save_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("写入文件失败: {}", e))
}

/// 获取下载目录
#[tauri::command]
pub fn get_downloads_dir() -> Result<String, String> {
    Ok(resolve_downloads_dir()?.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_auto_backup_settings() -> Result<AutoBackupSettings, String> {
    let config = config::get_user_config();
    build_auto_backup_settings(&config)
}

#[tauri::command]
pub fn save_auto_backup_settings(
    enabled: bool,
    include_accounts: bool,
    include_config: bool,
    retention_days: i32,
) -> Result<AutoBackupSettings, String> {
    let (next_include_accounts, next_include_config) =
        config::normalize_auto_backup_selection(include_accounts, include_config);
    let next_retention_days = config::sanitize_auto_backup_retention_days(retention_days);
    let new_config = config::patch_user_config(move |current| {
        current.auto_backup_enabled = enabled;
        current.auto_backup_include_accounts = next_include_accounts;
        current.auto_backup_include_config = next_include_config;
        current.auto_backup_retention_days = next_retention_days;
        Ok(())
    })?;
    build_auto_backup_settings(&new_config)
}

#[tauri::command]
pub fn update_auto_backup_last_run(
    last_backup_at: Option<String>,
) -> Result<AutoBackupSettings, String> {
    let normalized_last_backup_at = last_backup_at.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let new_config = config::patch_user_config(move |current| {
        current.auto_backup_last_backup_at = normalized_last_backup_at;
        Ok(())
    })?;
    build_auto_backup_settings(&new_config)
}

#[tauri::command]
pub fn write_auto_backup_file(file_name: String, content: String) -> Result<String, String> {
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    if !safe_name.ends_with(".json") {
        return Err("自动备份主文件必须为 JSON".to_string());
    }
    let dir = ensure_auto_backup_dir_path()?;
    let path = dir.join(&safe_name);
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入自动备份文件失败: {}", err))?;

    if let Some(archive_name) = auto_backup_archive_file_name(&safe_name) {
        let archive_path = dir.join(archive_name);
        let zip_bytes = build_auto_backup_zip_bytes(&safe_name, &content)?;
        crate::modules::atomic_write::write_bytes_atomic(&archive_path, &zip_bytes)
            .map_err(|err| format!("写入自动备份压缩包失败: {}", err))?;
    }

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn read_auto_backup_file(file_name: String) -> Result<String, String> {
    let path = resolve_auto_backup_file_path(&file_name)?;
    backup_json_from_path(&path)
}

#[tauri::command]
pub fn copy_auto_backup_file(file_name: String, target_path: String) -> Result<String, String> {
    let source_path = resolve_auto_backup_file_path(&file_name)?;
    if !source_path.exists() {
        return Err("备份文件不存在".to_string());
    }
    let target = PathBuf::from(target_path.trim());
    if target.as_os_str().is_empty() {
        return Err("目标路径不能为空".to_string());
    }
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| format!("创建下载目录失败: {}", err))?;
        }
    }
    fs::copy(&source_path, &target).map_err(|err| format!("复制备份文件失败: {}", err))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub fn list_auto_backup_files() -> Result<Vec<AutoBackupFileEntry>, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("读取自动备份目录失败: {}", err))?;
    let mut json_stems = std::collections::HashSet::new();

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取自动备份文件失败: {}", err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        paths.push(path);
    }

    for path in &paths {
        if let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
        {
            json_stems.insert(stem.to_string());
        }
    }

    for path in paths {
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.ends_with(".json") || name.ends_with(".zip") => name.to_string(),
            _ => continue,
        };
        if file_name.ends_with(".zip") {
            if let Some(json_name) = auto_backup_json_file_name(&file_name) {
                if json_stems.contains(json_name.trim_end_matches(".json")) {
                    continue;
                }
            }
        }
        let metadata =
            fs::metadata(&path).map_err(|err| format!("读取备份文件信息失败: {}", err))?;
        let archive_name = if file_name.ends_with(".json") {
            auto_backup_archive_file_name(&file_name)
        } else {
            None
        };
        let archive_path = archive_name
            .as_ref()
            .map(|name| dir.join(name))
            .filter(|path| path.exists());
        let archive_metadata = archive_path
            .as_ref()
            .and_then(|path| fs::metadata(path).ok());
        let json_content = backup_json_from_path(&path).ok();
        files.push(AutoBackupFileEntry {
            file_name,
            path: path.to_string_lossy().to_string(),
            file_kind: path
                .extension()
                .and_then(|item| item.to_str())
                .unwrap_or("json")
                .to_string(),
            size_bytes: metadata.len(),
            modified_at_ms: metadata.modified().ok().and_then(system_time_to_unix_ms),
            archive_file_name: archive_path
                .as_ref()
                .and_then(|path| path.file_name().and_then(|name| name.to_str()))
                .map(|name| name.to_string()),
            archive_path: archive_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            archive_size_bytes: archive_metadata.map(|metadata| metadata.len()),
            platforms: json_content
                .as_deref()
                .map(collect_auto_backup_platforms)
                .unwrap_or_default(),
        });
    }

    files.sort_by(|left, right| {
        right
            .modified_at_ms
            .unwrap_or_default()
            .cmp(&left.modified_at_ms.unwrap_or_default())
            .then_with(|| right.file_name.cmp(&left.file_name))
    });

    Ok(files)
}

#[tauri::command]
pub fn delete_auto_backup_file(file_name: String) -> Result<(), String> {
    let path = resolve_auto_backup_file_path(&file_name)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|err| format!("删除自动备份文件失败: {}", err))?;
    if file_name.ends_with(".json") {
        if let Some(archive_name) = auto_backup_archive_file_name(&file_name) {
            let archive_path = resolve_auto_backup_file_path(&archive_name)?;
            if archive_path.exists() {
                fs::remove_file(&archive_path)
                    .map_err(|err| format!("删除自动备份压缩包失败: {}", err))?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn cleanup_auto_backup_files(retention_days: i32) -> Result<Vec<String>, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let normalized_retention_days = config::sanitize_auto_backup_retention_days(retention_days);
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(Duration::from_secs(
            normalized_retention_days as u64 * 24 * 60 * 60,
        ))
        .unwrap_or(now);

    let mut deleted = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("读取自动备份目录失败: {}", err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取自动备份文件失败: {}", err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.ends_with(".json") || name.ends_with(".zip") => name.to_string(),
            _ => continue,
        };
        let metadata =
            fs::metadata(&path).map_err(|err| format!("读取备份文件信息失败: {}", err))?;
        let modified = match metadata.modified() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if modified >= cutoff {
            continue;
        }
        fs::remove_file(&path).map_err(|err| format!("清理过期备份失败: {}", err))?;
        deleted.push(file_name);
    }

    deleted.sort();
    Ok(deleted)
}

#[tauri::command]
pub fn open_auto_backup_dir() -> Result<(), String> {
    let path = ensure_auto_backup_dir_path()?;
    open_path_in_system(path.as_path())
}

#[tauri::command]
pub fn get_webdav_sync_settings() -> Result<WebdavSyncSettings, String> {
    let config = config::get_user_config();
    Ok(build_webdav_sync_settings(&config))
}

#[tauri::command]
pub fn save_webdav_sync_settings(
    enabled: bool,
    url: String,
    username: String,
    password: Option<String>,
    clear_password: Option<bool>,
    remote_dir: String,
    retention_days: i32,
) -> Result<WebdavSyncSettings, String> {
    let new_config = config::patch_user_config(move |current| {
        let next_password =
            resolve_webdav_password_update(&current.webdav_sync_password, password, clear_password);
        let (next_url, next_username, next_remote_dir) =
            validate_webdav_sync_config(enabled, &url, &username, &next_password, &remote_dir)?;

        current.webdav_sync_enabled = enabled;
        current.webdav_sync_url = next_url;
        current.webdav_sync_username = next_username;
        current.webdav_sync_password = next_password;
        current.webdav_sync_remote_dir = next_remote_dir;
        current.webdav_sync_retention_days =
            config::sanitize_webdav_sync_retention_days(retention_days);
        Ok(())
    })?;
    Ok(build_webdav_sync_settings(&new_config))
}

#[tauri::command]
pub async fn test_webdav_sync_connection(
    url: String,
    username: String,
    password: Option<String>,
    clear_password: Option<bool>,
    remote_dir: String,
) -> Result<modules::webdav_sync::WebdavTestResult, String> {
    let current = config::get_user_config();
    let next_password =
        resolve_webdav_password_update(&current.webdav_sync_password, password, clear_password);
    let connection =
        modules::webdav_sync::connection_from_parts(&url, &username, &next_password, &remote_dir)?;
    modules::webdav_sync::test_connection(&connection).await
}

#[tauri::command]
pub async fn upload_auto_backup_to_webdav(
    file_name: String,
) -> Result<modules::webdav_sync::WebdavUploadResult, String> {
    let config = config::get_user_config();
    if !config.webdav_sync_enabled {
        return Err("WebDAV 同步未启用".to_string());
    }

    let connection = modules::webdav_sync::connection_from_config(&config)?;
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    if !safe_name.ends_with(".json") {
        return Err("WebDAV 同步入口文件必须为 JSON 备份".to_string());
    }

    let archive_name = auto_backup_archive_file_name(&safe_name)
        .ok_or_else(|| "无法获取对应的压缩包名称".to_string())?;
    let archive_path = resolve_auto_backup_file_path(&archive_name)?;
    if !archive_path.exists() {
        return Err("本地备份压缩包不存在".to_string());
    }

    let archive_bytes =
        fs::read(&archive_path).map_err(|err| format!("读取本地备份压缩包失败: {}", err))?;

    let sync_client = modules::webdav_sync::WebdavSyncClient::new(&connection)?;

    let mut uploaded_files = Vec::new();
    uploaded_files.push(
        sync_client
            .upload_backup_bytes(&archive_name, archive_bytes)
            .await?,
    );

    let deleted_files = sync_client
        .cleanup_remote_backups(config::sanitize_webdav_sync_retention_days(
            config.webdav_sync_retention_days,
        ))
        .await?;
    let uploaded_at = chrono::Utc::now().to_rfc3339();
    let remote_dir = connection.remote_dir.clone();

    config::patch_user_config(|current| {
        current.webdav_sync_last_upload_at = Some(uploaded_at.clone());
        current.webdav_sync_last_upload_file_name = Some(archive_name.clone());
        Ok(())
    })?;

    Ok(modules::webdav_sync::WebdavUploadResult {
        uploaded_files,
        deleted_files,
        uploaded_at,
        remote_dir,
    })
}

#[tauri::command]
pub async fn list_webdav_backup_files(
) -> Result<Vec<modules::webdav_sync::WebdavBackupFileEntry>, String> {
    let config = config::get_user_config();
    let connection = modules::webdav_sync::connection_from_config(&config)?;
    modules::webdav_sync::list_remote_backups(&connection).await
}

#[tauri::command]
pub async fn read_webdav_backup_file(file_name: String) -> Result<String, String> {
    let config = config::get_user_config();
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    let connection = modules::webdav_sync::connection_from_config(&config)?;

    let downloaded_at = chrono::Utc::now().to_rfc3339();
    let content = if safe_name.ends_with(".zip") {
        let bytes = modules::webdav_sync::read_remote_backup_bytes(&connection, &safe_name).await?;
        backup_json_from_zip_bytes(&bytes)?
    } else if safe_name.ends_with(".json") {
        modules::webdav_sync::read_remote_backup(&connection, &safe_name).await?
    } else {
        return Err("不支持的备份文件格式".to_string());
    };

    config::patch_user_config(move |current| {
        current.webdav_sync_last_download_at = Some(downloaded_at);
        current.webdav_sync_last_download_file_name = Some(safe_name);
        Ok(())
    })?;
    Ok(content)
}

#[tauri::command]
pub async fn delete_webdav_backup_file(file_name: String) -> Result<(), String> {
    let config = config::get_user_config();
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    let connection = modules::webdav_sync::connection_from_config(&config)?;
    modules::webdav_sync::delete_remote_backup(&connection, &safe_name).await
}

/// 获取网络服务配置
#[tauri::command]
pub fn get_network_config() -> Result<NetworkConfig, String> {
    let user_config = config::get_user_config();
    let ws_actual_port = config::get_actual_port();
    let report_actual_port = web_report::get_actual_port();

    Ok(NetworkConfig {
        ws_enabled: user_config.ws_enabled,
        ws_port: user_config.ws_port,
        actual_port: ws_actual_port,
        default_port: DEFAULT_WS_PORT,
        report_enabled: user_config.report_enabled,
        report_port: user_config.report_port,
        report_actual_port,
        report_default_port: DEFAULT_REPORT_PORT,
        report_token: user_config.report_token,
        global_proxy_enabled: user_config.global_proxy_enabled,
        global_proxy_url: user_config.global_proxy_url,
        global_proxy_no_proxy: user_config.global_proxy_no_proxy,
    })
}

/// 保存网络服务配置
#[tauri::command]
pub fn save_network_config(
    ws_enabled: bool,
    ws_port: u16,
    report_enabled: Option<bool>,
    report_port: Option<u16>,
    report_token: Option<String>,
    global_proxy_enabled: Option<bool>,
    global_proxy_url: Option<String>,
    global_proxy_no_proxy: Option<String>,
) -> Result<bool, String> {
    let mut needs_restart = false;
    config::patch_user_config(|current| {
        let next_report_enabled = report_enabled.unwrap_or(current.report_enabled);
        let next_report_port = report_port.unwrap_or(current.report_port);
        let next_report_token = report_token
            .unwrap_or_else(|| current.report_token.clone())
            .trim()
            .to_string();
        let next_global_proxy_enabled =
            global_proxy_enabled.unwrap_or(current.global_proxy_enabled);
        let next_global_proxy_url = global_proxy_url
            .unwrap_or_else(|| current.global_proxy_url.clone())
            .trim()
            .to_string();
        let next_global_proxy_no_proxy = global_proxy_no_proxy
            .unwrap_or_else(|| current.global_proxy_no_proxy.clone())
            .trim()
            .to_string();

        if next_report_enabled && next_report_token.is_empty() {
            return Err("网页查询服务 token 不能为空".to_string());
        }
        if next_global_proxy_enabled && next_global_proxy_url.is_empty() {
            return Err("启用全局代理时，代理地址不能为空".to_string());
        }

        needs_restart = current.ws_port != ws_port
            || current.ws_enabled != ws_enabled
            || current.report_enabled != next_report_enabled
            || current.report_port != next_report_port
            || current.report_token != next_report_token;

        current.ws_enabled = ws_enabled;
        current.ws_port = ws_port;
        current.report_enabled = next_report_enabled;
        current.report_port = next_report_port;
        current.report_token = next_report_token;
        current.global_proxy_enabled = next_global_proxy_enabled;
        current.global_proxy_url = next_global_proxy_url;
        current.global_proxy_no_proxy = next_global_proxy_no_proxy;
        Ok(())
    })?;

    Ok(needs_restart)
}

/// 获取系统可用的终端列表
#[tauri::command]
pub async fn get_available_terminals() -> Result<Vec<String>, String> {
    let mut available = Vec::new();
    available.push("system".to_string());

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let terminals = [
            (
                "Terminal",
                vec![
                    "/System/Applications/Utilities/Terminal.app".to_string(),
                    "/Applications/Utilities/Terminal.app".to_string(),
                ],
            ),
            (
                "iTerm2",
                vec![
                    "/Applications/iTerm.app".to_string(),
                    "/Applications/iTerm 2.app".to_string(),
                    format!("{}/Applications/iTerm.app", home),
                ],
            ),
            (
                "Warp",
                vec![
                    "/Applications/Warp.app".to_string(),
                    format!("{}/Applications/Warp.app", home),
                ],
            ),
            (
                "Ghostty",
                vec![
                    "/Applications/Ghostty.app".to_string(),
                    format!("{}/Applications/Ghostty.app", home),
                ],
            ),
            (
                "WezTerm",
                vec![
                    "/Applications/WezTerm.app".to_string(),
                    format!("{}/Applications/WezTerm.app", home),
                ],
            ),
            (
                "Kitty",
                vec![
                    "/Applications/Kitty.app".to_string(),
                    format!("{}/Applications/Kitty.app", home),
                ],
            ),
            (
                "Alacritty",
                vec![
                    "/Applications/Alacritty.app".to_string(),
                    format!("{}/Applications/Alacritty.app", home),
                ],
            ),
        ];
        for (name, paths) in terminals {
            for path in paths {
                if !path.is_empty() && std::path::Path::new(&path).exists() {
                    available.push(name.to_string());
                    break;
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Windows 下检查可执行文件是否在 PATH 中
        let terminals = ["cmd", "PowerShell", "pwsh", "wt"];
        for name in terminals {
            if is_command_available(name) {
                available.push(name.to_string());
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let terminals = [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "xterm",
            "alacritty",
            "kitty",
        ];
        for name in terminals {
            if is_command_available(name) {
                available.push(name.to_string());
            }
        }
    }

    Ok(available)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn is_command_available(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    let check_cmd = "where";
    #[cfg(target_os = "linux")]
    let check_cmd = "which";

    let mut command = std::process::Command::new(check_cmd);
    command
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.status().map(|s| s.success()).unwrap_or(false)
}

/// 获取诊断上报配置
#[tauri::command]
pub fn get_diagnostics_config() -> modules::diagnostics::DiagnosticsConfig {
    modules::diagnostics::get_diagnostics_config()
}

/// 保存诊断上报配置
#[tauri::command]
pub fn save_diagnostics_config(
    error_reporting_enabled: bool,
    error_reporting_debug: Option<bool>,
) -> Result<(), String> {
    modules::diagnostics::save_diagnostics_config(error_reporting_enabled, error_reporting_debug)
}

/// 记录前端启动阶段，只写本地日志，不触发远端上报
#[tauri::command]
pub fn diagnostics_frontend_stage(stage: String, detail: Option<serde_json::Value>) {
    modules::diagnostics::record_frontend_stage(stage, detail);
}

/// 标记前端已完成启动
#[tauri::command]
pub fn diagnostics_frontend_ready(stage: Option<String>) {
    modules::diagnostics::mark_frontend_ready(stage);
}

/// 捕获前端诊断事件并异步上报
#[tauri::command]
pub fn diagnostics_capture_event(event: modules::diagnostics::DiagnosticsClientEvent) {
    modules::diagnostics::capture_client_event(event);
}

/// 获取通用设置配置
#[tauri::command]
pub fn get_general_config(app: tauri::AppHandle) -> Result<GeneralConfig, String> {
    let started = Instant::now();
    let user_config = config::get_user_config();
    let app_auto_launch_enabled =
        get_app_auto_launch_enabled(&app).unwrap_or(user_config.app_auto_launch_enabled);

    let close_behavior_str = match user_config.close_behavior {
        CloseWindowBehavior::Ask => "ask",
        CloseWindowBehavior::Minimize => "minimize",
        CloseWindowBehavior::Quit => "quit",
    };
    let minimize_behavior_str = match user_config.minimize_behavior {
        MinimizeWindowBehavior::DockAndTray => "dock_and_tray",
        MinimizeWindowBehavior::TrayOnly => "tray_only",
    };

    let result = GeneralConfig {
        language: user_config.language,
        default_terminal: user_config.default_terminal,
        theme: user_config.theme,
        theme_color: config::normalize_theme_color(&user_config.theme_color),
        external_network_enabled: user_config.external_network_enabled,
        webdav_allowed_domains: user_config.webdav_allowed_domains,
        reduced_motion_enabled: user_config.reduced_motion_enabled,
        ui_scale: user_config.ui_scale,
        auto_refresh_minutes: user_config.auto_refresh_minutes,
        codex_auto_refresh_minutes: user_config.codex_auto_refresh_minutes,
        codex_sync_wsl: user_config.codex_sync_wsl,
        codex_app_ui_injection_enabled: user_config.codex_app_ui_injection_enabled,
        codex_wsl_config_dir: user_config.codex_wsl_config_dir,
        zed_auto_refresh_minutes: user_config.zed_auto_refresh_minutes,
        ghcp_auto_refresh_minutes: user_config.ghcp_auto_refresh_minutes,
        windsurf_auto_refresh_minutes: user_config.windsurf_auto_refresh_minutes,
        kiro_auto_refresh_minutes: user_config.kiro_auto_refresh_minutes,
        cursor_auto_refresh_minutes: user_config.cursor_auto_refresh_minutes,
        grok_auto_refresh_minutes: user_config.grok_auto_refresh_minutes,
        grok_sync_official_auth_on_switch: user_config.grok_sync_official_auth_on_switch,
        claude_auto_refresh_minutes: user_config.claude_auto_refresh_minutes,
        codebuddy_auto_refresh_minutes: user_config.codebuddy_auto_refresh_minutes,
        codebuddy_cn_auto_refresh_minutes: user_config.codebuddy_cn_auto_refresh_minutes,
        workbuddy_auto_refresh_minutes: user_config.workbuddy_auto_refresh_minutes,
        qoder_auto_refresh_minutes: user_config.qoder_auto_refresh_minutes,
        zcode_auto_refresh_minutes: user_config.zcode_auto_refresh_minutes,
        trae_auto_refresh_minutes: user_config.trae_auto_refresh_minutes,
        trae_solo_auto_refresh_minutes: user_config.trae_solo_auto_refresh_minutes,
        trae_cn_auto_refresh_minutes: user_config.trae_cn_auto_refresh_minutes,
        trae_solo_cn_auto_refresh_minutes: user_config.trae_solo_cn_auto_refresh_minutes,
        close_behavior: close_behavior_str.to_string(),
        minimize_behavior: minimize_behavior_str.to_string(),
        hide_dock_icon: user_config.hide_dock_icon,
        tray_icon_style: user_config.tray_icon_style.as_str().to_string(),
        floating_card_show_on_startup: user_config.floating_card_show_on_startup,
        startup_minimized: user_config.startup_minimized,
        remember_main_window_state: user_config.remember_main_window_state,
        startup_page: config::normalize_startup_page(&user_config.startup_page),
        floating_card_always_on_top: user_config.floating_card_always_on_top,
        app_auto_launch_enabled,
        token_keeper_enabled: user_config.token_keeper_enabled,
        auto_import_from_local_enabled: user_config.auto_import_from_local_enabled,
        antigravity_startup_wakeup_enabled: user_config.antigravity_startup_wakeup_enabled,
        antigravity_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.antigravity_startup_wakeup_delay_seconds,
        ),
        codex_startup_wakeup_enabled: user_config.codex_startup_wakeup_enabled,
        codex_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.codex_startup_wakeup_delay_seconds,
        ),
        floating_card_confirm_on_close: user_config.floating_card_confirm_on_close,
        opencode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.opencode_app_path,
        ),
        antigravity_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.antigravity_app_path,
        ),
        codex_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codex_app_path,
        ),
        claude_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.claude_app_path,
        ),
        claude_app_scan_roots: user_config.claude_app_scan_roots,
        codex_specified_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codex_specified_app_path,
        ),
        zed_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.zed_app_path,
        ),
        vscode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.vscode_app_path,
        ),
        windsurf_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.windsurf_app_path,
        ),
        kiro_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.kiro_app_path,
        ),
        cursor_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.cursor_app_path,
        ),
        codebuddy_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codebuddy_app_path,
        ),
        codebuddy_share_sessions_on_switch: user_config.codebuddy_share_sessions_on_switch,
        codebuddy_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codebuddy_cn_app_path,
        ),
        qoder_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.qoder_app_path,
        ),
        zcode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.zcode_app_path,
        ),
        trae_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_app_path,
        ),
        trae_solo_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_solo_app_path,
        ),
        trae_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_cn_app_path,
        ),
        trae_solo_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_solo_cn_app_path,
        ),
        trae_app_scan_roots: user_config.trae_app_scan_roots,
        trae_solo_app_scan_roots: user_config.trae_solo_app_scan_roots,
        trae_cn_app_scan_roots: user_config.trae_cn_app_scan_roots,
        trae_solo_cn_app_scan_roots: user_config.trae_solo_cn_app_scan_roots,
        workbuddy_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.workbuddy_app_path,
        ),
        workbuddy_share_sessions_on_switch: user_config.workbuddy_share_sessions_on_switch,
        opencode_sync_on_switch: user_config.opencode_sync_on_switch,
        opencode_auth_overwrite_on_switch: user_config.opencode_auth_overwrite_on_switch,
        ghcp_opencode_sync_on_switch: user_config.ghcp_opencode_sync_on_switch,
        ghcp_opencode_auth_overwrite_on_switch: user_config.ghcp_opencode_auth_overwrite_on_switch,
        ghcp_launch_on_switch: user_config.ghcp_launch_on_switch,
        openclaw_auth_overwrite_on_switch: user_config.openclaw_auth_overwrite_on_switch,
        hermes_auth_overwrite_on_switch: user_config.hermes_auth_overwrite_on_switch,
        codex_launch_on_switch: user_config.codex_launch_on_switch,
        antigravity_launch_on_switch: user_config.antigravity_launch_on_switch,
        codex_restart_specified_app_on_switch: user_config.codex_restart_specified_app_on_switch,
        codex_local_access_entry_visible: user_config.codex_local_access_entry_visible,
        top_right_ad_visible: user_config.top_right_ad_visible,
        antigravity_dual_switch_no_restart_enabled: user_config
            .antigravity_dual_switch_no_restart_enabled,
        auto_switch_enabled: user_config.auto_switch_enabled,
        auto_switch_threshold: user_config.auto_switch_threshold,
        auto_switch_credits_enabled: user_config.auto_switch_credits_enabled,
        auto_switch_credits_threshold: user_config.auto_switch_credits_threshold,
        auto_switch_scope_mode: user_config.auto_switch_scope_mode,
        auto_switch_selected_group_ids: user_config.auto_switch_selected_group_ids,
        auto_switch_account_scope_mode: user_config.auto_switch_account_scope_mode,
        auto_switch_selected_account_ids: user_config.auto_switch_selected_account_ids,
        codex_auto_switch_enabled: user_config.codex_auto_switch_enabled,
        codex_auto_switch_primary_threshold: user_config.codex_auto_switch_primary_threshold,
        codex_auto_switch_secondary_threshold: user_config.codex_auto_switch_secondary_threshold,
        codex_auto_switch_account_scope_mode: user_config.codex_auto_switch_account_scope_mode,
        codex_auto_switch_selected_account_ids: user_config.codex_auto_switch_selected_account_ids,
        quota_alert_enabled: user_config.quota_alert_enabled,
        quota_alert_threshold: user_config.quota_alert_threshold,
        codex_quota_alert_enabled: user_config.codex_quota_alert_enabled,
        codex_quota_alert_threshold: user_config.codex_quota_alert_threshold,
        zed_quota_alert_enabled: user_config.zed_quota_alert_enabled,
        zed_quota_alert_threshold: user_config.zed_quota_alert_threshold,
        codex_quota_alert_primary_threshold: user_config.codex_quota_alert_primary_threshold,
        codex_quota_alert_secondary_threshold: user_config.codex_quota_alert_secondary_threshold,
        ghcp_quota_alert_enabled: user_config.ghcp_quota_alert_enabled,
        ghcp_quota_alert_threshold: user_config.ghcp_quota_alert_threshold,
        windsurf_quota_alert_enabled: user_config.windsurf_quota_alert_enabled,
        windsurf_quota_alert_threshold: user_config.windsurf_quota_alert_threshold,
        kiro_quota_alert_enabled: user_config.kiro_quota_alert_enabled,
        kiro_quota_alert_threshold: user_config.kiro_quota_alert_threshold,
        cursor_quota_alert_enabled: user_config.cursor_quota_alert_enabled,
        cursor_quota_alert_threshold: user_config.cursor_quota_alert_threshold,
        grok_quota_alert_enabled: user_config.grok_quota_alert_enabled,
        grok_quota_alert_threshold: user_config.grok_quota_alert_threshold,
        claude_quota_alert_enabled: user_config.claude_quota_alert_enabled,
        claude_quota_alert_threshold: user_config.claude_quota_alert_threshold,
        codebuddy_quota_alert_enabled: user_config.codebuddy_quota_alert_enabled,
        codebuddy_quota_alert_threshold: user_config.codebuddy_quota_alert_threshold,
        codebuddy_cn_quota_alert_enabled: user_config.codebuddy_cn_quota_alert_enabled,
        codebuddy_cn_quota_alert_threshold: user_config.codebuddy_cn_quota_alert_threshold,
        qoder_quota_alert_enabled: user_config.qoder_quota_alert_enabled,
        qoder_quota_alert_threshold: user_config.qoder_quota_alert_threshold,
        trae_quota_alert_enabled: user_config.trae_quota_alert_enabled,
        trae_quota_alert_threshold: user_config.trae_quota_alert_threshold,
        trae_solo_quota_alert_enabled: user_config.trae_solo_quota_alert_enabled,
        trae_solo_quota_alert_threshold: user_config.trae_solo_quota_alert_threshold,
        trae_cn_quota_alert_enabled: user_config.trae_cn_quota_alert_enabled,
        trae_cn_quota_alert_threshold: user_config.trae_cn_quota_alert_threshold,
        trae_solo_cn_quota_alert_enabled: user_config.trae_solo_cn_quota_alert_enabled,
        trae_solo_cn_quota_alert_threshold: user_config.trae_solo_cn_quota_alert_threshold,
        workbuddy_quota_alert_enabled: user_config.workbuddy_quota_alert_enabled,
        workbuddy_quota_alert_threshold: user_config.workbuddy_quota_alert_threshold,
    };

    modules::logger::log_info(&format!(
        "[StartupPerf][SystemCommand] get_general_config completed in {}ms: auto_refresh={}, codex={}, zed={}, ghcp={}, windsurf={}, kiro={}, cursor={}, codebuddy={}, codebuddy_cn={}, workbuddy={}, qoder={}, zcode={}, trae={}, auto_switch={}",
        started.elapsed().as_millis(),
        result.auto_refresh_minutes,
        result.codex_auto_refresh_minutes,
        result.zed_auto_refresh_minutes,
        result.ghcp_auto_refresh_minutes,
        result.windsurf_auto_refresh_minutes,
        result.kiro_auto_refresh_minutes,
        result.cursor_auto_refresh_minutes,
        result.codebuddy_auto_refresh_minutes,
        result.codebuddy_cn_auto_refresh_minutes,
        result.workbuddy_auto_refresh_minutes,
        result.qoder_auto_refresh_minutes,
        result.zcode_auto_refresh_minutes,
        result.trae_auto_refresh_minutes,
        result.auto_switch_enabled
    ));

    Ok(result)
}

/// 按字段保存通用设置配置。
#[tauri::command]
pub fn patch_general_config(
    app: tauri::AppHandle,
    updates: JsonMap<String, JsonValue>,
) -> Result<(), String> {
    let _save_guard = lock_general_config_transaction()?;

    if updates.is_empty() {
        return Ok(());
    }

    // 在修改系统自启动状态前完成字段和类型校验，避免无效请求留下外部副作用。
    let mut preview = config::get_user_config();
    apply_general_config_updates(&mut preview, &updates)?;

    let requested_auto_launch = updates
        .get("app_auto_launch_enabled")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| "配置字段 app_auto_launch_enabled 必须为布尔值".to_string())
        })
        .transpose()?;
    let previous_auto_launch = requested_auto_launch
        .map(|_| get_app_auto_launch_enabled(&app))
        .transpose()?;
    let auto_launch_os_changed = requested_auto_launch
        .zip(previous_auto_launch)
        .map(|(requested, previous)| requested != previous)
        .unwrap_or(false);
    if auto_launch_os_changed {
        apply_app_auto_launch_enabled(
            &app,
            requested_auto_launch.expect("requested auto launch should exist"),
        )?;
    }

    let mut language_changed = false;
    let mut token_keeper_enabled_changed = false;
    let mut auto_import_from_local_enabled_changed = false;
    let mut floating_always_on_top_changed = false;
    #[cfg(target_os = "macos")]
    let mut hide_dock_icon_changed = false;
    #[cfg(target_os = "macos")]
    let mut tray_icon_style_changed = false;

    let patch_result = config::patch_user_config(|current| {
        let previous_language = current.language.clone();
        let previous_token_keeper_enabled = current.token_keeper_enabled;
        let previous_auto_import_from_local_enabled = current.auto_import_from_local_enabled;
        let previous_floating_always_on_top = current.floating_card_always_on_top;
        #[cfg(target_os = "macos")]
        let previous_hide_dock_icon = current.hide_dock_icon;
        #[cfg(target_os = "macos")]
        let previous_tray_icon_style = current.tray_icon_style;

        apply_general_config_updates(current, &updates)?;

        language_changed = previous_language != current.language;
        token_keeper_enabled_changed =
            previous_token_keeper_enabled != current.token_keeper_enabled;
        auto_import_from_local_enabled_changed =
            previous_auto_import_from_local_enabled != current.auto_import_from_local_enabled;
        floating_always_on_top_changed =
            previous_floating_always_on_top != current.floating_card_always_on_top;
        #[cfg(target_os = "macos")]
        {
            hide_dock_icon_changed = previous_hide_dock_icon != current.hide_dock_icon;
            tray_icon_style_changed = previous_tray_icon_style != current.tray_icon_style;
        }
        Ok(())
    });

    let new_config = match patch_result {
        Ok(config) => config,
        Err(error) => {
            if auto_launch_os_changed {
                if let Some(previous) = previous_auto_launch {
                    if let Err(rollback_error) = apply_app_auto_launch_enabled(&app, previous) {
                        modules::logger::log_error(&format!(
                            "[SystemConfig] 配置保存失败后回滚应用自启动状态失败: {}",
                            rollback_error
                        ));
                    }
                }
            }
            return Err(error);
        }
    };

    if token_keeper_enabled_changed {
        modules::provider_token_keeper::notify_config_changed(
            app.clone(),
            new_config.token_keeper_enabled,
        );
    }

    if auto_import_from_local_enabled_changed {
        modules::auto_local_import::notify_config_changed(
            new_config.auto_import_from_local_enabled,
        );
    }

    if floating_always_on_top_changed {
        if let Err(err) = modules::floating_card_window::apply_floating_card_always_on_top(&app) {
            modules::logger::log_warn(&format!(
                "[FloatingCard] 保存通用设置后应用置顶状态失败: {}",
                err
            ));
        }
    }

    #[cfg(target_os = "macos")]
    if hide_dock_icon_changed {
        crate::apply_macos_activation_policy(&app);
    }

    #[cfg(target_os = "macos")]
    if tray_icon_style_changed {
        if let Err(err) = modules::tray::apply_tray_icon_style(&app) {
            modules::logger::log_warn(&format!("[Tray] 保存通用设置后应用图标样式失败: {}", err));
        }
    }

    if language_changed {
        websocket::broadcast_language_changed(&new_config.language, "desktop");
        modules::sync_settings::write_sync_setting("language", &new_config.language);
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[Tray] 语言变更后刷新托盘失败: {}", err));
        }
    }

    Ok(())
}

/// 立即扫描并导入本机当前登录账号（开启「本机账号自动导入」后调用）。
#[tauri::command]
pub async fn scan_auto_local_import(
    app: tauri::AppHandle,
) -> Result<modules::auto_local_import::AutoLocalImportScanResult, String> {
    modules::auto_local_import::scan_now(app).await
}

// --- Codex SSH sync (#1404 vertical slice) ---
#[tauri::command]
pub fn codex_ssh_list_servers() -> Result<modules::codex_ssh::CodexSshListResult, String> {
    let (servers, selected_id) = modules::codex_ssh::list_servers()?;
    Ok(modules::codex_ssh::CodexSshListResult {
        servers,
        selected_id,
    })
}

#[tauri::command]
pub fn codex_ssh_upsert_server(
    server: modules::codex_ssh::CodexSshServer,
) -> Result<modules::codex_ssh::CodexSshServer, String> {
    modules::codex_ssh::upsert_server(server)
}

#[tauri::command]
pub fn codex_ssh_delete_server(id: String) -> Result<(), String> {
    modules::codex_ssh::delete_server(&id)
}

#[tauri::command]
pub fn codex_ssh_select_server(id: String) -> Result<(), String> {
    modules::codex_ssh::select_server(&id)
}

#[tauri::command]
pub fn codex_ssh_test_connection(id: String) -> Result<String, String> {
    modules::codex_ssh::test_connection(&id)
}

#[tauri::command]
pub fn codex_ssh_sync_current(id: String) -> Result<String, String> {
    modules::codex_ssh::sync_current_account(&id)
}

/// Managed provider id for local API LB (#980 vertical slice).
#[tauri::command]
pub fn codex_managed_lb_provider_id() -> String {
    "cockpit-codex-lb".to_string()
}

#[tauri::command]
pub fn codebuddy_list_local_session_files(
    limit: Option<u32>,
) -> Result<Vec<modules::codebuddy_session_list::CodebuddySessionFileEntry>, String> {
    Ok(modules::codebuddy_session_list::list_local_session_files(
        limit.unwrap_or(100) as usize,
    ))
}

/// 保存完整通用设置配置（兼容旧前端调用）。
#[tauri::command]
pub fn save_general_config(
    app: tauri::AppHandle,
    language: String,
    default_terminal: Option<String>,
    theme: String,
    theme_color: Option<String>,
    external_network_enabled: Option<bool>,
    webdav_allowed_domains: Option<String>,
    ui_scale: Option<f64>,
    auto_refresh_minutes: i32,
    codex_auto_refresh_minutes: i32,
    codex_sync_wsl: Option<bool>,
    codex_wsl_config_dir: Option<String>,
    zed_auto_refresh_minutes: Option<i32>,
    ghcp_auto_refresh_minutes: Option<i32>,
    windsurf_auto_refresh_minutes: Option<i32>,
    kiro_auto_refresh_minutes: Option<i32>,
    cursor_auto_refresh_minutes: Option<i32>,
    grok_auto_refresh_minutes: Option<i32>,
    grok_sync_official_auth_on_switch: Option<bool>,
    claude_auto_refresh_minutes: Option<i32>,
    codebuddy_auto_refresh_minutes: Option<i32>,
    codebuddy_cn_auto_refresh_minutes: Option<i32>,
    workbuddy_auto_refresh_minutes: Option<i32>,
    qoder_auto_refresh_minutes: Option<i32>,
    zcode_auto_refresh_minutes: Option<i32>,
    trae_auto_refresh_minutes: Option<i32>,
    trae_solo_auto_refresh_minutes: Option<i32>,
    trae_cn_auto_refresh_minutes: Option<i32>,
    trae_solo_cn_auto_refresh_minutes: Option<i32>,
    close_behavior: String,
    minimize_behavior: Option<String>,
    hide_dock_icon: Option<bool>,
    tray_icon_style: Option<String>,
    floating_card_show_on_startup: Option<bool>,
    startup_minimized: Option<bool>,
    startup_page: Option<String>,
    floating_card_always_on_top: Option<bool>,
    app_auto_launch_enabled: Option<bool>,
    token_keeper_enabled: Option<bool>,
    auto_import_from_local_enabled: Option<bool>,
    antigravity_startup_wakeup_enabled: Option<bool>,
    antigravity_startup_wakeup_delay_seconds: Option<i32>,
    codex_startup_wakeup_enabled: Option<bool>,
    codex_startup_wakeup_delay_seconds: Option<i32>,
    floating_card_confirm_on_close: Option<bool>,
    opencode_app_path: String,
    antigravity_app_path: String,
    codex_app_path: String,
    claude_app_path: Option<String>,
    claude_app_scan_roots: Option<String>,
    codex_specified_app_path: Option<String>,
    zed_app_path: Option<String>,
    vscode_app_path: String,
    windsurf_app_path: Option<String>,
    kiro_app_path: Option<String>,
    cursor_app_path: Option<String>,
    codebuddy_app_path: Option<String>,
    codebuddy_cn_app_path: Option<String>,
    qoder_app_path: Option<String>,
    zcode_app_path: Option<String>,
    trae_app_path: Option<String>,
    trae_solo_app_path: Option<String>,
    trae_cn_app_path: Option<String>,
    trae_solo_cn_app_path: Option<String>,
    trae_app_scan_roots: Option<String>,
    trae_solo_app_scan_roots: Option<String>,
    trae_cn_app_scan_roots: Option<String>,
    trae_solo_cn_app_scan_roots: Option<String>,
    workbuddy_app_path: Option<String>,
    opencode_sync_on_switch: bool,
    opencode_auth_overwrite_on_switch: Option<bool>,
    ghcp_opencode_sync_on_switch: Option<bool>,
    ghcp_opencode_auth_overwrite_on_switch: Option<bool>,
    ghcp_launch_on_switch: Option<bool>,
    openclaw_auth_overwrite_on_switch: Option<bool>,
    hermes_auth_overwrite_on_switch: Option<bool>,
    codex_launch_on_switch: bool,
    antigravity_launch_on_switch: Option<bool>,
    codex_restart_specified_app_on_switch: Option<bool>,
    codex_local_access_entry_visible: Option<bool>,
    top_right_ad_visible: Option<bool>,
    antigravity_dual_switch_no_restart_enabled: Option<bool>,
    auto_switch_enabled: Option<bool>,
    auto_switch_threshold: Option<i32>,
    auto_switch_credits_enabled: Option<bool>,
    auto_switch_credits_threshold: Option<i32>,
    auto_switch_scope_mode: Option<String>,
    auto_switch_selected_group_ids: Option<Vec<String>>,
    auto_switch_account_scope_mode: Option<String>,
    auto_switch_selected_account_ids: Option<Vec<String>>,
    codex_auto_switch_enabled: Option<bool>,
    codex_auto_switch_primary_threshold: Option<i32>,
    codex_auto_switch_secondary_threshold: Option<i32>,
    codex_auto_switch_account_scope_mode: Option<String>,
    codex_auto_switch_selected_account_ids: Option<Vec<String>>,
    quota_alert_enabled: Option<bool>,
    quota_alert_threshold: Option<i32>,
    codex_quota_alert_enabled: Option<bool>,
    codex_quota_alert_threshold: Option<i32>,
    zed_quota_alert_enabled: Option<bool>,
    zed_quota_alert_threshold: Option<i32>,
    codex_quota_alert_primary_threshold: Option<i32>,
    codex_quota_alert_secondary_threshold: Option<i32>,
    ghcp_quota_alert_enabled: Option<bool>,
    ghcp_quota_alert_threshold: Option<i32>,
    windsurf_quota_alert_enabled: Option<bool>,
    windsurf_quota_alert_threshold: Option<i32>,
    kiro_quota_alert_enabled: Option<bool>,
    kiro_quota_alert_threshold: Option<i32>,
    cursor_quota_alert_enabled: Option<bool>,
    cursor_quota_alert_threshold: Option<i32>,
    grok_quota_alert_enabled: Option<bool>,
    grok_quota_alert_threshold: Option<i32>,
    claude_quota_alert_enabled: Option<bool>,
    claude_quota_alert_threshold: Option<i32>,
    codebuddy_quota_alert_enabled: Option<bool>,
    codebuddy_quota_alert_threshold: Option<i32>,
    codebuddy_cn_quota_alert_enabled: Option<bool>,
    codebuddy_cn_quota_alert_threshold: Option<i32>,
    qoder_quota_alert_enabled: Option<bool>,
    qoder_quota_alert_threshold: Option<i32>,
    trae_quota_alert_enabled: Option<bool>,
    trae_quota_alert_threshold: Option<i32>,
    trae_solo_quota_alert_enabled: Option<bool>,
    trae_solo_quota_alert_threshold: Option<i32>,
    trae_cn_quota_alert_enabled: Option<bool>,
    trae_cn_quota_alert_threshold: Option<i32>,
    trae_solo_cn_quota_alert_enabled: Option<bool>,
    trae_solo_cn_quota_alert_threshold: Option<i32>,
    workbuddy_quota_alert_enabled: Option<bool>,
    workbuddy_quota_alert_threshold: Option<i32>,
) -> Result<(), String> {
    let normalized_language = language.to_lowercase();
    let language_for_broadcast = normalized_language.clone();
    let normalized_opencode_path =
        modules::process::normalize_windows_user_facing_path(&opencode_app_path);
    let normalized_antigravity_path =
        modules::process::normalize_windows_user_facing_path(&antigravity_app_path);
    let normalized_codex_path =
        modules::process::normalize_windows_user_facing_path(&codex_app_path);
    let normalized_vscode_path =
        modules::process::normalize_windows_user_facing_path(&vscode_app_path);
    let normalized_codex_wsl_config_dir =
        codex_wsl_config_dir.map(|value| value.trim().to_string());
    let normalized_claude_path =
        claude_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_claude_app_scan_roots =
        claude_app_scan_roots.map(|value| value.trim().to_string());
    let normalized_codex_specified_app_path = codex_specified_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_zed_path =
        zed_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_windsurf_path =
        windsurf_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_kiro_path =
        kiro_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_cursor_path =
        cursor_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_codebuddy_path = codebuddy_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_codebuddy_cn_path = codebuddy_cn_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_qoder_path =
        qoder_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_zcode_path =
        zcode_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_trae_path =
        trae_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_trae_solo_path = trae_solo_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_trae_cn_path =
        trae_cn_app_path.map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_trae_solo_cn_path = trae_solo_cn_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));
    let normalized_trae_app_scan_roots = trae_app_scan_roots.map(|value| value.trim().to_string());
    let normalized_trae_solo_app_scan_roots =
        trae_solo_app_scan_roots.map(|value| value.trim().to_string());
    let normalized_trae_cn_app_scan_roots =
        trae_cn_app_scan_roots.map(|value| value.trim().to_string());
    let normalized_trae_solo_cn_app_scan_roots =
        trae_solo_cn_app_scan_roots.map(|value| value.trim().to_string());
    let normalized_workbuddy_path = workbuddy_app_path
        .map(|value| modules::process::normalize_windows_user_facing_path(&value));

    let close_behavior_value = match close_behavior.as_str() {
        "minimize" => CloseWindowBehavior::Minimize,
        "quit" => CloseWindowBehavior::Quit,
        _ => CloseWindowBehavior::Ask,
    };
    let minimize_behavior_value = minimize_behavior.as_deref().and_then(|value| match value {
        "dock_and_tray" => Some(MinimizeWindowBehavior::DockAndTray),
        "tray_only" => Some(MinimizeWindowBehavior::TrayOnly),
        _ => None,
    });
    let tray_icon_style_value = tray_icon_style.as_deref().map(TrayIconStyle::from_str);

    let mut language_changed = false;
    let mut token_keeper_enabled_changed = false;
    let mut auto_import_from_local_enabled_changed = false;
    let mut current_app_auto_launch_enabled = false;
    #[cfg(target_os = "macos")]
    let mut hide_dock_icon_changed = false;
    #[cfg(target_os = "macos")]
    let mut tray_icon_style_changed = false;

    let new_config = config::patch_user_config(|current| {
        language_changed = current.language != normalized_language;
        token_keeper_enabled_changed = token_keeper_enabled
            .map(|enabled| current.token_keeper_enabled != enabled)
            .unwrap_or(false);
        auto_import_from_local_enabled_changed = auto_import_from_local_enabled
            .map(|enabled| current.auto_import_from_local_enabled != enabled)
            .unwrap_or(false);
        current_app_auto_launch_enabled = current.app_auto_launch_enabled;
        #[cfg(target_os = "macos")]
        {
            hide_dock_icon_changed = hide_dock_icon
                .map(|hidden| current.hide_dock_icon != hidden)
                .unwrap_or(false);
            tray_icon_style_changed = tray_icon_style_value
                .as_ref()
                .map(|style| current.tray_icon_style != *style)
                .unwrap_or(false);
        }

        current.language = normalized_language.clone();
        if let Some(value) = default_terminal {
            current.default_terminal = value;
        }
        current.theme = theme;
        current.ui_scale = sanitize_ui_scale(ui_scale.unwrap_or(current.ui_scale));
        current.auto_refresh_minutes = auto_refresh_minutes;
        current.codex_auto_refresh_minutes = codex_auto_refresh_minutes;
        if let Some(value) = codex_sync_wsl {
            current.codex_sync_wsl = value;
        }
        if let Some(value) = normalized_codex_wsl_config_dir {
            current.codex_wsl_config_dir = value;
        }
        if let Some(value) = zed_auto_refresh_minutes {
            current.zed_auto_refresh_minutes = value;
        }
        if let Some(value) = ghcp_auto_refresh_minutes {
            current.ghcp_auto_refresh_minutes = value;
        }
        if let Some(value) = windsurf_auto_refresh_minutes {
            current.windsurf_auto_refresh_minutes = value;
        }
        if let Some(value) = kiro_auto_refresh_minutes {
            current.kiro_auto_refresh_minutes = value;
        }
        if let Some(value) = cursor_auto_refresh_minutes {
            current.cursor_auto_refresh_minutes = value;
        }
        if let Some(value) = grok_auto_refresh_minutes {
            current.grok_auto_refresh_minutes = value;
        }
        if let Some(value) = grok_sync_official_auth_on_switch {
            current.grok_sync_official_auth_on_switch = value;
        }
        if let Some(value) = claude_auto_refresh_minutes {
            current.claude_auto_refresh_minutes = value;
        }
        if let Some(value) = codebuddy_auto_refresh_minutes {
            current.codebuddy_auto_refresh_minutes = value;
        }
        if let Some(value) = codebuddy_cn_auto_refresh_minutes {
            current.codebuddy_cn_auto_refresh_minutes = value;
        }
        if let Some(value) = workbuddy_auto_refresh_minutes {
            current.workbuddy_auto_refresh_minutes = value;
        }
        if let Some(value) = qoder_auto_refresh_minutes {
            current.qoder_auto_refresh_minutes = value;
        }
        if let Some(value) = zcode_auto_refresh_minutes {
            current.zcode_auto_refresh_minutes = value;
        }
        if let Some(value) = trae_auto_refresh_minutes {
            current.trae_auto_refresh_minutes = value;
        }
        if let Some(value) = trae_solo_auto_refresh_minutes {
            current.trae_solo_auto_refresh_minutes = value;
        }
        if let Some(value) = trae_cn_auto_refresh_minutes {
            current.trae_cn_auto_refresh_minutes = value;
        }
        if let Some(value) = trae_solo_cn_auto_refresh_minutes {
            current.trae_solo_cn_auto_refresh_minutes = value;
        }

        current.close_behavior = close_behavior_value;
        if let Some(value) = minimize_behavior_value {
            current.minimize_behavior = value;
        }
        if let Some(value) = hide_dock_icon {
            current.hide_dock_icon = value;
        }
        if let Some(value) = tray_icon_style_value {
            current.tray_icon_style = value;
        }
        if let Some(value) = floating_card_show_on_startup {
            current.floating_card_show_on_startup = value;
        }
        if let Some(value) = startup_minimized {
            current.startup_minimized = value;
        }
        if let Some(value) = startup_page {
            current.startup_page = config::normalize_startup_page(&value);
        }
        if let Some(value) = theme_color {
            current.theme_color = config::normalize_theme_color(&value);
        }
        if let Some(value) = external_network_enabled {
            current.external_network_enabled = value;
        }
        if let Some(value) = webdav_allowed_domains {
            current.webdav_allowed_domains = value.trim().to_string();
        }
        if let Some(value) = floating_card_always_on_top {
            current.floating_card_always_on_top = value;
        }
        if let Some(value) = app_auto_launch_enabled {
            current.app_auto_launch_enabled = value;
        }
        if let Some(value) = token_keeper_enabled {
            current.token_keeper_enabled = value;
        }
        if let Some(value) = auto_import_from_local_enabled {
            current.auto_import_from_local_enabled = value;
        }
        if let Some(value) = antigravity_startup_wakeup_enabled {
            current.antigravity_startup_wakeup_enabled = value;
        }
        if let Some(value) = antigravity_startup_wakeup_delay_seconds {
            current.antigravity_startup_wakeup_delay_seconds =
                sanitize_startup_wakeup_delay_seconds(value);
        }
        if let Some(value) = codex_startup_wakeup_enabled {
            current.codex_startup_wakeup_enabled = value;
        }
        if let Some(value) = codex_startup_wakeup_delay_seconds {
            current.codex_startup_wakeup_delay_seconds =
                sanitize_startup_wakeup_delay_seconds(value);
        }
        if let Some(value) = floating_card_confirm_on_close {
            current.floating_card_confirm_on_close = value;
        }

        current.opencode_app_path = normalized_opencode_path;
        current.antigravity_app_path = normalized_antigravity_path;
        current.codex_app_path = normalized_codex_path;
        current.vscode_app_path = normalized_vscode_path;
        if let Some(value) = normalized_claude_path {
            current.claude_app_path = value;
        }
        if let Some(value) = normalized_claude_app_scan_roots {
            current.claude_app_scan_roots = value;
        }
        if let Some(value) = normalized_codex_specified_app_path {
            current.codex_specified_app_path = value;
        }
        if let Some(value) = normalized_zed_path {
            current.zed_app_path = value;
        }
        if let Some(value) = normalized_windsurf_path {
            current.windsurf_app_path = value;
        }
        if let Some(value) = normalized_kiro_path {
            current.kiro_app_path = value;
        }
        if let Some(value) = normalized_cursor_path {
            current.cursor_app_path = value;
        }
        if let Some(value) = normalized_codebuddy_path {
            current.codebuddy_app_path = value;
        }
        if let Some(value) = normalized_codebuddy_cn_path {
            current.codebuddy_cn_app_path = value;
        }
        if let Some(value) = normalized_qoder_path {
            current.qoder_app_path = value;
        }
        if let Some(value) = normalized_zcode_path {
            current.zcode_app_path = value;
        }
        if let Some(value) = normalized_trae_path {
            current.trae_app_path = value;
        }
        if let Some(value) = normalized_trae_solo_path {
            current.trae_solo_app_path = value;
        }
        if let Some(value) = normalized_trae_cn_path {
            current.trae_cn_app_path = value;
        }
        if let Some(value) = normalized_trae_solo_cn_path {
            current.trae_solo_cn_app_path = value;
        }
        if let Some(value) = normalized_trae_app_scan_roots {
            current.trae_app_scan_roots = value;
        }
        if let Some(value) = normalized_trae_solo_app_scan_roots {
            current.trae_solo_app_scan_roots = value;
        }
        if let Some(value) = normalized_trae_cn_app_scan_roots {
            current.trae_cn_app_scan_roots = value;
        }
        if let Some(value) = normalized_trae_solo_cn_app_scan_roots {
            current.trae_solo_cn_app_scan_roots = value;
        }
        if let Some(value) = normalized_workbuddy_path {
            current.workbuddy_app_path = value;
        }

        let next_opencode_auth_overwrite_on_switch =
            opencode_auth_overwrite_on_switch.unwrap_or(current.opencode_auth_overwrite_on_switch);
        current.opencode_auth_overwrite_on_switch = next_opencode_auth_overwrite_on_switch;
        current.opencode_sync_on_switch = if next_opencode_auth_overwrite_on_switch {
            opencode_sync_on_switch
        } else {
            false
        };

        let next_ghcp_opencode_auth_overwrite_on_switch = ghcp_opencode_auth_overwrite_on_switch
            .unwrap_or(current.ghcp_opencode_auth_overwrite_on_switch);
        current.ghcp_opencode_auth_overwrite_on_switch =
            next_ghcp_opencode_auth_overwrite_on_switch;
        current.ghcp_opencode_sync_on_switch = if next_ghcp_opencode_auth_overwrite_on_switch {
            ghcp_opencode_sync_on_switch.unwrap_or(current.ghcp_opencode_sync_on_switch)
        } else {
            false
        };
        if let Some(value) = ghcp_launch_on_switch {
            current.ghcp_launch_on_switch = value;
        }
        if let Some(value) = openclaw_auth_overwrite_on_switch {
            current.openclaw_auth_overwrite_on_switch = value;
        }
        if let Some(value) = hermes_auth_overwrite_on_switch {
            current.hermes_auth_overwrite_on_switch = value;
        }
        current.codex_launch_on_switch = codex_launch_on_switch;
        if let Some(value) = antigravity_launch_on_switch {
            current.antigravity_launch_on_switch = value;
        }
        if let Some(value) = codex_restart_specified_app_on_switch {
            current.codex_restart_specified_app_on_switch = value;
        }
        if let Some(value) = codex_local_access_entry_visible {
            current.codex_local_access_entry_visible = value;
        }
        if let Some(value) = top_right_ad_visible {
            current.top_right_ad_visible = value;
        }
        if let Some(value) = antigravity_dual_switch_no_restart_enabled {
            current.antigravity_dual_switch_no_restart_enabled = value;
        }

        if let Some(value) = auto_switch_enabled {
            current.auto_switch_enabled = value;
        }
        if let Some(value) = auto_switch_threshold {
            current.auto_switch_threshold = value;
        }
        if let Some(value) = auto_switch_credits_enabled {
            current.auto_switch_credits_enabled = value;
        }
        if let Some(value) = auto_switch_credits_threshold {
            current.auto_switch_credits_threshold = value;
        }
        if let Some(value) = auto_switch_scope_mode
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            current.auto_switch_scope_mode = value;
        }
        if let Some(value) = auto_switch_selected_group_ids {
            current.auto_switch_selected_group_ids = value;
        }
        if let Some(value) = auto_switch_account_scope_mode {
            current.auto_switch_account_scope_mode =
                normalize_auto_switch_account_scope_mode(&value);
        }
        if let Some(value) = auto_switch_selected_account_ids {
            current.auto_switch_selected_account_ids =
                normalize_auto_switch_selected_account_ids(&value);
        }
        if let Some(value) = codex_auto_switch_enabled {
            current.codex_auto_switch_enabled = value;
        }
        if let Some(value) = codex_auto_switch_primary_threshold {
            current.codex_auto_switch_primary_threshold = value;
        }
        if let Some(value) = codex_auto_switch_secondary_threshold {
            current.codex_auto_switch_secondary_threshold = value;
        }
        if let Some(value) = codex_auto_switch_account_scope_mode {
            current.codex_auto_switch_account_scope_mode =
                normalize_auto_switch_account_scope_mode(&value);
        }
        if let Some(value) = codex_auto_switch_selected_account_ids {
            current.codex_auto_switch_selected_account_ids =
                normalize_auto_switch_selected_account_ids(&value);
        }

        if let Some(value) = quota_alert_enabled {
            current.quota_alert_enabled = value;
        }
        if let Some(value) = quota_alert_threshold {
            current.quota_alert_threshold = value;
        }
        if let Some(value) = codex_quota_alert_enabled {
            current.codex_quota_alert_enabled = value;
        }
        apply_codex_quota_alert_thresholds(
            current,
            codex_quota_alert_threshold,
            codex_quota_alert_primary_threshold,
            codex_quota_alert_secondary_threshold,
        );
        if let Some(value) = zed_quota_alert_enabled {
            current.zed_quota_alert_enabled = value;
        }
        if let Some(value) = zed_quota_alert_threshold {
            current.zed_quota_alert_threshold = value;
        }
        if let Some(value) = ghcp_quota_alert_enabled {
            current.ghcp_quota_alert_enabled = value;
        }
        if let Some(value) = ghcp_quota_alert_threshold {
            current.ghcp_quota_alert_threshold = value;
        }
        if let Some(value) = windsurf_quota_alert_enabled {
            current.windsurf_quota_alert_enabled = value;
        }
        if let Some(value) = windsurf_quota_alert_threshold {
            current.windsurf_quota_alert_threshold = value;
        }
        if let Some(value) = kiro_quota_alert_enabled {
            current.kiro_quota_alert_enabled = value;
        }
        if let Some(value) = kiro_quota_alert_threshold {
            current.kiro_quota_alert_threshold = value;
        }
        if let Some(value) = cursor_quota_alert_enabled {
            current.cursor_quota_alert_enabled = value;
        }
        if let Some(value) = cursor_quota_alert_threshold {
            current.cursor_quota_alert_threshold = value;
        }
        if let Some(value) = grok_quota_alert_enabled {
            current.grok_quota_alert_enabled = value;
        }
        if let Some(value) = grok_quota_alert_threshold {
            current.grok_quota_alert_threshold = value;
        }
        if let Some(value) = claude_quota_alert_enabled {
            current.claude_quota_alert_enabled = value;
        }
        if let Some(value) = claude_quota_alert_threshold {
            current.claude_quota_alert_threshold = value;
        }
        if let Some(value) = codebuddy_quota_alert_enabled {
            current.codebuddy_quota_alert_enabled = value;
        }
        if let Some(value) = codebuddy_quota_alert_threshold {
            current.codebuddy_quota_alert_threshold = value;
        }
        if let Some(value) = codebuddy_cn_quota_alert_enabled {
            current.codebuddy_cn_quota_alert_enabled = value;
        }
        if let Some(value) = codebuddy_cn_quota_alert_threshold {
            current.codebuddy_cn_quota_alert_threshold = value;
        }
        if let Some(value) = qoder_quota_alert_enabled {
            current.qoder_quota_alert_enabled = value;
        }
        if let Some(value) = qoder_quota_alert_threshold {
            current.qoder_quota_alert_threshold = value;
        }
        if let Some(value) = trae_quota_alert_enabled {
            current.trae_quota_alert_enabled = value;
        }
        if let Some(value) = trae_quota_alert_threshold {
            current.trae_quota_alert_threshold = value;
        }
        if let Some(value) = trae_solo_quota_alert_enabled {
            current.trae_solo_quota_alert_enabled = value;
        }
        if let Some(value) = trae_solo_quota_alert_threshold {
            current.trae_solo_quota_alert_threshold = value;
        }
        if let Some(value) = trae_cn_quota_alert_enabled {
            current.trae_cn_quota_alert_enabled = value;
        }
        if let Some(value) = trae_cn_quota_alert_threshold {
            current.trae_cn_quota_alert_threshold = value;
        }
        if let Some(value) = trae_solo_cn_quota_alert_enabled {
            current.trae_solo_cn_quota_alert_enabled = value;
        }
        if let Some(value) = trae_solo_cn_quota_alert_threshold {
            current.trae_solo_cn_quota_alert_threshold = value;
        }
        if let Some(value) = workbuddy_quota_alert_enabled {
            current.workbuddy_quota_alert_enabled = value;
        }
        if let Some(value) = workbuddy_quota_alert_threshold {
            current.workbuddy_quota_alert_threshold = value;
        }

        Ok(())
    })?;

    if token_keeper_enabled_changed {
        modules::provider_token_keeper::notify_config_changed(
            app.clone(),
            new_config.token_keeper_enabled,
        );
    }

    if auto_import_from_local_enabled_changed {
        modules::auto_local_import::notify_config_changed(
            new_config.auto_import_from_local_enabled,
        );
    }

    if current_app_auto_launch_enabled != new_config.app_auto_launch_enabled {
        apply_app_auto_launch_enabled(&app, new_config.app_auto_launch_enabled)?;
    }

    if let Err(err) = modules::floating_card_window::apply_floating_card_always_on_top(&app) {
        modules::logger::log_warn(&format!(
            "[FloatingCard] 保存通用设置后应用置顶状态失败: {}",
            err
        ));
    }

    #[cfg(target_os = "macos")]
    if hide_dock_icon_changed {
        crate::apply_macos_activation_policy(&app);
    }

    #[cfg(target_os = "macos")]
    if tray_icon_style_changed {
        if let Err(err) = modules::tray::apply_tray_icon_style(&app) {
            modules::logger::log_warn(&format!("[Tray] 保存通用设置后应用图标样式失败: {}", err));
        }
    }

    if language_changed {
        websocket::broadcast_language_changed(&language_for_broadcast, "desktop");
        modules::sync_settings::write_sync_setting("language", &normalized_language);
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[Tray] 语言变更后刷新托盘失败: {}", err));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn save_refresh_interval_config(
    auto_refresh_minutes: Option<i32>,
    codex_auto_refresh_minutes: Option<i32>,
) -> Result<(), String> {
    config::patch_user_config(|current| {
        if let Some(value) = auto_refresh_minutes {
            current.auto_refresh_minutes = value;
        }
        if let Some(value) = codex_auto_refresh_minutes {
            current.codex_auto_refresh_minutes = value;
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn save_tray_platform_layout(
    app: tauri::AppHandle,
    sort_mode: String,
    ordered_platform_ids: Vec<String>,
    tray_platform_ids: Vec<String>,
    ordered_entry_ids: Option<Vec<String>>,
    platform_groups: Option<Vec<modules::tray_layout::TrayLayoutGroup>>,
) -> Result<(), String> {
    modules::tray_layout::save_tray_layout(
        sort_mode,
        ordered_platform_ids,
        tray_platform_ids,
        ordered_entry_ids,
        platform_groups,
    )?;
    modules::tray::update_tray_menu(&app)?;
    Ok(())
}

#[tauri::command]
pub fn set_app_path(app: String, path: String) -> Result<(), String> {
    let normalized_path = modules::process::normalize_windows_user_facing_path(&path);
    config::patch_user_config(move |current| {
        match app.as_str() {
            "antigravity" | "antigravity_ide" | "antigravity_legacy" => {
                current.antigravity_app_path = normalized_path
            }
            "codex" => current.codex_app_path = normalized_path,
            "claude" => current.claude_app_path = normalized_path,
            "zed" => current.zed_app_path = normalized_path,
            "vscode" => current.vscode_app_path = normalized_path,
            "windsurf" => current.windsurf_app_path = normalized_path,
            "kiro" => current.kiro_app_path = normalized_path,
            "cursor" => current.cursor_app_path = normalized_path,
            "codebuddy" => current.codebuddy_app_path = normalized_path,
            "codebuddy_cn" => current.codebuddy_cn_app_path = normalized_path,
            "qoder" => current.qoder_app_path = normalized_path,
            "zcode" => current.zcode_app_path = normalized_path,
            "trae" => current.trae_app_path = normalized_path,
            "trae_solo" => current.trae_solo_app_path = normalized_path,
            "trae_cn" => current.trae_cn_app_path = normalized_path,
            "trae_solo_cn" => current.trae_solo_cn_app_path = normalized_path,
            "workbuddy" => current.workbuddy_app_path = normalized_path,
            "opencode" => current.opencode_app_path = normalized_path,
            _ => return Err("未知应用类型".to_string()),
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_claude_app_scan_roots(scan_roots: String) -> Result<(), String> {
    let normalized = scan_roots.trim().to_string();
    config::patch_user_config(move |current| {
        current.claude_app_scan_roots = normalized;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_trae_app_scan_roots(app: Option<String>, scan_roots: String) -> Result<(), String> {
    let normalized = scan_roots.trim().to_string();
    let target = app
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("trae")
        .to_string();
    config::patch_user_config(move |current| {
        match target.as_str() {
            "trae" => current.trae_app_scan_roots = normalized,
            "trae_solo" => current.trae_solo_app_scan_roots = normalized,
            "trae_cn" => current.trae_cn_app_scan_roots = normalized,
            "trae_solo_cn" => current.trae_solo_cn_app_scan_roots = normalized,
            _ => return Err("鏈煡搴旂敤绫诲瀷".to_string()),
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_launch_on_switch(enabled: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.codex_launch_on_switch = enabled;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_local_access_entry_visible(enabled: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.codex_local_access_entry_visible = enabled;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn detect_app_path(app: String, force: Option<bool>) -> Result<Option<String>, String> {
    let force = force.unwrap_or(false);
    match app.as_str() {
        "windsurf" => Ok(modules::windsurf_instance::detect_and_save_windsurf_launch_path(force)),
        "kiro" => Ok(modules::kiro_instance::detect_and_save_kiro_launch_path(
            force,
        )),
        "cursor" => Ok(modules::cursor_instance::detect_and_save_cursor_launch_path(force)),
        "claude" => Ok(modules::claude_instance::detect_and_save_claude_launch_path(force)),
        "antigravity" | "antigravity_ide" | "antigravity_legacy" | "codex" | "zed" | "vscode"
        | "codebuddy" | "codebuddy_cn" | "qoder" | "zcode" | "trae" | "trae_solo" | "trae_cn"
        | "trae_solo_cn" | "opencode" | "workbuddy" => Ok(
            modules::process::detect_and_save_app_path(app.as_str(), force),
        ),
        _ => Err("未知应用类型".to_string()),
    }
}

#[tauri::command]
pub async fn scan_claude_desktop_launch_targets(
    scan_roots: Option<String>,
) -> Result<Vec<modules::claude_instance::ClaudeDesktopLaunchCandidate>, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = scan_roots;
        let task = tauri::async_runtime::spawn_blocking(|| {
            modules::process::scan_app_launch_targets("claude", None)
        });
        let candidates = match tokio::time::timeout(Duration::from_secs(2), task).await {
            Ok(Ok(result)) => result?,
            Ok(Err(error)) => return Err(format!("检测运行中的 Claude 任务失败: {error}")),
            Err(_) => return Err("检测运行中的 Claude 超时，请重试".to_string()),
        };
        return Ok(candidates
            .into_iter()
            .map(
                |candidate| modules::claude_instance::ClaudeDesktopLaunchCandidate {
                    target_type: candidate.target_type,
                    label: candidate.label,
                    target: candidate.target,
                    source: candidate.source,
                    supports_multi_instance: candidate.supports_multi_instance,
                },
            )
            .collect());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let roots = scan_roots
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        Ok(modules::claude_instance::scan_claude_desktop_launch_targets(roots))
    }
}

#[tauri::command]
pub async fn scan_app_launch_targets(
    app: String,
    scan_roots: Option<String>,
) -> Result<Vec<modules::process::AppLaunchCandidate>, String> {
    match app.as_str() {
        "antigravity" | "antigravity_ide" | "antigravity_legacy" | "codex" | "claude"
        | "vscode" | "windsurf" | "kiro" | "cursor" | "codebuddy" | "codebuddy_cn" | "qoder"
        | "zcode" | "trae" | "trae_solo" | "trae_cn" | "trae_solo_cn" | "workbuddy" | "zed"
        | "opencode" => {}
        _ => return Err("未知应用类型".to_string()),
    }
    let _ = scan_roots;

    let task = tauri::async_runtime::spawn_blocking(move || {
        modules::process::scan_app_launch_targets(app.as_str(), None)
    });
    match tokio::time::timeout(Duration::from_secs(2), task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("检测运行中的应用任务失败: {error}")),
        Err(_) => Err("检测运行中的应用超时，请重试".to_string()),
    }
}

#[tauri::command]
pub async fn get_antigravity_installed_version_info(
    target: Option<String>,
    scan_mode: Option<String>,
) -> Result<Option<AntigravityInstalledVersionInfo>, String> {
    let scan_mode = normalize_antigravity_version_scan_mode(scan_mode.as_deref());
    let timeout_ms = match scan_mode {
        AntigravityVersionScanMode::Quick => ANTIGRAVITY_VERSION_BADGE_TIMEOUT_MS,
        AntigravityVersionScanMode::Full => ANTIGRAVITY_VERSION_FULL_SCAN_TIMEOUT_MS,
    };
    let target_for_task = target.clone();

    let task = tauri::async_runtime::spawn_blocking(move || match scan_mode {
        AntigravityVersionScanMode::Quick => {
            resolve_antigravity_installed_version_info_quick_for_target(target_for_task.as_deref())
        }
        AntigravityVersionScanMode::Full => {
            resolve_antigravity_installed_version_info_for_target(target_for_task.as_deref())
        }
    });

    match tokio::time::timeout(Duration::from_millis(timeout_ms), task).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => Err(format!("Antigravity 版本检测任务失败: {}", error)),
        Err(_) => Ok(None),
    }
}

/// 通知插件关闭/开启唤醒功能（互斥）
#[tauri::command]
pub fn set_wakeup_override(enabled: bool) -> Result<(), String> {
    websocket::broadcast_wakeup_override(enabled);
    Ok(())
}

/// 执行窗口关闭操作
/// action: "minimize" | "quit"
/// remember: 是否记住选择
#[tauri::command]
pub fn handle_window_close(
    window: tauri::Window,
    action: String,
    remember: bool,
) -> Result<(), String> {
    modules::logger::log_info(&format!(
        "[Window] 用户选择: action={}, remember={}",
        action, remember
    ));

    // 如果需要记住选择，更新配置
    if remember {
        let close_behavior = match action.as_str() {
            "minimize" => CloseWindowBehavior::Minimize,
            "quit" => CloseWindowBehavior::Quit,
            _ => CloseWindowBehavior::Ask,
        };
        config::patch_user_config(move |current| {
            current.close_behavior = close_behavior;
            Ok(())
        })?;
        modules::logger::log_info(&format!("[Window] 已保存关闭行为设置: {}", action));
    }

    // 执行操作
    match action.as_str() {
        "minimize" => {
            if let Err(err) = modules::floating_card_window::destroy_main_window_to_tray(&window) {
                modules::logger::log_warn(&format!("[Window] 销毁主窗口失败，回退隐藏: {}", err));
                let _ = window.hide();
                modules::process_memory::trim_idle_process_memory();
            }
            modules::logger::log_info("[Window] 窗口已关闭到托盘");
        }
        "quit" => {
            modules::floating_card_window::request_app_exit();
            window.app_handle().exit(0);
        }
        _ => {
            return Err("无效的操作".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub fn main_window_take_pending_navigation() -> Result<Option<String>, String> {
    modules::floating_card_window::take_pending_main_window_navigation()
}

#[tauri::command]
pub fn show_floating_card_window(app: tauri::AppHandle) -> Result<(), String> {
    modules::floating_card_window::show_floating_card_window(&app, true)
}

#[tauri::command]
pub fn show_instance_floating_card_window(
    app: tauri::AppHandle,
    context: modules::floating_card_window::FloatingCardInstanceContext,
) -> Result<(), String> {
    modules::floating_card_window::show_instance_floating_card_window(&app, context, true)
}

#[tauri::command]
pub fn get_floating_card_context(
    window_label: String,
) -> Result<Option<modules::floating_card_window::FloatingCardInstanceContext>, String> {
    modules::floating_card_window::get_floating_card_context(&window_label)
}

#[tauri::command]
pub fn hide_floating_card_window(app: tauri::AppHandle) -> Result<(), String> {
    modules::floating_card_window::hide_floating_card_window(&app, false)
}

#[tauri::command]
pub fn hide_current_floating_card_window(window: tauri::Window) -> Result<(), String> {
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_floating_card_always_on_top(
    app: tauri::AppHandle,
    always_on_top: bool,
) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_always_on_top = always_on_top;
        Ok(())
    })?;
    modules::floating_card_window::apply_floating_card_always_on_top(&app)
}

#[tauri::command]
pub fn set_current_floating_card_window_always_on_top(
    window: tauri::Window,
    always_on_top: bool,
) -> Result<(), String> {
    window
        .set_always_on_top(always_on_top)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_floating_card_confirm_on_close(confirm_on_close: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_confirm_on_close = confirm_on_close;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn save_floating_card_position(x: i32, y: i32) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_position_x = Some(x);
        current.floating_card_position_y = Some(y);
        Ok(())
    })?;
    Ok(())
}

/// Must run window recreate on the UI/main thread. Sync invoke handlers run on a
/// worker pool; building a WebView there hangs on Windows after tray destroy.
#[tauri::command]
pub async fn show_main_window_and_navigate(
    app: tauri::AppHandle,
    page: String,
) -> Result<(), String> {
    modules::floating_card_window::show_main_window_and_navigate_async(app, page).await
}

#[tauri::command]
pub fn external_import_take_pending(
) -> Option<modules::external_import::ExternalProviderImportPayload> {
    modules::external_import::take_pending_external_import()
}

#[tauri::command]
pub async fn external_import_fetch_import_url(import_url: String) -> Result<String, String> {
    const MAX_IMPORT_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

    let import_url = import_url.trim();
    if import_url.is_empty() {
        return Err("导入包地址为空".to_string());
    }

    let parsed = Url::parse(import_url).map_err(|err| format!("导入包地址无效: {}", err))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("导入包地址仅支持 http/https".to_string());
    }

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| format!("创建网络客户端失败: {}", err))?
        .get(parsed)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| format!("拉取导入包失败: {}", err))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("拉取导入包失败: HTTP {}", status.as_u16()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("读取导入包失败: {}", err))?;
    if bytes.len() > MAX_IMPORT_BUNDLE_BYTES {
        return Err("导入包过大".to_string());
    }

    String::from_utf8(bytes.to_vec()).map_err(|_| "导入包不是有效 UTF-8 文本".to_string())
}

/// 打开指定文件夹（如不存在则创建）
#[tauri::command]
pub async fn open_folder(path: String) -> Result<(), String> {
    let folder_path = std::path::Path::new(&path);

    // 如果目录不存在则创建
    if !folder_path.exists() {
        std::fs::create_dir_all(folder_path).map_err(|e| format!("创建文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    Ok(())
}

/// 删除损坏的文件（会先备份）
#[tauri::command]
pub async fn delete_corrupted_file(path: String) -> Result<(), String> {
    let file_path = std::path::Path::new(&path);

    if !file_path.exists() {
        // 文件不存在，直接返回成功
        return Ok(());
    }

    // 创建备份文件名
    let timestamp = chrono::Utc::now().timestamp();
    let backup_name = format!("{}.corrupted.{}", path, timestamp);

    // 备份文件
    std::fs::rename(&path, &backup_name).map_err(|e| format!("备份损坏文件失败: {}", e))?;

    modules::logger::log_info(&format!(
        "已备份并删除损坏文件: {} -> {}",
        path, backup_name
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_codex_quota_alert_thresholds, apply_general_config_updates,
        lock_general_config_transaction, UserConfig,
    };
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn general_config_transaction_lock_serializes_side_effecting_writes() {
        let first_guard = lock_general_config_transaction().expect("acquire first transaction");
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            attempt_tx.send(()).expect("signal lock attempt");
            let _guard = lock_general_config_transaction().expect("acquire second transaction");
            acquired_tx.send(()).expect("signal lock acquisition");
        });

        attempt_rx.recv().expect("wait for lock attempt");
        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first_guard);
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second transaction should continue after unlock");
        worker.join().expect("join transaction worker");
    }

    #[test]
    fn general_config_patch_only_changes_submitted_fields() {
        let mut config = UserConfig {
            theme: "dark".to_string(),
            auto_refresh_minutes: 10,
            ..UserConfig::default()
        };
        let updates = serde_json::json!({ "theme": "light" })
            .as_object()
            .expect("patch should be an object")
            .clone();

        apply_general_config_updates(&mut config, &updates).expect("patch should succeed");

        assert_eq!(config.theme, "light");
        assert_eq!(config.auto_refresh_minutes, 10);
    }

    #[test]
    fn general_config_patch_persists_session_sharing_switches() {
        let mut config = UserConfig::default();
        let updates = serde_json::json!({
            "codebuddy_share_sessions_on_switch": true,
            "workbuddy_share_sessions_on_switch": true,
        })
        .as_object()
        .expect("patch should be an object")
        .clone();

        apply_general_config_updates(&mut config, &updates)
            .expect("session sharing patch should succeed");

        assert!(config.codebuddy_share_sessions_on_switch);
        assert!(config.workbuddy_share_sessions_on_switch);
    }

    #[test]
    fn general_config_patch_rejects_non_general_fields() {
        let mut config = UserConfig::default();
        let updates = serde_json::json!({ "webdav_sync_password": "secret" })
            .as_object()
            .expect("patch should be an object")
            .clone();

        let error = apply_general_config_updates(&mut config, &updates)
            .expect_err("unsupported field should fail");

        assert!(error.contains("webdav_sync_password"));
    }

    #[test]
    fn unrelated_general_save_preserves_distinct_codex_quota_thresholds() {
        let mut config = UserConfig {
            codex_quota_alert_threshold: 20,
            codex_quota_alert_primary_threshold: 10,
            codex_quota_alert_secondary_threshold: 30,
            ..UserConfig::default()
        };

        apply_codex_quota_alert_thresholds(&mut config, Some(20), None, None);

        assert_eq!(config.codex_quota_alert_primary_threshold, 10);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 30);
    }

    #[test]
    fn changed_legacy_codex_quota_threshold_updates_both_windows() {
        let mut config = UserConfig {
            codex_quota_alert_threshold: 20,
            codex_quota_alert_primary_threshold: 10,
            codex_quota_alert_secondary_threshold: 30,
            ..UserConfig::default()
        };

        apply_codex_quota_alert_thresholds(&mut config, Some(40), None, None);

        assert_eq!(config.codex_quota_alert_threshold, 40);
        assert_eq!(config.codex_quota_alert_primary_threshold, 40);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 40);
    }

    #[test]
    fn explicit_codex_quota_window_thresholds_take_precedence() {
        let mut config = UserConfig::default();

        apply_codex_quota_alert_thresholds(&mut config, Some(40), Some(15), Some(25));

        assert_eq!(config.codex_quota_alert_threshold, 40);
        assert_eq!(config.codex_quota_alert_primary_threshold, 15);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 25);
    }
}
