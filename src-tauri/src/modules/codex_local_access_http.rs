// Codex Local Access：HTTP framing helpers and local gateway request/response plumbing。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let header_text = String::from_utf8_lossy(header_bytes);
    for line in header_text.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|e| format!("非法 Content-Length: {}", e));
        }
    }
    Ok(0)
}

async fn read_http_request<R>(
    stream: &mut R,
    request_read_timeout: Duration,
) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;

    loop {
        let bytes_read = timeout(request_read_timeout, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取请求超时".to_string())?
            .map_err(|e| format!("读取请求失败: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("请求体过大".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&buffer) {
                content_length = parse_content_length(&buffer[..end])?;
                if content_length > MAX_HTTP_REQUEST_BYTES.saturating_sub(end) {
                    return Err("请求体过大".to_string());
                }
                header_end = Some(end);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end.saturating_add(content_length) {
                return Ok(buffer[..(end + content_length)].to_vec());
            }
        }
    }

    Err("请求不完整".to_string())
}

fn parse_http_request(raw: &[u8]) -> Result<ParsedRequest, String> {
    let Some(header_end) = find_header_end(raw) else {
        return Err("缺少 HTTP 头结束标记".to_string());
    };

    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or("请求行为空")?.trim();

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("请求行缺少 method")?.to_string();
    let target = parts.next().ok_or("请求行缺少 target")?.to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok(ParsedRequest {
        method,
        target,
        headers,
        body: raw[header_end..].to_vec(),
    })
}

fn normalize_proxy_target(target: &str) -> Result<String, String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let parsed = url::Url::parse(target).map_err(|e| format!("解析请求地址失败: {}", e))?;
        let mut next = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            next.push('?');
            next.push_str(query);
        }
        return Ok(next);
    }

    let parsed = url::Url::parse(&format!("http://localhost{}", target))
        .map_err(|e| format!("解析请求路径失败: {}", e))?;
    let mut next = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        next.push('?');
        next.push_str(query);
    }
    Ok(next)
}

fn extract_local_api_key(headers: &HashMap<String, String>) -> Option<String> {
    if let Some(value) = headers.get("authorization") {
        let trimmed = value.trim();
        if let Some(rest) = trimmed.strip_prefix("Bearer ") {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("bearer ") {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    headers
        .get("x-api-key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_local_models_request(target: &str) -> bool {
    target == "/v1/models" || target.starts_with("/v1/models?")
}

fn build_local_models_response(model_ids: &[String]) -> Value {
    let data: Vec<Value> = model_ids
        .iter()
        .cloned()
        .into_iter()
        .map(|model| {
            json!({
                "id": model,
                "object": "model",
                "created": 0,
                "owned_by": "openai",
            })
        })
        .collect();

    json!({
        "object": "list",
        "data": data,
    })
}

fn build_codex_client_models_response(model_ids: &[String]) -> Value {
    codex_protocol::build_codex_client_models_response(model_ids)
}

fn lookup_client_model_context_window(windows: &HashMap<String, i64>, slug: &str) -> Option<i64> {
    let slug = slug.trim();
    if slug.is_empty() {
        return None;
    }
    let candidates = [
        slug,
        slug.rsplit_once('/').map(|(_, tail)| tail).unwrap_or(""),
    ];
    for candidate in candidates {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        if let Some(window) = windows.get(candidate).copied().or_else(|| {
            windows.iter().find_map(|(name, value)| {
                name.trim()
                    .eq_ignore_ascii_case(candidate)
                    .then_some(*value)
            })
        }) {
            if window > 0 {
                return Some(window);
            }
        }
    }
    None
}

fn model_context_windows_for_account_ids(account_ids: &[String]) -> HashMap<String, i64> {
    let mut merged = HashMap::new();
    for account_id in account_ids {
        let Some(account) = codex_account::load_account(account_id) else {
            continue;
        };
        for (name, window) in account.api_model_context_windows {
            let key = name.trim();
            if key.is_empty() || window <= 0 {
                continue;
            }
            merged.insert(key.to_string(), window);
        }
    }
    merged
}

fn apply_explicit_context_windows_to_client_models(
    mut catalog: Value,
    windows: &HashMap<String, i64>,
) -> Value {
    if windows.is_empty() {
        return catalog;
    }
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return catalog;
    };
    for model in models {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(window) = lookup_client_model_context_window(windows, &slug) else {
            continue;
        };
        if let Some(object) = model.as_object_mut() {
            object.insert("context_window".to_string(), json!(window));
            object.insert("max_context_window".to_string(), json!(window));
        }
    }
    catalog
}

fn usage_number(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).or_else(|| {
        value
            .and_then(Value::as_i64)
            .filter(|number| *number >= 0)
            .map(|number| number as u64)
    })
}

fn non_null_child<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|item| !item.is_null())
}

fn extract_usage_capture(value: &Value) -> Option<UsageCapture> {
    let usage = non_null_child(value, "usage")
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usage"))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| item.get("response"))
                .and_then(|item| non_null_child(item, "usage"))
        })
        .or_else(|| non_null_child(value, "usageMetadata"))
        .or_else(|| non_null_child(value, "usage_metadata"))
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usageMetadata"))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usage_metadata"))
        })?;

    let input_tokens = usage_number(
        usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .or_else(|| usage.get("promptTokenCount")),
    )
    .unwrap_or(0);
    let output_tokens = usage_number(
        usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .or_else(|| usage.get("candidatesTokenCount")),
    )
    .unwrap_or(0);
    let explicit_total_tokens = usage_number(
        usage
            .get("total_tokens")
            .or_else(|| usage.get("totalTokenCount")),
    );
    let cached_tokens = usage_number(
        usage
            .get("cached_tokens")
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|item| item.get("cached_tokens"))
            })
            .or_else(|| {
                usage
                    .get("prompt_tokens_details")
                    .and_then(|item| item.get("cached_tokens"))
            })
            .or_else(|| usage.get("cachedContentTokenCount")),
    )
    .unwrap_or(0);
    let reasoning_tokens = usage_number(
        usage
            .get("reasoning_tokens")
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|item| item.get("reasoning_tokens"))
            })
            .or_else(|| {
                usage
                    .get("completion_tokens_details")
                    .and_then(|item| item.get("reasoning_tokens"))
            })
            .or_else(|| usage.get("thoughtsTokenCount")),
    )
    .unwrap_or(0);

    Some(UsageCapture {
        input_tokens,
        output_tokens,
        total_tokens: if explicit_total_tokens.unwrap_or(0) == 0 {
            input_tokens
                .saturating_add(output_tokens)
                .saturating_add(reasoning_tokens)
        } else {
            explicit_total_tokens.unwrap_or(0)
        },
        cached_tokens,
        reasoning_tokens,
        token_breakdown: None,
    })
}

