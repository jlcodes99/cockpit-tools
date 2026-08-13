use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::modules::logger;
use crate::modules::workbuddy_account;

/// WorkBuddy 成长中心地址（网页会话默认打开此页）。
/// 网页端为纯 cookie 鉴权，且登录态 cookie 为 httpOnly，
/// 因此不走 JS 注入，而是用「每账号独立 data_directory」持久化 cookie（见下）。
const WORKBUDDY_WEB_URL: &str = "https://www.workbuddy.cn/profile/growth-center";

/// 会话管理全局状态。
/// 每个打开的网页会话以 account_id 为主键（同一账号只能有一个活跃网页会话）。
pub struct WorkviewSession {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub webview_label: String,
    pub console_url: String,
    pub started_at: i64,
}

static SESSIONS: Mutex<Option<HashMap<String, WorkviewSession>>> = Mutex::new(None);

fn sessions() -> MutexGuard {
    let mut g = SESSIONS.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

type MutexGuard = std::sync::MutexGuard<'static, Option<HashMap<String, WorkviewSession>>>;

/// 返回给前端的会话摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkviewSessionInfo {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub webview_label: String,
    pub console_url: String,
    pub started_at: i64,
}

/// 网页会话是否受支持（本客户端使用系统 WebView，始终支持）。
#[tauri::command]
pub fn is_workbuddy_webview_supported() -> bool {
    true
}

/// 打开某个 WorkBuddy 账号的网页会话（成长中心）。
///
/// 方案：为每个账号分配独立的 WebView `data_directory`（类 Chrome 多 profile），
/// cookie 由系统 WebView 持久化到该目录，天然绕开 httpOnly 限制与 Windows 同步死锁。
/// 账号首次在窗口内登录一次后，cookie 留在自己的目录；之后再次打开即为登录态。
///
/// 注意：Windows 上 WebView2 窗口必须在主线程创建，否则会出现白屏且无法关闭。
/// 因此窗口的 `build()` 通过 `run_on_main_thread` 调度到主线程执行。
#[tauri::command]
pub async fn open_workbuddy_webview(
    app: AppHandle,
    account_id: String,
) -> Result<WorkviewSessionInfo, String> {
    let account = workbuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在：{}", account_id))?;

    // 账号级开关：显式关闭则拒绝打开
    if let Some(false) = account.web_session_enabled {
        return Err("该账号已禁用网页会话".to_string());
    }

    let email = account.email.clone();
    let webview_label = format!("workbuddy-webview-{}", account_id);

    // 同一账号只允许一个活跃会话
    {
        let mut s = sessions();
        if let Some(map) = s.as_mut() {
            if let Some(existing) = map.get(&account_id) {
                if app.get_webview_window(&existing.webview_label).is_some() {
                    return Ok(session_to_info(existing));
                }
            }
        }
    }

    // 每账号独立的 cookie 数据目录（物理隔离，避免串号）
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败：{}", e))?
        .join("webviews")
        .join("workbuddy")
        .join(&account_id);
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("创建 webview 数据目录失败：{}", e))?;

    let url = WORKBUDDY_WEB_URL.to_string();
    let console_url = url.clone();
    let webview_label_for_window = webview_label.clone();
    let email_for_window = email.clone();
    let data_dir_for_window = data_dir;
    let account_id_key = account_id.clone();

    // WebView2 窗口创建必须在主线程进行（见本函数注释）。
    // `&app` 用于调用 run_on_main_thread，`app_for_thread`（clone）移入闭包供窗口构建使用，
    // 避免 `app` 同时被借用又被 move 的 E0505 冲突。
    let app_for_thread = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<tauri::WebviewWindow, tauri::Error>>();
    app.run_on_main_thread(move || {
        let builder = WebviewWindowBuilder::new(
            &app_for_thread,
            &webview_label_for_window,
            WebviewUrl::External(url.parse().unwrap()),
        )
        .title(format!("WorkBuddy 成长中心 - {}", email_for_window))
        .data_directory(data_dir_for_window)
        .inner_size(1280.0, 800.0)
        .min_inner_size(800.0, 600.0)
        .center()
        .focused(true);

        let _ = tx.send(builder.build());
    })
    .map_err(|e| format!("调度主线程创建窗口失败：{}", e))?;

    let _window = rx
        .await
        .map_err(|_| "创建窗口任务已取消".to_string())?
        .map_err(|e| format!("创建网页窗口失败：{}", e))?;

    let session = WorkviewSession {
        id: account_id_key.clone(),
        account_id: account_id_key.clone(),
        email,
        webview_label: webview_label.clone(),
        console_url,
        started_at: now_ms(),
    };

    {
        let mut s = sessions();
        if let Some(map) = s.as_mut() {
            map.insert(account_id_key.clone(), session);
        }
    }

    logger::log_info(&format!(
        "[WorkBuddy WebView] 已打开账号 {} 的网页会话（独立 cookie 目录）",
        account_id_key
    ));

    let info = {
        let s = sessions();
        s.as_ref()
            .and_then(|m| m.get(&account_id_key))
            .map(session_to_info)
            .unwrap()
    };
    Ok(info)
}

/// 关闭某个账号的网页会话。
/// 关闭窗口同样调度到主线程，避免在 Windows 上出现卡死。
#[tauri::command]
pub async fn close_workbuddy_webview(app: AppHandle, account_id: String) -> Result<(), String> {
    let label = {
        let s = sessions();
        s.as_ref()
            .and_then(|m| m.get(&account_id))
            .map(|sess| sess.webview_label.clone())
    };

    if let Some(label) = label {
        let app_for_window = app.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), tauri::Error>>();
        app.run_on_main_thread(move || {
            let result = if let Some(window) = app_for_window.get_webview_window(&label) {
                window.close()
            } else {
                Ok(())
            };
            let _ = tx.send(result);
        })
        .map_err(|e| format!("调度主线程关闭窗口失败：{}", e))?;
        rx.await
            .map_err(|_| "关闭窗口任务已取消".to_string())?
            .map_err(|e| format!("关闭网页窗口失败：{}", e))?;

        let mut s = sessions();
        if let Some(map) = s.as_mut() {
            map.remove(&account_id);
        }
        logger::log_info(&format!(
            "[WorkBuddy WebView] 已关闭账号 {} 的网页会话",
            account_id
        ));
    }
    Ok(())
}

/// 列出当前所有活跃的网页会话。
#[tauri::command]
pub fn list_workbuddy_webview_sessions(app: AppHandle) -> Result<Vec<WorkviewSessionInfo>, String> {
    let mut s = sessions();
    let map = match s.as_mut() {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };

    // 清理已关闭的窗口
    let mut to_remove = Vec::new();
    for (account_id, sess) in map.iter() {
        if app.get_webview_window(&sess.webview_label).is_none() {
            to_remove.push(account_id.clone());
        }
    }
    for id in to_remove {
        map.remove(&id);
    }

    let infos: Vec<WorkviewSessionInfo> = map.values().map(session_to_info).collect();
    Ok(infos)
}

fn session_to_info(s: &WorkviewSession) -> WorkviewSessionInfo {
    WorkviewSessionInfo {
        id: s.id.clone(),
        account_id: s.account_id.clone(),
        email: s.email.clone(),
        webview_label: s.webview_label.clone(),
        console_url: s.console_url.clone(),
        started_at: s.started_at,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
