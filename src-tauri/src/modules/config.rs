//! 配置服务模块
//! 管理应用配置，包括 WebSocket 端口等

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

/// 默认 WebSocket 端口
pub const DEFAULT_WS_PORT: u16 = 19528;
/// 默认网页查询服务端口
pub const DEFAULT_REPORT_PORT: u16 = 18081;

/// 端口尝试范围（从配置端口开始，最多尝试 100 个）
pub const PORT_RANGE: u16 = 100;

/// 服务状态配置文件名（供外部客户端读取）
const SERVER_STATUS_FILE: &str = "server.json";

/// 用户配置文件名
const USER_CONFIG_FILE: &str = "config.json";

/// 数据目录名
const DATA_DIR: &str = ".antigravity_cockpit";

/// 服务状态（写入共享文件供其他客户端读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    /// WebSocket 服务端口（实际绑定的端口）
    pub ws_port: u16,
    /// 服务版本
    pub version: String,
    /// 进程 ID（用于检测服务是否存活）
    pub pid: u32,
    /// 启动时间戳
    pub started_at: i64,
}

/// 用户配置（持久化存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// WebSocket 服务是否启用
    #[serde(default = "default_ws_enabled")]
    pub ws_enabled: bool,
    /// WebSocket 首选端口（用户配置的，实际可能不同）
    #[serde(default = "default_ws_port")]
    pub ws_port: u16,
    /// 网页查询服务是否启用
    #[serde(default = "default_report_enabled")]
    pub report_enabled: bool,
    /// 网页查询服务首选端口
    #[serde(default = "default_report_port")]
    pub report_port: u16,
    /// 网页查询服务访问令牌
    #[serde(default = "default_report_token")]
    pub report_token: String,
    /// 全局代理开关（仅对受管启动链路生效）
    #[serde(default = "default_global_proxy_enabled")]
    pub global_proxy_enabled: bool,
    /// 全局代理地址（如 http://127.0.0.1:7890）
    #[serde(default = "default_global_proxy_url")]
    pub global_proxy_url: String,
    /// NO_PROXY 白名单（逗号分隔）
    #[serde(default = "default_global_proxy_no_proxy")]
    pub global_proxy_no_proxy: String,
    /// 界面语言
    #[serde(default = "default_language")]
    pub language: String,
    /// 默认终端
    #[serde(default = "default_default_terminal")]
    pub default_terminal: String,
    /// 应用主题
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 界面缩放比例
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f64,
    /// 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_auto_refresh")]
    pub auto_refresh_minutes: i32,
    /// Codex 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_codex_auto_refresh")]
    pub codex_auto_refresh_minutes: i32,
    /// Zed 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_zed_auto_refresh")]
    pub zed_auto_refresh_minutes: i32,
    /// GitHub Copilot 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_ghcp_auto_refresh")]
    pub ghcp_auto_refresh_minutes: i32,
    /// Windsurf 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_windsurf_auto_refresh")]
    pub windsurf_auto_refresh_minutes: i32,
    /// Kiro 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_kiro_auto_refresh")]
    pub kiro_auto_refresh_minutes: i32,
    /// Cursor 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_cursor_auto_refresh")]
    pub cursor_auto_refresh_minutes: i32,
    /// Gemini 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_gemini_auto_refresh")]
    pub gemini_auto_refresh_minutes: i32,
    /// Gemini 切号时是否同步覆盖 WSL 配置 (Windows Only)
    #[serde(default = "default_gemini_sync_wsl")]
    pub gemini_sync_wsl: bool,
    /// CodeBuddy 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_codebuddy_auto_refresh")]
    pub codebuddy_auto_refresh_minutes: i32,
    /// CodeBuddy CN 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_codebuddy_cn_auto_refresh")]
    pub codebuddy_cn_auto_refresh_minutes: i32,
    /// WorkBuddy 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_workbuddy_auto_refresh")]
    pub workbuddy_auto_refresh_minutes: i32,
    /// Qoder 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_qoder_auto_refresh")]
    pub qoder_auto_refresh_minutes: i32,
    /// Trae 自动刷新间隔（分钟），-1 表示禁用
    #[serde(default = "default_trae_auto_refresh")]
    pub trae_auto_refresh_minutes: i32,
    /// 窗口关闭行为
    #[serde(default = "default_close_behavior")]
    pub close_behavior: CloseWindowBehavior,
    /// 窗口最小化行为（macOS）
    #[serde(default = "default_minimize_behavior")]
    pub minimize_behavior: MinimizeWindowBehavior,
    /// 是否隐藏 Dock 图标（macOS）
    #[serde(default = "default_hide_dock_icon")]
    pub hide_dock_icon: bool,
    /// 是否在启动后自动显示悬浮卡片
    #[serde(default = "default_floating_card_show_on_startup")]
    pub floating_card_show_on_startup: bool,
    /// 悬浮卡片是否默认置顶
    #[serde(default = "default_floating_card_always_on_top")]
    pub floating_card_always_on_top: bool,
    /// 是否启用应用开机自启动
    #[serde(default = "default_app_auto_launch_enabled")]
    pub app_auto_launch_enabled: bool,
    /// 是否在应用启动后触发 Antigravity 唤醒
    #[serde(default = "default_antigravity_startup_wakeup_enabled")]
    pub antigravity_startup_wakeup_enabled: bool,
    /// Antigravity 启动后唤醒延时（秒），0 表示立即
    #[serde(default = "default_antigravity_startup_wakeup_delay_seconds")]
    pub antigravity_startup_wakeup_delay_seconds: i32,
    /// 是否在应用启动后触发 Codex 唤醒
    #[serde(default = "default_codex_startup_wakeup_enabled")]
    pub codex_startup_wakeup_enabled: bool,
    /// Codex 启动后唤醒延时（秒），0 表示立即
    #[serde(default = "default_codex_startup_wakeup_delay_seconds")]
    pub codex_startup_wakeup_delay_seconds: i32,
    /// 关闭悬浮卡片前是否显示确认弹框
    #[serde(default = "default_floating_card_confirm_on_close")]
    pub floating_card_confirm_on_close: bool,
    /// 是否启用定期自动备份
    #[serde(default = "default_auto_backup_enabled")]
    pub auto_backup_enabled: bool,
    /// 自动备份是否包含账号数据
    #[serde(default = "default_auto_backup_include_accounts")]
    pub auto_backup_include_accounts: bool,
    /// 自动备份是否包含配置数据
    #[serde(default = "default_auto_backup_include_config")]
    pub auto_backup_include_config: bool,
    /// 自动备份保留天数
    #[serde(default = "default_auto_backup_retention_days")]
    pub auto_backup_retention_days: i32,
    /// 自动备份保留天数是否已执行 v0.22.1 迁移（3 -> 15）
    #[serde(default = "default_auto_backup_retention_days_migrated")]
    pub auto_backup_retention_days_migrated: bool,
    /// 最近一次自动备份时间（ISO 8601）
    #[serde(default)]
    pub auto_backup_last_backup_at: Option<String>,
    /// 悬浮卡片保存的横向位置（物理像素）
    #[serde(default)]
    pub floating_card_position_x: Option<i32>,
    /// 悬浮卡片保存的纵向位置（物理像素）
    #[serde(default)]
    pub floating_card_position_y: Option<i32>,
    /// OpenCode 启动路径（为空则使用默认路径）
    #[serde(default = "default_opencode_app_path")]
    pub opencode_app_path: String,
    /// Antigravity 启动路径（为空则使用默认路径）
    #[serde(default = "default_antigravity_app_path")]
    pub antigravity_app_path: String,
    /// Codex 启动路径（为空则使用默认路径）
    #[serde(default = "default_codex_app_path")]
    pub codex_app_path: String,
    /// 切换 Codex 后需联动重启的指定应用路径
    #[serde(default = "default_codex_specified_app_path")]
    pub codex_specified_app_path: String,
    /// Zed 启动路径（为空则使用默认路径）
    #[serde(default = "default_zed_app_path")]
    pub zed_app_path: String,
    /// VS Code 启动路径（为空则使用默认路径）
    #[serde(default = "default_vscode_app_path")]
    pub vscode_app_path: String,
    /// Windsurf 启动路径（为空则使用默认路径）
    #[serde(default = "default_windsurf_app_path")]
    pub windsurf_app_path: String,
    /// Kiro 启动路径（为空则使用默认路径）
    #[serde(default = "default_kiro_app_path")]
    pub kiro_app_path: String,
    /// Cursor 启动路径（为空则使用默认路径）
    #[serde(default = "default_cursor_app_path")]
    pub cursor_app_path: String,
    /// CodeBuddy 启动路径（为空则使用默认路径）
    #[serde(default = "default_codebuddy_app_path")]
    pub codebuddy_app_path: String,
    /// CodeBuddy CN 启动路径（为空则使用默认路径）
    #[serde(default = "default_codebuddy_cn_app_path")]
    pub codebuddy_cn_app_path: String,
    /// Qoder 启动路径（为空则使用默认路径）
    #[serde(default = "default_qoder_app_path")]
    pub qoder_app_path: String,
    /// Trae 启动路径（为空则使用默认路径）
    #[serde(default = "default_trae_app_path")]
    pub trae_app_path: String,
    /// WorkBuddy 启动路径（为空则使用默认路径）
    #[serde(default = "default_workbuddy_app_path")]
    pub workbuddy_app_path: String,
    /// 切换 Codex 时是否自动重启 OpenCode
    #[serde(default = "default_opencode_sync_on_switch")]
    pub opencode_sync_on_switch: bool,
    /// 切换 Codex 时是否覆盖 OpenCode 登录信息
    #[serde(default = "default_opencode_auth_overwrite_on_switch")]
    pub opencode_auth_overwrite_on_switch: bool,
    /// 切换 GitHub Copilot 时是否自动重启 OpenCode
    #[serde(default = "default_ghcp_opencode_sync_on_switch")]
    pub ghcp_opencode_sync_on_switch: bool,
    /// 切换 GitHub Copilot 时是否覆盖 OpenCode 登录信息
    #[serde(default = "default_ghcp_opencode_auth_overwrite_on_switch")]
    pub ghcp_opencode_auth_overwrite_on_switch: bool,
    /// 切换 GitHub Copilot 时是否自动启动 GitHub Copilot
    #[serde(default = "default_ghcp_launch_on_switch")]
    pub ghcp_launch_on_switch: bool,
    /// 切换 Codex 时是否覆盖 OpenClaw 登录信息
    #[serde(default = "default_openclaw_auth_overwrite_on_switch")]
    pub openclaw_auth_overwrite_on_switch: bool,
    /// 切换 Codex 时是否自动启动/重启 Codex App
    #[serde(default = "default_codex_launch_on_switch")]
    pub codex_launch_on_switch: bool,
    /// 切换 Codex 时是否自动重启指定应用
    #[serde(default = "default_codex_restart_specified_app_on_switch")]
    pub codex_restart_specified_app_on_switch: bool,
    /// 是否在 Codex 总览中显示 API 服务入口
    #[serde(default = "default_codex_local_access_entry_visible")]
    pub codex_local_access_entry_visible: bool,
    /// Antigravity 切号是否启用“本地落盘 + 扩展无感”且不重启
    #[serde(default = "default_antigravity_dual_switch_no_restart_enabled")]
    pub antigravity_dual_switch_no_restart_enabled: bool,
    /// 是否启用自动切号
    #[serde(default = "default_auto_switch_enabled")]
    pub auto_switch_enabled: bool,
    /// 自动切号阈值（百分比），任意模型配额低于此值触发
    #[serde(default = "default_auto_switch_threshold")]
    pub auto_switch_threshold: i32,
    /// 是否启用 Credits 阈值自动切号
    #[serde(default = "default_auto_switch_credits_enabled")]
    pub auto_switch_credits_enabled: bool,
    /// Credits 自动切号阈值（剩余值）
    #[serde(default = "default_auto_switch_credits_threshold")]
    pub auto_switch_credits_threshold: i32,
    /// 自动切号触发模式：any_group | selected_groups
    #[serde(default = "default_auto_switch_scope_mode")]
    pub auto_switch_scope_mode: String,
    /// 自动切号指定模型分组（分组 ID）
    #[serde(default = "default_auto_switch_selected_group_ids")]
    pub auto_switch_selected_group_ids: Vec<String>,
    /// 自动切号账号范围模式：all_accounts | selected_accounts
    #[serde(default = "default_auto_switch_account_scope_mode")]
    pub auto_switch_account_scope_mode: String,
    /// 自动切号指定账号（账号 ID）
    #[serde(default = "default_auto_switch_selected_account_ids")]
    pub auto_switch_selected_account_ids: Vec<String>,
    /// 是否启用 Codex 自动切号
    #[serde(default = "default_codex_auto_switch_enabled")]
    pub codex_auto_switch_enabled: bool,
    /// Codex primary_window 自动切号阈值（百分比）
    #[serde(default = "default_codex_auto_switch_primary_threshold")]
    pub codex_auto_switch_primary_threshold: i32,
    /// Codex secondary_window 自动切号阈值（百分比）
    #[serde(default = "default_codex_auto_switch_secondary_threshold")]
    pub codex_auto_switch_secondary_threshold: i32,
    /// Codex 自动切号账号范围模式：all_accounts | selected_accounts
    #[serde(default = "default_codex_auto_switch_account_scope_mode")]
    pub codex_auto_switch_account_scope_mode: String,
    /// Codex 自动切号指定账号（账号 ID）
    #[serde(default = "default_codex_auto_switch_selected_account_ids")]
    pub codex_auto_switch_selected_account_ids: Vec<String>,
    /// 是否启用配额预警通知
    #[serde(default = "default_quota_alert_enabled")]
    pub quota_alert_enabled: bool,
    /// 配额预警阈值（百分比），任意模型配额低于此值触发
    #[serde(default = "default_quota_alert_threshold")]
    pub quota_alert_threshold: i32,
    /// 是否启用 Codex 配额预警通知
    #[serde(default = "default_codex_quota_alert_enabled")]
    pub codex_quota_alert_enabled: bool,
    /// Codex 配额预警阈值（百分比）
    #[serde(default = "default_codex_quota_alert_threshold")]
    pub codex_quota_alert_threshold: i32,
    /// 是否启用 Zed 配额预警通知
    #[serde(default = "default_zed_quota_alert_enabled")]
    pub zed_quota_alert_enabled: bool,
    /// Zed 配额预警阈值（百分比）
    #[serde(default = "default_zed_quota_alert_threshold")]
    pub zed_quota_alert_threshold: i32,
    /// Codex primary_window 配额预警阈值（百分比）
    #[serde(default = "default_codex_quota_alert_primary_threshold")]
    pub codex_quota_alert_primary_threshold: i32,
    /// Codex secondary_window 配额预警阈值（百分比）
    #[serde(default = "default_codex_quota_alert_secondary_threshold")]
    pub codex_quota_alert_secondary_threshold: i32,
    /// 是否启用 GitHub Copilot 配额预警通知
    #[serde(default = "default_ghcp_quota_alert_enabled")]
    pub ghcp_quota_alert_enabled: bool,
    /// GitHub Copilot 配额预警阈值（百分比）
    #[serde(default = "default_ghcp_quota_alert_threshold")]
    pub ghcp_quota_alert_threshold: i32,
    /// 是否启用 Windsurf 配额预警通知
    #[serde(default = "default_windsurf_quota_alert_enabled")]
    pub windsurf_quota_alert_enabled: bool,
    /// Windsurf 配额预警阈值（百分比）
    #[serde(default = "default_windsurf_quota_alert_threshold")]
    pub windsurf_quota_alert_threshold: i32,
    /// 是否启用 Kiro 配额预警通知
    #[serde(default = "default_kiro_quota_alert_enabled")]
    pub kiro_quota_alert_enabled: bool,
    /// Kiro 配额预警阈值（百分比）
    #[serde(default = "default_kiro_quota_alert_threshold")]
    pub kiro_quota_alert_threshold: i32,
    /// 是否启用 Cursor 配额预警通知
    #[serde(default = "default_cursor_quota_alert_enabled")]
    pub cursor_quota_alert_enabled: bool,
    /// Cursor 配额预警阈值（百分比）
    #[serde(default = "default_cursor_quota_alert_threshold")]
    pub cursor_quota_alert_threshold: i32,
    /// 是否启用 Gemini 配额预警通知
    #[serde(default = "default_gemini_quota_alert_enabled")]
    pub gemini_quota_alert_enabled: bool,
    /// Gemini 配额预警阈值（百分比）
    #[serde(default = "default_gemini_quota_alert_threshold")]
    pub gemini_quota_alert_threshold: i32,
    /// 是否启用 CodeBuddy 配额预警通知
    #[serde(default = "default_codebuddy_quota_alert_enabled")]
    pub codebuddy_quota_alert_enabled: bool,
    /// CodeBuddy 配额预警阈值（百分比）
    #[serde(default = "default_codebuddy_quota_alert_threshold")]
    pub codebuddy_quota_alert_threshold: i32,
    /// 是否启用 CodeBuddy CN 配额预警通知
    #[serde(default = "default_codebuddy_cn_quota_alert_enabled")]
    pub codebuddy_cn_quota_alert_enabled: bool,
    /// CodeBuddy CN 配额预警阈值（百分比）
    #[serde(default = "default_codebuddy_cn_quota_alert_threshold")]
    pub codebuddy_cn_quota_alert_threshold: i32,
    /// 是否启用 Qoder 配额预警通知
    #[serde(default = "default_qoder_quota_alert_enabled")]
    pub qoder_quota_alert_enabled: bool,
    /// Qoder 配额预警阈值（百分比）
    #[serde(default = "default_qoder_quota_alert_threshold")]
    pub qoder_quota_alert_threshold: i32,
    /// 是否启用 Trae 配额预警通知
    #[serde(default = "default_trae_quota_alert_enabled")]
    pub trae_quota_alert_enabled: bool,
    /// Trae 配额预警阈值（百分比）
    #[serde(default = "default_trae_quota_alert_threshold")]
    pub trae_quota_alert_threshold: i32,
    /// 是否启用 WorkBuddy 配额预警通知
    #[serde(default = "default_workbuddy_quota_alert_enabled")]
    pub workbuddy_quota_alert_enabled: bool,
    /// WorkBuddy 配额预警阈值（百分比）
    #[serde(default = "default_workbuddy_quota_alert_threshold")]
    pub workbuddy_quota_alert_threshold: i32,
}