fn extract_response_id(value: &Value) -> Option<String> {
    non_null_child(value, "id")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "id"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_response_model(value: &Value) -> Option<String> {
    for candidate in [
        value.get("model"),
        value.pointer("/response/model"),
        value.pointer("/data/model"),
    ] {
        if let Some(model) = candidate
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(model.to_string());
        }
    }
    None
}

fn should_treat_response_as_stream(content_type: &str, request_is_stream: bool) -> bool {
    request_is_stream
        || content_type
            .to_ascii_lowercase()
            .contains("text/event-stream")
}

fn find_sse_frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    if buffer.len() < 2 {
        return None;
    }

    for index in 0..buffer.len().saturating_sub(1) {
        if index + 3 < buffer.len() && &buffer[index..index + 4] == b"\r\n\r\n" {
            return Some((index, 4));
        }
        if &buffer[index..index + 2] == b"\n\n" {
            return Some((index, 2));
        }
    }

    None
}

impl ResponseUsageCollector {
    fn new(is_stream: bool) -> Self {
        Self {
            is_stream,
            body: Vec::new(),
            stream_buffer: Vec::new(),
            usage: None,
            response_id: None,
            response_model: None,
            terminal_error: None,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        if self.is_stream {
            self.feed_stream_chunk(chunk);
        } else {
            self.body.extend_from_slice(chunk);
        }
    }

    fn finish(mut self) -> ResponseCapture {
        if self.is_stream {
            self.process_stream_buffer(true);
            ResponseCapture {
                usage: self.usage,
                response_id: self.response_id,
                response_model: self.response_model,
                terminal_error: self.terminal_error,
            }
        } else {
            let parsed = serde_json::from_slice::<Value>(&self.body).ok();
            ResponseCapture {
                usage: parsed.as_ref().and_then(extract_usage_capture),
                response_id: parsed.as_ref().and_then(extract_response_id),
                response_model: parsed.as_ref().and_then(extract_response_model),
                terminal_error: None,
            }
        }
    }

    fn feed_stream_chunk(&mut self, chunk: &[u8]) {
        self.stream_buffer.extend_from_slice(chunk);
        self.process_stream_buffer(false);
    }

    fn process_stream_buffer(&mut self, flush_tail: bool) {
        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_stream_frame(&frame);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_stream_frame(&frame);
        }
    }

    fn process_stream_frame(&mut self, frame: &[u8]) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        if let Ok(value) = serde_json::from_str::<Value>(&payload) {
            if self.terminal_error.is_none() {
                if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &value)
                {
                    self.terminal_error = Some(format_upstream_response_failed_error(&signal));
                }
            }
            if let Some(usage) = extract_usage_capture(&value) {
                self.usage = Some(usage);
            }
            if self.response_id.is_none() {
                self.response_id = extract_response_id(&value);
            }
            if self.response_model.is_none() {
                self.response_model = extract_response_model(&value);
            }
        }
    }
}

fn resolve_upstream_target(target: &str) -> Result<String, String> {
    let trimmed = if target.starts_with("/v1") {
        target.trim_start_matches("/v1")
    } else if target.starts_with(BACKEND_CODEX_PREFIX) {
        target.trim_start_matches(BACKEND_CODEX_PREFIX)
    } else {
        return Err("仅支持 /v1 或 /backend-api/codex 路径".to_string());
    };

    if trimmed.is_empty() {
        Ok("/".to_string())
    } else if trimmed.starts_with('/') {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("/{}", trimmed))
    }
}

fn account_upstream_base_url(account: &CodexAccount) -> String {
    if !account.is_api_key_auth() {
        return UPSTREAM_CODEX_BASE_URL.to_string();
    }

    let candidate = account
        .api_base_url
        .as_deref()
        .and_then(normalize_upstream_base_url_string);

    // Prefer non-loopback account URL.
    if let Some(url) = candidate.as_ref() {
        if parse_http_url_host_port(url)
            .map(|(host, _)| !is_loopback_http_host(&host))
            .unwrap_or(true)
        {
            return url.clone();
        }
    }

    // Recover real upstream when account was polluted with a local gateway URL.
    if let Some(recovered) = lookup_codex_model_provider_base_url(
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    ) {
        if parse_http_url_host_port(&recovered)
            .map(|(host, _)| !is_loopback_http_host(&host))
            .unwrap_or(true)
        {
            return recovered;
        }
    }

    if matches!(
        account.api_provider_mode,
        CodexApiProviderMode::OpenaiBuiltin
    ) {
        return DEFAULT_OPENAI_RESPONSES_BASE_URL.to_string();
    }

    // Intentional local provider (e.g. Ollama on another port): keep as-is.
    candidate.unwrap_or_else(|| DEFAULT_OPENAI_RESPONSES_BASE_URL.to_string())
}

fn account_upstream_token(account: &CodexAccount) -> Result<String, String> {
    let token = if account.is_api_key_auth() {
        account.openai_api_key.as_deref().unwrap_or_default()
    } else {
        account.tokens.access_token.as_str()
    }
    .trim();

    if token.is_empty() {
        if account.is_api_key_auth() {
            Err("API Key 账号缺少上游 API Key".to_string())
        } else {
            Err("OAuth 账号缺少 access_token".to_string())
        }
    } else {
        Ok(token.to_string())
    }
}

fn build_upstream_url(account: &CodexAccount, target: &str) -> Result<String, String> {
    let base_url = account_upstream_base_url(account);
    Url::parse(&base_url).map_err(|e| format!("上游 Base URL 无效: {}", e))?;
    Ok(format!("{}{}", base_url.trim_end_matches('/'), target))
}

fn is_stream_request(headers: &HashMap<String, String>, body: &[u8]) -> bool {
    if let Some(accept) = headers.get("accept") {
        if accept.to_ascii_lowercase().contains("text/event-stream") {
            return true;
        }
    }

    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn resolve_upstream_account_id(account: &CodexAccount) -> Option<String> {
    account
        .account_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            codex_account::extract_chatgpt_account_id_from_access_token(
                &account.tokens.access_token,
            )
        })
}

