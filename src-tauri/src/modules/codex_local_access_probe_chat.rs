// Codex Local Access：Gateway probes, chat dialogs and streaming test adapters。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
#[derive(Debug, Clone)]
struct LocalAccessGatewayProbeFailure {
    status: Option<u16>,
    message: String,
    detail: Option<String>,
    gateway_output: Option<String>,
}

fn truncate_diagnostic_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut result = value.chars().take(max_chars).collect::<String>();
    result.push_str("...");
    result
}

fn clean_diagnostic_text(value: impl Into<String>) -> Option<String> {
    let text = value.into().trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(truncate_diagnostic_text(&text, 4000))
    }
}

fn extract_gateway_error_message(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "网关未返回错误内容".to_string();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = value.get("error").and_then(Value::as_str) {
            return message.to_string();
        }
        if let Some(message) = value
            .get("error")
            .and_then(|item| item.get("message"))
            .and_then(Value::as_str)
        {
            return message.to_string();
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return message.to_string();
        }
    }

    truncate_diagnostic_text(trimmed, 800)
}

fn build_failure_result(failure: CodexLocalAccessTestFailure) -> CodexLocalAccessTestResult {
    CodexLocalAccessTestResult {
        model_id: failure.model_id.clone(),
        latency_ms: None,
        output: None,
        failure: Some(failure),
    }
}

fn local_access_test_failure(
    title: impl Into<String>,
    stage: impl Into<String>,
    cause: impl Into<String>,
    suggestion: impl Into<String>,
    model_id: Option<String>,
) -> CodexLocalAccessTestFailure {
    CodexLocalAccessTestFailure {
        title: title.into(),
        stage: stage.into(),
        cause: cause.into(),
        suggestion: suggestion.into(),
        status: None,
        model_id,
        detail: None,
        gateway_output: None,
    }
}

fn emit_chat_test_stream_event(app: &AppHandle, session_id: &str, payload: Value) {
    let mut event = Map::new();
    event.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    if let Value::Object(payload) = payload {
        for (key, value) in payload {
            event.insert(key, value);
        }
    }
    let _ = app.emit(
        CODEX_LOCAL_ACCESS_CHAT_TEST_STREAM_EVENT,
        Value::Object(event),
    );
}

async fn run_local_access_test_dialog(
    base_url: &str,
    api_key: &str,
    model_id: &str,
) -> Result<(u64, String), LocalAccessGatewayProbeFailure> {
    run_local_access_chat_dialog(
        base_url,
        api_key,
        model_id,
        vec![CodexLocalAccessChatMessage {
            role: "user".to_string(),
            content: "Reply with exactly: pong".to_string(),
        }],
    )
    .await
}