/// 窗口关闭行为
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CloseWindowBehavior {
    /// 每次询问
    Ask,
    /// 最小化到托盘
    Minimize,
    /// 退出应用
    Quit,
}

impl Default for CloseWindowBehavior {
    fn default() -> Self {
        CloseWindowBehavior::Ask
    }
}

/// 窗口最小化行为（macOS）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MinimizeWindowBehavior {
    /// 程序坞 + 菜单栏（系统默认最小化）
    DockAndTray,
    /// 仅菜单栏（最小化时隐藏窗口）
    TrayOnly,
}

impl Default for MinimizeWindowBehavior {
    fn default() -> Self {
        MinimizeWindowBehavior::DockAndTray
    }
}

fn default_ws_enabled() -> bool {
    true
}
fn default_ws_port() -> u16 {
    DEFAULT_WS_PORT
}
fn default_report_enabled() -> bool {
    false
}
fn default_report_port() -> u16 {
    DEFAULT_REPORT_PORT
}
fn default_report_token() -> String {
    "change-this-token".to_string()
}
fn default_global_proxy_enabled() -> bool {
    false
}
fn default_global_proxy_url() -> String {
    String::new()
}
fn default_global_proxy_no_proxy() -> String {
    "127.0.0.1,localhost,::1".to_string()
}
fn default_language() -> String {
    "zh-cn".to_string()
}
fn default_default_terminal() -> String {
    "system".to_string()
}
fn default_theme() -> String {
    "system".to_string()
}
fn default_ui_scale() -> f64 {
    1.0
}
fn default_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_codex_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_zed_auto_refresh() -> i32 {
    10
}
fn default_ghcp_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_windsurf_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_kiro_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_cursor_auto_refresh() -> i32 {
    10
} // 默认 10 分钟
fn default_gemini_auto_refresh() -> i32 {
    10
}
fn default_gemini_sync_wsl() -> bool {
    true
}
fn default_codebuddy_auto_refresh() -> i32 {
    10
}
fn default_codebuddy_cn_auto_refresh() -> i32 {
    10
}
fn default_workbuddy_auto_refresh() -> i32 {
    10
}
fn default_qoder_auto_refresh() -> i32 {
    10
}
fn default_trae_auto_refresh() -> i32 {
    10
}
fn default_close_behavior() -> CloseWindowBehavior {
    CloseWindowBehavior::Ask
}
fn default_minimize_behavior() -> MinimizeWindowBehavior {
    MinimizeWindowBehavior::DockAndTray
}
fn default_hide_dock_icon() -> bool {
    false
}
fn default_floating_card_show_on_startup() -> bool {
    false
}
fn default_floating_card_always_on_top() -> bool {
    false
}
fn default_app_auto_launch_enabled() -> bool {
    false
}
fn default_antigravity_startup_wakeup_enabled() -> bool {
    false
}
fn default_antigravity_startup_wakeup_delay_seconds() -> i32 {
    0
}
fn default_codex_startup_wakeup_enabled() -> bool {
    false
}
fn default_codex_startup_wakeup_delay_seconds() -> i32 {
    0
}
fn default_floating_card_confirm_on_close() -> bool {
    true
}
pub fn default_auto_backup_enabled() -> bool {
    true
}
pub fn default_auto_backup_include_accounts() -> bool {
    true
}
pub fn default_auto_backup_include_config() -> bool {
    true
}
pub fn default_auto_backup_retention_days() -> i32 {
    15
}
fn default_auto_backup_retention_days_migrated() -> bool {
    true
}
pub fn sanitize_auto_backup_retention_days(raw: i32) -> i32 {
    raw.clamp(1, 365)
}
pub fn normalize_auto_backup_selection(
    include_accounts: bool,
    include_config: bool,
) -> (bool, bool) {
    if !include_accounts && !include_config {
        (
            default_auto_backup_include_accounts(),
            default_auto_backup_include_config(),
        )
    } else {
        (include_accounts, include_config)
    }
}
fn default_opencode_app_path() -> String {
    String::new()
}
fn default_antigravity_app_path() -> String {
    String::new()
}
fn default_codex_app_path() -> String {
    String::new()
}
fn default_codex_specified_app_path() -> String {
    String::new()
}
fn default_zed_app_path() -> String {
    String::new()
}
fn default_vscode_app_path() -> String {
    String::new()
}
fn default_windsurf_app_path() -> String {
    String::new()
}
fn default_kiro_app_path() -> String {
    String::new()
}
fn default_cursor_app_path() -> String {
    String::new()
}
fn default_codebuddy_app_path() -> String {
    String::new()
}
fn default_codebuddy_cn_app_path() -> String {
    String::new()
}
fn default_qoder_app_path() -> String {
    String::new()
}
fn default_trae_app_path() -> String {
    String::new()
}
fn default_workbuddy_app_path() -> String {
    String::new()
}
fn default_opencode_sync_on_switch() -> bool {
    false
}
fn default_opencode_auth_overwrite_on_switch() -> bool {
    false
}
fn default_ghcp_opencode_sync_on_switch() -> bool {
    false
}
fn default_ghcp_opencode_auth_overwrite_on_switch() -> bool {
    false
}
fn default_ghcp_launch_on_switch() -> bool {
    true
}
fn default_openclaw_auth_overwrite_on_switch() -> bool {
    false
}
fn default_codex_launch_on_switch() -> bool {
    true
}
fn default_codex_restart_specified_app_on_switch() -> bool {
    false
}
fn default_codex_local_access_entry_visible() -> bool {
    true
}
fn default_antigravity_dual_switch_no_restart_enabled() -> bool {
    false
}
fn default_auto_switch_enabled() -> bool {
    false
}
fn default_auto_switch_threshold() -> i32 {
    5
}
fn default_auto_switch_credits_enabled() -> bool {
    false
}
fn default_auto_switch_credits_threshold() -> i32 {
    5
}
fn default_auto_switch_scope_mode() -> String {
    "any_group".to_string()
}
fn default_auto_switch_selected_group_ids() -> Vec<String> {
    Vec::new()
}
fn default_auto_switch_account_scope_mode() -> String {
    "all_accounts".to_string()
}
fn default_auto_switch_selected_account_ids() -> Vec<String> {
    Vec::new()
}
fn default_codex_auto_switch_enabled() -> bool {
    false
}
fn default_codex_auto_switch_primary_threshold() -> i32 {
    20
}
fn default_codex_auto_switch_secondary_threshold() -> i32 {
    20
}
fn default_codex_auto_switch_account_scope_mode() -> String {
    "all_accounts".to_string()
}
fn default_codex_auto_switch_selected_account_ids() -> Vec<String> {
    Vec::new()
}
fn default_quota_alert_enabled() -> bool {
    false
}
fn default_quota_alert_threshold() -> i32 {
    20
}
fn default_codex_quota_alert_enabled() -> bool {
    false
}
fn default_codex_quota_alert_threshold() -> i32 {
    20
}
fn default_zed_quota_alert_enabled() -> bool {
    false
}
fn default_zed_quota_alert_threshold() -> i32 {
    20
}
fn default_codex_quota_alert_primary_threshold() -> i32 {
    20
}
fn default_codex_quota_alert_secondary_threshold() -> i32 {
    20
}
fn default_ghcp_quota_alert_enabled() -> bool {
    false
}
fn default_ghcp_quota_alert_threshold() -> i32 {
    20
}
fn default_windsurf_quota_alert_enabled() -> bool {
    false
}
fn default_windsurf_quota_alert_threshold() -> i32 {
    20
}
fn default_kiro_quota_alert_enabled() -> bool {
    false
}
fn default_kiro_quota_alert_threshold() -> i32 {
    20
}
fn default_cursor_quota_alert_enabled() -> bool {
    false
}
fn default_cursor_quota_alert_threshold() -> i32 {
    20
}
fn default_gemini_quota_alert_enabled() -> bool {
    false
}
fn default_gemini_quota_alert_threshold() -> i32 {
    20
}
fn default_codebuddy_quota_alert_enabled() -> bool {
    false
}
fn default_codebuddy_quota_alert_threshold() -> i32 {
    20
}
fn default_codebuddy_cn_quota_alert_enabled() -> bool {
    false
}
fn default_codebuddy_cn_quota_alert_threshold() -> i32 {
    20
}
fn default_qoder_quota_alert_enabled() -> bool {
    false
}
fn default_qoder_quota_alert_threshold() -> i32 {
    20
}
fn default_trae_quota_alert_enabled() -> bool {
    false
}
fn default_trae_quota_alert_threshold() -> i32 {
    20
}
fn default_workbuddy_quota_alert_enabled() -> bool {
    false
}
fn default_workbuddy_quota_alert_threshold() -> i32 {
    20
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            ws_enabled: true,
            ws_port: DEFAULT_WS_PORT,
            report_enabled: default_report_enabled(),
            report_port: default_report_port(),
            report_token: default_report_token(),
            global_proxy_enabled: default_global_proxy_enabled(),
            global_proxy_url: default_global_proxy_url(),
            global_proxy_no_proxy: default_global_proxy_no_proxy(),
            language: default_language(),
            default_terminal: default_default_terminal(),
            theme: default_theme(),
            ui_scale: default_ui_scale(),
            auto_refresh_minutes: default_auto_refresh(),
            codex_auto_refresh_minutes: default_codex_auto_refresh(),
            zed_auto_refresh_minutes: default_zed_auto_refresh(),
            ghcp_auto_refresh_minutes: default_ghcp_auto_refresh(),
            windsurf_auto_refresh_minutes: default_windsurf_auto_refresh(),
            kiro_auto_refresh_minutes: default_kiro_auto_refresh(),
            cursor_auto_refresh_minutes: default_cursor_auto_refresh(),
            gemini_auto_refresh_minutes: default_gemini_auto_refresh(),
            gemini_sync_wsl: default_gemini_sync_wsl(),
            codebuddy_auto_refresh_minutes: default_codebuddy_auto_refresh(),
            codebuddy_cn_auto_refresh_minutes: default_codebuddy_cn_auto_refresh(),
            workbuddy_auto_refresh_minutes: default_workbuddy_auto_refresh(),
            qoder_auto_refresh_minutes: default_qoder_auto_refresh(),
            trae_auto_refresh_minutes: default_trae_auto_refresh(),
            close_behavior: default_close_behavior(),
            minimize_behavior: default_minimize_behavior(),
            hide_dock_icon: default_hide_dock_icon(),
            floating_card_show_on_startup: default_floating_card_show_on_startup(),
            floating_card_always_on_top: default_floating_card_always_on_top(),
            app_auto_launch_enabled: default_app_auto_launch_enabled(),
            antigravity_startup_wakeup_enabled: default_antigravity_startup_wakeup_enabled(),
            antigravity_startup_wakeup_delay_seconds:
                default_antigravity_startup_wakeup_delay_seconds(),
            codex_startup_wakeup_enabled: default_codex_startup_wakeup_enabled(),
            codex_startup_wakeup_delay_seconds: default_codex_startup_wakeup_delay_seconds(),
            floating_card_confirm_on_close: default_floating_card_confirm_on_close(),
            auto_backup_enabled: default_auto_backup_enabled(),
            auto_backup_include_accounts: default_auto_backup_include_accounts(),
            auto_backup_include_config: default_auto_backup_include_config(),
            auto_backup_retention_days: default_auto_backup_retention_days(),
            auto_backup_retention_days_migrated: default_auto_backup_retention_days_migrated(),
            auto_backup_last_backup_at: None,
            floating_card_position_x: None,
            floating_card_position_y: None,
            opencode_app_path: default_opencode_app_path(),
            antigravity_app_path: default_antigravity_app_path(),
            codex_app_path: default_codex_app_path(),
            codex_specified_app_path: default_codex_specified_app_path(),
            zed_app_path: default_zed_app_path(),
            vscode_app_path: default_vscode_app_path(),
            windsurf_app_path: default_windsurf_app_path(),
            kiro_app_path: default_kiro_app_path(),
            cursor_app_path: default_cursor_app_path(),
            codebuddy_app_path: default_codebuddy_app_path(),
            codebuddy_cn_app_path: default_codebuddy_cn_app_path(),
            qoder_app_path: default_qoder_app_path(),
            trae_app_path: default_trae_app_path(),
            workbuddy_app_path: default_workbuddy_app_path(),
            opencode_sync_on_switch: default_opencode_sync_on_switch(),
            opencode_auth_overwrite_on_switch: default_opencode_auth_overwrite_on_switch(),
            ghcp_opencode_sync_on_switch: default_ghcp_opencode_sync_on_switch(),
            ghcp_opencode_auth_overwrite_on_switch: default_ghcp_opencode_auth_overwrite_on_switch(
            ),
            ghcp_launch_on_switch: default_ghcp_launch_on_switch(),
            openclaw_auth_overwrite_on_switch: default_openclaw_auth_overwrite_on_switch(),
            codex_launch_on_switch: default_codex_launch_on_switch(),
            codex_restart_specified_app_on_switch: default_codex_restart_specified_app_on_switch(),
            codex_local_access_entry_visible: default_codex_local_access_entry_visible(),
            antigravity_dual_switch_no_restart_enabled:
                default_antigravity_dual_switch_no_restart_enabled(),
            auto_switch_enabled: default_auto_switch_enabled(),
            auto_switch_threshold: default_auto_switch_threshold(),
            auto_switch_credits_enabled: default_auto_switch_credits_enabled(),
            auto_switch_credits_threshold: default_auto_switch_credits_threshold(),
            auto_switch_scope_mode: default_auto_switch_scope_mode(),
            auto_switch_selected_group_ids: default_auto_switch_selected_group_ids(),
            auto_switch_account_scope_mode: default_auto_switch_account_scope_mode(),
            auto_switch_selected_account_ids: default_auto_switch_selected_account_ids(),
            codex_auto_switch_enabled: default_codex_auto_switch_enabled(),
            codex_auto_switch_primary_threshold: default_codex_auto_switch_primary_threshold(),
            codex_auto_switch_secondary_threshold: default_codex_auto_switch_secondary_threshold(),
            codex_auto_switch_account_scope_mode: default_codex_auto_switch_account_scope_mode(),
            codex_auto_switch_selected_account_ids: default_codex_auto_switch_selected_account_ids(
            ),
            quota_alert_enabled: default_quota_alert_enabled(),
            quota_alert_threshold: default_quota_alert_threshold(),
            codex_quota_alert_enabled: default_codex_quota_alert_enabled(),
            codex_quota_alert_threshold: default_codex_quota_alert_threshold(),
            zed_quota_alert_enabled: default_zed_quota_alert_enabled(),
            zed_quota_alert_threshold: default_zed_quota_alert_threshold(),
            codex_quota_alert_primary_threshold: default_codex_quota_alert_primary_threshold(),
            codex_quota_alert_secondary_threshold: default_codex_quota_alert_secondary_threshold(),
            ghcp_quota_alert_enabled: default_ghcp_quota_alert_enabled(),
            ghcp_quota_alert_threshold: default_ghcp_quota_alert_threshold(),
            windsurf_quota_alert_enabled: default_windsurf_quota_alert_enabled(),
            windsurf_quota_alert_threshold: default_windsurf_quota_alert_threshold(),
            kiro_quota_alert_enabled: default_kiro_quota_alert_enabled(),
            kiro_quota_alert_threshold: default_kiro_quota_alert_threshold(),
            cursor_quota_alert_enabled: default_cursor_quota_alert_enabled(),
            cursor_quota_alert_threshold: default_cursor_quota_alert_threshold(),
            gemini_quota_alert_enabled: default_gemini_quota_alert_enabled(),
            gemini_quota_alert_threshold: default_gemini_quota_alert_threshold(),
            codebuddy_quota_alert_enabled: default_codebuddy_quota_alert_enabled(),
            codebuddy_quota_alert_threshold: default_codebuddy_quota_alert_threshold(),
            codebuddy_cn_quota_alert_enabled: default_codebuddy_cn_quota_alert_enabled(),
            codebuddy_cn_quota_alert_threshold: default_codebuddy_cn_quota_alert_threshold(),
            qoder_quota_alert_enabled: default_qoder_quota_alert_enabled(),
            qoder_quota_alert_threshold: default_qoder_quota_alert_threshold(),
            trae_quota_alert_enabled: default_trae_quota_alert_enabled(),
            trae_quota_alert_threshold: default_trae_quota_alert_threshold(),
            workbuddy_quota_alert_enabled: default_workbuddy_quota_alert_enabled(),
            workbuddy_quota_alert_threshold: default_workbuddy_quota_alert_threshold(),
        }
    }
}

