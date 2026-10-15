// Codex Local Access：共享 WebSocket 传输与回归验证辅助。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。

fn is_websocket_upgrade_request(request: &ParsedRequest) -> bool {
    let upgrade = header_value(&request.headers, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection = header_value(&request.headers, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    upgrade && connection && header_value(&request.headers, "sec-websocket-key").is_some()
}

fn websocket_accept_value(sec_websocket_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(sec_websocket_key.trim().as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    general_purpose::STANDARD.encode(hasher.finalize())
}

async fn accept_downstream_websocket(
    mut stream: TcpStream,
    request: &ParsedRequest,
) -> Result<WebSocketStream<TcpStream>, String> {
    let sec_key = header_value(&request.headers, "sec-websocket-key")
        .ok_or_else(|| "WebSocket 握手缺少 Sec-WebSocket-Key".to_string())?;
    let accept_value = websocket_accept_value(sec_key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        accept_value
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("写入 WebSocket 握手响应失败: {}", e))?;
    Ok(WebSocketStream::from_raw_socket(stream, Role::Server, None).await)
}

async fn read_initial_websocket_payload(
    downstream: &mut WebSocketStream<TcpStream>,
    initial_message_timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + initial_message_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("等待 WebSocket 首个 response.create 消息超时".to_string());
        }
        let message = timeout(remaining, downstream.next())
            .await
            .map_err(|_| "等待 WebSocket 首个 response.create 消息超时".to_string())?
            .ok_or_else(|| "客户端在发送首个 WebSocket 消息前已断开".to_string())?
            .map_err(|e| format!("读取 WebSocket 首个消息失败: {}", e))?;

        match message {
            Message::Text(text) => return Ok(text.to_string().into_bytes()),
            Message::Binary(bytes) => return Ok(bytes.to_vec()),
            Message::Ping(bytes) => {
                downstream
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|e| format!("回复 WebSocket Ping 失败: {}", e))?;
            }
            Message::Pong(_) => {}
            Message::Close(frame) => {
                let _ = downstream.send(Message::Close(frame)).await;
                return Err("客户端在发送首个 WebSocket 消息前已关闭连接".to_string());
            }
            _ => {}
        }
    }
}

fn prepare_websocket_initial_request(
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
    default_service_tier: Option<&str>,
) -> Result<(), String> {
    let mut body_value = parse_request_body_json(&request.body)
        .ok_or_else(|| "WebSocket response.create 消息必须是合法 JSON".to_string())?;
    let request_has_service_tier = request_body_has_service_tier(&body_value);
    rewrite_request_model_alias_value(&mut body_value);
    codex_protocol::normalize_responses_body_for_codex_with_lite(
        &mut body_value,
        request_uses_responses_lite(request),
    );
    if !request_has_service_tier {
        apply_default_service_tier_if_missing(&mut body_value, default_service_tier);
    }
    let body_obj = body_value
        .as_object_mut()
        .ok_or_else(|| "WebSocket response.create 消息必须是 JSON 对象".to_string())?;
    body_obj.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    request.body = serde_json::to_vec(&body_value)
        .map_err(|e| format!("序列化 WebSocket response.create 消息失败: {}", e))?;
    request
        .headers
        .insert("content-type".to_string(), "application/json".to_string());
    align_codex_prompt_cache(request, api_key)?;
    apply_codex_official_headers(request);
    Ok(())
}

fn build_upstream_websocket_url(account: &CodexAccount, target: &str) -> Result<String, String> {
    let http_url = build_upstream_url(account, target)?;
    let mut parsed =
        Url::parse(&http_url).map_err(|e| format!("上游 WebSocket URL 无效: {}", e))?;
    let next_scheme = match parsed.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => return Err(format!("上游 WebSocket 不支持 {} 协议", other)),
    };
    parsed
        .set_scheme(next_scheme)
        .map_err(|_| "切换上游 WebSocket 协议失败".to_string())?;
    Ok(parsed.to_string())
}

fn should_skip_websocket_upstream_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "host"
            | "content-length"
            | "connection"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-protocol"
            | "sec-websocket-extensions"
            | "accept-encoding"
            | "proxy-connection"
            | "x-api-key"
            | "x-agtools-local-request-kind"
    )
}

fn websocket_header_value(value: impl Into<String>) -> Result<WsHeaderValue, String> {
    WsHeaderValue::from_str(&value.into()).map_err(|e| format!("无效 WebSocket 请求头值: {}", e))
}

