use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::Manager;
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
const AUTO_BACKUP_DIR_NAME: &str = "backups";

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
    /// 界面缩放比例（WebView Zoom）
    pub ui_scale: f64,
    /// 自动刷新间隔（分钟），-1 表示禁用
    pub auto_refresh_minutes: i32,
    /// Codex 自动刷新间隔（分钟），-1 表示禁用
    pub codex_auto_refresh_minutes: i32,
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
    /// Gemini 自动刷新间隔（分钟），-1 表示禁用
    pub gemini_auto_refresh_minutes: i32,
    /// Gemini 切号时是否同步覆盖 WSL 配置 (Windows Only)
    pub gemini_sync_wsl: bool,
    /// CodeBuddy 自动刷新间隔（分钟），-1 表示禁用
    pub codebuddy_auto_refresh_minutes: i32,
    /// CodeBuddy CN 自动刷新间隔（分钟），-1 表示禁用
    pub codebuddy_cn_auto_refresh_minutes: i32,
    /// WorkBuddy 自动刷新间隔（分钟），-1 表示禁用
    pub workbuddy_auto_refresh_minutes: i32,
    /// Qoder 自动刷新间隔（分钟），-1 表示禁用
    pub qoder_auto_refresh_minutes: i32,
    /// Trae 自动刷新间隔（分钟），-1 表示禁用
    pub trae_auto_refresh_minutes: i32,
    /// 窗口关闭行为: "ask", "minimize", "quit"
    pub close_behavior: String,
    /// 窗口最小化行为（macOS）: "dock_and_tray", "tray_only"
    pub minimize_behavior: String,
    /// 是否隐藏 Dock 图标（macOS）
    pub hide_dock_icon: bool,
    /// 是否在启动时显示悬浮卡片
    pub floating_card_show_on_startup: bool,
    /// 悬浮卡片是否默认置顶
    pub floating_card_always_on_top: bool,
    /// 是否启用应用开机自启动
    pub app_auto_launch_enabled: bool,
    /// 是否在应用启动后触发 Antigravity 唤醒
    pub antigravity_startup_wakeup_enabled: bool,
    /// Antigravity 启动后唤醒延时（秒）
    pub antigravity_startup_wakeup_delay_seconds: i32,
    /// 是否在应用启动后触发 Codex 唤醒
    pub codex_startup_wakeup_enabled: bool,
    /// Codex 启动后唤醒延时（秒）
    pub codex_startup_wakeup_delay_seconds: i32,
    /// 关闭悬浮卡片前是否显示确认弹框
    pub floating_card_confirm_on_close: bool,
    /// OpenCode 启动路径（为空则使用默认路径）
    pub opencode_app_path: String,
    /// Antigravity 启动路径（为空则使用默认路径）
    pub antigravity_app_path: String,
    /// Codex 启动路径（为空则使用默认路径）
    pub codex_app_path: String,
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
    /// CodeBuddy CN 启动路径（为空则使用默认路径）
    pub codebuddy_cn_app_path: String,
    /// Qoder 启动路径（为空则使用默认路径）
    pub qoder_app_path: String,
    /// Trae 启动路径（为空则使用默认路径）
    pub trae_app_path: String,
    /// WorkBuddy 启动路径（为空则使用默认路径）
    pub workbuddy_app_path: String,
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
    /// 切换 Codex 时是否自动启动/重启 Codex App
    pub codex_launch_on_switch: bool,
    /// 切换 Codex 时是否自动重启指定应用
    pub codex_restart_specified_app_on_switch: bool,
    /// 是否在 Codex 总览中显示 API 服务入口
    pub codex_local_access_entry_visible: bool,
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
    /// 是否启用 Gemini 配额预警通知
    pub gemini_quota_alert_enabled: bool,
    /// Gemini 配额预警阈值（百分比）
    pub gemini_quota_alert_threshold: i32,
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
    /// 是否启用 WorkBuddy 配额预警通知
    pub workbuddy_quota_alert_enabled: bool,
    /// WorkBuddy 配额预警阈值（百分比）
    pub workbuddy_quota_alert_threshold: i32,
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

const DEFAULT_UI_SCALE: f64 = 1.0;
const MIN_UI_SCALE: f64 = 0.8;
const MAX_UI_SCALE: f64 = 2.0;
const MAX_STARTUP_WAKEUP_DELAY_SECONDS: i32 = 24 * 60 * 60;
const AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";

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
    let current = config::get_user_config();
    let (next_include_accounts, next_include_config) =
        config::normalize_auto_backup_selection(include_accounts, include_config);
    let next_retention_days = config::sanitize_auto_backup_retention_days(retention_days);
    let new_config = UserConfig {
        auto_backup_enabled: enabled,
        auto_backup_include_accounts: next_include_accounts,
        auto_backup_include_config: next_include_config,
        auto_backup_retention_days: next_retention_days,
        ..current
    };
    config::save_user_config(&new_config)?;
    build_auto_backup_settings(&new_config)
}