/// 运行时状态
struct RuntimeState {
    /// 当前实际使用的端口
    actual_port: Option<u16>,
    /// 用户配置
    user_config: UserConfig,
}

/// 全局运行时状态
static RUNTIME_STATE: OnceLock<RwLock<RuntimeState>> = OnceLock::new();
static INHERITED_PROXY_ENV: OnceLock<Vec<(&'static str, Option<String>)>> = OnceLock::new();

fn get_runtime_state() -> &'static RwLock<RuntimeState> {
    RUNTIME_STATE.get_or_init(|| {
        let initial_config = load_user_config().unwrap_or_default();
        // 让应用内 reqwest 客户端与用户全局代理设置保持一致。
        sync_global_proxy_env(&initial_config);
        RwLock::new(RuntimeState {
            actual_port: None,
            user_config: initial_config,
        })
    })
}

const MANAGED_PROXY_SET_KEYS: [&str; 6] = [
    "http_proxy",
    "https_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
];

const MANAGED_PROXY_NO_PROXY_KEYS: [&str; 2] = ["no_proxy", "NO_PROXY"];

fn inherited_proxy_env() -> &'static Vec<(&'static str, Option<String>)> {
    INHERITED_PROXY_ENV.get_or_init(|| {
        MANAGED_PROXY_SET_KEYS
            .iter()
            .chain(MANAGED_PROXY_NO_PROXY_KEYS.iter())
            .map(|key| (*key, std::env::var(key).ok()))
            .collect()
    })
}