fn websocket_target_host_port(request: &WsClientRequest) -> Result<(String, u16), String> {
    let uri = request.uri();
    let host = uri
        .host()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "上游 WebSocket URL 缺少 Host".to_string())?
        .to_string();
    let port = uri
        .port_u16()
        .or_else(|| match uri.scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or_else(|| "上游 WebSocket URL 缺少端口".to_string())?;
    Ok((host, port))
}

async fn tcp_connect_with_timeout(
    addr: &str,
    label: &str,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("连接 {} 超时", label))?
        .map_err(|e| format!("连接 {} 失败: {}", label, e))
}

fn decode_proxy_credential(value: &str) -> String {
    urlencoding::decode(value)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| value.to_string())
}

fn proxy_authorization_header(proxy_url: &Url) -> Option<String> {
    if proxy_url.username().is_empty() {
        return None;
    }
    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let credential = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    Some(format!("Proxy-Authorization: Basic {}\r\n", credential))
}

async fn connect_http_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket 上游代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket 上游代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket HTTP 代理", connect_timeout).await?;
    let target_addr = format!("{}:{}", target_host, target_port);
    let auth_header = proxy_authorization_header(proxy_url).unwrap_or_default();
    let request = format!(
        "CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\nProxy-Connection: Keep-Alive\r\n{auth_header}\r\n"
    );
    timeout(connect_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "发送 WebSocket 代理 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket 代理 CONNECT 请求失败: {}", e))?;

    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if response.len() > CODEX_WEBSOCKET_PROXY_CONNECT_MAX_BYTES {
            return Err("WebSocket 代理 CONNECT 响应过大".to_string());
        }
        let read = timeout(connect_timeout, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取 WebSocket 代理 CONNECT 响应超时".to_string())?
            .map_err(|e| format!("读取 WebSocket 代理 CONNECT 响应失败: {}", e))?;
        if read == 0 {
            return Err("WebSocket 代理在 CONNECT 完成前关闭连接".to_string());
        }
        response.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&response) {
            let header_text = String::from_utf8_lossy(&response[..header_end]);
            let status_line = header_text
                .lines()
                .next()
                .ok_or_else(|| "WebSocket 代理 CONNECT 响应为空".to_string())?;
            let status = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| format!("WebSocket 代理 CONNECT 响应状态无效: {}", status_line))?;
            if (200..300).contains(&status) {
                return Ok(stream);
            }
            return Err(format!("WebSocket 代理 CONNECT 失败: HTTP {}", status));
        }
    }
}

async fn socks5_read_exact(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    connect_timeout: Duration,
) -> Result<(), String> {
    timeout(connect_timeout, stream.read_exact(buffer))
        .await
        .map_err(|_| "读取 WebSocket SOCKS5 代理响应超时".to_string())?
        .map_err(|e| format!("读取 WebSocket SOCKS5 代理响应失败: {}", e))?;
    Ok(())
}