#[tauri::command]
pub fn update_auto_backup_last_run(
    last_backup_at: Option<String>,
) -> Result<AutoBackupSettings, String> {
    let current = config::get_user_config();
    let normalized_last_backup_at = last_backup_at.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let new_config = UserConfig {
        auto_backup_last_backup_at: normalized_last_backup_at,
        ..current
    };
    config::save_user_config(&new_config)?;
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
    let current = config::get_user_config();
    let next_report_enabled = report_enabled.unwrap_or(current.report_enabled);
    let next_report_port = report_port.unwrap_or(current.report_port);
    let next_report_token = report_token
        .unwrap_or_else(|| current.report_token.clone())
        .trim()
        .to_string();
    let next_global_proxy_enabled = global_proxy_enabled.unwrap_or(current.global_proxy_enabled);
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

    let needs_restart = current.ws_port != ws_port
        || current.ws_enabled != ws_enabled
        || current.report_enabled != next_report_enabled
        || current.report_port != next_report_port
        || current.report_token != next_report_token;

    let new_config = UserConfig {
        ws_enabled,
        ws_port,
        report_enabled: next_report_enabled,
        report_port: next_report_port,
        report_token: next_report_token,
        global_proxy_enabled: next_global_proxy_enabled,
        global_proxy_url: next_global_proxy_url,
        global_proxy_no_proxy: next_global_proxy_no_proxy,
        // 保留其他设置不变
        language: current.language,
        default_terminal: current.default_terminal,
        theme: current.theme,
        ui_scale: current.ui_scale,
        auto_refresh_minutes: current.auto_refresh_minutes,
        codex_auto_refresh_minutes: current.codex_auto_refresh_minutes,
        zed_auto_refresh_minutes: current.zed_auto_refresh_minutes,
        ghcp_auto_refresh_minutes: current.ghcp_auto_refresh_minutes,
        windsurf_auto_refresh_minutes: current.windsurf_auto_refresh_minutes,
        kiro_auto_refresh_minutes: current.kiro_auto_refresh_minutes,
        cursor_auto_refresh_minutes: current.cursor_auto_refresh_minutes,
        gemini_auto_refresh_minutes: current.gemini_auto_refresh_minutes,
        gemini_sync_wsl: current.gemini_sync_wsl,
        codebuddy_auto_refresh_minutes: current.codebuddy_auto_refresh_minutes,
        codebuddy_cn_auto_refresh_minutes: current.codebuddy_cn_auto_refresh_minutes,
        workbuddy_auto_refresh_minutes: current.workbuddy_auto_refresh_minutes,
        qoder_auto_refresh_minutes: current.qoder_auto_refresh_minutes,
        trae_auto_refresh_minutes: current.trae_auto_refresh_minutes,
        close_behavior: current.close_behavior,
        minimize_behavior: current.minimize_behavior,
        hide_dock_icon: current.hide_dock_icon,
        floating_card_show_on_startup: current.floating_card_show_on_startup,
        floating_card_always_on_top: current.floating_card_always_on_top,
        app_auto_launch_enabled: current.app_auto_launch_enabled,
        antigravity_startup_wakeup_enabled: current.antigravity_startup_wakeup_enabled,
        antigravity_startup_wakeup_delay_seconds: current.antigravity_startup_wakeup_delay_seconds,
        codex_startup_wakeup_enabled: current.codex_startup_wakeup_enabled,
        codex_startup_wakeup_delay_seconds: current.codex_startup_wakeup_delay_seconds,
        floating_card_confirm_on_close: current.floating_card_confirm_on_close,
        auto_backup_enabled: current.auto_backup_enabled,
        auto_backup_include_accounts: current.auto_backup_include_accounts,
        auto_backup_include_config: current.auto_backup_include_config,
        auto_backup_retention_days: current.auto_backup_retention_days,
        auto_backup_retention_days_migrated: current.auto_backup_retention_days_migrated,
        auto_backup_last_backup_at: current.auto_backup_last_backup_at,
        floating_card_position_x: current.floating_card_position_x,
        floating_card_position_y: current.floating_card_position_y,
        opencode_app_path: current.opencode_app_path,
        antigravity_app_path: current.antigravity_app_path,
        codex_app_path: current.codex_app_path,
        codex_specified_app_path: current.codex_specified_app_path,
        zed_app_path: current.zed_app_path,
        vscode_app_path: current.vscode_app_path,
        windsurf_app_path: current.windsurf_app_path,
        kiro_app_path: current.kiro_app_path,
        cursor_app_path: current.cursor_app_path,
        codebuddy_app_path: current.codebuddy_app_path,
        codebuddy_cn_app_path: current.codebuddy_cn_app_path,
        qoder_app_path: current.qoder_app_path,
        trae_app_path: current.trae_app_path,
        workbuddy_app_path: current.workbuddy_app_path,
        opencode_sync_on_switch: current.opencode_sync_on_switch,
        opencode_auth_overwrite_on_switch: current.opencode_auth_overwrite_on_switch,
        ghcp_opencode_sync_on_switch: current.ghcp_opencode_sync_on_switch,
        ghcp_opencode_auth_overwrite_on_switch: current.ghcp_opencode_auth_overwrite_on_switch,
        ghcp_launch_on_switch: current.ghcp_launch_on_switch,
        openclaw_auth_overwrite_on_switch: current.openclaw_auth_overwrite_on_switch,
        codex_launch_on_switch: current.codex_launch_on_switch,
        codex_restart_specified_app_on_switch: current.codex_restart_specified_app_on_switch,
        codex_local_access_entry_visible: current.codex_local_access_entry_visible,
        antigravity_dual_switch_no_restart_enabled: current
            .antigravity_dual_switch_no_restart_enabled,
        auto_switch_enabled: current.auto_switch_enabled,
        auto_switch_threshold: current.auto_switch_threshold,
        auto_switch_credits_enabled: current.auto_switch_credits_enabled,
        auto_switch_credits_threshold: current.auto_switch_credits_threshold,
        auto_switch_scope_mode: current.auto_switch_scope_mode,
        auto_switch_selected_group_ids: current.auto_switch_selected_group_ids,
        auto_switch_account_scope_mode: current.auto_switch_account_scope_mode,
        auto_switch_selected_account_ids: current.auto_switch_selected_account_ids,
        codex_auto_switch_enabled: current.codex_auto_switch_enabled,
        codex_auto_switch_primary_threshold: current.codex_auto_switch_primary_threshold,
        codex_auto_switch_secondary_threshold: current.codex_auto_switch_secondary_threshold,
        codex_auto_switch_account_scope_mode: current.codex_auto_switch_account_scope_mode,
        codex_auto_switch_selected_account_ids: current.codex_auto_switch_selected_account_ids,
        quota_alert_enabled: current.quota_alert_enabled,
        quota_alert_threshold: current.quota_alert_threshold,
        codex_quota_alert_enabled: current.codex_quota_alert_enabled,
        codex_quota_alert_threshold: current.codex_quota_alert_threshold,
        zed_quota_alert_enabled: current.zed_quota_alert_enabled,
        zed_quota_alert_threshold: current.zed_quota_alert_threshold,
        codex_quota_alert_primary_threshold: current.codex_quota_alert_primary_threshold,
        codex_quota_alert_secondary_threshold: current.codex_quota_alert_secondary_threshold,
        ghcp_quota_alert_enabled: current.ghcp_quota_alert_enabled,
        ghcp_quota_alert_threshold: current.ghcp_quota_alert_threshold,
        windsurf_quota_alert_enabled: current.windsurf_quota_alert_enabled,
        windsurf_quota_alert_threshold: current.windsurf_quota_alert_threshold,
        kiro_quota_alert_enabled: current.kiro_quota_alert_enabled,
        kiro_quota_alert_threshold: current.kiro_quota_alert_threshold,
        cursor_quota_alert_enabled: current.cursor_quota_alert_enabled,
        cursor_quota_alert_threshold: current.cursor_quota_alert_threshold,
        gemini_quota_alert_enabled: current.gemini_quota_alert_enabled,
        gemini_quota_alert_threshold: current.gemini_quota_alert_threshold,
        codebuddy_quota_alert_enabled: current.codebuddy_quota_alert_enabled,
        codebuddy_quota_alert_threshold: current.codebuddy_quota_alert_threshold,
        codebuddy_cn_quota_alert_enabled: current.codebuddy_cn_quota_alert_enabled,
        codebuddy_cn_quota_alert_threshold: current.codebuddy_cn_quota_alert_threshold,
        qoder_quota_alert_enabled: current.qoder_quota_alert_enabled,
        qoder_quota_alert_threshold: current.qoder_quota_alert_threshold,
        trae_quota_alert_enabled: current.trae_quota_alert_enabled,
        trae_quota_alert_threshold: current.trae_quota_alert_threshold,
        workbuddy_quota_alert_enabled: current.workbuddy_quota_alert_enabled,
        workbuddy_quota_alert_threshold: current.workbuddy_quota_alert_threshold,
    };

    config::save_user_config(&new_config)?;

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
        let terminals = ["cmd", "powershell", "pwsh", "wt"];
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

/// 获取通用设置配置
#[tauri::command]
pub fn get_general_config(app: tauri::AppHandle) -> Result<GeneralConfig, String> {
    let started = Instant::now();
    let mut user_config = config::get_user_config();
    let app_auto_launch_enabled =
        get_app_auto_launch_enabled(&app).unwrap_or(user_config.app_auto_launch_enabled);
    if app_auto_launch_enabled != user_config.app_auto_launch_enabled {
        user_config.app_auto_launch_enabled = app_auto_launch_enabled;
        if let Err(err) = config::save_user_config(&user_config) {
            modules::logger::log_warn(&format!(
                "[SystemConfig] 同步应用自启动状态到本地配置失败: {}",
                err
            ));
        }
    }

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
        ui_scale: user_config.ui_scale,
        auto_refresh_minutes: user_config.auto_refresh_minutes,
        codex_auto_refresh_minutes: user_config.codex_auto_refresh_minutes,
        zed_auto_refresh_minutes: user_config.zed_auto_refresh_minutes,
        ghcp_auto_refresh_minutes: user_config.ghcp_auto_refresh_minutes,
        windsurf_auto_refresh_minutes: user_config.windsurf_auto_refresh_minutes,
        kiro_auto_refresh_minutes: user_config.kiro_auto_refresh_minutes,
        cursor_auto_refresh_minutes: user_config.cursor_auto_refresh_minutes,
        gemini_auto_refresh_minutes: user_config.gemini_auto_refresh_minutes,
        gemini_sync_wsl: user_config.gemini_sync_wsl,
        codebuddy_auto_refresh_minutes: user_config.codebuddy_auto_refresh_minutes,
        codebuddy_cn_auto_refresh_minutes: user_config.codebuddy_cn_auto_refresh_minutes,
        workbuddy_auto_refresh_minutes: user_config.workbuddy_auto_refresh_minutes,
        qoder_auto_refresh_minutes: user_config.qoder_auto_refresh_minutes,
        trae_auto_refresh_minutes: user_config.trae_auto_refresh_minutes,
        close_behavior: close_behavior_str.to_string(),
        minimize_behavior: minimize_behavior_str.to_string(),
        hide_dock_icon: user_config.hide_dock_icon,
        floating_card_show_on_startup: user_config.floating_card_show_on_startup,
        floating_card_always_on_top: user_config.floating_card_always_on_top,
        app_auto_launch_enabled,
        antigravity_startup_wakeup_enabled: user_config.antigravity_startup_wakeup_enabled,
        antigravity_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.antigravity_startup_wakeup_delay_seconds,
        ),
        codex_startup_wakeup_enabled: user_config.codex_startup_wakeup_enabled,
        codex_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.codex_startup_wakeup_delay_seconds,
        ),
        floating_card_confirm_on_close: user_config.floating_card_confirm_on_close,
        opencode_app_path: user_config.opencode_app_path,
        antigravity_app_path: user_config.antigravity_app_path,
        codex_app_path: user_config.codex_app_path,
        codex_specified_app_path: user_config.codex_specified_app_path,
        zed_app_path: user_config.zed_app_path,
        vscode_app_path: user_config.vscode_app_path,
        windsurf_app_path: user_config.windsurf_app_path,
        kiro_app_path: user_config.kiro_app_path,
        cursor_app_path: user_config.cursor_app_path,
        codebuddy_app_path: user_config.codebuddy_app_path,
        codebuddy_cn_app_path: user_config.codebuddy_cn_app_path,
        qoder_app_path: user_config.qoder_app_path,
        trae_app_path: user_config.trae_app_path,
        workbuddy_app_path: user_config.workbuddy_app_path,
        opencode_sync_on_switch: user_config.opencode_sync_on_switch,
        opencode_auth_overwrite_on_switch: user_config.opencode_auth_overwrite_on_switch,
        ghcp_opencode_sync_on_switch: user_config.ghcp_opencode_sync_on_switch,
        ghcp_opencode_auth_overwrite_on_switch: user_config.ghcp_opencode_auth_overwrite_on_switch,
        ghcp_launch_on_switch: user_config.ghcp_launch_on_switch,
        openclaw_auth_overwrite_on_switch: user_config.openclaw_auth_overwrite_on_switch,
        codex_launch_on_switch: user_config.codex_launch_on_switch,
        codex_restart_specified_app_on_switch: user_config.codex_restart_specified_app_on_switch,
        codex_local_access_entry_visible: user_config.codex_local_access_entry_visible,
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
        gemini_quota_alert_enabled: user_config.gemini_quota_alert_enabled,
        gemini_quota_alert_threshold: user_config.gemini_quota_alert_threshold,
        codebuddy_quota_alert_enabled: user_config.codebuddy_quota_alert_enabled,
        codebuddy_quota_alert_threshold: user_config.codebuddy_quota_alert_threshold,
        codebuddy_cn_quota_alert_enabled: user_config.codebuddy_cn_quota_alert_enabled,
        codebuddy_cn_quota_alert_threshold: user_config.codebuddy_cn_quota_alert_threshold,
        qoder_quota_alert_enabled: user_config.qoder_quota_alert_enabled,
        qoder_quota_alert_threshold: user_config.qoder_quota_alert_threshold,
        trae_quota_alert_enabled: user_config.trae_quota_alert_enabled,
        trae_quota_alert_threshold: user_config.trae_quota_alert_threshold,
        workbuddy_quota_alert_enabled: user_config.workbuddy_quota_alert_enabled,
        workbuddy_quota_alert_threshold: user_config.workbuddy_quota_alert_threshold,
    };

    modules::logger::log_info(&format!(
        "[StartupPerf][SystemCommand] get_general_config completed in {}ms: auto_refresh={}, codex={}, zed={}, ghcp={}, windsurf={}, kiro={}, cursor={}, gemini={}, codebuddy={}, codebuddy_cn={}, workbuddy={}, qoder={}, trae={}, auto_switch={}",
        started.elapsed().as_millis(),
        result.auto_refresh_minutes,
        result.codex_auto_refresh_minutes,
        result.zed_auto_refresh_minutes,
        result.ghcp_auto_refresh_minutes,
        result.windsurf_auto_refresh_minutes,
        result.kiro_auto_refresh_minutes,
        result.cursor_auto_refresh_minutes,
        result.gemini_auto_refresh_minutes,
        result.codebuddy_auto_refresh_minutes,
        result.codebuddy_cn_auto_refresh_minutes,
        result.workbuddy_auto_refresh_minutes,
        result.qoder_auto_refresh_minutes,
        result.trae_auto_refresh_minutes,
        result.auto_switch_enabled
    ));

    Ok(result)
}