fn managed_proxy_env_pairs(config: &UserConfig) -> Vec<(&'static str, String)> {
    if !config.global_proxy_enabled {
        return Vec::new();
    }

    let proxy_url = config.global_proxy_url.trim();
    if proxy_url.is_empty() {
        return Vec::new();
    }

    let mut pairs = Vec::with_capacity(8);
    for key in MANAGED_PROXY_SET_KEYS {
        pairs.push((key, proxy_url.to_string()));
    }

    let no_proxy = config.global_proxy_no_proxy.trim();
    if !no_proxy.is_empty() {
        for key in MANAGED_PROXY_NO_PROXY_KEYS {
            pairs.push((key, no_proxy.to_string()));
        }
    }

    pairs
}

fn clear_managed_proxy_env() {
    for key in MANAGED_PROXY_SET_KEYS {
        std::env::remove_var(key);
    }
    for key in MANAGED_PROXY_NO_PROXY_KEYS {
        std::env::remove_var(key);
    }
}

fn restore_inherited_proxy_env() {
    clear_managed_proxy_env();

    let mut restored_keys = Vec::new();
    for (key, value) in inherited_proxy_env() {
        if let Some(value) = value {
            std::env::set_var(key, value);
            restored_keys.push(*key);
        }
    }

    if restored_keys.is_empty() {
        crate::modules::logger::log_info(
            "[Proxy] 应用内未启用全局代理，已恢复启动时继承环境（未携带代理变量）",
        );
        return;
    }

    crate::modules::logger::log_info(&format!(
        "[Proxy] 应用内未启用全局代理，已恢复启动时继承环境 keys={}",
        restored_keys.join(",")
    ));
}