fn extract_upstream_error_message(body: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(body).ok()?;

    if let Some(message) = parsed
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
    {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed
        .get("detail")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
    {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed.get("message").and_then(Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed.get("error").and_then(Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn summarize_upstream_error(status: StatusCode, body: &str) -> String {
    let detail = extract_upstream_error_message(body).unwrap_or_else(|| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            format!("上游接口返回状态 {}", status.as_u16())
        } else {
            trimmed.to_string()
        }
    });

    format!("{}: {}", status.as_u16(), detail)
}

fn is_image_generation_capability_error(status: StatusCode, body: &str) -> bool {
    if !matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("image generation is not enabled")
        || lower.contains("image_generation is not enabled")
        || (lower.contains("image_generation") && lower.contains("not enabled"))
}

fn friendly_image_generation_capability_error(account_email: &str) -> String {
    let account_email = account_email.trim();
    if account_email.is_empty() {
        return "当前上游账号未启用图片生成能力。请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”，或换用具备图片能力的账号。".to_string();
    }
    format!(
        "账号 {} 未启用图片生成能力。请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”，或换用具备图片能力的账号。",
        account_email
    )
}

fn classify_upstream_error_category(status: StatusCode, body: &str) -> Option<&'static str> {
    if is_image_generation_capability_error(status, body) {
        return Some("image_generation_not_enabled");
    }
    if status == StatusCode::UNAUTHORIZED {
        return Some("auth_unavailable");
    }
    if parse_codex_retry_after(status, body).is_some() {
        return Some("usage_limit_reached");
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("context length")
        || lower.contains("context_length")
        || lower.contains("context_too_large")
        || lower.contains("too many tokens")
    {
        return Some("context_too_large");
    }
    if lower.contains("selected model is at capacity") || lower.contains("model is at capacity") {
        return Some("model_capacity");
    }
    None
}

fn should_try_next_account(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::UNAUTHORIZED {
        return true;
    }
    if is_image_generation_capability_error(status, body) {
        return true;
    }
    if matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) {
        return true;
    }

    let lower = body.to_ascii_lowercase();
    let quota_exhausted = lower.contains("usage_limit_reached")
        || lower.contains("limit reached")
        || lower.contains("insufficient_quota")
        || lower.contains("quota exceeded")
        || lower.contains("quota exceeded");
    let model_capacity =
        lower.contains("selected model is at capacity") || lower.contains("model is at capacity");

    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN
    ) && (quota_exhausted || model_capacity)
}

fn json_response(status: u16, status_text: &str, body: &Value) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        status,
        status_text,
        body_bytes.len(),
        CORS_ALLOW_HEADERS
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body_bytes);
    response
}

fn gateway_error_code(status: u16) -> &'static str {
    match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        429 => "rate_limited",
        502 => "upstream_unavailable",
        503 => "service_unavailable",
        _ => "codex_local_access_error",
    }
}

fn gateway_proxy_diagnostics_message(diagnostics: &UpstreamProxyDiagnostics) -> String {
    match diagnostics.proxy_source {
        UpstreamProxySource::ApiService => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => format!("当前使用 API 代理地址：{}。", proxy_url),
            None => "当前 API 代理地址为空。".to_string(),
        },
        UpstreamProxySource::Global => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => format!("当前 API 代理地址为空，已跟随全局代理：{}。", proxy_url),
            None => "当前 API 代理地址为空，已尝试跟随全局代理。".to_string(),
        },
        UpstreamProxySource::SystemEnv => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => {
                format!(
                    "当前 API 代理地址为空，且全局代理未启用或未配置，已使用环境代理：{}。",
                    proxy_url
                )
            }
            None => {
                "当前 API 代理地址为空，且全局代理未启用或未配置，已尝试使用环境代理。".to_string()
            }
        },
        UpstreamProxySource::SystemAuto => {
            "当前 API 代理地址为空，且全局代理与环境代理均未配置，已回退到系统自动代理配置；如仍失败，请在 API 代理地址中填写 Clash 的 HTTP/mixed 端口。".to_string()
        }
    }
}

fn upstream_proxy_source_code(source: UpstreamProxySource) -> &'static str {
    match source {
        UpstreamProxySource::ApiService => "api_service",
        UpstreamProxySource::Global => "global",
        UpstreamProxySource::SystemEnv => "system_env",
        UpstreamProxySource::SystemAuto => "system_auto",
    }
}

fn gateway_user_visible_error_message(
    status: u16,
    message: &str,
    proxy_diagnostics: Option<&UpstreamProxyDiagnostics>,
) -> String {
    if status != StatusCode::BAD_GATEWAY.as_u16() {
        return message.to_string();
    }

    let proxy_context = proxy_diagnostics
        .map(|diagnostics| format!(" {}", gateway_proxy_diagnostics_message(diagnostics)))
        .unwrap_or_default();
    format!(
        "Codex API 服务连接上游失败。API 代理地址留空时会依次使用全局代理、环境代理、系统自动代理；如需固定出口，建议填写 API 代理地址（例如 http://127.0.0.1:7890）后重试。{} 如果 Codex 客户端仍显示 502 且 API 服务没有请求记录，请检查代理工具是否拦截或屏蔽 localhost / 127.0.0.1。原始错误：{}",
        proxy_context, message
    )
}

fn gateway_error_body(
    status: u16,
    message: &str,
    proxy_diagnostics: Option<&UpstreamProxyDiagnostics>,
) -> Value {
    let mut error = Map::new();
    error.insert(
        "message".to_string(),
        Value::String(gateway_user_visible_error_message(
            status,
            message,
            proxy_diagnostics,
        )),
    );
    error.insert(
        "type".to_string(),
        Value::String("codex_local_access_error".to_string()),
    );
    let error_code = if status == StatusCode::TOO_MANY_REQUESTS.as_u16()
        && message.starts_with("API key token limit exceeded")
    {
        "token_limit_exceeded"
    } else {
        gateway_error_code(status)
    };
    error.insert("code".to_string(), Value::String(error_code.to_string()));
    error.insert("status".to_string(), json!(status));

    if let Some(diagnostics) = proxy_diagnostics {
        error.insert(
            "upstreamProxy".to_string(),
            json!({
                "source": upstream_proxy_source_code(diagnostics.proxy_source),
                "proxyUrl": diagnostics.proxy_url.clone(),
            }),
        );
    }

    let mut body = Map::new();
    body.insert("error".to_string(), Value::Object(error));
    Value::Object(body)
}

fn options_response() -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 0\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        CORS_ALLOW_HEADERS
    );
    headers.into_bytes()
}

fn log_field_or_dash(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
}

fn escape_failure_detail(detail: &str) -> String {
    detail.replace('\r', "\\r").replace('\n', "\\n")
}