async fn connect_socks5_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket SOCKS5 代理", connect_timeout).await?;

    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let use_auth = !username.is_empty();
    let greeting: &[u8] = if use_auth {
        &[0x05, 0x02, 0x00, 0x02]
    } else {
        &[0x05, 0x01, 0x00]
    };
    timeout(connect_timeout, stream.write_all(greeting))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 握手超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 握手失败: {}", e))?;

    let mut method_response = [0u8; 2];
    socks5_read_exact(&mut stream, &mut method_response, connect_timeout).await?;
    if method_response[0] != 0x05 {
        return Err("WebSocket SOCKS5 代理响应版本无效".to_string());
    }
    if method_response[1] == 0xff {
        return Err("WebSocket SOCKS5 代理不接受当前认证方式".to_string());
    }
    if method_response[1] == 0x02 {
        let username_bytes = username.as_bytes();
        let password_bytes = password.as_bytes();
        if username_bytes.len() > u8::MAX as usize || password_bytes.len() > u8::MAX as usize {
            return Err("WebSocket SOCKS5 代理用户名或密码过长".to_string());
        }
        let mut auth_request = Vec::with_capacity(3 + username_bytes.len() + password_bytes.len());
        auth_request.push(0x01);
        auth_request.push(username_bytes.len() as u8);
        auth_request.extend_from_slice(username_bytes);
        auth_request.push(password_bytes.len() as u8);
        auth_request.extend_from_slice(password_bytes);
        timeout(connect_timeout, stream.write_all(&auth_request))
            .await
            .map_err(|_| "发送 WebSocket SOCKS5 认证超时".to_string())?
            .map_err(|e| format!("发送 WebSocket SOCKS5 认证失败: {}", e))?;
        let mut auth_response = [0u8; 2];
        socks5_read_exact(&mut stream, &mut auth_response, connect_timeout).await?;
        if auth_response != [0x01, 0x00] {
            return Err("WebSocket SOCKS5 代理认证失败".to_string());
        }
    } else if method_response[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 代理返回不支持的认证方式: {}",
            method_response[1]
        ));
    }

    let target_host_bytes = target_host.as_bytes();
    if target_host_bytes.len() > u8::MAX as usize {
        return Err("WebSocket SOCKS5 目标 Host 过长".to_string());
    }
    let mut connect_request = Vec::with_capacity(7 + target_host_bytes.len());
    connect_request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, target_host_bytes.len() as u8]);
    connect_request.extend_from_slice(target_host_bytes);
    connect_request.extend_from_slice(&target_port.to_be_bytes());
    timeout(connect_timeout, stream.write_all(&connect_request))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 CONNECT 请求失败: {}", e))?;

    let mut reply_header = [0u8; 4];
    socks5_read_exact(&mut stream, &mut reply_header, connect_timeout).await?;
    if reply_header[0] != 0x05 {
        return Err("WebSocket SOCKS5 CONNECT 响应版本无效".to_string());
    }
    if reply_header[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 CONNECT 失败，状态码 {}",
            reply_header[1]
        ));
    }
    let addr_len = match reply_header[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0u8; 1];
            socks5_read_exact(&mut stream, &mut len, connect_timeout).await?;
            len[0] as usize
        }
        0x04 => 16,
        other => return Err(format!("WebSocket SOCKS5 CONNECT 地址类型无效: {}", other)),
    };
    let mut bound_addr = vec![0u8; addr_len + 2];
    socks5_read_exact(&mut stream, &mut bound_addr, connect_timeout).await?;
    Ok(stream)
}

async fn connect_upstream_websocket_socket(
    request: &WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let (target_host, target_port) = websocket_target_host_port(request)?;
    let signature = current_upstream_http_client_signature(upstream_proxy_url, connect_timeout);
    let Some(proxy_url) = signature.proxy_url.as_deref() else {
        return tcp_connect_with_timeout(
            &format!("{}:{}", target_host, target_port),
            "Codex 上游 WebSocket",
            connect_timeout,
        )
        .await;
    };
    let proxy_url =
        Url::parse(proxy_url).map_err(|e| format!("WebSocket 上游代理地址无效: {}", e))?;
    match proxy_url.scheme() {
        "http" => {
            connect_http_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout).await
        }
        "socks5" | "socks5h" => {
            connect_socks5_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout)
                .await
        }
        "https" => {
            Err("WebSocket 上游代理暂不支持 https 代理，请改用 http 或 socks5 代理地址".to_string())
        }
        other => Err(format!("WebSocket 上游代理不支持 {} 协议", other)),
    }
}

impl WebSocketConnectError {
    fn upstream(message: String) -> Self {
        Self {
            status: None,
            message,
            category: "upstream_websocket".to_string(),
        }
    }
}

fn websocket_connect_error_from_http_response(
    status: StatusCode,
    body: String,
) -> WebSocketConnectError {
    let category = classify_upstream_error_category(status, &body)
        .unwrap_or("upstream_websocket")
        .to_string();
    let message = if body.trim().is_empty() {
        format!("Codex 上游 WebSocket 握手失败: HTTP {}", status.as_u16())
    } else {
        format!(
            "Codex 上游 WebSocket 握手失败: {}",
            summarize_upstream_error(status, &body)
        )
    };
    WebSocketConnectError {
        status: Some(status.as_u16()),
        message,
        category,
    }
}

fn websocket_connect_error_from_tungstenite(error: WsError) -> WebSocketConnectError {
    match error {
        WsError::Http(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = response
                .body()
                .as_deref()
                .map(String::from_utf8_lossy)
                .map(Cow::into_owned)
                .unwrap_or_default();
            websocket_connect_error_from_http_response(status, body)
        }
        other => {
            WebSocketConnectError::upstream(format!("连接 Codex 上游 WebSocket 失败: {}", other))
        }
    }
}

