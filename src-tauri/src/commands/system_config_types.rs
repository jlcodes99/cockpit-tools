// System commands：General configuration types, version discovery and patch normalization。
// 通过 include! 保持原 commands::system 作用域和 Tauri command 路径。
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use url::Url;

use crate::modules;
use crate::modules::config::{
    self, CloseWindowBehavior, MinimizeWindowBehavior, UserConfig, DEFAULT_REPORT_PORT,
    DEFAULT_WS_PORT,
};
use crate::modules::web_report;
use crate::modules::websocket;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    /// 是否全局允许 Codex app-server 第三方客户端（账户级开关仍可单独放行）
    pub codex_cli_only_allow_app_server_clients: bool,
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
    /// 切换 Grok 时是否自动重启 OpenCode
    pub grok_opencode_sync_on_switch: bool,
    /// 切换 Grok 时是否覆盖 OpenCode 登录信息
    pub grok_opencode_auth_overwrite_on_switch: bool,
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
    /// 是否在 macOS 菜单栏显示当前账号剩余额度
    pub menu_bar_quota_enabled: bool,
    /// 是否显示账号标识前 4 位
    pub menu_bar_show_account_prefix: bool,
    /// 菜单栏额度监控平台
    pub menu_bar_quota_platform: String,
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
    /// OAuth hosted login 使用的官方桌面版本（为空则跟随远端默认值）
    pub codex_oauth_app_version: String,
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
    /// 切换 CodeBuddy CN 账号时是否在本机账号间合并本地会话
    pub codebuddy_cn_share_sessions_on_switch: bool,
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
    pub trae_share_sessions_on_switch: bool,
    pub trae_solo_share_sessions_on_switch: bool,
    pub trae_cn_share_sessions_on_switch: bool,
    pub trae_solo_cn_share_sessions_on_switch: bool,
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
    /// 是否隐藏 Codex 总览中的中转站 / New API 类额度面板
    pub codex_hide_relay_quota: bool,
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
    /// Claude 额度 UI 是否显示「剩余%」（默认 false，保持历史「已用%」）
    pub claude_quota_display_remaining: bool,
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
const MIN_UI_SCALE: f64 = 0.3;
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
        #[cfg(any(target_os = "linux", test))]
        if antigravity_product_json_is_explicitly_unrelated(&value) {
            continue;
        }
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

#[cfg(any(target_os = "linux", test))]
fn antigravity_product_json_is_explicitly_unrelated(value: &serde_json::Value) -> bool {
    if json_string_field(value, &["ideVersion"]).is_some() {
        return false;
    }
    json_string_field(
        value,
        &["nameShort", "nameLong", "productName", "applicationName"],
    )
    .is_some_and(|name| !name.to_ascii_lowercase().contains("antigravity"))
}

fn antigravity_product_json_target(root: &Path) -> Option<&'static str> {
    for path in antigravity_product_json_candidates(root) {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let has_ide_version = json_string_field(&value, &["ideVersion"]).is_some();
        if !has_ide_version && json_string_field(&value, &["version"]).is_none() {
            continue;
        }
        let product_name = json_string_field(
            &value,
            &["nameShort", "nameLong", "productName", "applicationName"],
        )
        .map(|name| name.to_ascii_lowercase());
        if has_ide_version
            || product_name
                .as_deref()
                .is_some_and(|name| name.contains("antigravity") && name.contains("ide"))
        {
            return Some("antigravity_ide");
        }
        if product_name
            .as_deref()
            .is_some_and(|name| name.contains("antigravity") && !name.contains("ide"))
        {
            return Some("antigravity");
        }
        // A valid but unrelated primary product.json should not prevent the
        // fallback layout from identifying the configured Antigravity root.
        continue;
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

    #[cfg(unix)]
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(not(unix))]
    let normalized = path.to_path_buf();
    crate::modules::process::antigravity_install_root_from_path(&normalized)
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
    antigravity_metadata_root_matches_target_with_product_metadata(
        root,
        target,
        cfg!(target_os = "linux"),
    )
}

fn antigravity_metadata_root_matches_target_with_product_metadata(
    root: &Path,
    target: Option<&str>,
    prefer_product_metadata: bool,
) -> bool {
    let Some(target) = normalize_antigravity_metadata_target(target) else {
        return true;
    };
    if prefer_product_metadata {
        if let Some(metadata_target) = antigravity_product_json_target(root) {
            return metadata_target == target;
        }
    }
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

        if normalize_antigravity_metadata_target(target) != Some("antigravity") {
            if let Some(home) = dirs::home_dir() {
                let user_local_share = home.join(".local/share/antigravity-ide");
                if user_local_share.exists() {
                    push_unique_antigravity_candidate(&mut candidates, user_local_share);
                }
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
            | "codex_cli_only_allow_app_server_clients"
            | "codex_wsl_config_dir"
            | "zed_auto_refresh_minutes"
            | "ghcp_auto_refresh_minutes"
            | "windsurf_auto_refresh_minutes"
            | "kiro_auto_refresh_minutes"
            | "cursor_auto_refresh_minutes"
            | "grok_auto_refresh_minutes"
            | "grok_sync_official_auth_on_switch"
            | "grok_opencode_sync_on_switch"
            | "grok_opencode_auth_overwrite_on_switch"
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
            | "menu_bar_quota_enabled"
            | "menu_bar_show_account_prefix"
            | "menu_bar_quota_platform"
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
            | "codex_oauth_app_version"
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
            | "codebuddy_cn_share_sessions_on_switch"
            | "qoder_app_path"
            | "zcode_app_path"
            | "trae_app_path"
            | "trae_solo_app_path"
            | "trae_cn_app_path"
            | "trae_solo_cn_app_path"
            | "trae_share_sessions_on_switch"
            | "trae_solo_share_sessions_on_switch"
            | "trae_cn_share_sessions_on_switch"
            | "trae_solo_cn_share_sessions_on_switch"
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
            | "codex_hide_relay_quota"
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
            | "claude_quota_display_remaining"
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
    if updates.contains_key("menu_bar_quota_platform") {
        let platform = next.menu_bar_quota_platform.trim();
        next.menu_bar_quota_platform = modules::tray::PlatformId::from_str(platform)
            .map(|value| value.as_str().to_string())
            .unwrap_or_else(|| "codex".to_string());
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
    trim_string_field!("codex_oauth_app_version", codex_oauth_app_version);
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
    if updates.contains_key("grok_opencode_sync_on_switch")
        || updates.contains_key("grok_opencode_auth_overwrite_on_switch")
    {
        next.grok_opencode_sync_on_switch =
            next.grok_opencode_auth_overwrite_on_switch && next.grok_opencode_sync_on_switch;
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