fn log_codex_api_failure(
    addr: Option<&std::net::SocketAddr>,
    request: Option<&ParsedRequest>,
    status: Option<u16>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    latency_ms: Option<u64>,
    detail: &str,
) {
    let addr_text = addr
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let status_text = status
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let latency_text = latency_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let method = request.map(|value| value.method.as_str()).unwrap_or("-");
    let target = request.map(|value| value.target.as_str()).unwrap_or("-");

    logger::log_codex_api_warn(&format!(
        "[CodexLocalAccess][Failure] addr={} method={} target={} status={} account_id={} account_email={} latency_ms={} detail={}",
        addr_text,
        method,
        target,
        status_text,
        log_field_or_dash(account_id),
        log_field_or_dash(account_email),
        latency_text,
        escape_failure_detail(detail),
    ));
}

async fn write_json_error_response(
    stream: &mut TcpStream,
    addr: Option<&std::net::SocketAddr>,
    request: Option<&ParsedRequest>,
    status: u16,
    status_text: &str,
    message: &str,
    account_id: Option<&str>,
    account_email: Option<&str>,
    latency_ms: Option<u64>,
) -> Result<(), String> {
    log_codex_api_failure(
        addr,
        request,
        Some(status),
        account_id,
        account_email,
        latency_ms,
        message,
    );

    let response = json_response(
        status,
        status_text,
        &gateway_error_body(status, message, None),
    );
    stream
        .write_all(&response)
        .await
        .map_err(|e| format!("写入错误响应失败: {}", e))
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        status,
        status_text,
        content_type,
        body.len(),
        CORS_ALLOW_HEADERS
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|e| format!("写入响应头失败: {}", e))?;
    stream
        .write_all(body)
        .await
        .map_err(|e| format!("写入响应体失败: {}", e))?;
    Ok(())
}

async fn write_chunked_response_headers(
    stream: &mut TcpStream,
    status: StatusCode,
    status_text: &str,
    content_type: &str,
    upstream_headers: &reqwest::header::HeaderMap,
) -> Result<(), String> {
    let mut response_headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n",
        status.as_u16(),
        status_text,
        content_type,
        CORS_ALLOW_HEADERS
    );

    for header_name in ["x-request-id", "openai-processing-ms"] {
        if let Some(value) = upstream_headers
            .get(header_name)
            .and_then(|item| item.to_str().ok())
        {
            response_headers.push_str(&format!("{}: {}\r\n", header_name, value));
        }
    }

    response_headers.push_str("\r\n");
    stream
        .write_all(response_headers.as_bytes())
        .await
        .map_err(|e| format!("写入响应头失败: {}", e))
}

async fn write_chunked_response_chunk(stream: &mut TcpStream, chunk: &[u8]) -> Result<(), String> {
    if chunk.is_empty() {
        return Ok(());
    }

    let prefix = format!("{:X}\r\n", chunk.len());
    stream
        .write_all(prefix.as_bytes())
        .await
        .map_err(|e| format!("写入响应分块前缀失败: {}", e))?;
    stream
        .write_all(chunk)
        .await
        .map_err(|e| format!("写入响应分块失败: {}", e))?;
    stream
        .write_all(b"\r\n")
        .await
        .map_err(|e| format!("写入响应分块结束失败: {}", e))
}

async fn finish_chunked_response(stream: &mut TcpStream) -> Result<(), String> {
    stream
        .write_all(b"0\r\n\r\n")
        .await
        .map_err(|e| format!("写入响应结束失败: {}", e))
}

fn parse_responses_payload_from_upstream(body_bytes: &[u8]) -> Result<Value, String> {
    if let Ok(parsed) = serde_json::from_slice::<Value>(body_bytes) {
        if let Some(signal) = upstream_response_failed_signal(None, &parsed) {
            return Err(format_upstream_response_failed_error(&signal));
        }
        return Ok(parsed);
    }

    let mut stream_buffer = body_bytes.to_vec();
    let mut completed_response: Option<Value> = None;
    let mut output_text = String::new();
    let mut output_items: Vec<Value> = Vec::new();

    let mut process_frame = |frame: &[u8]| -> Result<(), String> {
        if frame.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(());
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };
        if payload == "[DONE]" {
            return Ok(());
        }

        let Ok(value) = serde_json::from_str::<Value>(&payload) else {
            return Ok(());
        };
        if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &value) {
            return Err(format_upstream_response_failed_error(&signal));
        }
        match value
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("")
        {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    output_text.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if output_text.trim().is_empty() {
                    if let Some(done_text) = value.get("text").and_then(Value::as_str) {
                        output_text.push_str(done_text);
                    }
                }
            }
            "response.output_item.done" => {
                if let Some(item) = value.get("item") {
                    output_items.push(item.clone());
                }
            }
            event_type if is_responses_completion_event(event_type) => {
                if let Some(response) = value.get("response") {
                    completed_response = Some(response.clone());
                } else {
                    completed_response = Some(value.clone());
                }
            }
            _ => {}
        }
        Ok(())
    };

    loop {
        let Some((boundary_index, separator_len)) = find_sse_frame_boundary(&stream_buffer) else {
            break;
        };
        let frame = stream_buffer[..boundary_index].to_vec();
        stream_buffer.drain(..boundary_index + separator_len);
        process_frame(&frame)?;
    }
    if !stream_buffer.is_empty() {
        process_frame(&stream_buffer)?;
    }

    let Some(response_value) = completed_response else {
        return Err(
            "解析上游 responses 响应失败: 非 JSON 且未捕获 response.completed/response.done"
                .to_string(),
        );
    };

    let mut root = Map::new();
    match response_value {
        Value::Object(mut response_object) => {
            if response_object
                .get("output")
                .and_then(Value::as_array)
                .map(|items| items.is_empty())
                .unwrap_or(true)
                && !output_items.is_empty()
            {
                response_object.insert("output".to_string(), Value::Array(output_items));
            }
            if !output_text.trim().is_empty() {
                response_object.insert("output_text".to_string(), Value::String(output_text));
            }
            root.insert("response".to_string(), Value::Object(response_object));
        }
        other => {
            root.insert("response".to_string(), other);
            if !output_items.is_empty() {
                root.insert("output".to_string(), Value::Array(output_items));
            }
            if !output_text.trim().is_empty() {
                root.insert("output_text".to_string(), Value::String(output_text));
            }
        }
    }

    Ok(Value::Object(root))
}

fn mime_type_from_output_format(output_format: &str) -> String {
    let output_format = output_format.trim();
    if output_format.contains('/') {
        return output_format.to_string();
    }
    match output_format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "webp" => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

fn extract_images_from_responses_payload(
    response_body: &Value,
) -> (
    Vec<ImageCallResult>,
    i64,
    Option<Value>,
    Option<ImageCallResult>,
) {
    let root = response_payload_root(response_body);
    let created = root
        .get("created_at")
        .or_else(|| root.get("created"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let mut results = Vec::new();
    let mut first_meta = None;

    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
                continue;
            }
            let result = item
                .get("result")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let Some(result) = result else {
                continue;
            };
            let entry = ImageCallResult {
                result: result.to_string(),
                revised_prompt: item
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                output_format: item
                    .get("output_format")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                size: item
                    .get("size")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                background: item
                    .get("background")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                quality: item
                    .get("quality")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            };
            if first_meta.is_none() {
                first_meta = Some(entry.clone());
            }
            results.push(entry);
        }
    }

    let usage = root
        .get("tool_usage")
        .and_then(|tool_usage| tool_usage.get("image_gen"))
        .filter(|value| value.is_object())
        .cloned();

    (results, created, usage, first_meta)
}

