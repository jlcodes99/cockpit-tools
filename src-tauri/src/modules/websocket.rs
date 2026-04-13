//! WebSocket 服务模块
//! 提供本地 WebSocket 服务供 VS Code 扩展实时通信

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;

use super::config::{get_preferred_port, init_server_status, PORT_RANGE};

/// 消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsMessage {
    // ============ 事件通知（Tools -> 扩展） ============
    /// 服务就绪
    #[serde(rename = "event.ready")]
    Ready { version: String },

    /// 数据已变更，请刷新
    #[serde(rename = "event.data_changed")]
    DataChanged { source: String },

    /// 语言已变更
    #[serde(rename = "event.language_changed")]
    LanguageChanged { language: String, source: String },

    /// 账号切换完成
    #[serde(rename = "event.account_switched")]
    AccountSwitched { account_id: String, email: String },

    /// 切换账号错误
    #[serde(rename = "event.switch_error")]
    SwitchError { message: String },

    /// 唤醒功能互斥开关
    #[serde(rename = "event.wakeup_override")]
    WakeupOverride { enabled: bool },

    /// 触发扩展执行无感切号
    #[serde(rename = "event.plugin_switch_account")]
    PluginSwitchAccountEvent {
        request_id: String,
        target_email: String,
        switch_mode: String,
        trigger_type: String,
        trigger_source: String,
        reason: String,
    },

    // ============ 请求（扩展 -> Tools） ============
    /// 请求获取账号列表
    #[serde(rename = "request.get_accounts")]
    GetAccounts { request_id: String },

    /// 请求获取账号列表（包含 Token）
    #[serde(rename = "request.get_accounts_with_tokens")]
    GetAccountsWithTokens { request_id: String },

    /// 请求获取当前账号
    #[serde(rename = "request.get_current_account")]
    GetCurrentAccount { request_id: String },

    /// 请求切换账号（真正的切换）
    #[serde(rename = "request.switch_account")]
    SwitchAccount {
        account_id: String,
        #[serde(default)]
        request_id: Option<String>,
    },

    /// 请求设置语言
    #[serde(rename = "request.set_language")]
    SetLanguage {
        request_id: String,
        language: String,
        source: Option<String>,
    },

    /// 请求添加/更新账号（扩展端登录后同步）
    #[serde(rename = "request.add_account")]
    AddAccount {
        request_id: String,
        email: String,
        refresh_token: String,
        access_token: Option<String>,
        expires_at: Option<i64>,
    },

    /// 请求删除账号（扩展端删除后同步）
    #[serde(rename = "request.delete_account")]
    DeleteAccountByEmail { request_id: String, email: String },

    /// 通知数据已变更
    #[serde(rename = "request.data_changed")]
    NotifyDataChanged { source: String },

    /// Ping（心跳）
    #[serde(rename = "ping")]
    Ping,

    /// Pong（心跳响应）
    #[serde(rename = "pong")]
    Pong,

    // ============ 响应（Tools -> 扩展） ============
    /// 账号列表响应
    #[serde(rename = "response.accounts")]
    AccountsResponse {
        request_id: String,
        accounts: Vec<AccountInfo>,
        current_account_id: Option<String>,
    },

    /// 账号列表响应（包含 Token）
    #[serde(rename = "response.accounts_with_tokens")]
    AccountsWithTokensResponse {
        request_id: String,
        accounts: Vec<AccountTokenInfo>,
        current_account_id: Option<String>,
    },

    /// 当前账号响应
    #[serde(rename = "response.current_account")]
    CurrentAccountResponse {
        request_id: String,
        account: Option<AccountInfo>,
    },

    /// 操作成功响应
    #[serde(rename = "response.success")]
    SuccessResponse { request_id: String, message: String },

    /// 错误响应
    #[serde(rename = "response.error")]
    ErrorResponse { request_id: String, error: String },

    /// 扩展无感切号响应
    #[serde(rename = "response.plugin_switch_account")]
    PluginSwitchAccountResponse {
        execution_id: String,
        request_id: Option<String>,
        success: bool,
        effective_mode: String,
        from_email: Option<String>,
        to_email: String,
        duration_ms: u64,
        error_code: Option<String>,
        error_message: Option<String>,
        finished_at: String,
    },
}

/// 账号信息（用于 WebSocket 传输）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub is_current: bool,
    pub disabled: bool,
    pub has_fingerprint: bool,
    pub last_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
}