/// 保存通用设置配置
#[tauri::command]
pub fn save_general_config(
    app: tauri::AppHandle,
    language: String,
    default_terminal: Option<String>,
    theme: String,
    ui_scale: Option<f64>,
    auto_refresh_minutes: i32,
    codex_auto_refresh_minutes: i32,
    zed_auto_refresh_minutes: Option<i32>,
    ghcp_auto_refresh_minutes: Option<i32>,
    windsurf_auto_refresh_minutes: Option<i32>,
    kiro_auto_refresh_minutes: Option<i32>,
    cursor_auto_refresh_minutes: Option<i32>,
    gemini_auto_refresh_minutes: Option<i32>,
    gemini_sync_wsl: Option<bool>,
    codebuddy_auto_refresh_minutes: Option<i32>,
    codebuddy_cn_auto_refresh_minutes: Option<i32>,
    workbuddy_auto_refresh_minutes: Option<i32>,
    qoder_auto_refresh_minutes: Option<i32>,
    trae_auto_refresh_minutes: Option<i32>,
    close_behavior: String,
    minimize_behavior: Option<String>,
    hide_dock_icon: Option<bool>,
    floating_card_show_on_startup: Option<bool>,
    floating_card_always_on_top: Option<bool>,
    app_auto_launch_enabled: Option<bool>,
    antigravity_startup_wakeup_enabled: Option<bool>,
    antigravity_startup_wakeup_delay_seconds: Option<i32>,
    codex_startup_wakeup_enabled: Option<bool>,
    codex_startup_wakeup_delay_seconds: Option<i32>,
    floating_card_confirm_on_close: Option<bool>,
    opencode_app_path: String,
    antigravity_app_path: String,
    codex_app_path: String,
    codex_specified_app_path: Option<String>,
    zed_app_path: Option<String>,
    vscode_app_path: String,
    windsurf_app_path: Option<String>,
    kiro_app_path: Option<String>,
    cursor_app_path: Option<String>,
    codebuddy_app_path: Option<String>,
    codebuddy_cn_app_path: Option<String>,
    qoder_app_path: Option<String>,
    trae_app_path: Option<String>,
    workbuddy_app_path: Option<String>,
    opencode_sync_on_switch: bool,
    opencode_auth_overwrite_on_switch: Option<bool>,
    ghcp_opencode_sync_on_switch: Option<bool>,
    ghcp_opencode_auth_overwrite_on_switch: Option<bool>,
    ghcp_launch_on_switch: Option<bool>,
    openclaw_auth_overwrite_on_switch: Option<bool>,
    codex_launch_on_switch: bool,
    codex_restart_specified_app_on_switch: Option<bool>,
    codex_local_access_entry_visible: Option<bool>,
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
    gemini_quota_alert_enabled: Option<bool>,
    gemini_quota_alert_threshold: Option<i32>,
    codebuddy_quota_alert_enabled: Option<bool>,
    codebuddy_quota_alert_threshold: Option<i32>,
    codebuddy_cn_quota_alert_enabled: Option<bool>,
    codebuddy_cn_quota_alert_threshold: Option<i32>,
    qoder_quota_alert_enabled: Option<bool>,
    qoder_quota_alert_threshold: Option<i32>,
    trae_quota_alert_enabled: Option<bool>,
    trae_quota_alert_threshold: Option<i32>,
    workbuddy_quota_alert_enabled: Option<bool>,
    workbuddy_quota_alert_threshold: Option<i32>,
) -> Result<(), String> {
    let current = config::get_user_config();
    let normalized_opencode_path = opencode_app_path.trim().to_string();
    let normalized_antigravity_path = antigravity_app_path.trim().to_string();
    let normalized_codex_path = codex_app_path.trim().to_string();
    let normalized_codex_specified_app_path = codex_specified_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.codex_specified_app_path.clone());
    let normalized_zed_path = zed_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.zed_app_path.clone());
    let normalized_vscode_path = vscode_app_path.trim().to_string();
    let normalized_ui_scale = sanitize_ui_scale(ui_scale.unwrap_or(current.ui_scale));
    let normalized_windsurf_path = windsurf_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.windsurf_app_path.clone());
    let normalized_kiro_path = kiro_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.kiro_app_path.clone());
    let normalized_cursor_path = cursor_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.cursor_app_path.clone());
    let normalized_codebuddy_path = codebuddy_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.codebuddy_app_path.clone());
    let normalized_codebuddy_cn_path = codebuddy_cn_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.codebuddy_cn_app_path.clone());
    let normalized_qoder_path = qoder_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.qoder_app_path.clone());
    let normalized_trae_path = trae_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.trae_app_path.clone());
    let normalized_workbuddy_path = workbuddy_app_path
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.workbuddy_app_path.clone());
    // 标准化语言代码为小写，确保与插件端格式一致
    let normalized_language = language.to_lowercase();
    let language_changed = current.language != normalized_language;
    let language_for_broadcast = normalized_language.clone();

    // 解析关闭行为
    let close_behavior_enum = match close_behavior.as_str() {
        "minimize" => CloseWindowBehavior::Minimize,
        "quit" => CloseWindowBehavior::Quit,
        _ => CloseWindowBehavior::Ask,
    };
    let minimize_behavior_enum = match minimize_behavior.as_deref() {
        Some("dock_and_tray") => MinimizeWindowBehavior::DockAndTray,
        Some("tray_only") => MinimizeWindowBehavior::TrayOnly,
        Some(_) | None => current.minimize_behavior.clone(),
    };
    let hide_dock_icon_value = hide_dock_icon.unwrap_or(current.hide_dock_icon);
    let floating_card_show_on_startup_value =
        floating_card_show_on_startup.unwrap_or(current.floating_card_show_on_startup);
    let floating_card_always_on_top_value =
        floating_card_always_on_top.unwrap_or(current.floating_card_always_on_top);
    let app_auto_launch_enabled_value =
        app_auto_launch_enabled.unwrap_or(current.app_auto_launch_enabled);
    let antigravity_startup_wakeup_enabled_value =
        antigravity_startup_wakeup_enabled.unwrap_or(current.antigravity_startup_wakeup_enabled);
    let antigravity_startup_wakeup_delay_seconds_value = sanitize_startup_wakeup_delay_seconds(
        antigravity_startup_wakeup_delay_seconds
            .unwrap_or(current.antigravity_startup_wakeup_delay_seconds),
    );
    let codex_startup_wakeup_enabled_value =
        codex_startup_wakeup_enabled.unwrap_or(current.codex_startup_wakeup_enabled);
    let codex_startup_wakeup_delay_seconds_value = sanitize_startup_wakeup_delay_seconds(
        codex_startup_wakeup_delay_seconds.unwrap_or(current.codex_startup_wakeup_delay_seconds),
    );
    let floating_card_confirm_on_close_value =
        floating_card_confirm_on_close.unwrap_or(current.floating_card_confirm_on_close);
    let next_codex_quota_alert_threshold =
        codex_quota_alert_threshold.unwrap_or(current.codex_quota_alert_threshold);
    let next_opencode_auth_overwrite_on_switch =
        opencode_auth_overwrite_on_switch.unwrap_or(current.opencode_auth_overwrite_on_switch);
    let next_opencode_sync_on_switch = if next_opencode_auth_overwrite_on_switch {
        opencode_sync_on_switch
    } else {
        false
    };
    let next_ghcp_opencode_auth_overwrite_on_switch = ghcp_opencode_auth_overwrite_on_switch
        .unwrap_or(current.ghcp_opencode_auth_overwrite_on_switch);
    let next_ghcp_opencode_sync_on_switch = if next_ghcp_opencode_auth_overwrite_on_switch {
        ghcp_opencode_sync_on_switch.unwrap_or(current.ghcp_opencode_sync_on_switch)
    } else {
        false
    };
    let current_app_auto_launch_enabled = current.app_auto_launch_enabled;
    #[cfg(target_os = "macos")]
    let hide_dock_icon_changed = current.hide_dock_icon != hide_dock_icon_value;

    let new_config = UserConfig {
        // 保留网络设置不变
        ws_enabled: current.ws_enabled,
        ws_port: current.ws_port,
        report_enabled: current.report_enabled,
        report_port: current.report_port,
        report_token: current.report_token,
        global_proxy_enabled: current.global_proxy_enabled,
        global_proxy_url: current.global_proxy_url,
        global_proxy_no_proxy: current.global_proxy_no_proxy,
        // 更新通用设置
        language: normalized_language.clone(),
        default_terminal: default_terminal.unwrap_or(current.default_terminal),
        theme,
        ui_scale: normalized_ui_scale,
        auto_refresh_minutes,
        codex_auto_refresh_minutes,
        zed_auto_refresh_minutes: zed_auto_refresh_minutes
            .unwrap_or(current.zed_auto_refresh_minutes),
        ghcp_auto_refresh_minutes: ghcp_auto_refresh_minutes
            .unwrap_or(current.ghcp_auto_refresh_minutes),
        windsurf_auto_refresh_minutes: windsurf_auto_refresh_minutes
            .unwrap_or(current.windsurf_auto_refresh_minutes),
        kiro_auto_refresh_minutes: kiro_auto_refresh_minutes
            .unwrap_or(current.kiro_auto_refresh_minutes),
        cursor_auto_refresh_minutes: cursor_auto_refresh_minutes
            .unwrap_or(current.cursor_auto_refresh_minutes),
        gemini_auto_refresh_minutes: gemini_auto_refresh_minutes
            .unwrap_or(current.gemini_auto_refresh_minutes),
        gemini_sync_wsl: gemini_sync_wsl
            .unwrap_or(current.gemini_sync_wsl),
        codebuddy_auto_refresh_minutes: codebuddy_auto_refresh_minutes
            .unwrap_or(current.codebuddy_auto_refresh_minutes),
        codebuddy_cn_auto_refresh_minutes: codebuddy_cn_auto_refresh_minutes
            .unwrap_or(current.codebuddy_cn_auto_refresh_minutes),
        workbuddy_auto_refresh_minutes: workbuddy_auto_refresh_minutes
            .unwrap_or(current.workbuddy_auto_refresh_minutes),
        qoder_auto_refresh_minutes: qoder_auto_refresh_minutes
            .unwrap_or(current.qoder_auto_refresh_minutes),
        trae_auto_refresh_minutes: trae_auto_refresh_minutes
            .unwrap_or(current.trae_auto_refresh_minutes),
        close_behavior: close_behavior_enum,
        minimize_behavior: minimize_behavior_enum,
        hide_dock_icon: hide_dock_icon_value,
        floating_card_show_on_startup: floating_card_show_on_startup_value,
        floating_card_always_on_top: floating_card_always_on_top_value,
        app_auto_launch_enabled: app_auto_launch_enabled_value,
        antigravity_startup_wakeup_enabled: antigravity_startup_wakeup_enabled_value,
        antigravity_startup_wakeup_delay_seconds: antigravity_startup_wakeup_delay_seconds_value,
        codex_startup_wakeup_enabled: codex_startup_wakeup_enabled_value,
        codex_startup_wakeup_delay_seconds: codex_startup_wakeup_delay_seconds_value,
        floating_card_confirm_on_close: floating_card_confirm_on_close_value,
        floating_card_position_x: current.floating_card_position_x,
        floating_card_position_y: current.floating_card_position_y,
        opencode_app_path: normalized_opencode_path,
        antigravity_app_path: normalized_antigravity_path,
        codex_app_path: normalized_codex_path,
        codex_specified_app_path: normalized_codex_specified_app_path,
        zed_app_path: normalized_zed_path,
        vscode_app_path: normalized_vscode_path,
        windsurf_app_path: normalized_windsurf_path,
        kiro_app_path: normalized_kiro_path,
        cursor_app_path: normalized_cursor_path,
        codebuddy_app_path: normalized_codebuddy_path,
        codebuddy_cn_app_path: normalized_codebuddy_cn_path,
        qoder_app_path: normalized_qoder_path,
        trae_app_path: normalized_trae_path,
        workbuddy_app_path: normalized_workbuddy_path,
        opencode_sync_on_switch: next_opencode_sync_on_switch,
        opencode_auth_overwrite_on_switch: next_opencode_auth_overwrite_on_switch,
        ghcp_opencode_sync_on_switch: next_ghcp_opencode_sync_on_switch,
        ghcp_opencode_auth_overwrite_on_switch: next_ghcp_opencode_auth_overwrite_on_switch,
        ghcp_launch_on_switch: ghcp_launch_on_switch.unwrap_or(current.ghcp_launch_on_switch),
        openclaw_auth_overwrite_on_switch: openclaw_auth_overwrite_on_switch
            .unwrap_or(current.openclaw_auth_overwrite_on_switch),
        codex_launch_on_switch,
        codex_restart_specified_app_on_switch: codex_restart_specified_app_on_switch
            .unwrap_or(current.codex_restart_specified_app_on_switch),
        codex_local_access_entry_visible: codex_local_access_entry_visible
            .unwrap_or(current.codex_local_access_entry_visible),
        antigravity_dual_switch_no_restart_enabled: antigravity_dual_switch_no_restart_enabled
            .unwrap_or(current.antigravity_dual_switch_no_restart_enabled),
        auto_switch_enabled: auto_switch_enabled.unwrap_or(current.auto_switch_enabled),
        auto_switch_threshold: auto_switch_threshold.unwrap_or(current.auto_switch_threshold),
        auto_switch_credits_enabled: auto_switch_credits_enabled
            .unwrap_or(current.auto_switch_credits_enabled),
        auto_switch_credits_threshold: auto_switch_credits_threshold
            .unwrap_or(current.auto_switch_credits_threshold),
        auto_switch_scope_mode: auto_switch_scope_mode
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or(current.auto_switch_scope_mode),
        auto_switch_selected_group_ids: auto_switch_selected_group_ids
            .unwrap_or(current.auto_switch_selected_group_ids),
        auto_switch_account_scope_mode: normalize_auto_switch_account_scope_mode(
            auto_switch_account_scope_mode
                .as_deref()
                .unwrap_or(current.auto_switch_account_scope_mode.as_str()),
        ),
        auto_switch_selected_account_ids: normalize_auto_switch_selected_account_ids(
            auto_switch_selected_account_ids
                .as_deref()
                .unwrap_or(current.auto_switch_selected_account_ids.as_slice()),
        ),
        codex_auto_switch_enabled: codex_auto_switch_enabled
            .unwrap_or(current.codex_auto_switch_enabled),
        codex_auto_switch_primary_threshold: codex_auto_switch_primary_threshold
            .unwrap_or(current.codex_auto_switch_primary_threshold),
        codex_auto_switch_secondary_threshold: codex_auto_switch_secondary_threshold
            .unwrap_or(current.codex_auto_switch_secondary_threshold),
        codex_auto_switch_account_scope_mode: normalize_auto_switch_account_scope_mode(
            codex_auto_switch_account_scope_mode
                .as_deref()
                .unwrap_or(current.codex_auto_switch_account_scope_mode.as_str()),
        ),
        codex_auto_switch_selected_account_ids: normalize_auto_switch_selected_account_ids(
            codex_auto_switch_selected_account_ids
                .as_deref()
                .unwrap_or(current.codex_auto_switch_selected_account_ids.as_slice()),
        ),
        quota_alert_enabled: quota_alert_enabled.unwrap_or(current.quota_alert_enabled),
        quota_alert_threshold: quota_alert_threshold.unwrap_or(current.quota_alert_threshold),
        codex_quota_alert_enabled: codex_quota_alert_enabled
            .unwrap_or(current.codex_quota_alert_enabled),
        codex_quota_alert_threshold: next_codex_quota_alert_threshold,
        zed_quota_alert_enabled: zed_quota_alert_enabled.unwrap_or(current.zed_quota_alert_enabled),
        zed_quota_alert_threshold: zed_quota_alert_threshold
            .unwrap_or(current.zed_quota_alert_threshold),
        codex_quota_alert_primary_threshold: codex_quota_alert_primary_threshold
            .unwrap_or(next_codex_quota_alert_threshold),
        codex_quota_alert_secondary_threshold: codex_quota_alert_secondary_threshold
            .unwrap_or(next_codex_quota_alert_threshold),
        ghcp_quota_alert_enabled: ghcp_quota_alert_enabled
            .unwrap_or(current.ghcp_quota_alert_enabled),
        ghcp_quota_alert_threshold: ghcp_quota_alert_threshold
            .unwrap_or(current.ghcp_quota_alert_threshold),
        windsurf_quota_alert_enabled: windsurf_quota_alert_enabled
            .unwrap_or(current.windsurf_quota_alert_enabled),
        windsurf_quota_alert_threshold: windsurf_quota_alert_threshold
            .unwrap_or(current.windsurf_quota_alert_threshold),
        kiro_quota_alert_enabled: kiro_quota_alert_enabled
            .unwrap_or(current.kiro_quota_alert_enabled),
        kiro_quota_alert_threshold: kiro_quota_alert_threshold
            .unwrap_or(current.kiro_quota_alert_threshold),
        cursor_quota_alert_enabled: cursor_quota_alert_enabled
            .unwrap_or(current.cursor_quota_alert_enabled),
        cursor_quota_alert_threshold: cursor_quota_alert_threshold
            .unwrap_or(current.cursor_quota_alert_threshold),
        gemini_quota_alert_enabled: gemini_quota_alert_enabled
            .unwrap_or(current.gemini_quota_alert_enabled),
        gemini_quota_alert_threshold: gemini_quota_alert_threshold
            .unwrap_or(current.gemini_quota_alert_threshold),
        codebuddy_quota_alert_enabled: codebuddy_quota_alert_enabled
            .unwrap_or(current.codebuddy_quota_alert_enabled),
        codebuddy_quota_alert_threshold: codebuddy_quota_alert_threshold
            .unwrap_or(current.codebuddy_quota_alert_threshold),
        codebuddy_cn_quota_alert_enabled: codebuddy_cn_quota_alert_enabled
            .unwrap_or(current.codebuddy_cn_quota_alert_enabled),
        codebuddy_cn_quota_alert_threshold: codebuddy_cn_quota_alert_threshold
            .unwrap_or(current.codebuddy_cn_quota_alert_threshold),
        qoder_quota_alert_enabled: qoder_quota_alert_enabled
            .unwrap_or(current.qoder_quota_alert_enabled),
        qoder_quota_alert_threshold: qoder_quota_alert_threshold
            .unwrap_or(current.qoder_quota_alert_threshold),
        trae_quota_alert_enabled: trae_quota_alert_enabled
            .unwrap_or(current.trae_quota_alert_enabled),
        trae_quota_alert_threshold: trae_quota_alert_threshold
            .unwrap_or(current.trae_quota_alert_threshold),
        workbuddy_quota_alert_enabled: workbuddy_quota_alert_enabled
            .unwrap_or(current.workbuddy_quota_alert_enabled),
        workbuddy_quota_alert_threshold: workbuddy_quota_alert_threshold
            .unwrap_or(current.workbuddy_quota_alert_threshold),
        auto_backup_enabled: current.auto_backup_enabled,
        auto_backup_include_accounts: current.auto_backup_include_accounts,
        auto_backup_include_config: current.auto_backup_include_config,
        auto_backup_retention_days: current.auto_backup_retention_days,
        auto_backup_retention_days_migrated: current.auto_backup_retention_days_migrated,
        auto_backup_last_backup_at: current.auto_backup_last_backup_at,
    };

    config::save_user_config(&new_config)?;

    if current_app_auto_launch_enabled != app_auto_launch_enabled_value {
        apply_app_auto_launch_enabled(&app, app_auto_launch_enabled_value)?;
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

    if language_changed {
        // 广播语言变更（如果有客户端连接，会通过 WebSocket 发送）
        websocket::broadcast_language_changed(&language_for_broadcast, "desktop");

        // 同时写入共享文件（供插件端离线时启动读取）
        // 因为无法确定插件端是否收到了 WebSocket 消息，保守策略是总是写入
        // 但为了减少写入，可以检查是否有客户端连接
        // 这里简化处理：总是写入，插件端启动时会比较时间戳
        modules::sync_settings::write_sync_setting("language", &normalized_language);

        // 仅在语言变更时刷新托盘菜单，避免无关配置触发托盘重建
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[Tray] 语言变更后刷新托盘失败: {}", err));
        }
    }

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
    let mut current = config::get_user_config();
    let normalized_path = path.trim().to_string();
    match app.as_str() {
        "antigravity" => current.antigravity_app_path = normalized_path,
        "codex" => current.codex_app_path = normalized_path,
        "zed" => current.zed_app_path = normalized_path,
        "vscode" => current.vscode_app_path = normalized_path,
        "windsurf" => current.windsurf_app_path = normalized_path,
        "kiro" => current.kiro_app_path = normalized_path,
        "cursor" => current.cursor_app_path = normalized_path,
        "codebuddy" => current.codebuddy_app_path = normalized_path,
        "codebuddy_cn" => current.codebuddy_cn_app_path = normalized_path,
        "qoder" => current.qoder_app_path = normalized_path,
        "trae" => current.trae_app_path = normalized_path,
        "workbuddy" => current.workbuddy_app_path = normalized_path,
        "opencode" => current.opencode_app_path = normalized_path,
        _ => return Err("未知应用类型".to_string()),
    }
    config::save_user_config(&current)?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_launch_on_switch(enabled: bool) -> Result<(), String> {
    let current = config::get_user_config();
    if current.codex_launch_on_switch == enabled {
        return Ok(());
    }
    let new_config = UserConfig {
        codex_launch_on_switch: enabled,
        ..current
    };
    config::save_user_config(&new_config)
}