fn build_images_api_payload(response_body: &Value, response_format: &str) -> Result<Value, String> {
    let (results, created, usage, first_meta) =
        extract_images_from_responses_payload(response_body);
    if results.is_empty() {
        return Err("upstream did not return image output".to_string());
    }

    let response_format = if response_format.trim().is_empty() {
        "b64_json"
    } else {
        response_format.trim()
    };
    let mut data = Vec::new();
    for image in results {
        let mut item = Map::new();
        if response_format.eq_ignore_ascii_case("url") {
            let mime_type = mime_type_from_output_format(&image.output_format);
            item.insert(
                "url".to_string(),
                Value::String(format!("data:{};base64,{}", mime_type, image.result)),
            );
        } else {
            item.insert("b64_json".to_string(), Value::String(image.result));
        }
        if !image.revised_prompt.is_empty() {
            item.insert(
                "revised_prompt".to_string(),
                Value::String(image.revised_prompt),
            );
        }
        data.push(Value::Object(item));
    }

    let mut out = Map::new();
    out.insert("created".to_string(), json!(created));
    out.insert("data".to_string(), Value::Array(data));

    if let Some(meta) = first_meta {
        if !meta.background.is_empty() {
            out.insert("background".to_string(), Value::String(meta.background));
        }
        if !meta.output_format.is_empty() {
            out.insert(
                "output_format".to_string(),
                Value::String(meta.output_format),
            );
        }
        if !meta.quality.is_empty() {
            out.insert("quality".to_string(), Value::String(meta.quality));
        }
        if !meta.size.is_empty() {
            out.insert("size".to_string(), Value::String(meta.size));
        }
    }
    if let Some(usage) = usage {
        out.insert("usage".to_string(), usage);
    }

    Ok(Value::Object(out))
}

fn push_named_sse_payload(stream_body: &mut String, event_name: &str, payload: Value) {
    let event_name = event_name.trim();
    if !event_name.is_empty() {
        stream_body.push_str("event: ");
        stream_body.push_str(event_name);
        stream_body.push('\n');
    }
    push_sse_payload(stream_body, payload);
}

#[derive(Debug)]
struct ImageStreamTransformer {
    response_format: String,
    stream_prefix: String,
    stream_buffer: Vec<u8>,
    response_capture: ResponseCapture,
}

impl ImageStreamTransformer {
    fn new(response_format: &str, stream_prefix: &str) -> Self {
        Self {
            response_format: if response_format.trim().is_empty() {
                "b64_json".to_string()
            } else {
                response_format.trim().to_ascii_lowercase()
            },
            stream_prefix: stream_prefix.to_string(),
            stream_buffer: Vec::new(),
            response_capture: ResponseCapture::default(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }
        self.stream_buffer.extend_from_slice(chunk);
        self.process_buffer(false)
    }

    fn finish(mut self) -> (Vec<u8>, ResponseCapture) {
        let output = self.process_buffer(true);
        (output, self.response_capture)
    }

    fn process_buffer(&mut self, flush_tail: bool) -> Vec<u8> {
        let mut stream_body = String::new();

        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_frame(&frame, &mut stream_body);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_frame(&frame, &mut stream_body);
        }

        stream_body.into_bytes()
    }

    fn process_frame(&mut self, frame: &[u8], stream_body: &mut String) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        let Ok(event) = serde_json::from_str::<Value>(&payload) else {
            return;
        };
        if self.response_capture.terminal_error.is_none() {
            if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &event) {
                self.response_capture.terminal_error =
                    Some(format_upstream_response_failed_error(&signal));
            }
        }
        if let Some(usage) = extract_usage_capture(&event) {
            self.response_capture.usage = Some(usage);
        }
        if self.response_capture.response_id.is_none() {
            self.response_capture.response_id = extract_response_id(&event);
        }
        if self.response_capture.response_model.is_none() {
            self.response_capture.response_model = extract_response_model(&event);
        }

        match event
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("")
        {
            "response.image_generation_call.partial_image" => {
                let Some(b64) = event
                    .get("partial_image_b64")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return;
                };
                let output_format = event
                    .get("output_format")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let event_name = format!("{}.partial_image", self.stream_prefix);
                let mut data = Map::new();
                data.insert("type".to_string(), Value::String(event_name.clone()));
                data.insert(
                    "partial_image_index".to_string(),
                    json!(event
                        .get("partial_image_index")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)),
                );
                if self.response_format == "url" {
                    let mime_type = mime_type_from_output_format(output_format);
                    data.insert(
                        "url".to_string(),
                        Value::String(format!("data:{};base64,{}", mime_type, b64)),
                    );
                } else {
                    data.insert("b64_json".to_string(), Value::String(b64.to_string()));
                }
                push_named_sse_payload(stream_body, &event_name, Value::Object(data));
            }
            event_type if is_responses_completion_event(event_type) => {
                let (results, _, usage, _) = extract_images_from_responses_payload(&event);
                if results.is_empty() {
                    push_named_sse_payload(
                        stream_body,
                        "error",
                        json!({ "error": "upstream did not return image output" }),
                    );
                    return;
                }
                let event_name = format!("{}.completed", self.stream_prefix);
                for image in results {
                    let mut data = Map::new();
                    data.insert("type".to_string(), Value::String(event_name.clone()));
                    if self.response_format == "url" {
                        let mime_type = mime_type_from_output_format(&image.output_format);
                        data.insert(
                            "url".to_string(),
                            Value::String(format!("data:{};base64,{}", mime_type, image.result)),
                        );
                    } else {
                        data.insert("b64_json".to_string(), Value::String(image.result));
                    }
                    if let Some(usage) = usage.clone() {
                        data.insert("usage".to_string(), usage);
                    }
                    push_named_sse_payload(stream_body, &event_name, Value::Object(data));
                }
            }
            _ => {}
        }
    }
}