/// 账号信息（包含 Token，用于同步）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountTokenInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub is_current: bool,
    pub disabled: bool,
    pub has_fingerprint: bool,
    pub last_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at: i64,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSwitchAccountResponsePayload {
    pub execution_id: String,
    pub request_id: Option<String>,
    pub success: bool,
    pub effective_mode: String,
    pub from_email: Option<String>,
    pub to_email: String,
    pub duration_ms: u64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub finished_at: String,
}

/// 已连接的客户端信息
#[derive(Debug)]
struct Client {
    _addr: SocketAddr,
}

/// WebSocket 服务状态
pub struct WsServer {
    /// 广播发送器
    tx: broadcast::Sender<String>,
    /// 已连接的客户端
    clients: Arc<RwLock<HashMap<SocketAddr, Client>>>,
}

impl WsServer {
    /// 创建新的 WebSocket 服务
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            tx,
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 广播消息给所有客户端
    pub fn broadcast(&self, message: WsMessage) {
        if let Ok(json) = serde_json::to_string(&message) {
            let _ = self.tx.send(json);
        }
    }
}

/// 全局 WebSocket 服务实例
static WS_SERVER: std::sync::OnceLock<Arc<WsServer>> = std::sync::OnceLock::new();
static PLUGIN_SWITCH_PENDING: std::sync::LazyLock<
    Mutex<HashMap<String, oneshot::Sender<PluginSwitchAccountResponsePayload>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(target_os = "windows")]
static WSL_PREFIXES16: std::sync::LazyLock<Vec<(u8, u8)>> =
    std::sync::LazyLock::new(resolve_wsl_network_prefixes16);

#[cfg(target_os = "windows")]
fn push_prefix16(prefixes: &mut Vec<(u8, u8)>, ip_str: &str) {
    if let Ok(IpAddr::V4(v4)) = ip_str.parse::<IpAddr>() {
        let octets = v4.octets();
        let prefix = (octets[0], octets[1]);
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
}

#[cfg(target_os = "windows")]
fn resolve_wsl_network_prefixes16() -> Vec<(u8, u8)> {
    let mut prefixes = Vec::new();

    let resolv_output = std::process::Command::new("wsl.exe")
        .args(["-e", "sh", "-c", "cat /etc/resolv.conf"])
        .output();
    if let Ok(output) = resolv_output {
        if output.status.success() {
            let content = String::from_utf8_lossy(&output.stdout);
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with("nameserver") {
                    continue;
                }
                if let Some(ip_str) = trimmed.split_whitespace().nth(1) {
                    push_prefix16(&mut prefixes, ip_str);
                }
            }
        }
    }

    let route_output = std::process::Command::new("wsl.exe")
        .args(["-e", "sh", "-c", "ip route show default"])
        .output();
    if let Ok(output) = route_output {
        if output.status.success() {
            let content = String::from_utf8_lossy(&output.stdout);
            for line in content.lines() {
                let mut parts = line.split_whitespace();
                while let Some(part) = parts.next() {
                    if part == "via" {
                        if let Some(ip_str) = parts.next() {
                            push_prefix16(&mut prefixes, ip_str);
                        }
                        break;
                    }
                }
            }
        }
    }

    prefixes
}

fn is_allowed_remote_client(addr: &SocketAddr) -> bool {
    let ip = addr.ip();
    if ip.is_loopback() {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        if let IpAddr::V4(peer_v4) = ip {
            let octets = peer_v4.octets();
            let peer_prefix = (octets[0], octets[1]);
            if WSL_PREFIXES16.iter().any(|prefix| *prefix == peer_prefix) {
                return true;
            }
        }
    }

    false
}

/// 获取全局 WebSocket 服务实例
pub fn get_server() -> &'static Arc<WsServer> {
    WS_SERVER.get_or_init(|| Arc::new(WsServer::new()))
}

/// 广播数据变更通知
pub fn broadcast_data_changed(source: &str) {
    let server = get_server();
    server.broadcast(WsMessage::DataChanged {
        source: source.to_string(),
    });
    crate::modules::logger::log_info(&format!("[WS] 广播数据变更: {}", source));

    // 同时发送 Tauri 事件通知前端刷新
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit("accounts:refresh", source);
    }
}

/// 广播语言变更
pub fn broadcast_language_changed(language: &str, source: &str) {
    let server = get_server();
    server.broadcast(WsMessage::LanguageChanged {
        language: language.to_string(),
        source: source.to_string(),
    });
    crate::modules::logger::log_info(&format!(
        "[WS] 广播语言变更: {} (source={})",
        language, source
    ));

    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit("settings:language_changed", language);
    }
}