#[tauri::command]
pub fn set_codex_local_access_entry_visible(enabled: bool) -> Result<(), String> {
    let current = config::get_user_config();
    if current.codex_local_access_entry_visible == enabled {
        return Ok(());
    }
    let new_config = UserConfig {
        codex_local_access_entry_visible: enabled,
        ..current
    };
    config::save_user_config(&new_config)
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
        "antigravity" | "codex" | "zed" | "vscode" | "codebuddy" | "codebuddy_cn" | "qoder"
        | "trae" | "opencode" | "workbuddy" => Ok(modules::process::detect_and_save_app_path(
            app.as_str(),
            force,
        )),
        _ => Err("未知应用类型".to_string()),
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
        let current = config::get_user_config();
        let close_behavior = match action.as_str() {
            "minimize" => CloseWindowBehavior::Minimize,
            "quit" => CloseWindowBehavior::Quit,
            _ => CloseWindowBehavior::Ask,
        };

        let new_config = UserConfig {
            close_behavior,
            ..current
        };

        config::save_user_config(&new_config)?;
        modules::logger::log_info(&format!("[Window] 已保存关闭行为设置: {}", action));
    }

    // 执行操作
    match action.as_str() {
        "minimize" => {
            let _ = window.hide();
            modules::logger::log_info("[Window] 窗口已最小化到托盘");
        }
        "quit" => {
            window.app_handle().exit(0);
        }
        _ => {
            return Err("无效的操作".to_string());
        }
    }

    Ok(())
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
    let current = config::get_user_config();
    if current.floating_card_always_on_top == always_on_top {
        return modules::floating_card_window::apply_floating_card_always_on_top(&app);
    }

    let new_config = UserConfig {
        floating_card_always_on_top: always_on_top,
        ..current
    };
    config::save_user_config(&new_config)?;
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
    let current = config::get_user_config();
    if current.floating_card_confirm_on_close == confirm_on_close {
        return Ok(());
    }

    let new_config = UserConfig {
        floating_card_confirm_on_close: confirm_on_close,
        ..current
    };
    config::save_user_config(&new_config)
}

#[tauri::command]
pub fn save_floating_card_position(x: i32, y: i32) -> Result<(), String> {
    let current = config::get_user_config();
    if current.floating_card_position_x == Some(x) && current.floating_card_position_y == Some(y) {
        return Ok(());
    }

    let new_config = UserConfig {
        floating_card_position_x: Some(x),
        floating_card_position_y: Some(y),
        ..current
    };
    config::save_user_config(&new_config)
}

#[tauri::command]
pub fn show_main_window_and_navigate(app: tauri::AppHandle, page: String) -> Result<(), String> {
    modules::floating_card_window::show_main_window_and_navigate(&app, &page)
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