async fn run_local_access_chat_stream_dialog(
    app: &AppHandle,
    session_id: &str,
    base_url: &str,
    api_key: &str,
    model_id: &str,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(), LocalAccessGatewayProbeFailure> {
    let url = local_access_chat_completions_url(base_url);
    let client = match build_localhost_http_client(Duration::from_secs(90), "本地 API 流式对话测试")
    {
        Ok(client) => client,
        Err(error) => {
            return Err(LocalAccessGatewayProbeFailure {
                status: None,
                message: format!("创建本地 API 流式对话测试客户端失败: {}", error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    let body = json!({
        "model": model_id,
        "stream": true,
        "messages": messages
            .into_iter()
            .map(|message| {
                json!({
                    "role": message.role,
                    "content": message.content,
                })
            })
            .collect::<Vec<_>>(),
        "max_tokens": 1024
    });
    let started_at = Instant::now();
    let response = match client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", api_key.trim()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream")
        // The dialog sends an ordinary text probe. Keep hosted image generation
        // available on the API Service, but do not advertise it to an upstream
        // provider group that may not have image capability enabled.
        .header(
            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE,
        )
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Err(LocalAccessGatewayProbeFailure {
                status: error.status().map(|status| status.as_u16()),
                message: format!("无法连接本地 API 服务 {}: {}", url, error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        let body_text = match response.text().await {
            Ok(text) => text,
            Err(error) => {
                return Err(LocalAccessGatewayProbeFailure {
                    status: Some(status.as_u16()),
                    message: format!("读取本地 API 对话响应失败: {}", error),
                    detail: Some(error.to_string()),
                    gateway_output: None,
                });
            }
        };
        return Err(LocalAccessGatewayProbeFailure {
            status: Some(status.as_u16()),
            message: extract_gateway_error_message(&body_text),
            detail: clean_diagnostic_text(body_text.clone()),
            gateway_output: clean_diagnostic_text(format!(
                "HTTP {}\n{}",
                status.as_u16(),
                body_text
            )),
        });
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                return Err(LocalAccessGatewayProbeFailure {
                    status: Some(status.as_u16()),
                    message: format!("读取本地 API 流式响应失败: {}", error),
                    detail: Some(error.to_string()),
                    gateway_output: None,
                });
            }
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));

        while let Some(index) = buffer.find("\n\n") {
            let frame = buffer[..index].to_string();
            buffer = buffer[index + 2..].to_string();
            if handle_local_access_chat_stream_frame(app, session_id, &frame) {
                emit_chat_test_stream_event(
                    app,
                    session_id,
                    json!({
                        "type": "done",
                        "modelId": model_id,
                        "latencyMs": started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    }),
                );
                return Ok(());
            }
        }
    }

    if !buffer.trim().is_empty() && handle_local_access_chat_stream_frame(app, session_id, &buffer)
    {
        emit_chat_test_stream_event(
            app,
            session_id,
            json!({
                "type": "done",
                "modelId": model_id,
                "latencyMs": started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            }),
        );
        return Ok(());
    }

    emit_chat_test_stream_event(
        app,
        session_id,
        json!({
            "type": "done",
            "modelId": model_id,
            "latencyMs": started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        }),
    );
    Ok(())
}

fn handle_local_access_chat_stream_frame(app: &AppHandle, session_id: &str, frame: &str) -> bool {
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    let data = data.trim();
    if data.is_empty() {
        return false;
    }
    if data == "[DONE]" {
        return true;
    }

    if let Ok(value) = serde_json::from_str::<Value>(data) {
        let delta = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"));
        if let Some(content) = delta
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            emit_chat_test_stream_event(
                app,
                session_id,
                json!({
                    "type": "delta",
                    "content": content,
                }),
            );
        }
        if let Some(reasoning) = delta
            .and_then(|delta| delta.get("reasoning_content"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            emit_chat_test_stream_event(
                app,
                session_id,
                json!({
                    "type": "delta",
                    "reasoning": reasoning,
                }),
            );
        }
    }
    false
}

async fn run_local_access_chat_dialog(
    base_url: &str,
    api_key: &str,
    model_id: &str,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(u64, String), LocalAccessGatewayProbeFailure> {
    let url = local_access_chat_completions_url(base_url);
    let client = match build_localhost_http_client(Duration::from_secs(90), "本地 API 对话测试")
    {
        Ok(client) => client,
        Err(error) => {
            return Err(LocalAccessGatewayProbeFailure {
                status: None,
                message: format!("创建本地 API 对话测试客户端失败: {}", error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    let body = json!({
        "model": model_id,
        "stream": false,
        "messages": messages
            .into_iter()
            .map(|message| {
                json!({
                    "role": message.role,
                    "content": message.content,
                })
            })
            .collect::<Vec<_>>(),
        "max_tokens": 1024
    });
    let started_at = Instant::now();
    let response = match client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", api_key.trim()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        // The dialog sends an ordinary text probe. Keep hosted image generation
        // available on the API Service, but do not advertise it to an upstream
        // provider group that may not have image capability enabled.
        .header(
            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE,
        )
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Err(LocalAccessGatewayProbeFailure {
                status: error.status().map(|status| status.as_u16()),
                message: format!("无法连接本地 API 服务 {}: {}", url, error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };
    let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    let status = response.status();
    let body_text = match response.text().await {
        Ok(text) => text,
        Err(error) => {
            return Err(LocalAccessGatewayProbeFailure {
                status: Some(status.as_u16()),
                message: format!("读取本地 API 对话响应失败: {}", error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    if status.is_success() {
        return Ok((
            latency_ms,
            extract_chat_completion_output(&body_text)
                .unwrap_or_else(|| truncate_diagnostic_text(body_text.trim(), 4000)),
        ));
    }

    Err(LocalAccessGatewayProbeFailure {
        status: Some(status.as_u16()),
        message: extract_gateway_error_message(&body_text),
        detail: clean_diagnostic_text(body_text.clone()),
        gateway_output: clean_diagnostic_text(format!("HTTP {}\n{}", status.as_u16(), body_text)),
    })
}

fn local_access_chat_completions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{}/chat/completions", trimmed)
    } else {
        format!("{}{}", trimmed, CHAT_COMPLETIONS_PATH)
    }
}

fn extract_chat_completion_output(body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(body).ok()?;
    let message = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"));
    if let Some(content) = message.and_then(|message| message.get("content")) {
        if let Some(text) = content
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
        if let Some(parts) = content.as_array() {
            let text = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .or_else(|| part.get("content"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("");
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("output_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn is_quota_or_rate_limit_message(status: Option<u16>, message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    matches!(status, Some(429))
        || lower.contains("usage_limit_reached")
        || lower.contains("limit reached")
        || lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("cooldown")
        || lower.contains("额度")
        || lower.contains("限流")
        || lower.contains("冷却")
}

fn is_image_generation_capability_message(status: Option<u16>, message: &str) -> bool {
    if !matches!(status, Some(400 | 403 | 422)) {
        return false;
    }
    let lower = message.to_ascii_lowercase();
    lower.contains("image_generation_not_enabled")
        || lower.contains("image generation is not enabled")
        || lower.contains("image_generation is not enabled")
        || (lower.contains("image_generation") && lower.contains("not enabled"))
        || message.contains("未启用图片生成能力")
}

fn classify_gateway_probe_failure(
    model_id: &str,
    probe_failure: LocalAccessGatewayProbeFailure,
) -> CodexLocalAccessTestFailure {
    let status = probe_failure.status;
    let message = probe_failure.message.trim();
    let lower = message.to_ascii_lowercase();
    let (title, stage, suggestion) = if status.is_none() {
        (
            "无法连接本地网关",
            "本地网关连接",
            "确认 API 服务仍在运行，端口未被系统占用或安全软件拦截；如端口异常，可先清理端口或更换端口后重试。",
        )
    } else if matches!(status, Some(401)) {
        if lower.contains("authorization") || message.contains("密钥") || lower.contains("api-key")
        {
            (
                "本地 API 服务密钥无效",
                "本地网关鉴权",
                "重置 API 服务密钥后重新复制 Base URL/API Key，并确认测试请求使用的是最新配置。",
            )
        } else {
            (
                "Codex 账号鉴权失败",
                "上游账号鉴权",
                "刷新该 Codex 账号额度或重新导入账号；如果账号已退出登录或令牌过期，需要重新登录后再测试。",
            )
        }
    } else if is_image_generation_capability_message(status, message) {
        (
            "图片生成能力不可用",
            "上游图片能力",
            "如果只是普通文本对话报错，请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”；如果需要生图，请换用具备图片能力的 Codex 账号。",
        )
    } else if is_quota_or_rate_limit_message(status, message) {
        (
            "上游限流或额度不足",
            "上游额度",
            "查看账号额度池，切换到仍有额度的账号，或等待冷却窗口结束后重试。",
        )
    } else if matches!(status, Some(502) | Some(503) | Some(504)) {
        if message.contains("暂无可用账号")
            || message.contains("集合暂无")
            || message.contains("Free 账号")
            || message.contains("API Key 账号")
        {
            (
                "账号池暂无可用账号",
                "账号池路由",
                "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号，并确认未被 Free 账号限制拦截。",
            )
        } else {
            (
                "上游服务或代理不可用",
                "上游请求",
                "检查 API 服务代理地址、网络连通性和 Codex 上游服务状态；如果 API 服务没有请求记录，检查代理工具是否拦截 localhost / 127.0.0.1。",
            )
        }
    } else {
        (
            "本地网关请求失败",
            "本地网关响应",
            "根据 HTTP 状态和网关返回内容处理；如果是账号相关错误，优先刷新或重新导入对应账号。",
        )
    };

    CodexLocalAccessTestFailure {
        title: title.to_string(),
        stage: stage.to_string(),
        cause: if let Some(status) = status {
            format!("本地网关返回 HTTP {}：{}", status, message)
        } else {
            message.to_string()
        },
        suggestion: suggestion.to_string(),
        status,
        model_id: Some(model_id.to_string()),
        detail: probe_failure.detail,
        gateway_output: probe_failure.gateway_output,
    }
}

pub async fn test_local_access_with_dialog() -> Result<CodexLocalAccessTestResult, String> {
    ensure_runtime_loaded().await?;
    let state = snapshot_state().await?;
    let Some(collection) = state.collection.clone() else {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务集合尚未创建",
            "检测前置条件",
            "当前没有可用于本地 API 服务的账号集合配置。",
            "先在 API 服务弹框中选择账号并保存，然后启用服务后再测试。",
            None,
        )));
    };
    if !collection.enabled {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务未启用",
            "检测前置条件",
            "当前 API 服务处于停用状态，无法通过本地网关发起测试对话。",
            "先启用 API 服务，再重新执行健康检测。",
            None,
        )));
    }
    if !state.running {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务未运行",
            "本地网关进程",
            "API 服务配置已启用，但本地网关当前没有监听端口。",
            "先启动 API 服务；如果端口被占用，清理端口或更换端口后重试。",
            None,
        )));
    }
    if collection.account_ids.is_empty() {
        return Ok(build_failure_result(local_access_test_failure(
            "账号集合为空",
            "账号池配置",
            "API 服务集合中没有账号，网关没有可路由的上游账号。",
            "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号后再测试。",
            None,
        )));
    }

    let base_url = state
        .base_url
        .clone()
        .unwrap_or_else(|| build_collection_base_url(&collection));
    let Some(model_id) = state.model_ids.first().cloned() else {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务暂无可用模型",
            "模型配置",
            "当前 API 服务没有可用于检测的模型 ID。",
            "确认账号集合中至少有一个可用账号，并刷新模型/账号状态后再测试。",
            None,
        )));
    };
    if model_id.trim().is_empty() {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务暂无可用模型",
            "模型配置",
            "当前 API 服务没有可用于检测的模型 ID。",
            "确认账号集合中至少有一个可用账号，并刷新模型/账号状态后再测试。",
            None,
        )));
    }
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }

    match run_local_access_test_dialog(&base_url, &collection.api_key, &model_id).await {
        Ok((latency_ms, output)) => Ok(CodexLocalAccessTestResult {
            model_id: Some(model_id),
            latency_ms: Some(latency_ms),
            output: Some(output),
            failure: None,
        }),
        Err(probe_failure) => Ok(build_failure_result(classify_gateway_probe_failure(
            &model_id,
            probe_failure,
        ))),
    }
}

pub async fn chat_local_access_with_dialog(
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<CodexLocalAccessChatResult, String> {
    ensure_runtime_loaded().await?;
    let state = snapshot_state().await?;
    let model_id = model_id.trim().to_string();
    if model_id.is_empty() {
        return Err("请选择用于测试的模型 ID。".to_string());
    }

    let normalized_messages = messages
        .into_iter()
        .filter_map(|message| {
            let role = message.role.trim().to_ascii_lowercase();
            let content = message.content.trim().to_string();
            if content.is_empty() {
                return None;
            }
            if !matches!(role.as_str(), "system" | "user" | "assistant") {
                return None;
            }
            Some(CodexLocalAccessChatMessage { role, content })
        })
        .collect::<Vec<_>>();
    if normalized_messages.is_empty()
        || !normalized_messages
            .iter()
            .any(|message| message.role.as_str() == "user")
    {
        return Err("请输入至少一条用户消息后再发送。".to_string());
    }

    let Some(collection) = state.collection.clone() else {
        return Ok(CodexLocalAccessChatResult {
            model_id,
            latency_ms: None,
            output: None,
            failure: Some(local_access_test_failure(
                "API 服务集合尚未创建",
                "检测前置条件",
                "当前没有可用于本地 API 服务的账号集合配置。",
                "先在 API 服务弹框中选择账号并保存，然后启用服务后再对话。",
                None,
            )),
        });
    };
    if !collection.enabled {
        return Ok(CodexLocalAccessChatResult {
            model_id,
            latency_ms: None,
            output: None,
            failure: Some(local_access_test_failure(
                "API 服务未启用",
                "检测前置条件",
                "当前 API 服务处于停用状态，无法通过本地网关发起测试对话。",
                "先启用 API 服务，再重新发送消息。",
                None,
            )),
        });
    }
    if !state.running {
        return Ok(CodexLocalAccessChatResult {
            model_id,
            latency_ms: None,
            output: None,
            failure: Some(local_access_test_failure(
                "API 服务未运行",
                "本地网关进程",
                "API 服务配置已启用，但本地网关当前没有监听端口。",
                "先启动 API 服务；如果端口被占用，清理端口或更换端口后重试。",
                None,
            )),
        });
    }
    if collection.account_ids.is_empty() {
        return Ok(CodexLocalAccessChatResult {
            model_id,
            latency_ms: None,
            output: None,
            failure: Some(local_access_test_failure(
                "账号集合为空",
                "账号池配置",
                "API 服务集合中没有账号，网关没有可路由的上游账号。",
                "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号后再对话。",
                None,
            )),
        });
    }

    let base_url = state
        .base_url
        .clone()
        .unwrap_or_else(|| build_collection_base_url(&collection));
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }

    match run_local_access_chat_dialog(
        &base_url,
        &collection.api_key,
        &model_id,
        normalized_messages,
    )
    .await
    {
        Ok((latency_ms, output)) => Ok(CodexLocalAccessChatResult {
            model_id,
            latency_ms: Some(latency_ms),
            output: Some(output),
            failure: None,
        }),
        Err(probe_failure) => Ok(CodexLocalAccessChatResult {
            model_id: model_id.clone(),
            latency_ms: None,
            output: None,
            failure: Some(classify_gateway_probe_failure(&model_id, probe_failure)),
        }),
    }
}

pub async fn stream_chat_local_access_with_dialog(
    app: AppHandle,
    session_id: String,
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(), String> {
    ensure_runtime_loaded().await?;
    let state = snapshot_state().await?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("测试会话 ID 不能为空。".to_string());
    }
    let model_id = model_id.trim().to_string();
    if model_id.is_empty() {
        return Err("请选择用于测试的模型 ID。".to_string());
    }

    let normalized_messages = messages
        .into_iter()
        .filter_map(|message| {
            let role = message.role.trim().to_ascii_lowercase();
            let content = message.content.trim().to_string();
            if content.is_empty() {
                return None;
            }
            if !matches!(role.as_str(), "system" | "user" | "assistant") {
                return None;
            }
            Some(CodexLocalAccessChatMessage { role, content })
        })
        .collect::<Vec<_>>();
    if normalized_messages.is_empty()
        || !normalized_messages
            .iter()
            .any(|message| message.role.as_str() == "user")
    {
        return Err("请输入至少一条用户消息后再发送。".to_string());
    }

    let emit_failure = |failure: CodexLocalAccessTestFailure| {
        emit_chat_test_stream_event(
            &app,
            &session_id,
            json!({
                "type": "error",
                "failure": failure,
            }),
        );
    };

    let Some(collection) = state.collection.clone() else {
        emit_failure(local_access_test_failure(
            "API 服务集合尚未创建",
            "检测前置条件",
            "当前没有可用于本地 API 服务的账号集合配置。",
            "先在 API 服务弹框中选择账号并保存，然后启用服务后再对话。",
            None,
        ));
        return Ok(());
    };
    if !collection.enabled {
        emit_failure(local_access_test_failure(
            "API 服务未启用",
            "检测前置条件",
            "当前 API 服务处于停用状态，无法通过本地网关发起测试对话。",
            "先启用 API 服务，再重新发送消息。",
            None,
        ));
        return Ok(());
    }
    if !state.running {
        emit_failure(local_access_test_failure(
            "API 服务未运行",
            "本地网关进程",
            "API 服务配置已启用，但本地网关当前没有监听端口。",
            "先启动 API 服务；如果端口被占用，清理端口或更换端口后重试。",
            None,
        ));
        return Ok(());
    }
    if collection.account_ids.is_empty() {
        emit_failure(local_access_test_failure(
            "账号集合为空",
            "账号池配置",
            "API 服务集合中没有账号，网关没有可路由的上游账号。",
            "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号后再对话。",
            None,
        ));
        return Ok(());
    }

    let base_url = state
        .base_url
        .clone()
        .unwrap_or_else(|| build_collection_base_url(&collection));
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }

    match run_local_access_chat_stream_dialog(
        &app,
        &session_id,
        &base_url,
        &collection.api_key,
        &model_id,
        normalized_messages,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(probe_failure) => {
            emit_failure(classify_gateway_probe_failure(&model_id, probe_failure));
            Ok(())
        }
    }
}