/// 广播账号切换完成
pub fn broadcast_account_switched(account_id: &str, email: &str) {
    let server = get_server();
    server.broadcast(WsMessage::AccountSwitched {
        account_id: account_id.to_string(),
        email: email.to_string(),
    });
    crate::modules::logger::log_info("[WS] 广播账号切换");
}

/// 广播唤醒互斥开关
pub fn broadcast_wakeup_override(enabled: bool) {
    let server = get_server();
    server.broadcast(WsMessage::WakeupOverride { enabled });
    crate::modules::logger::log_info(&format!("[WS] 广播唤醒互斥: enabled={}", enabled));
}

pub async fn connected_client_count() -> usize {
    let server = get_server();
    let clients = server.clients.read().await;
    clients.len()
}

pub async fn wait_for_connected_clients(timeout_ms: u64) -> Result<usize, String> {
    let initial_count = connected_client_count().await;
    if initial_count > 0 {
        return Ok(initial_count);
    }

    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    while started.elapsed() < timeout {
        let count = connected_client_count().await;
        if count > 0 {
            return Ok(count);
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    Err(format!("等待扩展连接超时（{}ms）", timeout_ms))
}

pub async fn request_plugin_switch_account(
    target_email: &str,
    switch_mode: &str,
    trigger_type: &str,
    trigger_source: &str,
    reason: &str,
    timeout_ms: u64,
) -> Result<PluginSwitchAccountResponsePayload, String> {
    let email = target_email.trim();
    if email.is_empty() {
        return Err("目标账号邮箱不能为空".to_string());
    }

    let request_id = format!("plugin_switch_{}", uuid::Uuid::new_v4());
    let (tx, rx) = oneshot::channel::<PluginSwitchAccountResponsePayload>();
    {
        let mut pending = PLUGIN_SWITCH_PENDING.lock().await;
        pending.insert(request_id.clone(), tx);
    }

    let msg = WsMessage::PluginSwitchAccountEvent {
        request_id: request_id.clone(),
        target_email: email.to_string(),
        switch_mode: switch_mode.to_string(),
        trigger_type: trigger_type.to_string(),
        trigger_source: trigger_source.to_string(),
        reason: reason.to_string(),
    };

    let server = get_server();
    let json = serde_json::to_string(&msg).map_err(|e| format!("序列化无感切号请求失败: {}", e))?;
    if server.tx.send(json).is_err() {
        let mut pending = PLUGIN_SWITCH_PENDING.lock().await;
        pending.remove(&request_id);
        return Err("扩展未连接，无法执行无感切号".to_string());
    }

    let wait = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await;
    match wait {
        Ok(Ok(payload)) => Ok(payload),
        Ok(Err(_)) => Err("扩展已断开，未收到无感切号结果".to_string()),
        Err(_) => {
            let mut pending = PLUGIN_SWITCH_PENDING.lock().await;
            pending.remove(&request_id);
            Err(format!("等待扩展无感切号响应超时（{}ms）", timeout_ms))
        }
    }
}

/// 启动 WebSocket 服务（支持动态端口尝试）
pub async fn start_server() {
    // 从用户配置获取首选端口
    let preferred_port = get_preferred_port();

    // 尝试绑定端口，如果失败则尝试下一个
    let mut port = preferred_port;
    let mut listener = None;

    for attempt in 0..PORT_RANGE {
        let addr = format!("0.0.0.0:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(l) => {
                listener = Some(l);
                if attempt > 0 {
                    crate::modules::logger::log_info(&format!(
                        "[WS] 配置端口 {} 被占用，使用端口: {}",
                        preferred_port, port
                    ));
                }
                break;
            }
            Err(e) => {
                if attempt < PORT_RANGE - 1 {
                    port += 1;
                } else {
                    crate::modules::logger::log_error(&format!(
                        "[WS] 无法绑定端口 ({}-{})，最后错误: {}",
                        preferred_port,
                        preferred_port + PORT_RANGE - 1,
                        e
                    ));
                    return;
                }
            }
        }
    }

    let listener = match listener {
        Some(l) => l,
        None => return,
    };

    // 保存服务状态到共享文件（供 VS Code 扩展读取）
    if let Err(e) = init_server_status(port) {
        crate::modules::logger::log_error(&format!("[WS] 保存服务状态失败: {}", e));
    }

    crate::modules::logger::log_info(&format!(
        "[WS] WebSocket 服务已启动: ws://127.0.0.1:{}",
        port
    ));

    let server = get_server();

    while let Ok((stream, addr)) = listener.accept().await {
        if !is_allowed_remote_client(&addr) {
            crate::modules::logger::log_warn(&format!("[WS] 鎷掔粷闈炵櫧鍚嶅崟鏉ユ簮: {}", addr));
            continue;
        }
        let server_clone = Arc::clone(server);
        tokio::spawn(handle_connection(server_clone, stream, addr));
    }
}

/// 处理单个客户端连接
async fn handle_connection(server: Arc<WsServer>, stream: TcpStream, addr: SocketAddr) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            crate::modules::logger::log_error(&format!("[WS] 握手失败 {}: {}", addr, e));
            return;
        }
    };

    crate::modules::logger::log_info(&format!("[WS] 新连接: {}", addr));

    // 添加客户端
    {
        let mut clients = server.clients.write().await;
        clients.insert(addr, Client { _addr: addr });
    }

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // 发送 Ready 消息
    let ready_msg = WsMessage::Ready {
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if let Ok(json) = serde_json::to_string(&ready_msg) {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }

    // 订阅广播
    let mut broadcast_rx = server.tx.subscribe();

    loop {
        tokio::select! {
            // 接收客户端消息
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_client_message(&server, &mut ws_sender, &text).await {
                            crate::modules::logger::log_error(&format!("[WS] 处理消息失败: {}", e));
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        crate::modules::logger::log_info(&format!("[WS] 客户端断开: {}", addr));
                        break;
                    }
                    Some(Err(e)) => {
                        crate::modules::logger::log_error(&format!("[WS] 接收错误 {}: {}", addr, e));
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
            // 发送广播消息
            msg = broadcast_rx.recv() => {
                if let Ok(json) = msg {
                    if ws_sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    // 移除客户端
    {
        let mut clients = server.clients.write().await;
        clients.remove(&addr);
    }

    crate::modules::logger::log_info(&format!("[WS] 连接关闭: {}", addr));
}

/// 处理客户端消息
async fn handle_client_message(
    server: &WsServer,
    sender: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    text: &str,
) -> Result<(), String> {
    let msg: WsMessage = serde_json::from_str(text).map_err(|e| format!("解析消息失败: {}", e))?;

    match msg {
        WsMessage::Ping => {
            let pong = serde_json::to_string(&WsMessage::Pong).unwrap();
            sender
                .send(Message::Text(pong.into()))
                .await
                .map_err(|e| format!("发送 Pong 失败: {}", e))?;
        }

        WsMessage::GetAccounts { request_id } => {
            crate::modules::logger::log_info("[WS] 收到获取账号列表请求");

            let response = match get_accounts_info() {
                Ok((accounts, current_id)) => WsMessage::AccountsResponse {
                    request_id,
                    accounts,
                    current_account_id: current_id,
                },
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::GetAccountsWithTokens { request_id } => {
            crate::modules::logger::log_info("[WS] 收到获取账号列表(含Token)请求");

            let response = match get_accounts_with_tokens_info() {
                Ok((accounts, current_id)) => WsMessage::AccountsWithTokensResponse {
                    request_id,
                    accounts,
                    current_account_id: current_id,
                },
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::GetCurrentAccount { request_id } => {
            crate::modules::logger::log_info("[WS] 收到获取当前账号请求");

            let response = match get_current_account_info() {
                Ok(account) => WsMessage::CurrentAccountResponse {
                    request_id,
                    account,
                },
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::SwitchAccount {
            account_id,
            request_id,
        } => {
            crate::modules::logger::log_info("[WS] 收到切换请求");

            // 异步执行切换
            let server_clone = server.tx.clone();
            tokio::spawn(async move {
                let dual_no_restart_enabled = crate::modules::config::get_user_config()
                    .antigravity_dual_switch_no_restart_enabled;
                let switch_result = if dual_no_restart_enabled {
                    crate::modules::account::switch_account_dual_no_restart(
                        &account_id,
                        "manual",
                        "tools.ws.request_switch_account",
                        "ws_request_switch_account",
                        None,
                    )
                    .await
                } else {
                    crate::modules::account::switch_account_internal(&account_id).await
                };

                match switch_result {
                    Ok(account) => {
                        // 无感双通道链路内已广播 account_switched，这里避免重复广播。
                        if !dual_no_restart_enabled {
                            let msg = WsMessage::AccountSwitched {
                                account_id: account.id,
                                email: account.email,
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = server_clone.send(json);
                            }
                        }
                        // 通知 Tools 前端刷新当前账号与账号列表，避免插件端切换后 UI 仍显示旧标识。
                        broadcast_data_changed("ws_switch_account");
                        if let Some(request_id) = request_id.clone() {
                            let response = WsMessage::SuccessResponse {
                                request_id,
                                message: "切换账号成功".to_string(),
                            };
                            if let Ok(json) = serde_json::to_string(&response) {
                                let _ = server_clone.send(json);
                            }
                        }
                    }
                    Err(e) => {
                        let error_message = e;
                        let msg = WsMessage::SwitchError {
                            message: error_message.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = server_clone.send(json);
                        }
                        if let Some(request_id) = request_id {
                            let response = WsMessage::ErrorResponse {
                                request_id,
                                error: error_message,
                            };
                            if let Ok(json) = serde_json::to_string(&response) {
                                let _ = server_clone.send(json);
                            }
                        }
                    }
                }
            });
        }

        WsMessage::SetLanguage {
            request_id,
            language,
            source,
        } => {
            crate::modules::logger::log_info(&format!("[WS] 收到语言设置请求: {}", language));

            let response = match handle_set_language(&language, source.as_deref()) {
                Ok(msg) => WsMessage::SuccessResponse {
                    request_id,
                    message: msg,
                },
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::AddAccount {
            request_id,
            email,
            refresh_token,
            access_token,
            expires_at,
        } => {
            crate::modules::logger::log_info("[WS] 收到添加账号请求");

            let response = match handle_add_account(
                &email,
                &refresh_token,
                access_token.as_deref(),
                expires_at,
            ) {
                Ok(msg) => {
                    // 广播数据变更（同时发送 Tauri 事件通知前端）
                    broadcast_data_changed("extension_add_account");
                    WsMessage::SuccessResponse {
                        request_id,
                        message: msg,
                    }
                }
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::DeleteAccountByEmail { request_id, email } => {
            crate::modules::logger::log_info("[WS] 收到删除账号请求");

            let response = match handle_delete_account_by_email(&email) {
                Ok(msg) => {
                    // 广播数据变更（同时发送 Tauri 事件通知前端）
                    broadcast_data_changed("extension_delete_account");
                    WsMessage::SuccessResponse {
                        request_id,
                        message: msg,
                    }
                }
                Err(e) => WsMessage::ErrorResponse {
                    request_id,
                    error: e,
                },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                sender
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| format!("发送响应失败: {}", e))?;
            }
        }

        WsMessage::NotifyDataChanged { source } => {
            crate::modules::logger::log_info(&format!("[WS] 收到数据变更通知: {}", source));
            // 广播给其他客户端
            server.broadcast(WsMessage::DataChanged { source });
        }

        WsMessage::PluginSwitchAccountResponse {
            execution_id,
            request_id,
            success,
            effective_mode,
            from_email,
            to_email,
            duration_ms,
            error_code,
            error_message,
            finished_at,
        } => {
            let response_payload = PluginSwitchAccountResponsePayload {
                execution_id,
                request_id: request_id.clone(),
                success,
                effective_mode,
                from_email,
                to_email,
                duration_ms,
                error_code,
                error_message,
                finished_at,
            };

            let Some(req_id) = request_id else {
                crate::modules::logger::log_warn("[WS] 收到无感切号响应但缺少 request_id");
                return Ok(());
            };

            let sender = {
                let mut pending = PLUGIN_SWITCH_PENDING.lock().await;
                pending.remove(&req_id)
            };
            if let Some(pending_tx) = sender {
                let _ = pending_tx.send(response_payload);
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[WS] 收到无匹配请求的无感切号响应: request_id={}",
                    req_id
                ));
            }
        }

        _ => {}
    }

    Ok(())
}

/// 获取账号列表信息
fn get_accounts_info() -> Result<(Vec<AccountInfo>, Option<String>), String> {
    use crate::modules::account;

    let accounts = account::list_accounts()?;
    let current_id = account::get_current_account_id()?;

    let account_infos: Vec<AccountInfo> = accounts
        .iter()
        .map(|acc| {
            let subscription_tier = acc
                .quota
                .as_ref()
                .and_then(|quota| quota.subscription_tier.clone());
            AccountInfo {
                id: acc.id.clone(),
                email: acc.email.clone(),
                name: acc.name.clone(),
                is_current: current_id.as_ref() == Some(&acc.id),
                disabled: acc.disabled,
                has_fingerprint: acc.fingerprint_id.is_some(),
                last_used: acc.last_used,
                subscription_tier,
            }
        })
        .collect();

    Ok((account_infos, current_id))
}

/// 获取账号列表信息（包含 Token）
fn get_accounts_with_tokens_info() -> Result<(Vec<AccountTokenInfo>, Option<String>), String> {
    use crate::modules::account;

    let accounts = account::list_accounts()?;
    let current_id = account::get_current_account_id()?;

    let account_infos: Vec<AccountTokenInfo> = accounts
        .iter()
        .map(|acc| {
            let subscription_tier = acc
                .quota
                .as_ref()
                .and_then(|quota| quota.subscription_tier.clone());
            AccountTokenInfo {
                id: acc.id.clone(),
                email: acc.email.clone(),
                name: acc.name.clone(),
                is_current: current_id.as_ref() == Some(&acc.id),
                disabled: acc.disabled,
                has_fingerprint: acc.fingerprint_id.is_some(),
                last_used: acc.last_used,
                subscription_tier,
                refresh_token: acc.token.refresh_token.clone(),
                access_token: acc.token.access_token.clone(),
                expires_at: acc.token.expiry_timestamp,
                project_id: acc.token.project_id.clone(),
            }
        })
        .collect();

    Ok((account_infos, current_id))
}

/// 获取当前账号信息
fn get_current_account_info() -> Result<Option<AccountInfo>, String> {
    use crate::modules::account;

    let current = account::get_current_account()?;
    let current_id = account::get_current_account_id()?;

    Ok(current.map(|acc| {
        let subscription_tier = acc
            .quota
            .as_ref()
            .and_then(|quota| quota.subscription_tier.clone());
        AccountInfo {
            id: acc.id.clone(),
            email: acc.email.clone(),
            name: acc.name.clone(),
            is_current: current_id.as_ref() == Some(&acc.id),
            disabled: acc.disabled,
            has_fingerprint: acc.fingerprint_id.is_some(),
            last_used: acc.last_used,
            subscription_tier,
        }
    }))
}

/// 处理添加账号请求
fn handle_add_account(
    email: &str,
    refresh_token: &str,
    access_token: Option<&str>,
    expires_at: Option<i64>,
) -> Result<String, String> {
    use crate::models::TokenData;
    use crate::modules::account;

    // 计算 expires_in（如果提供了 expires_at，计算距离现在的秒数）
    let expires_in = expires_at
        .map(|ts| ts - chrono::Utc::now().timestamp())
        .filter(|&secs| secs > 0)
        .unwrap_or(3600); // 默认 1 小时

    // 使用 TokenData::new 构建
    let token = TokenData::new(
        access_token.unwrap_or("").to_string(),
        refresh_token.to_string(),
        expires_in,
        Some(email.to_string()),
        None,
        None,
    );

    // 使用 upsert_account 添加或更新账号
    account::upsert_account(email.to_string(), None, token)?;

    crate::modules::logger::log_info("[WS] 账号已同步");
    Ok(format!("账号已同步: {}", email))
}

/// 处理删除账号请求（按邮箱）
fn handle_delete_account_by_email(email: &str) -> Result<String, String> {
    use crate::modules::account;

    // 查找账号 ID
    let accounts = account::list_accounts()?;
    let target = accounts.iter().find(|a| a.email == email);

    match target {
        Some(acc) => {
            account::delete_account(&acc.id)?;
            crate::modules::logger::log_info("[WS] 账号已删除");
            Ok(format!("账号已删除: {}", email))
        }
        None => {
            // 账号不存在不算错误，可能本来就没有
            crate::modules::logger::log_info("[WS] 账号不存在，无需删除");
            Ok(format!("账号不存在: {}", email))
        }
    }
}

/// 处理语言设置请求
fn handle_set_language(language: &str, source: Option<&str>) -> Result<String, String> {
    use crate::modules::config::{self, UserConfig};

    if language.trim().is_empty() {
        return Err("语言不能为空".to_string());
    }

    // 标准化语言代码为小写，确保格式一致
    let normalized = language.to_lowercase();

    let current = config::get_user_config();
    if current.language == normalized {
        return Ok(format!("语言已是 {}", normalized));
    }

    let new_config = UserConfig {
        language: normalized.clone(),
        ..current
    };

    config::save_user_config(&new_config)?;

    broadcast_language_changed(&normalized, source.unwrap_or("ws"));

    Ok(format!("语言已更新为 {}", normalized))
}