async fn write_chat_completions_compatible_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    stream_mode: bool,
    requested_model: &str,
    original_request_body: &[u8],
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let upstream_headers = upstream.headers().clone();

    if stream_mode {
        write_chunked_response_headers(
            stream,
            status,
            status_text,
            "text/event-stream; charset=utf-8",
            &upstream_headers,
        )
        .await?;

        let mut transformer =
            ChatCompletionStreamTransformer::new(original_request_body, requested_model);
        let mut body_stream = upstream.bytes_stream();
        let stream_started_at = Instant::now();
        let mut first_chunk_logged = false;
        loop {
            let stream_total_timeout = duration_from_millis(
                timeouts.legacy_stream_total_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
            );
            if stream_started_at.elapsed() > stream_total_timeout {
                let message = format!(
                    "读取上游流式响应超时: 总时长超过 {} 秒",
                    stream_total_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_total_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                return Err(message);
            }

            let stream_idle_timeout = duration_from_millis(
                timeouts.legacy_stream_idle_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
            );
            let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
                .await
                .map_err(|_| {
                    let message = format!(
                        "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                        stream_idle_timeout.as_secs()
                    );
                    legacy_debug_log(
                        debug_logs,
                        format!(
                            "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                            request.method,
                            request.target,
                            started_at.elapsed().as_millis(),
                            message
                        ),
                    );
                    message
                })?;
            let Some(chunk_result) = next_chunk else {
                break;
            };
            let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
            if chunk.is_empty() {
                continue;
            }
            if !first_chunk_logged {
                first_chunk_logged = true;
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        chunk.len()
                    ),
                );
            }
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
        if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    status.as_u16(),
                    started_at.elapsed().as_millis(),
                    escape_failure_detail(&terminal_error)
                ),
            );
            return Err(terminal_error.to_string());
        }
        legacy_debug_log(
            debug_logs,
            format!(
                "stream_completed method={} target={} status={} latency_ms={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis()
            ),
        );
        return Ok(response_capture);
    }

    let body_bytes = upstream
        .bytes()
        .await
        .map_err(|e| format!("读取上游 responses 响应失败: {}", e))?;
    let parsed = parse_responses_payload_from_upstream(&body_bytes)?;
    let response_capture = ResponseCapture {
        usage: extract_usage_capture(&parsed),
        response_id: extract_response_id(&parsed),
        response_model: extract_response_model(&parsed),
        terminal_error: None,
    };
    let chat_payload =
        build_chat_completion_payload(&parsed, requested_model, original_request_body);

    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| format!("序列化 chat/completions 响应失败: {}", e))?;
    write_http_response(
        stream,
        status.as_u16(),
        status_text,
        "application/json; charset=utf-8",
        &payload_bytes,
    )
    .await?;

    Ok(response_capture)
}

async fn write_images_compatible_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    stream_mode: bool,
    response_format: &str,
    stream_prefix: &str,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let upstream_headers = upstream.headers().clone();

    if stream_mode {
        write_chunked_response_headers(
            stream,
            status,
            status_text,
            "text/event-stream; charset=utf-8",
            &upstream_headers,
        )
        .await?;

        let mut transformer = ImageStreamTransformer::new(response_format, stream_prefix);
        let mut body_stream = upstream.bytes_stream();
        let stream_started_at = Instant::now();
        let mut first_chunk_logged = false;
        loop {
            let stream_total_timeout = duration_from_millis(
                timeouts.legacy_stream_total_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
            );
            if stream_started_at.elapsed() > stream_total_timeout {
                let message = format!(
                    "读取上游流式响应超时: 总时长超过 {} 秒",
                    stream_total_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_total_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                return Err(message);
            }

            let stream_idle_timeout = duration_from_millis(
                timeouts.legacy_stream_idle_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
            );
            let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
                .await
                .map_err(|_| {
                    let message = format!(
                        "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                        stream_idle_timeout.as_secs()
                    );
                    legacy_debug_log(
                        debug_logs,
                        format!(
                            "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                            request.method,
                            request.target,
                            started_at.elapsed().as_millis(),
                            message
                        ),
                    );
                    message
                })?;
            let Some(chunk_result) = next_chunk else {
                break;
            };
            let chunk = chunk_result.map_err(|e| format!("读取上游图片响应失败: {}", e))?;
            if chunk.is_empty() {
                continue;
            }
            if !first_chunk_logged {
                first_chunk_logged = true;
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        chunk.len()
                    ),
                );
            }
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
        if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    status.as_u16(),
                    started_at.elapsed().as_millis(),
                    escape_failure_detail(&terminal_error)
                ),
            );
            return Err(terminal_error.to_string());
        }
        legacy_debug_log(
            debug_logs,
            format!(
                "stream_completed method={} target={} status={} latency_ms={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis()
            ),
        );
        return Ok(response_capture);
    }

    let body_bytes = upstream
        .bytes()
        .await
        .map_err(|e| format!("读取上游图片响应失败: {}", e))?;
    let parsed = parse_responses_payload_from_upstream(&body_bytes)?;
    let response_capture = ResponseCapture {
        usage: extract_usage_capture(&parsed),
        response_id: extract_response_id(&parsed),
        response_model: extract_response_model(&parsed),
        terminal_error: None,
    };
    let images_payload = build_images_api_payload(&parsed, response_format)?;
    let payload_bytes = serde_json::to_vec(&images_payload)
        .map_err(|e| format!("序列化 images 响应失败: {}", e))?;

    write_http_response(
        stream,
        status.as_u16(),
        status_text,
        "application/json; charset=utf-8",
        &payload_bytes,
    )
    .await?;

    Ok(response_capture)
}