async fn connect_upstream_websocket_request(
    request: WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let socket = connect_upstream_websocket_socket(&request, upstream_proxy_url, connect_timeout)
        .await
        .map_err(WebSocketConnectError::upstream)?;
    let (upstream, _) = client_async_tls_with_config(request, socket, None, None)
        .await
        .map_err(websocket_connect_error_from_tungstenite)?;
    Ok(upstream)
}

async fn connect_upstream_websocket(
    request: &ParsedRequest,
    account: &CodexAccount,
    upstream_target: &str,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let ws_url = build_upstream_websocket_url(account, upstream_target)
        .map_err(WebSocketConnectError::upstream)?;
    let upstream_token =
        account_upstream_token(account).map_err(WebSocketConnectError::upstream)?;
    let mut upstream_request = ws_url.as_str().into_client_request().map_err(|e| {
        WebSocketConnectError::upstream(format!("创建上游 WebSocket 请求失败: {}", e))
    })?;

    let session_id = header_value(&request.headers, "session-id")
        .or_else(|| header_value(&request.headers, "session_id"));
    for (name, value) in &request.headers {
        if should_skip_websocket_upstream_header(name.as_str()) {
            continue;
        }
        if matches!(name.as_str(), "session_id" | "session-id") {
            continue;
        }
        if !account.is_api_key_auth() && matches!(name.as_str(), "user-agent" | "originator") {
            continue;
        }
        let header_name = WsHeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            WebSocketConnectError::upstream(format!("无效 WebSocket 请求头 {}: {}", name, e))
        })?;
        let header_value =
            websocket_header_value(value.clone()).map_err(WebSocketConnectError::upstream)?;
        upstream_request
            .headers_mut()
            .insert(header_name, header_value);
    }

    upstream_request.headers_mut().insert(
        "Authorization",
        websocket_header_value(format!("Bearer {}", upstream_token))
            .map_err(WebSocketConnectError::upstream)?,
    );
    if !account.is_api_key_auth() {
        upstream_request.headers_mut().insert(
            "User-Agent",
            websocket_header_value(DEFAULT_CODEX_USER_AGENT)
                .map_err(WebSocketConnectError::upstream)?,
        );
        upstream_request.headers_mut().insert(
            "Originator",
            websocket_header_value(DEFAULT_CODEX_ORIGINATOR)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    if let Some(session_id) = session_id {
        upstream_request.headers_mut().insert(
            "Session-Id",
            websocket_header_value(session_id).map_err(WebSocketConnectError::upstream)?,
        );
    }
    if !account.is_api_key_auth() {
        if let Some(account_id) = resolve_upstream_account_id(account) {
            upstream_request.headers_mut().insert(
                "ChatGPT-Account-Id",
                websocket_header_value(account_id).map_err(WebSocketConnectError::upstream)?,
            );
        }
    }
    let beta_header = header_value(&request.headers, "openai-beta").unwrap_or_default();
    if !beta_header.contains("responses_websockets=") {
        upstream_request.headers_mut().insert(
            "OpenAI-Beta",
            websocket_header_value(CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    connect_upstream_websocket_request(upstream_request, upstream_proxy_url, connect_timeout).await
}


fn websocket_capture_from_message(message: &Message, capture: &mut ResponseCapture) {
    let parsed = match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    };
    let Some(value) = parsed else {
        return;
    };
    if let Some(usage) = extract_usage_capture(&value) {
        capture.usage = Some(usage);
    }
    if capture.response_id.is_none() {
        capture.response_id = extract_response_id(&value);
    }
    if capture.response_model.is_none() {
        capture.response_model = extract_response_model(&value);
    }
}

fn websocket_message_value(message: &Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    }
}

fn websocket_error_status(value: &Value) -> Option<u16> {
    for key in ["status", "status_code"] {
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .and_then(|status| status.parse::<u16>().ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
    }

    None
}

fn build_websocket_error_body(value: &Value, status: u16) -> Value {
    let mut out = Map::new();
    out.insert("status".to_string(), json!(status));

    if let Some(body) = value.get("body") {
        out.insert("body".to_string(), body.clone());
        if let Some(error) = body.get("error") {
            out.insert("error".to_string(), error.clone());
            return Value::Object(out);
        }
    }

    if let Some(error) = value.get("error") {
        out.insert("error".to_string(), error.clone());
        return Value::Object(out);
    }

    out.insert(
        "error".to_string(),
        json!({
            "type": "server_error",
            "message": format!("HTTP {}", status),
        }),
    );
    Value::Object(out)
}

fn retry_after_duration_from_value(value: &Value) -> Option<Duration> {
    if let Some(seconds) = value.as_u64() {
        return Some(Duration::from_secs(seconds));
    }
    value
        .as_str()
        .map(str::trim)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn parse_websocket_retry_after_header(value: &Value) -> Option<Duration> {
    let headers = value.get("headers")?.as_object()?;
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("retry-after") {
            retry_after_duration_from_value(value)
        } else {
            None
        }
    })
}

fn websocket_error_matches(value: &Value, expected: &str) -> bool {
    for path in [
        &["error", "code"][..],
        &["error", "type"][..],
        &["body", "error", "code"][..],
        &["body", "error", "type"][..],
        &["code"][..],
        &["error"][..],
    ] {
        if extract_body_string_path(value, path).as_deref() == Some(expected) {
            return true;
        }
    }
    false
}

fn parse_websocket_upstream_error(message: &Message) -> Option<WebSocketUpstreamError> {
    let value = websocket_message_value(message)?;
    if value.get("type").and_then(Value::as_str).map(str::trim) != Some("error") {
        return None;
    }

    let status = websocket_error_status(&value)?;
    let body_value = build_websocket_error_body(&value, status);
    let body = serde_json::to_string(&body_value).unwrap_or_else(|_| value.to_string());
    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let usage_retry_after = parse_codex_retry_after(status_code, &body);
    let is_connection_limit = websocket_error_matches(&value, "websocket_connection_limit_reached");
    let category = if is_connection_limit {
        "websocket_connection_limit_reached"
    } else if usage_retry_after.is_some() || websocket_error_matches(&value, "usage_limit_reached")
    {
        "usage_limit_reached"
    } else {
        classify_upstream_error_category(status_code, &body).unwrap_or("upstream_websocket_error")
    }
    .to_string();
    let retry_after = usage_retry_after
        .or_else(|| parse_websocket_retry_after_header(&value))
        .or_else(|| is_connection_limit.then_some(Duration::ZERO));

    Some(WebSocketUpstreamError {
        status,
        body,
        category,
        retry_after,
    })
}

#[derive(Clone)]
struct WebSocketImageGenerationFilter {
    account: CodexAccount,
    fallback_mode: CodexLocalAccessImageGenerationMode,
    request_headers: HashMap<String, String>,
    responses_lite: bool,
}

async fn current_websocket_image_generation_mode(
    filter: &WebSocketImageGenerationFilter,
) -> CodexLocalAccessImageGenerationMode {
    let collection_mode = gateway_runtime()
        .lock()
        .await
        .collection
        .as_ref()
        .map(|collection| collection.image_generation_mode)
        .unwrap_or(filter.fallback_mode);
    request_image_generation_mode(collection_mode, &filter.request_headers)
}

fn filter_websocket_client_message(
    message: Message,
    account: &CodexAccount,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    responses_lite: bool,
) -> Result<Message, String> {
    fn filter_payload(
        body: &[u8],
        account: &CodexAccount,
        image_generation_mode: CodexLocalAccessImageGenerationMode,
        responses_lite: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut body_value = parse_request_body_json(body);
        let message_uses_responses_lite = responses_lite
            || body_value
                .as_ref()
                .and_then(|value| {
                    value
                        .get("model")
                        .or_else(|| value.pointer("/response/model"))
                })
                .and_then(Value::as_str)
                .is_some_and(codex_protocol::codex_model_uses_responses_lite);
        let lite_filtered = if message_uses_responses_lite {
            match body_value.as_mut() {
                Some(body_value) => {
                    if codex_protocol::filter_responses_lite_tools(body_value) {
                        Some(serde_json::to_vec(body_value).map_err(|error| {
                            format!(
                                "序列化 WebSocket Responses Lite 工具过滤结果失败: {}",
                                error
                            )
                        })?)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let source = lite_filtered.as_deref().unwrap_or(body);
        let effective_image_generation_mode = if message_uses_responses_lite
            && image_generation_mode == CodexLocalAccessImageGenerationMode::Enabled
        {
            CodexLocalAccessImageGenerationMode::ImagesOnly
        } else {
            image_generation_mode
        };
        let account_filtered = build_account_scoped_upstream_body(
            "/responses",
            source,
            account,
            effective_image_generation_mode,
            CodexLocalAccessRequestKind::Text,
        )?;
        match account_filtered {
            Cow::Borrowed(_) => Ok(lite_filtered),
            Cow::Owned(filtered) => Ok(Some(filtered)),
        }
    }

    match message {
        Message::Text(text) => {
            let body = text.to_string().into_bytes();
            let Some(filtered) =
                filter_payload(&body, account, image_generation_mode, responses_lite)?
            else {
                return Ok(Message::Text(text));
            };
            let filtered = String::from_utf8(filtered)
                .map_err(|error| format!("过滤 WebSocket 文本图片工具后不是 UTF-8: {}", error))?;
            Ok(Message::Text(filtered.into()))
        }
        Message::Binary(bytes) => {
            let Some(filtered) = filter_payload(
                bytes.as_ref(),
                account,
                image_generation_mode,
                responses_lite,
            )?
            else {
                return Ok(Message::Binary(bytes));
            };
            Ok(Message::Binary(filtered.into()))
        }
        other => Ok(other),
    }
}

async fn bridge_websocket_streams(
    downstream: WebSocketStream<TcpStream>,
    mut upstream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    first_payload: Vec<u8>,
    timeouts: CodexLocalAccessTimeouts,
    image_filter: Option<WebSocketImageGenerationFilter>,
) -> Result<WebSocketBridgeResult, String> {
    let first_payload = if let Some(filter) = image_filter.as_ref() {
        let mode = current_websocket_image_generation_mode(filter).await;
        build_account_scoped_upstream_body(
            "/responses",
            &first_payload,
            &filter.account,
            mode,
            CodexLocalAccessRequestKind::Text,
        )?
        .into_owned()
    } else {
        first_payload
    };
    let first_text = String::from_utf8(first_payload)
        .map_err(|e| format!("WebSocket response.create 不是合法 UTF-8: {}", e))?;
    upstream
        .send(Message::Text(first_text.into()))
        .await
        .map_err(|e| format!("发送首个 WebSocket 上游消息失败: {}", e))?;

    let (mut downstream_write, mut downstream_read) = downstream.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();
    let mut capture = ResponseCapture::default();
    let mut upstream_error = None;
    let heartbeat_interval = duration_from_millis(
        timeouts.websocket_heartbeat_interval_ms,
        CODEX_WEBSOCKET_HEARTBEAT_INTERVAL,
    );
    let idle_timeout = duration_from_millis(
        timeouts.websocket_idle_timeout_ms,
        CODEX_WEBSOCKET_IDLE_TIMEOUT,
    );
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                upstream_write
                    .send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|e| format!("发送 Codex 上游 WebSocket 心跳失败: {}", e))?;
                upstream_write
                    .flush()
                    .await
                    .map_err(|e| format!("刷新 Codex 上游 WebSocket 心跳失败: {}", e))?;
            }
            downstream_next = timeout(idle_timeout, downstream_read.next()) => {
                let downstream_next = downstream_next
                    .map_err(|_| "WebSocket 客户端空闲超时".to_string())?;
                let Some(message_result) = downstream_next else {
                    break;
                };
                let mut message = message_result
                    .map_err(|e| format!("读取 WebSocket 客户端消息失败: {}", e))?;
                if let Some(filter) = image_filter.as_ref() {
                    let mode = current_websocket_image_generation_mode(filter).await;
                    message = filter_websocket_client_message(
                        message,
                        &filter.account,
                        mode,
                        filter.responses_lite,
                    )?;
                }
                let should_close = matches!(message, Message::Close(_));
                upstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 WebSocket 客户端消息失败: {}", e))?;
                if should_close {
                    break;
                }
            }
            upstream_next = timeout(idle_timeout, upstream_read.next()) => {
                let upstream_next = upstream_next
                    .map_err(|_| "Codex 上游 WebSocket 空闲超时".to_string())?;
                let Some(message_result) = upstream_next else {
                    break;
                };
                let message = message_result
                    .map_err(|e| format!("读取 Codex 上游 WebSocket 消息失败: {}", e))?;
                websocket_capture_from_message(&message, &mut capture);
                let parsed_upstream_error = parse_websocket_upstream_error(&message);
                let should_close = matches!(message, Message::Close(_));
                downstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 Codex 上游 WebSocket 消息失败: {}", e))?;
                if let Some(error) = parsed_upstream_error {
                    upstream_error = Some(error);
                    break;
                }
                if should_close {
                    break;
                }
            }
        }
    }

    Ok(WebSocketBridgeResult {
        capture,
        upstream_error,
    })
}