pub fn sync_global_proxy_env(config: &UserConfig) {
    let pairs = managed_proxy_env_pairs(config);
    if pairs.is_empty() {
        restore_inherited_proxy_env();
        return;
    }

    clear_managed_proxy_env();

    let mut applied_keys = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        std::env::set_var(key, value);
        applied_keys.push(key);
    }

    crate::modules::logger::log_info(&format!(
        "[Proxy] 应用内全局代理环境已同步 keys={}",
        applied_keys.join(",")
    ));
}

/// 获取数据目录路径
pub fn get_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
    Ok(home.join(DATA_DIR))
}

/// 获取共享目录路径（供其他模块使用）
/// 与 get_data_dir 相同，但不返回 Result
pub fn get_shared_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(DATA_DIR))
        .unwrap_or_else(|| PathBuf::from(DATA_DIR))
}

/// 获取服务状态文件路径
pub fn get_server_status_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join(SERVER_STATUS_FILE))
}

/// 获取用户配置文件路径
pub fn get_user_config_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join(USER_CONFIG_FILE))
}

/// 加载用户配置
pub fn load_user_config() -> Result<UserConfig, String> {
    let config_path = get_user_config_path()?;

    if !config_path.exists() {
        return Ok(UserConfig::default());
    }

    let content =
        fs::read_to_string(&config_path).map_err(|e| format!("读取配置文件失败: {}", e))?;

    let mut value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;

    // 兼容旧配置：平台独立预警字段不存在时，继承历史全局预警配置
    if let Some(obj) = value.as_object_mut() {
        if !obj.contains_key("kiro_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("windsurf_auto_refresh_minutes")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_kiro_auto_refresh);
            obj.insert(
                "kiro_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("cursor_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("kiro_auto_refresh_minutes")
                .or_else(|| obj.get("windsurf_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_cursor_auto_refresh);
            obj.insert(
                "cursor_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("gemini_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("cursor_auto_refresh_minutes")
                .or_else(|| obj.get("kiro_auto_refresh_minutes"))
                .or_else(|| obj.get("windsurf_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_gemini_auto_refresh);
            obj.insert(
                "gemini_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("gemini_sync_wsl") {
            obj.insert(
                "gemini_sync_wsl".to_string(),
                json!(default_gemini_sync_wsl()),
            );
        }

        if !obj.contains_key("qoder_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("gemini_auto_refresh_minutes")
                .or_else(|| obj.get("cursor_auto_refresh_minutes"))
                .or_else(|| obj.get("kiro_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_qoder_auto_refresh);
            obj.insert(
                "qoder_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("codebuddy_cn_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("codebuddy_auto_refresh_minutes")
                .or_else(|| obj.get("gemini_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_codebuddy_cn_auto_refresh);
            obj.insert(
                "codebuddy_cn_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("workbuddy_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("codebuddy_cn_auto_refresh_minutes")
                .or_else(|| obj.get("codebuddy_auto_refresh_minutes"))
                .or_else(|| obj.get("gemini_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_workbuddy_auto_refresh);
            obj.insert(
                "workbuddy_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("trae_auto_refresh_minutes") {
            let inherited_refresh = obj
                .get("qoder_auto_refresh_minutes")
                .or_else(|| obj.get("gemini_auto_refresh_minutes"))
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or_else(default_trae_auto_refresh);
            obj.insert(
                "trae_auto_refresh_minutes".to_string(),
                json!(inherited_refresh),
            );
        }

        if !obj.contains_key("hide_dock_icon") {
            let inherited_hide_dock_icon = obj
                .get("minimize_behavior")
                .and_then(|v| v.as_str())
                .map(|v| v == "tray_only")
                .unwrap_or_else(default_hide_dock_icon);
            obj.insert(
                "hide_dock_icon".to_string(),
                json!(inherited_hide_dock_icon),
            );
        }

        if !obj.contains_key("floating_card_show_on_startup") {
            obj.insert(
                "floating_card_show_on_startup".to_string(),
                json!(default_floating_card_show_on_startup()),
            );
        }

        if !obj.contains_key("floating_card_always_on_top") {
            obj.insert(
                "floating_card_always_on_top".to_string(),
                json!(default_floating_card_always_on_top()),
            );
        }

        if !obj.contains_key("app_auto_launch_enabled") {
            obj.insert(
                "app_auto_launch_enabled".to_string(),
                json!(default_app_auto_launch_enabled()),
            );
        }

        if !obj.contains_key("antigravity_startup_wakeup_enabled") {
            obj.insert(
                "antigravity_startup_wakeup_enabled".to_string(),
                json!(default_antigravity_startup_wakeup_enabled()),
            );
        }

        if !obj.contains_key("antigravity_startup_wakeup_delay_seconds") {
            obj.insert(
                "antigravity_startup_wakeup_delay_seconds".to_string(),
                json!(default_antigravity_startup_wakeup_delay_seconds()),
            );
        }

        if !obj.contains_key("codex_startup_wakeup_enabled") {
            obj.insert(
                "codex_startup_wakeup_enabled".to_string(),
                json!(default_codex_startup_wakeup_enabled()),
            );
        }

        if !obj.contains_key("codex_startup_wakeup_delay_seconds") {
            obj.insert(
                "codex_startup_wakeup_delay_seconds".to_string(),
                json!(default_codex_startup_wakeup_delay_seconds()),
            );
        }

        if !obj.contains_key("codex_local_access_entry_visible") {
            obj.insert(
                "codex_local_access_entry_visible".to_string(),
                json!(default_codex_local_access_entry_visible()),
            );
        }

        if !obj.contains_key("floating_card_confirm_on_close") {
            obj.insert(
                "floating_card_confirm_on_close".to_string(),
                json!(default_floating_card_confirm_on_close()),
            );
        }
        if !obj.contains_key("auto_backup_enabled") {
            obj.insert(
                "auto_backup_enabled".to_string(),
                json!(default_auto_backup_enabled()),
            );
        }
        if !obj.contains_key("auto_backup_include_accounts") {
            obj.insert(
                "auto_backup_include_accounts".to_string(),
                json!(default_auto_backup_include_accounts()),
            );
        }
        if !obj.contains_key("auto_backup_include_config") {
            obj.insert(
                "auto_backup_include_config".to_string(),
                json!(default_auto_backup_include_config()),
            );
        }
        if !obj.contains_key("auto_backup_retention_days") {
            obj.insert(
                "auto_backup_retention_days".to_string(),
                json!(default_auto_backup_retention_days()),
            );
        }
        if !obj.contains_key("auto_backup_retention_days_migrated") {
            // 老配置没有该标记时，默认视为“尚未迁移”，以便执行一次 3->15 升级。
            obj.insert(
                "auto_backup_retention_days_migrated".to_string(),
                json!(false),
            );
        }
        if !obj.contains_key("auto_backup_last_backup_at") {
            obj.insert(
                "auto_backup_last_backup_at".to_string(),
                serde_json::Value::Null,
            );
        }

        if !obj.contains_key("report_enabled") {
            obj.insert(
                "report_enabled".to_string(),
                json!(default_report_enabled()),
            );
        }
        if !obj.contains_key("report_port") {
            obj.insert("report_port".to_string(), json!(default_report_port()));
        }
        if !obj.contains_key("report_token") {
            obj.insert("report_token".to_string(), json!(default_report_token()));
        }
        if !obj.contains_key("default_terminal") {
            obj.insert(
                "default_terminal".to_string(),
                json!(default_default_terminal()),
            );
        }
        if !obj.contains_key("global_proxy_enabled") {
            obj.insert(
                "global_proxy_enabled".to_string(),
                json!(default_global_proxy_enabled()),
            );
        }
        if !obj.contains_key("global_proxy_url") {
            obj.insert(
                "global_proxy_url".to_string(),
                json!(default_global_proxy_url()),
            );
        }
        if !obj.contains_key("global_proxy_no_proxy") {
            obj.insert(
                "global_proxy_no_proxy".to_string(),
                json!(default_global_proxy_no_proxy()),
            );
        }

        let legacy_enabled = obj
            .get("quota_alert_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(default_quota_alert_enabled);
        let legacy_threshold = obj
            .get("quota_alert_threshold")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or_else(default_quota_alert_threshold);
        let legacy_auto_switch_enabled = obj
            .get("codex_auto_switch_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(default_codex_auto_switch_enabled);
        let legacy_auto_switch_threshold = obj
            .get("codex_auto_switch_primary_threshold")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or_else(default_codex_auto_switch_primary_threshold);

        if !obj.contains_key("codex_quota_alert_enabled") {
            obj.insert(
                "codex_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("codex_quota_alert_threshold") {
            obj.insert(
                "codex_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("zed_quota_alert_enabled") {
            obj.insert("zed_quota_alert_enabled".to_string(), json!(legacy_enabled));
        }
        if !obj.contains_key("zed_quota_alert_threshold") {
            obj.insert(
                "zed_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("codex_auto_switch_enabled") {
            obj.insert(
                "codex_auto_switch_enabled".to_string(),
                json!(legacy_auto_switch_enabled),
            );
        }
        if !obj.contains_key("codex_auto_switch_primary_threshold") {
            obj.insert(
                "codex_auto_switch_primary_threshold".to_string(),
                json!(legacy_auto_switch_threshold),
            );
        }
        if !obj.contains_key("codex_auto_switch_secondary_threshold") {
            obj.insert(
                "codex_auto_switch_secondary_threshold".to_string(),
                json!(legacy_auto_switch_threshold),
            );
        }
        if !obj.contains_key("auto_switch_scope_mode") {
            obj.insert(
                "auto_switch_scope_mode".to_string(),
                json!(default_auto_switch_scope_mode()),
            );
        }
        if !obj.contains_key("auto_switch_credits_enabled") {
            obj.insert(
                "auto_switch_credits_enabled".to_string(),
                json!(default_auto_switch_credits_enabled()),
            );
        }
        if !obj.contains_key("auto_switch_credits_threshold") {
            obj.insert(
                "auto_switch_credits_threshold".to_string(),
                json!(default_auto_switch_credits_threshold()),
            );
        }
        if !obj.contains_key("auto_switch_selected_group_ids") {
            obj.insert(
                "auto_switch_selected_group_ids".to_string(),
                json!(default_auto_switch_selected_group_ids()),
            );
        }
        if !obj.contains_key("auto_switch_account_scope_mode") {
            obj.insert(
                "auto_switch_account_scope_mode".to_string(),
                json!(default_auto_switch_account_scope_mode()),
            );
        }
        if !obj.contains_key("auto_switch_selected_account_ids") {
            obj.insert(
                "auto_switch_selected_account_ids".to_string(),
                json!(default_auto_switch_selected_account_ids()),
            );
        }
        if !obj.contains_key("codex_auto_switch_account_scope_mode") {
            obj.insert(
                "codex_auto_switch_account_scope_mode".to_string(),
                json!(default_codex_auto_switch_account_scope_mode()),
            );
        }
        if !obj.contains_key("codex_auto_switch_selected_account_ids") {
            obj.insert(
                "codex_auto_switch_selected_account_ids".to_string(),
                json!(default_codex_auto_switch_selected_account_ids()),
            );
        }
        let codex_legacy_threshold = obj
            .get("codex_quota_alert_threshold")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or(legacy_threshold);
        if !obj.contains_key("codex_quota_alert_primary_threshold") {
            obj.insert(
                "codex_quota_alert_primary_threshold".to_string(),
                json!(codex_legacy_threshold),
            );
        }
        if !obj.contains_key("codex_quota_alert_secondary_threshold") {
            obj.insert(
                "codex_quota_alert_secondary_threshold".to_string(),
                json!(codex_legacy_threshold),
            );
        }
        if !obj.contains_key("ghcp_quota_alert_enabled") {
            obj.insert(
                "ghcp_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("ghcp_quota_alert_threshold") {
            obj.insert(
                "ghcp_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("windsurf_quota_alert_enabled") {
            obj.insert(
                "windsurf_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("windsurf_quota_alert_threshold") {
            obj.insert(
                "windsurf_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("kiro_quota_alert_enabled") {
            obj.insert(
                "kiro_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("kiro_quota_alert_threshold") {
            obj.insert(
                "kiro_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("cursor_quota_alert_enabled") {
            obj.insert(
                "cursor_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("cursor_quota_alert_threshold") {
            obj.insert(
                "cursor_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("gemini_quota_alert_enabled") {
            obj.insert(
                "gemini_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("gemini_quota_alert_threshold") {
            obj.insert(
                "gemini_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("codebuddy_quota_alert_enabled") {
            obj.insert(
                "codebuddy_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("codebuddy_quota_alert_threshold") {
            obj.insert(
                "codebuddy_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("codebuddy_cn_quota_alert_enabled") {
            obj.insert(
                "codebuddy_cn_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("codebuddy_cn_quota_alert_threshold") {
            obj.insert(
                "codebuddy_cn_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("qoder_quota_alert_enabled") {
            obj.insert(
                "qoder_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("qoder_quota_alert_threshold") {
            obj.insert(
                "qoder_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("trae_quota_alert_enabled") {
            obj.insert(
                "trae_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("trae_quota_alert_threshold") {
            obj.insert(
                "trae_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
        if !obj.contains_key("workbuddy_quota_alert_enabled") {
            obj.insert(
                "workbuddy_quota_alert_enabled".to_string(),
                json!(legacy_enabled),
            );
        }
        if !obj.contains_key("workbuddy_quota_alert_threshold") {
            obj.insert(
                "workbuddy_quota_alert_threshold".to_string(),
                json!(legacy_threshold),
            );
        }
    }

    let mut config: UserConfig =
        serde_json::from_value(value).map_err(|e| format!("解析配置文件失败: {}", e))?;
    let (include_accounts, include_config) = normalize_auto_backup_selection(
        config.auto_backup_include_accounts,
        config.auto_backup_include_config,
    );
    config.auto_backup_include_accounts = include_accounts;
    config.auto_backup_include_config = include_config;
    if !config.auto_backup_retention_days_migrated {
        if config.auto_backup_retention_days == 3 {
            // 兼容迁移：历史默认值为 3 天，统一升级为 15 天。
            config.auto_backup_retention_days = 15;
        }
        config.auto_backup_retention_days_migrated = true;
    }
    config.auto_backup_retention_days =
        sanitize_auto_backup_retention_days(config.auto_backup_retention_days);
    config.auto_backup_last_backup_at = config.auto_backup_last_backup_at.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    Ok(config)
}

/// 保存用户配置
pub fn save_user_config(config: &UserConfig) -> Result<(), String> {
    let config_path = get_user_config_path()?;
    let data_dir = get_data_dir()?;

    // 确保目录存在
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }

    let json =
        serde_json::to_string_pretty(config).map_err(|e| format!("序列化配置失败: {}", e))?;

    fs::write(&config_path, json).map_err(|e| format!("写入配置文件失败: {}", e))?;

    // 更新运行时状态
    if let Ok(mut state) = get_runtime_state().write() {
        state.user_config = config.clone();
    }

    sync_global_proxy_env(config);

    crate::modules::logger::log_info(&format!(
        "[Config] 用户配置已保存: ws_enabled={}, ws_port={}, report_enabled={}, report_port={}",
        config.ws_enabled, config.ws_port, config.report_enabled, config.report_port
    ));

    Ok(())
}

/// 获取用户配置（从内存）
pub fn get_user_config() -> UserConfig {
    get_runtime_state()
        .read()
        .map(|state| state.user_config.clone())
        .unwrap_or_default()
}

/// 获取用户配置的首选端口
pub fn get_preferred_port() -> u16 {
    get_user_config().ws_port
}

/// 获取当前实际使用的端口
pub fn get_actual_port() -> Option<u16> {
    get_runtime_state()
        .read()
        .ok()
        .and_then(|state| state.actual_port)
}

/// 保存服务状态到共享文件
pub fn save_server_status(status: &ServerStatus) -> Result<(), String> {
    let status_path = get_server_status_path()?;
    let data_dir = get_data_dir()?;

    // 确保目录存在
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }

    // 写入状态文件
    let json =
        serde_json::to_string_pretty(status).map_err(|e| format!("序列化状态失败: {}", e))?;

    fs::write(&status_path, json).map_err(|e| format!("写入状态文件失败: {}", e))?;

    crate::modules::logger::log_info(&format!(
        "[Config] 服务状态已保存: ws_port={}, pid={}",
        status.ws_port, status.pid
    ));

    Ok(())
}

/// 初始化服务状态（WebSocket 启动后调用）
pub fn init_server_status(actual_port: u16) -> Result<(), String> {
    // 更新运行时状态
    if let Ok(mut state) = get_runtime_state().write() {
        state.actual_port = Some(actual_port);
    }

    let status = ServerStatus {
        ws_port: actual_port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        started_at: chrono::Utc::now().timestamp(),
    };

    save_server_status(&status)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::UserConfig;

    #[test]
    fn openclaw_auth_overwrite_default_is_disabled() {
        let cfg = UserConfig::default();
        assert!(!cfg.openclaw_auth_overwrite_on_switch);
    }

    #[test]
    fn openclaw_auth_overwrite_missing_field_falls_back_to_disabled() {
        let cfg: UserConfig =
            serde_json::from_value(serde_json::json!({})).expect("反序列化默认配置应成功");
        assert!(!cfg.openclaw_auth_overwrite_on_switch);
    }
}