async fn write_gateway_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    response_adapter: GatewayResponseAdapter,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    match response_adapter {
        GatewayResponseAdapter::Passthrough { request_is_stream } => {
            write_upstream_response(
                stream,
                upstream,
                request_is_stream,
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
        GatewayResponseAdapter::ChatCompletions {
            stream: stream_mode,
            requested_model,
            original_request_body,
        } => {
            write_chat_completions_compatible_response(
                stream,
                upstream,
                stream_mode,
                requested_model.as_str(),
                original_request_body.as_slice(),
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
        GatewayResponseAdapter::Images {
            stream: stream_mode,
            response_format,
            stream_prefix,
        } => {
            write_images_compatible_response(
                stream,
                upstream,
                stream_mode,
                response_format.as_str(),
                stream_prefix.as_str(),
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
    }
}

async fn write_upstream_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    request_is_stream: bool,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let headers = upstream.headers().clone();
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8");
    let is_stream = should_treat_response_as_stream(content_type, request_is_stream);
    write_chunked_response_headers(stream, status, status_text, content_type, &headers).await?;

    let mut usage_collector = ResponseUsageCollector::new(is_stream);
    let mut body_stream = upstream.bytes_stream();
    let stream_started_at = Instant::now();
    let mut first_chunk_logged = false;
    loop {
        let stream_total_timeout = duration_from_millis(
            timeouts.legacy_stream_total_timeout_ms,
            DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
        );
        if stream_started_at.elapsed() > stream_total_timeout {
            let message = format!(
                "读取上游流式响应超时: 总时长超过 {} 秒",
                stream_total_timeout.as_secs()
            );
            legacy_debug_log(
                debug_logs && is_stream,
                format!(
                    "stream_total_timeout method={} target={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    started_at.elapsed().as_millis(),
                    message
                ),
            );
            return Err(message);
        }
        let stream_idle_timeout = duration_from_millis(
            timeouts.legacy_stream_idle_timeout_ms,
            DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
        );
        let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
            .await
            .map_err(|_| {
                let message = format!(
                    "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                    stream_idle_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs && is_stream,
                    format!(
                        "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                message
            })?;
        let Some(chunk_result) = next_chunk else {
            break;
        };
        let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
        if chunk.is_empty() {
            continue;
        }
        if is_stream && !first_chunk_logged {
            first_chunk_logged = true;
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                    request.method,
                    request.target,
                    started_at.elapsed().as_millis(),
                    chunk.len()
                ),
            );
        }
        write_chunked_response_chunk(stream, &chunk).await?;
        usage_collector.feed(&chunk);
    }

    finish_chunked_response(stream).await?;
    let response_capture = usage_collector.finish();
    if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
        legacy_debug_log(
            debug_logs && is_stream,
            format!(
                "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis(),
                escape_failure_detail(&terminal_error)
            ),
        );
        return Err(terminal_error.to_string());
    }
    legacy_debug_log(
        debug_logs && is_stream,
        format!(
            "stream_completed method={} target={} status={} latency_ms={}",
            request.method,
            request.target,
            status.as_u16(),
            started_at.elapsed().as_millis()
        ),
    );
    Ok(response_capture)
}

async fn force_refresh_gateway_account(
    account_id: &str,
    observed_generation: u64,
) -> Result<CodexAccount, String> {
    let account = codex_account::force_refresh_managed_account_after_observed(
        account_id,
        observed_generation,
        "本地网关上游返回 401",
    )
    .await?;
    cache_prepared_account(&account).await;
    Ok(account)
}

fn should_retry_upstream_send_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn format_reqwest_error_chain(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = StdError::source(error);
    while let Some(err) = source {
        let detail = err.to_string();
        if !detail.trim().is_empty() && parts.last().map(|item| item != &detail).unwrap_or(true) {
            parts.push(detail);
        }
        source = StdError::source(err);
    }
    parts.join(" | caused by: ")
}

fn format_upstream_network_error(error: &reqwest::Error) -> String {
    format!(
        "Codex 上游网络或代理不可用，未能连接到所选账号的上游服务。请检查网络、代理配置以及账号 Base URL 可访问性。技术细节: {}",
        format_reqwest_error_chain(error)
    )
}

fn backoff_retry_delay(retry_attempt: usize, base_delay_ms: u64, max_delay_ms: u64) -> Duration {
    let multiplier = match retry_attempt {
        0 | 1 => 1u32,
        2 => 2u32,
        _ => 4u32,
    };
    let base = Duration::from_millis(base_delay_ms);
    let max = Duration::from_millis(max_delay_ms);
    let delay = base.saturating_mul(multiplier);
    if delay > max {
        max
    } else {
        delay
    }
}

fn should_retry_single_account_upstream_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED
            | StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn build_account_scoped_upstream_body<'a>(
    target: &str,
    body: &'a [u8],
    account: &CodexAccount,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<Cow<'a, [u8]>, String> {
    if !is_responses_request(target) {
        return Ok(Cow::Borrowed(body));
    }

    let Some(mut body_value) = parse_request_body_json(body) else {
        return Ok(Cow::Borrowed(body));
    };
    let Some(body_obj) = body_value.as_object_mut() else {
        return Ok(Cow::Borrowed(body));
    };
    let remove_all_image_capabilities =
        !image_generation_tools_allowed(image_generation_mode, request_kind);
    if remove_all_image_capabilities {
        let changed = if image_generation_mode == CodexLocalAccessImageGenerationMode::ImagesOnly {
            remove_hosted_image_generation_capabilities_from_object(body_obj)
        } else {
            remove_image_generation_capabilities_from_object(body_obj)
        };
        if !changed {
            return Ok(Cow::Borrowed(body));
        }
        return serde_json::to_vec(&body_value)
            .map(Cow::Owned)
            .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e));
    }

    if has_hosted_image_generation_tool_conflict(body_obj) {
        if !remove_hosted_image_generation_capabilities_from_object(body_obj) {
            return Ok(Cow::Borrowed(body));
        }
        return serde_json::to_vec(&body_value)
            .map(Cow::Owned)
            .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e));
    }

    // Free OAuth 账号不具备官方托管的 image_generation 能力。
    // 第三方 OpenAI 兼容客户端可能会在普通文本请求中主动携带该工具；
    // 不能只跳过注入，否则仍会把不受支持的能力声明转发给上游并得到 403。
    // 这里只移除官方托管工具，保留客户端自定义的 image_gen 函数工具，
    // 以免改变第三方客户端自己的工具调用协议。
    if is_free_plan_type(account.plan_type.as_deref()) && !request_kind_is_image(request_kind) {
        let changed = remove_hosted_image_generation_capabilities_from_object(body_obj);
        if !changed {
            return Ok(Cow::Borrowed(body));
        }
        return serde_json::to_vec(&body_value)
            .map(Cow::Owned)
            .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e));
    }

    if !image_generation_tools_allowed(image_generation_mode, request_kind) {
        return Ok(Cow::Borrowed(body));
    }

    if is_free_plan_type(account.plan_type.as_deref())
        || !ensure_image_generation_tool_in_object(body_obj)
    {
        return Ok(Cow::Borrowed(body));
    }

    serde_json::to_vec(&body_value)
        .map(Cow::Owned)
        .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e))
}

async fn send_upstream_request(
    method: &str,
    target: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    account: &CodexAccount,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
    timeouts: &CodexLocalAccessTimeouts,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<reqwest::Response, String> {
    let url = build_upstream_url(account, target)?;
    let upstream_token = account_upstream_token(account)?;
    let authorization = format!("Bearer {}", upstream_token);
    send_upstream_request_with_authorization_url(
        method,
        &url,
        target,
        headers,
        body,
        account,
        &authorization,
        upstream_proxy_url,
        connect_timeout,
        timeouts,
        image_generation_mode,
        request_kind,
    )
    .await
}

async fn send_upstream_request_with_authorization_url(
    method: &str,
    url: &str,
    target: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    account: &CodexAccount,
    authorization: &str,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
    timeouts: &CodexLocalAccessTimeouts,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<reqwest::Response, String> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|e| format!("不支持的请求方法: {}", e))?;
    let client = upstream_http_client(upstream_proxy_url, connect_timeout)?;
    let upstream_body = build_account_scoped_upstream_body(
        target,
        body,
        account,
        image_generation_mode,
        request_kind,
    )?;
    let max_send_retries = timeouts.upstream_send_retry_attempts as usize;
    for retry_attempt in 0..=max_send_retries {
        let mut request = client.request(method.clone(), url);

        let session_id =
            header_value(headers, "session-id").or_else(|| header_value(headers, "session_id"));
        for (name, value) in headers {
            if matches!(
                name.as_str(),
                "authorization"
                    | "host"
                    | "content-length"
                    | "connection"
                    | "accept-encoding"
                    | "proxy-connection"
                    | "x-api-key"
                    | "x-agtools-local-request-kind"
                    | "session_id"
                    | "session-id"
            ) {
                continue;
            }
            if !account.is_api_key_auth() && matches!(name.as_str(), "user-agent" | "originator") {
                continue;
            }
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("无效请求头 {}: {}", name, e))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| format!("无效请求头值 {}: {}", name, e))?;
            request = request.header(header_name, header_value);
        }

        request = request.header(AUTHORIZATION, authorization);
        if !account.is_api_key_auth() {
            request = request.header(USER_AGENT, DEFAULT_CODEX_USER_AGENT);
            request = request.header("Originator", DEFAULT_CODEX_ORIGINATOR);
        }
        if let Some(session_id) = session_id {
            request = request.header("Session-Id", session_id);
        }
        if !account.is_api_key_auth() {
            if let Some(account_id) = resolve_upstream_account_id(account) {
                request = request.header("ChatGPT-Account-Id", account_id);
            }
        }
        if !headers.contains_key("accept") {
            request = request.header(
                ACCEPT,
                if is_stream_request(headers, upstream_body.as_ref()) {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            );
        }
        request = request.header("Connection", "Keep-Alive");
        if !headers.contains_key("content-type") && !upstream_body.is_empty() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        if !upstream_body.is_empty() {
            request = request.body(upstream_body.as_ref().to_vec());
        }

        match request.send().await {
            Ok(response) => return Ok(response),
            Err(error) => {
                let should_retry =
                    retry_attempt < max_send_retries && should_retry_upstream_send_error(&error);
                if !should_retry {
                    return Err(format_upstream_network_error(&error));
                }
                tokio::time::sleep(backoff_retry_delay(
                    retry_attempt + 1,
                    timeouts.upstream_send_retry_base_delay_ms,
                    timeouts.upstream_send_retry_max_delay_ms,
                ))
                .await;
            }
        }
    }

    Err("请求 Codex 上游失败: 未知错误".to_string())
}

const MAX_OPENAI_RESPONSES_REJECTED_FIELD_RETRIES: usize = 6;

struct OpenAIResponsesRejectedFieldRetryState {
    attempts: usize,
    seen_body_hashes: HashSet<[u8; 32]>,
}

impl OpenAIResponsesRejectedFieldRetryState {
    fn new(initial_body: &[u8]) -> Self {
        let mut state = Self {
            attempts: 0,
            seen_body_hashes: HashSet::with_capacity(
                MAX_OPENAI_RESPONSES_REJECTED_FIELD_RETRIES + 1,
            ),
        };
        state.remember(initial_body);
        state
    }

    fn allow(&mut self, next_body: &[u8]) -> bool {
        if next_body.is_empty() || self.attempts >= MAX_OPENAI_RESPONSES_REJECTED_FIELD_RETRIES {
            return false;
        }
        let body_hash: [u8; 32] = Sha256::digest(next_body).into();
        if !self.seen_body_hashes.insert(body_hash) {
            return false;
        }
        self.attempts += 1;
        true
    }

    fn remember(&mut self, body: &[u8]) {
        if !body.is_empty() {
            self.seen_body_hashes.insert(Sha256::digest(body).into());
        }
    }
}

fn normalize_openai_responses_rejected_field_retry_body(
    status: StatusCode,
    body: &[u8],
    response_body: &[u8],
) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    if status != StatusCode::BAD_REQUEST || body.is_empty() || response_body.is_empty() {
        return Ok(None);
    }
    let response: Value = match serde_json::from_slice(response_body) {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    let code = response
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let message = response
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if code != "unknown_parameter"
        && code != "unsupported_parameter"
        && !message.contains("unknown parameter")
        && !message.contains("unsupported parameter")
    {
        return Ok(None);
    }
    let mut param = response
        .pointer("/error/param")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if param.is_empty() {
        let pattern = regex::Regex::new(
            r#"(?i)(?:unknown|unsupported)[ _-]+parameter\s*(?::|=|is)?\s*[\"']?(max_output_tokens|input\[\d+\]\.namespace)(?:[\"']|\b)"#,
        )
        .map_err(|error| format!("编译 Responses 拒绝字段匹配规则失败: {error}"))?;
        param = pattern
            .captures(&message)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim().to_ascii_lowercase())
            .unwrap_or_default();
    }

    let mut request: Value = serde_json::from_slice(body)
        .map_err(|error| format!("解析 Responses 拒绝字段重试请求失败: {error}"))?;
    if param == "max_output_tokens" {
        let Some(object) = request.as_object_mut() else {
            return Ok(None);
        };
        if object.remove("max_output_tokens").is_none() {
            return Ok(None);
        }
        return serde_json::to_vec(&request)
            .map(|body| Some((body, "max_output_tokens parameter rejection")))
            .map_err(|error| format!("序列化 Responses 拒绝字段重试请求失败: {error}"));
    }

    let namespace_pattern = regex::Regex::new(r"(?i)^input\[(\d+)\]\.namespace$")
        .map_err(|error| format!("编译 Responses namespace 匹配规则失败: {error}"))?;
    let Some(index) = namespace_pattern
        .captures(&param)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<usize>().ok())
    else {
        return Ok(None);
    };
    let Some(item) = request
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .and_then(|input| input.get_mut(index))
        .and_then(Value::as_object_mut)
    else {
        return Ok(None);
    };
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        item_type.as_str(),
        "function_call" | "tool_call" | "custom_tool_call" | "mcp_tool_call"
    ) || item.remove("namespace").is_none()
    {
        return Ok(None);
    }
    serde_json::to_vec(&request)
        .map(|body| Some((body, "indexed namespace parameter rejection")))
        .map_err(|error| format!("序列化 Responses namespace 重试请求失败: {error}"))
}
