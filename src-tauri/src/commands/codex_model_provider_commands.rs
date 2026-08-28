// Codex 模型供应商命令实现。
//
// 本片段由 `commands/codex.rs` 通过 `include!` 纳入 `commands::codex` 模块，集中负责供应商
// 配置持久化、模型发现、连接测试和用量解析。对外调用仍使用原有
// `commands::codex::<command>` 路径，避免拆分影响命令注册和现有调用方。

const CODEX_GROUPS_FILE: &str = "codex_account_groups.json";
const CODEX_MODEL_PROVIDERS_FILE: &str = "codex_model_providers.json";
const CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS: u64 = 20;

#[tauri::command]
pub async fn load_codex_account_groups() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read codex groups: {}", e))
}

#[tauri::command]
pub async fn save_codex_account_groups(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex groups: {}", e))
}

#[tauri::command]
pub async fn load_codex_model_providers() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_MODEL_PROVIDERS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read codex model providers: {}", e))
}

#[tauri::command]
pub async fn save_codex_model_providers(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_MODEL_PROVIDERS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex model providers: {}", e))
}

fn codex_model_provider_models_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/models".to_string()
    } else {
        format!("{}/models", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_usage_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/usage".to_string()
    } else {
        format!("{}/usage", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_deepseek_balance_url(base_url: &str) -> Result<Option<String>, String> {
    let mut url = reqwest::Url::parse(base_url.trim())
        .map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
    {
        return Ok(None);
    }
    url.set_path("/user/balance");
    url.set_query(None);
    Ok(Some(url.to_string()))
}

fn codex_model_provider_opencode_go_usage_url(base_url: &str) -> Result<Option<String>, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("opencode.ai"))
    {
        return Ok(None);
    }
    let path = url.path().trim_end_matches('/').to_ascii_lowercase();
    if path != "/zen/go" && path != "/zen/go/v1" {
        return Ok(None);
    }
    url.set_path("/zen/go/v1/usage");
    url.set_query(None);
    url.set_fragment(None);
    Ok(Some(url.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexTokenPlanProvider {
    MiniMax,
    Zhipu,
}

fn codex_model_provider_token_plan_provider(
    base_url: &str,
) -> Result<Option<CodexTokenPlanProvider>, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let url = reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let Some(host) = url.host_str() else {
        return Ok(None);
    };
    let host = host.to_ascii_lowercase();
    if matches!(
        host.as_str(),
        "api.minimaxi.com" | "www.minimaxi.com" | "api.minimax.io" | "www.minimax.io"
    ) {
        return Ok(Some(CodexTokenPlanProvider::MiniMax));
    }
    if matches!(
        host.as_str(),
        "open.bigmodel.cn" | "bigmodel.cn" | "api.z.ai" | "z.ai"
    ) {
        return Ok(Some(CodexTokenPlanProvider::Zhipu));
    }
    Ok(None)
}

fn codex_model_provider_token_plan_urls(
    base_url: &str,
    provider: CodexTokenPlanProvider,
) -> Result<Vec<String>, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    url.set_query(None);
    url.set_fragment(None);
    let endpoints: &[&str] = match provider {
        CodexTokenPlanProvider::MiniMax => &[
            "/v1/token_plan/remains",
            "/v1/api/openplatform/coding_plan/remains",
        ],
        CodexTokenPlanProvider::Zhipu => &["/api/monitor/usage/quota/limit"],
    };
    Ok(endpoints
        .iter()
        .map(|endpoint| {
            let mut candidate = url.clone();
            candidate.set_path(endpoint);
            candidate.to_string()
        })
        .collect())
}

fn codex_model_provider_new_api_billing_url(
    base_url: &str,
    endpoint: &str,
) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let base_path = url.path().trim_end_matches('/');
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_new_api_api_url(base_url: &str, endpoint: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let mut base_path = url.path().trim_end_matches('/').to_string();
    if base_path == "/v1" {
        base_path.clear();
    }
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_failure(
    title: &str,
    stage: &str,
    cause: String,
    suggestion: &str,
    status: Option<u16>,
    detail: Option<String>,
) -> CodexLocalAccessTestResult {
    CodexLocalAccessTestResult {
        model_id: None,
        latency_ms: None,
        output: None,
        failure: Some(CodexLocalAccessTestFailure {
            title: title.to_string(),
            stage: stage.to_string(),
            cause,
            suggestion: suggestion.to_string(),
            status,
            model_id: None,
            detail,
            gateway_output: None,
        }),
    }
}

const CODEX_MODEL_PROVIDER_CHAT_TEST_PROGRESS_EVENT: &str = "codex://model-provider-test-progress";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestTarget {
    pub provider_id: String,
    pub provider_name: String,
    pub base_url: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub api_key: String,
    pub wire_api: Option<String>,
    #[serde(default)]
    pub model_catalog: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestRecord {
    pub provider_id: String,
    pub provider_name: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub wire_api: String,
    pub access_mode: String,
    pub model_id: Option<String>,
    pub success: bool,
    pub prompt: String,
    pub reply: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestBatchResult {
    pub run_id: String,
    pub records: Vec<CodexModelProviderChatTestRecord>,
    pub success_count: usize,
    pub failure_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestProgressPayload {
    pub run_id: String,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub running: bool,
    pub phase: String,
    pub current_provider_id: Option<String>,
    pub item: Option<CodexModelProviderChatTestRecord>,
}

fn emit_model_provider_chat_test_progress(
    app: &AppHandle,
    run_id: &str,
    total: usize,
    completed: usize,
    success_count: usize,
    failure_count: usize,
    running: bool,
    phase: &str,
    current_provider_id: Option<&str>,
    item: Option<CodexModelProviderChatTestRecord>,
) {
    let payload = CodexModelProviderChatTestProgressPayload {
        run_id: run_id.to_string(),
        total,
        completed,
        success_count,
        failure_count,
        running,
        phase: phase.to_string(),
        current_provider_id: current_provider_id.map(ToOwned::to_owned),
        item,
    };
    let _ = app.emit(CODEX_MODEL_PROVIDER_CHAT_TEST_PROGRESS_EVENT, payload);
}

fn normalize_model_provider_wire_api(value: Option<&str>, base_url: &str) -> String {
    match value.map(str::trim) {
        Some("chat_completions") | Some("chat") => return "chat_completions".to_string(),
        Some("responses") => return "responses".to_string(),
        _ => {}
    }
    // DeepSeek defaults to official Responses when the caller did not choose a protocol.
    if reqwest::Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
    {
        return "responses".to_string();
    }
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.contains("/chat/completions")
        || lower.contains("api.moonshot.cn")
        || lower.contains("api.siliconflow.cn")
        || lower.contains("api.siliconflow.com")
        || lower.contains("open.bigmodel.cn")
        || lower.contains("api.z.ai")
        || lower.contains("volces.com")
        || lower.contains("bytepluses.com")
        || lower.contains("qianfan.baidubce.com")
        || lower.contains("dashscope.aliyuncs.com")
        || lower.contains("api.stepfun.com")
        || lower.contains("api.stepfun.ai")
        || lower.contains("modelscope.cn")
        || lower.contains("api.longcat.chat")
        || lower.contains("api.minimax.io")
        || lower.contains("api.mini-max.chat")
        || lower.contains("api.minimaxi.com")
        || lower.contains("api.mimo.dev")
        || lower.contains("token-plan-cn.xiaomimimo.com")
        || lower.contains("api.novita.ai")
        || lower.contains("integrate.api.nvidia.com")
        || lower.contains("runapi.co")
        || lower.contains("relaxycode.com")
        || lower.contains("compshare.cn")
        || lower.contains("api.lemondata.cc")
        || lower.contains("e-flowcode.cc")
        || lower.contains("cc-api.pipellm.ai")
        || lower.contains("openrouter.ai")
        || lower.contains("api.therouter.ai")
    {
        "chat_completions".to_string()
    } else {
        "responses".to_string()
    }
}

const RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY: &[&str] =
    &["gpt-5.5", "gpt-5.4", "gpt-5", "gpt-4.1", "gpt-4o"];

fn is_image_generation_model_id(model_id: &str) -> bool {
    let lower = model_id.trim().to_ascii_lowercase();
    lower.starts_with("gpt-image") || lower.starts_with("dall-e") || lower.contains("image-gen")
}

fn first_non_empty_model_id(models: &[String]) -> Option<String> {
    models
        .iter()
        .map(|item| item.trim())
        .find(|item| !item.is_empty())
        .map(ToOwned::to_owned)
}

fn select_model_provider_chat_test_model(
    wire_api: &str,
    explicit_model: Option<&str>,
    model_catalog: &[String],
) -> Option<String> {
    if let Some(model) = explicit_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(model.to_string());
    }

    if wire_api.trim() == "responses" {
        for preferred in RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY {
            if let Some(model) = model_catalog
                .iter()
                .map(|item| item.trim())
                .find(|item| item.eq_ignore_ascii_case(preferred))
            {
                return Some(model.to_string());
            }
        }
        if let Some(model) = model_catalog
            .iter()
            .map(|item| item.trim())
            .find(|item| !item.is_empty() && !is_image_generation_model_id(item))
        {
            return Some(model.to_string());
        }
    }

    first_non_empty_model_id(model_catalog)
}

fn model_ids_from_provider_models(body: &serde_json::Value) -> Vec<String> {
    body.get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn first_model_from_provider_models(body: &serde_json::Value, wire_api: &str) -> Option<String> {
    let models = model_ids_from_provider_models(body);
    select_model_provider_chat_test_model(wire_api, None, &models)
}

async fn discover_model_provider_model(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    wire_api: &str,
) -> Option<String> {
    let url = codex_model_provider_models_url(base_url).ok()?;
    let response = client
        .get(url)
        .bearer_auth(api_key.trim())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    first_model_from_provider_models(&parsed, wire_api)
}

async fn run_single_model_provider_chat_test(
    client: &reqwest::Client,
    target: CodexModelProviderChatTestTarget,
    prompt: &str,
    model: Option<&str>,
    run_id: &str,
) -> CodexModelProviderChatTestRecord {
    let wire_api = normalize_model_provider_wire_api(target.wire_api.as_deref(), &target.base_url);
    let access_mode = "gateway".to_string();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let api_key = target.api_key.trim().to_string();
    if api_key.is_empty() {
        return CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: None,
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some("供应商缺少 API Key".to_string()),
            duration_ms: None,
            timestamp,
        };
    }
    let configured_model_id =
        select_model_provider_chat_test_model(&wire_api, model, &target.model_catalog);
    let model_id = match configured_model_id {
        Some(model_id) => Some(model_id),
        None => tokio::select! {
            model_id = discover_model_provider_model(
                client,
                &target.base_url,
                &api_key,
                &wire_api,
            ) => model_id,
            _ = async {
                while !codex_local_access::is_model_provider_chat_test_cancelled(run_id) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => None,
        },
    };
    if codex_local_access::is_model_provider_chat_test_cancelled(run_id) {
        return CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: None,
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some(codex_local_access::MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string()),
            duration_ms: None,
            timestamp,
        };
    }
    let Some(model_id) = model_id else {
        return CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: None,
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some("无法确定测试模型，请先配置模型目录或确认 /models 可用".to_string()),
            duration_ms: None,
            timestamp,
        };
    };

    let result = codex_local_access::run_model_provider_gateway_chat_test(
        codex_local_access::CodexModelProviderGatewayChatTestRequest {
            run_id: run_id.to_string(),
            provider_id: target.provider_id.clone(),
            provider_name: target.provider_name.clone(),
            base_url: target.base_url.clone(),
            api_key_id: target.api_key_id.clone(),
            api_key_name: target.api_key_name.clone(),
            api_key,
            wire_api: wire_api.clone(),
            model_catalog: target.model_catalog.clone(),
            model_id: model_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await
    .map(|result| (result.duration_ms, result.reply));

    match result {
        Ok((duration_ms, reply)) => CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: Some(model_id),
            success: true,
            prompt: prompt.to_string(),
            reply: Some(reply),
            error: None,
            duration_ms: Some(duration_ms),
            timestamp,
        },
        Err(error) => CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: Some(model_id),
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some(error),
            duration_ms: None,
            timestamp,
        },
    }
}

fn summarize_model_provider_models(body: &serde_json::Value) -> (Option<String>, Option<String>) {
    let ids: Vec<String> = body
        .get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .take(8)
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let first = ids.first().cloned();
    let output = if ids.is_empty() {
        None
    } else {
        Some(ids.join(", "))
    };
    (first, output)
}

fn list_model_provider_models(body: &serde_json::Value) -> Vec<CodexModelProviderModel> {
    let mut seen = std::collections::HashSet::new();
    body.get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(|id| id.as_str())?.trim();
                    if id.is_empty() {
                        return None;
                    }
                    let key = id.to_ascii_lowercase();
                    if !seen.insert(key) {
                        return None;
                    }
                    Some(CodexModelProviderModel {
                        id: id.to_string(),
                        display_name: item
                            .get("display_name")
                            .or_else(|| item.get("displayName"))
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageDetail {
    pub key: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModel {
    pub id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModelsResult {
    pub models: Vec<CodexModelProviderModel>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageSummary {
    pub mode: Option<String>,
    pub is_valid: Option<bool>,
    pub status: Option<String>,
    pub plan_name: Option<String>,
    pub remaining: Option<f64>,
    pub balance: Option<f64>,
    pub unit: Option<String>,
    pub quota_unlimited: Option<bool>,
    pub quota_limit: Option<f64>,
    pub quota_used: Option<f64>,
    pub quota_remaining: Option<f64>,
    pub today_requests: Option<i64>,
    pub today_total_tokens: Option<i64>,
    pub today_cost: Option<f64>,
    pub total_requests: Option<i64>,
    pub total_total_tokens: Option<i64>,
    pub total_cost: Option<f64>,
    pub model_stats_count: usize,
    pub latency_ms: u64,
    pub details: Vec<CodexModelProviderUsageDetail>,
}

fn json_f64_at(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_f64()
        .or_else(|| current.as_str()?.trim().parse::<f64>().ok())
}

fn json_i64_at(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

fn json_string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|item| item.to_string())
}

fn json_bool_at(value: &serde_json::Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

fn summarize_model_provider_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let model_stats_count = body
        .get("model_stats")
        .and_then(|value| value.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "mode",
        "Mode",
        json_string_at(body, &["mode"]),
    );
    push_usage_detail(
        &mut details,
        "status",
        "Status",
        json_string_at(body, &["status"]),
    );
    push_usage_detail(
        &mut details,
        "planName",
        "Plan",
        json_string_at(body, &["planName"]),
    );
    push_usage_detail(
        &mut details,
        "remaining",
        "Remaining",
        json_f64_at(body, &["remaining"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "balance",
        "Balance",
        json_f64_at(body, &["balance"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "todayRequests",
        "Today Requests",
        json_i64_at(body, &["usage", "today", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayTokens",
        "Today Tokens",
        json_i64_at(body, &["usage", "today", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayCost",
        "Today Cost",
        json_f64_at(body, &["usage", "today", "cost"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "totalRequests",
        "Total Requests",
        json_i64_at(body, &["usage", "total", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalTokens",
        "Total Tokens",
        json_i64_at(body, &["usage", "total", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalCost",
        "Total Cost",
        json_f64_at(body, &["usage", "total", "cost"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: json_string_at(body, &["mode"]),
        is_valid: json_bool_at(body, &["is_active"]).or_else(|| json_bool_at(body, &["isValid"])),
        status: json_string_at(body, &["status"]),
        plan_name: json_string_at(body, &["planName"]),
        remaining: json_f64_at(body, &["remaining"]),
        balance: json_f64_at(body, &["balance"]),
        unit: json_string_at(body, &["unit"]).or_else(|| json_string_at(body, &["quota", "unit"])),
        quota_unlimited: json_bool_at(body, &["quota", "unlimited"]),
        quota_limit: json_f64_at(body, &["quota", "limit"]),
        quota_used: json_f64_at(body, &["quota", "used"]),
        quota_remaining: json_f64_at(body, &["quota", "remaining"]),
        today_requests: json_i64_at(body, &["usage", "today", "requests"]),
        today_total_tokens: json_i64_at(body, &["usage", "today", "total_tokens"]),
        today_cost: json_f64_at(body, &["usage", "today", "cost"]),
        total_requests: json_i64_at(body, &["usage", "total", "requests"]),
        total_total_tokens: json_i64_at(body, &["usage", "total", "total_tokens"]),
        total_cost: json_f64_at(body, &["usage", "total", "cost"]),
        model_stats_count,
        latency_ms,
        details,
    }
}

fn summarize_deepseek_balance(
    body: &serde_json::Value,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let is_available = json_bool_at(body, &["is_available"]).unwrap_or(false);
    let balance_info = body
        .get("balance_infos")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| {
                    json_string_at(item, &["currency"])
                        .is_some_and(|currency| currency.eq_ignore_ascii_case("CNY"))
                })
                .or_else(|| items.first())
        });
    let currency = balance_info.and_then(|item| json_string_at(item, &["currency"]));
    let total_balance = balance_info.and_then(|item| json_f64_at(item, &["total_balance"]));
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "isAvailable",
        "Available",
        Some(is_available.to_string()),
    );
    push_usage_detail(&mut details, "currency", "Currency", currency.clone());
    for (key, label) in [
        ("total_balance", "Total Balance"),
        ("granted_balance", "Granted Balance"),
        ("topped_up_balance", "Topped-up Balance"),
    ] {
        push_usage_detail(
            &mut details,
            match key {
                "total_balance" => "totalBalance",
                "granted_balance" => "grantedBalance",
                _ => "toppedUpBalance",
            },
            label,
            balance_info
                .and_then(|item| json_f64_at(item, &[key]))
                .map(format_usage_number),
        );
    }

    CodexModelProviderUsageSummary {
        mode: Some("deepseek".to_string()),
        is_valid: Some(is_available),
        status: Some(
            if is_available {
                "available"
            } else {
                "unavailable"
            }
            .to_string(),
        ),
        plan_name: None,
        remaining: total_balance,
        balance: total_balance,
        unit: currency,
        quota_unlimited: None,
        quota_limit: None,
        quota_used: None,
        quota_remaining: total_balance,
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: None,
        model_stats_count: 0,
        latency_ms,
        details,
    }
}

fn json_f64_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|current| {
            current
                .as_f64()
                .or_else(|| current.as_str()?.trim().parse::<f64>().ok())
        })
    })
}

fn json_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|current| current.as_str())
            .map(str::trim)
            .filter(|current| !current.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn json_timestamp_seconds(value: &serde_json::Value) -> Option<i64> {
    if let Some(number) = value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
    {
        if !number.is_finite() || number <= 0.0 {
            return None;
        }
        let seconds = if number > 10_000_000_000.0 {
            number / 1000.0
        } else {
            number
        };
        return Some(seconds.floor() as i64);
    }
    value
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text.trim()).ok())
        .map(|date| date.timestamp())
}

fn json_timestamp_field(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(json_timestamp_seconds))
}

fn clamp_usage_percent(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn opencode_go_usage_window(body: &serde_json::Value, key: &str) -> Result<(f64, i64), String> {
    let window = body
        .get("usage")
        .and_then(|usage| usage.get(key))
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            format!(
                "PROVIDER_USAGE_PARSE_FAILED: OpenCode Go {} window missing",
                key
            )
        })?;
    if json_string_field(window, &["status"]).as_deref() != Some("ok") {
        return Err(format!(
            "PROVIDER_USAGE_PARSE_FAILED: OpenCode Go {} window unavailable",
            key
        ));
    }
    let used_percent = json_f64_field(window, &["percent"])
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(|| {
            format!(
                "PROVIDER_USAGE_PARSE_FAILED: OpenCode Go {} percent invalid",
                key
            )
        })?;
    let resets_at = json_timestamp_field(window, &["resetsAt", "resets_at"]).ok_or_else(|| {
        format!(
            "PROVIDER_USAGE_PARSE_FAILED: OpenCode Go {} reset missing",
            key
        )
    })?;
    Ok((used_percent, resets_at))
}

fn summarize_opencode_go_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let rolling = opencode_go_usage_window(body, "rolling")?;
    let weekly = opencode_go_usage_window(body, "weekly")?;
    let monthly = opencode_go_usage_window(body, "monthly")?;
    let used_percent = rolling.0.max(weekly.0).max(monthly.0);
    let remaining_percent = 100.0 - used_percent;
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "planName",
        "Plan",
        Some("OpenCode Go".to_string()),
    );
    for (key, label, (used, resets_at)) in [
        ("rolling", "5-Hour", rolling),
        ("weekly", "Weekly", weekly),
        ("monthly", "Monthly", monthly),
    ] {
        push_usage_detail(
            &mut details,
            &format!("{}UsedPercent", key),
            &format!("{} Used %", label),
            Some(format_usage_number(used)),
        );
        push_usage_detail(
            &mut details,
            &format!("{}RemainingPercent", key),
            &format!("{} Remaining %", label),
            Some(format_usage_number(100.0 - used)),
        );
        push_usage_detail(
            &mut details,
            &format!("{}ResetsAt", key),
            &format!("{} Reset", label),
            Some(resets_at.to_string()),
        );
    }

    Ok(CodexModelProviderUsageSummary {
        mode: Some("opencode_go".to_string()),
        is_valid: Some(true),
        status: Some(if remaining_percent > 0.0 {
            "available".to_string()
        } else {
            "exhausted".to_string()
        }),
        plan_name: Some("OpenCode Go".to_string()),
        remaining: Some(remaining_percent),
        balance: Some(remaining_percent),
        unit: Some("%".to_string()),
        quota_unlimited: None,
        quota_limit: Some(100.0),
        quota_used: Some(used_percent),
        quota_remaining: Some(remaining_percent),
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: None,
        model_stats_count: 0,
        latency_ms,
        details,
    })
}

fn token_plan_payload(body: &serde_json::Value) -> &serde_json::Value {
    body.get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body)
}

fn token_plan_remaining_percent(remaining: Option<f64>, total: Option<f64>) -> Option<f64> {
    match (remaining, total) {
        (Some(remaining), Some(total)) if total > 0.0 => {
            Some(clamp_usage_percent((remaining / total) * 100.0))
        }
        _ => None,
    }
}

fn summarize_minimax_token_plan_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let payload = token_plan_payload(body);
    let model_remains = payload
        .get("model_remains")
        .and_then(serde_json::Value::as_array);
    let model = model_remains
        .and_then(|models| {
            models
                .iter()
                .find(|item| {
                    json_string_field(item, &["model_name", "modelName"])
                        .is_some_and(|name| name.to_ascii_lowercase().starts_with("minimax-m"))
                })
                .or_else(|| models.first())
        })
        .unwrap_or(payload);

    let model_name = json_string_field(model, &["model_name", "modelName"]);
    let plan_name = json_string_field(payload, &["plan_name", "planName"])
        .or_else(|| json_string_field(body, &["plan_name", "planName"]))
        .or_else(|| Some("MiniMax Token Plan".to_string()));
    let interval_total = json_f64_field(
        model,
        &["current_interval_total_count", "currentIntervalTotalCount"],
    );
    let interval_remaining = json_f64_field(
        model,
        &[
            "current_interval_usage_count",
            "currentIntervalUsageCount",
            "current_interval_remaining",
            "currentIntervalRemaining",
        ],
    );
    let interval_remaining_percent = json_f64_field(
        model,
        &[
            "current_interval_remaining_percent",
            "currentIntervalRemainingPercent",
            "remaining_percent",
            "remainingPercent",
        ],
    )
    .map(clamp_usage_percent)
    .or_else(|| token_plan_remaining_percent(interval_remaining, interval_total));
    let weekly_total = json_f64_field(
        model,
        &["current_weekly_total_count", "currentWeeklyTotalCount"],
    );
    let weekly_remaining = json_f64_field(
        model,
        &[
            "current_weekly_usage_count",
            "currentWeeklyUsageCount",
            "current_weekly_remaining",
            "currentWeeklyRemaining",
        ],
    );
    let weekly_remaining_percent = json_f64_field(
        model,
        &[
            "current_weekly_remaining_percent",
            "currentWeeklyRemainingPercent",
            "weekly_remaining_percent",
            "weeklyRemainingPercent",
        ],
    )
    .map(clamp_usage_percent)
    .or_else(|| token_plan_remaining_percent(weekly_remaining, weekly_total));
    let remaining_percent = interval_remaining_percent
        .or(weekly_remaining_percent)
        .ok_or_else(|| {
            "PROVIDER_USAGE_PARSE_FAILED: MiniMax token plan fields missing".to_string()
        })?;
    let used_percent = 100.0 - remaining_percent;
    let interval_expires_at = json_timestamp_field(model, &["end_time", "endTime"]);
    let weekly_expires_at = json_timestamp_field(model, &["weekly_end_time", "weeklyEndTime"]);
    let mut details = Vec::new();
    push_usage_detail(&mut details, "planName", "Plan", plan_name.clone());
    push_usage_detail(&mut details, "modelName", "Model", model_name);
    push_usage_detail(
        &mut details,
        "remaining",
        "Remaining",
        Some(format_usage_number(remaining_percent)),
    );
    push_usage_detail(
        &mut details,
        "intervalRemaining",
        "Interval Remaining",
        interval_remaining.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "intervalLimit",
        "Interval Limit",
        interval_total.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "intervalRemainingPercent",
        "Interval Remaining %",
        interval_remaining_percent.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "intervalExpiresAt",
        "Interval Reset",
        interval_expires_at.map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "weeklyRemaining",
        "Weekly Remaining",
        weekly_remaining.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "weeklyLimit",
        "Weekly Limit",
        weekly_total.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "weeklyRemainingPercent",
        "Weekly Remaining %",
        weekly_remaining_percent.map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "weeklyExpiresAt",
        "Weekly Reset",
        weekly_expires_at.map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "expiresAt",
        "Next Reset",
        interval_expires_at
            .or(weekly_expires_at)
            .map(|value| value.to_string()),
    );

    Ok(CodexModelProviderUsageSummary {
        mode: Some("token_plan".to_string()),
        is_valid: Some(remaining_percent > 0.0),
        status: Some(if remaining_percent > 0.0 {
            "available".to_string()
        } else {
            "exhausted".to_string()
        }),
        plan_name,
        remaining: Some(remaining_percent),
        balance: Some(remaining_percent),
        unit: Some("%".to_string()),
        quota_unlimited: None,
        quota_limit: Some(100.0),
        quota_used: Some(used_percent),
        quota_remaining: Some(remaining_percent),
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: None,
        model_stats_count: model_remains.map_or(0, Vec::len),
        latency_ms,
        details,
    })
}

fn summarize_zhipu_token_plan_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let payload = token_plan_payload(body);
    let mut limits: Vec<&serde_json::Value> = payload
        .get("limits")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    json_string_field(item, &["type"])
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("TOKENS_LIMIT"))
                })
                .collect()
        })
        .unwrap_or_default();
    if limits.is_empty() {
        return Err("PROVIDER_USAGE_PARSE_FAILED: Zhipu token limit fields missing".to_string());
    }
    limits.sort_by_key(|item| {
        json_timestamp_field(item, &["nextResetTime", "next_reset_time"]).unwrap_or(i64::MAX)
    });

    let mut window_data = Vec::with_capacity(limits.len());
    for item in &limits {
        let total = json_f64_field(*item, &["usage", "total", "limit"]);
        let used = json_f64_field(*item, &["currentValue", "current_value", "used"]);
        let remaining = json_f64_field(*item, &["remaining", "remain"]);
        let used_percent = json_f64_field(*item, &["percentage", "usedPercentage"])
            .or_else(|| match (used, total) {
                (Some(used), Some(total)) if total > 0.0 => Some((used / total) * 100.0),
                _ => None,
            })
            .map(clamp_usage_percent)
            .ok_or_else(|| {
                "PROVIDER_USAGE_PARSE_FAILED: Zhipu token percentage missing".to_string()
            })?;
        let remaining_percent = clamp_usage_percent(100.0 - used_percent);
        let reset_at = json_timestamp_field(item, &["nextResetTime", "next_reset_time"]);
        window_data.push((
            total,
            used,
            remaining,
            used_percent,
            remaining_percent,
            reset_at,
        ));
    }

    let primary = window_data.first().copied().ok_or_else(|| {
        "PROVIDER_USAGE_PARSE_FAILED: Zhipu token limit fields missing".to_string()
    })?;
    let remaining_percent = primary.4;
    let plan_name =
        json_string_field(payload, &["level", "planName"]).or_else(|| Some("ZHIPU".to_string()));
    let mut details = Vec::new();
    push_usage_detail(&mut details, "planName", "Plan", plan_name.clone());
    push_usage_detail(
        &mut details,
        "remaining",
        "Remaining",
        Some(format_usage_number(remaining_percent)),
    );
    push_usage_detail(
        &mut details,
        "expiresAt",
        "Next Reset",
        primary.5.map(|value| value.to_string()),
    );
    for (index, (total, used, remaining, used_percent, remaining_percent, reset_at)) in
        window_data.into_iter().enumerate()
    {
        let prefix = if index == 0 {
            "interval"
        } else if index == 1 {
            "weekly"
        } else {
            "window"
        };
        push_usage_detail(
            &mut details,
            &format!("{}Limit", prefix),
            &format!("{} Limit", prefix),
            total.map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            &format!("{}Used", prefix),
            &format!("{} Used", prefix),
            used.map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            &format!("{}Remaining", prefix),
            &format!("{} Remaining", prefix),
            remaining.map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            &format!("{}UsedPercent", prefix),
            &format!("{} Used %", prefix),
            Some(format_usage_number(used_percent)),
        );
        push_usage_detail(
            &mut details,
            &format!("{}RemainingPercent", prefix),
            &format!("{} Remaining %", prefix),
            Some(format_usage_number(remaining_percent)),
        );
        push_usage_detail(
            &mut details,
            &format!("{}ExpiresAt", prefix),
            &format!("{} Reset", prefix),
            reset_at.map(|value| value.to_string()),
        );
    }

    Ok(CodexModelProviderUsageSummary {
        mode: Some("token_plan".to_string()),
        is_valid: Some(remaining_percent > 0.0),
        status: Some(if remaining_percent > 0.0 {
            "available".to_string()
        } else {
            "exhausted".to_string()
        }),
        plan_name,
        remaining: Some(remaining_percent),
        balance: Some(remaining_percent),
        unit: Some("%".to_string()),
        quota_unlimited: None,
        quota_limit: Some(100.0),
        quota_used: Some(100.0 - remaining_percent),
        quota_remaining: Some(remaining_percent),
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: None,
        model_stats_count: limits.len(),
        latency_ms,
        details,
    })
}

fn format_usage_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{:.0}", value)
    } else {
        format!("{:.4}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn push_usage_detail(
    details: &mut Vec<CodexModelProviderUsageDetail>,
    key: &str,
    label: &str,
    value: Option<String>,
) {
    let Some(value) = value else {
        return;
    };
    if value.trim().is_empty() {
        return;
    }
    details.push(CodexModelProviderUsageDetail {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
}

fn summarize_new_api_model_provider_usage(
    subscription: &serde_json::Value,
    usage: &serde_json::Value,
    token_usage: Option<&serde_json::Value>,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let raw_quota_limit = json_f64_at(subscription, &["hard_limit_usd"])
        .or_else(|| json_f64_at(subscription, &["soft_limit_usd"]))
        .or_else(|| json_f64_at(subscription, &["system_hard_limit_usd"]));
    let quota_used = json_f64_at(usage, &["total_usage"]).map(|value| value / 100.0);
    let token_data = token_usage.and_then(|value| value.get("data"));
    let quota_unlimited = token_data
        .and_then(|value| json_bool_at(value, &["unlimited_quota"]))
        .unwrap_or_else(|| {
            let hard = json_f64_at(subscription, &["hard_limit_usd"]);
            let soft = json_f64_at(subscription, &["soft_limit_usd"]);
            let system = json_f64_at(subscription, &["system_hard_limit_usd"]);
            matches!(
                (hard, soft, system),
                (Some(h), Some(s), Some(sys))
                    if (h - 100_000_000.0).abs() < f64::EPSILON
                        && (s - 100_000_000.0).abs() < f64::EPSILON
                        && (sys - 100_000_000.0).abs() < f64::EPSILON
            )
        });
    let quota_limit = if quota_unlimited {
        None
    } else {
        raw_quota_limit
    };
    let quota_remaining = match (quota_limit, quota_used) {
        (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
        _ => None,
    };
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "hardLimitUsd",
        "Hard Limit USD",
        json_f64_at(subscription, &["hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "softLimitUsd",
        "Soft Limit USD",
        json_f64_at(subscription, &["soft_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "systemHardLimitUsd",
        "System Hard Limit USD",
        json_f64_at(subscription, &["system_hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "accessUntil",
        "Access Until",
        json_i64_at(subscription, &["access_until"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "quotaUnlimited",
        "Unlimited Quota",
        Some(quota_unlimited.to_string()),
    );
    if let Some(token_data) = token_data {
        push_usage_detail(
            &mut details,
            "totalGranted",
            "Total Granted",
            json_f64_at(token_data, &["total_granted"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "totalAvailable",
            "Total Available",
            json_f64_at(token_data, &["total_available"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "expiresAt",
            "Expires At",
            json_i64_at(token_data, &["expires_at"]).map(|value| value.to_string()),
        );
        push_usage_detail(
            &mut details,
            "modelLimitsEnabled",
            "Model Limits",
            json_bool_at(token_data, &["model_limits_enabled"]).map(|value| value.to_string()),
        );
    }
    push_usage_detail(
        &mut details,
        "totalUsage",
        "Total Usage",
        json_f64_at(usage, &["total_usage"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: Some("new_api".to_string()),
        is_valid: None,
        status: None,
        plan_name: None,
        remaining: quota_remaining,
        balance: None,
        unit: Some("USD".to_string()),
        quota_unlimited: Some(quota_unlimited),
        quota_limit,
        quota_used,
        quota_remaining,
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: quota_used,
        model_stats_count: 0,
        latency_ms,
        details,
    }
}

#[tauri::command]
pub async fn codex_test_model_provider_connection(
    base_url: String,
    api_key: String,
    wire_api: Option<String>,
) -> Result<CodexLocalAccessTestResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(codex_model_provider_failure(
            "missing_api_key",
            "credential",
            "MISSING_API_KEY".to_string(),
            "add_api_key",
            None,
            None,
        ));
    }

    let url = match codex_model_provider_models_url(&base_url) {
        Ok(url) => url,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "invalid_base_url",
                "url",
                error,
                "check_base_url",
                None,
                None,
            ));
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let response = match client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "network_failed",
                "network",
                error.to_string(),
                "check_network",
                None,
                Some(format!("GET {}", url)),
            ));
        }
    };
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let suggestion = if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            "check_api_key"
        } else if status == reqwest::StatusCode::NOT_FOUND {
            "check_base_url"
        } else {
            "check_provider_status"
        };
        return Ok(codex_model_provider_failure(
            "provider_http_failed",
            "models",
            "HTTP_STATUS".to_string(),
            suggestion,
            Some(status.as_u16()),
            Some(text.chars().take(1000).collect()),
        ));
    }

    let parsed = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "response_parse_failed",
                "parse",
                error.to_string(),
                "check_openai_compatible_models",
                Some(status.as_u16()),
                Some(text.chars().take(1000).collect()),
            ));
        }
    };
    let (model_id, output) = summarize_model_provider_models(&parsed);
    let protocol = wire_api.unwrap_or_else(|| "auto".to_string());
    Ok(CodexLocalAccessTestResult {
        model_id,
        latency_ms: Some(latency_ms),
        output: output.or_else(|| Some(format!("{} connection ok", protocol))),
        failure: None,
    })
}

#[tauri::command]
pub async fn codex_model_provider_chat_test_batch(
    app: AppHandle,
    targets: Vec<CodexModelProviderChatTestTarget>,
    prompt: Option<String>,
    model: Option<String>,
    run_id: Option<String>,
) -> Result<CodexModelProviderChatTestBatchResult, String> {
    let cleaned_targets: Vec<CodexModelProviderChatTestTarget> = targets
        .into_iter()
        .filter(|target| {
            !target.provider_id.trim().is_empty() && !target.base_url.trim().is_empty()
        })
        .collect();
    if cleaned_targets.is_empty() {
        return Err("至少选择一个模型供应商".to_string());
    }
    let prompt = prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(codex_wakeup::DEFAULT_PROMPT)
        .to_string();
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let run_id = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let total = cleaned_targets.len();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    codex_local_access::register_model_provider_chat_test_run(&run_id);

    emit_model_provider_chat_test_progress(
        &app,
        &run_id,
        total,
        0,
        0,
        0,
        true,
        "batch_started",
        None,
        None,
    );

    let mut records = Vec::with_capacity(total);
    let mut success_count = 0usize;
    let mut failure_count = 0usize;
    for (index, target) in cleaned_targets.into_iter().enumerate() {
        if codex_local_access::is_model_provider_chat_test_cancelled(&run_id) {
            break;
        }
        emit_model_provider_chat_test_progress(
            &app,
            &run_id,
            total,
            index,
            success_count,
            failure_count,
            true,
            "provider_started",
            Some(&target.provider_id),
            None,
        );
        let record = run_single_model_provider_chat_test(
            &client,
            target,
            &prompt,
            model.as_deref(),
            &run_id,
        )
        .await;
        if codex_local_access::is_model_provider_chat_test_cancelled(&run_id) {
            break;
        }
        if record.success {
            success_count += 1;
        } else {
            failure_count += 1;
        }
        emit_model_provider_chat_test_progress(
            &app,
            &run_id,
            total,
            index + 1,
            success_count,
            failure_count,
            true,
            "provider_completed",
            Some(&record.provider_id),
            Some(record.clone()),
        );
        records.push(record);
    }

    let cancelled = codex_local_access::is_model_provider_chat_test_cancelled(&run_id);
    let completed = records.len();

    emit_model_provider_chat_test_progress(
        &app,
        &run_id,
        total,
        completed,
        success_count,
        failure_count,
        false,
        if cancelled {
            "batch_cancelled"
        } else {
            "batch_completed"
        },
        None,
        None,
    );
    codex_local_access::finish_model_provider_chat_test_run(&run_id);

    Ok(CodexModelProviderChatTestBatchResult {
        run_id,
        records,
        success_count,
        failure_count,
    })
}

#[tauri::command]
pub fn codex_cancel_model_provider_chat_test(run_id: String) -> Result<bool, String> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("MODEL_PROVIDER_TEST_RUN_ID_EMPTY".to_string());
    }
    Ok(codex_local_access::cancel_model_provider_chat_test_run(
        run_id,
    ))
}

#[tauri::command]
pub async fn codex_list_model_provider_models(
    base_url: String,
    api_key: String,
) -> Result<CodexModelProviderModelsResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let url = codex_model_provider_models_url(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let response = client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_MODELS_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_MODELS_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_MODELS_PARSE_FAILED: {}", e))?;
    Ok(CodexModelProviderModelsResult {
        models: list_model_provider_models(&parsed),
        latency_ms,
    })
}

#[tauri::command]
pub async fn codex_query_model_provider_usage(
    base_url: String,
    api_key: String,
    integration_type: Option<String>,
) -> Result<CodexModelProviderUsageSummary, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;

    if let Some(url) = codex_model_provider_opencode_go_usage_url(&base_url)? {
        return query_opencode_go_model_provider_usage(&client, &url, key).await;
    }

    if let Some(provider) = codex_model_provider_token_plan_provider(&base_url)? {
        return query_token_plan_model_provider_usage(&client, &base_url, key, provider).await;
    }

    if let Some(url) = codex_model_provider_deepseek_balance_url(&base_url)? {
        return query_deepseek_model_provider_balance(&client, &url, key).await;
    }

    let requested_type = integration_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match requested_type {
        Some("new_api") => query_new_api_model_provider_usage(&client, &base_url, key).await,
        Some("sub2api") => query_sub2api_model_provider_usage(&client, &base_url, key).await,
        Some(value) => Err(format!("PROVIDER_USAGE_TYPE_UNSUPPORTED: {}", value)),
        None => {
            let new_api_error =
                match query_new_api_model_provider_usage(&client, &base_url, key).await {
                    Ok(summary) => return Ok(summary),
                    Err(error) => error,
                };
            match query_sub2api_model_provider_usage(&client, &base_url, key).await {
                Ok(summary) => Ok(summary),
                Err(sub2api_error) => Err(format!(
                    "PROVIDER_USAGE_DETECT_FAILED: new_api: {}; sub2api: {}",
                    new_api_error, sub2api_error
                )),
            }
        }
    }
}

async fn query_deepseek_model_provider_balance(
    client: &reqwest::Client,
    url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let started = Instant::now();
    let response = client
        .get(url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    Ok(summarize_deepseek_balance(&parsed, latency_ms))
}

async fn query_opencode_go_model_provider_usage(
    client: &reqwest::Client,
    url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let started = Instant::now();
    let response = client
        .get(url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    if !status.is_success() {
        // Do not read an error body from this legacy path: it is unbounded and
        // could contain provider diagnostics or credential material.
        return Err(provider_usage_http_error(status));
    }
    let text = response.text().await.unwrap_or_default();
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    summarize_opencode_go_usage(&parsed, latency_ms)
}

fn provider_usage_http_error(status: reqwest::StatusCode) -> String {
    format!("PROVIDER_USAGE_HTTP_{}", status.as_u16())
}

async fn query_token_plan_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
    provider: CodexTokenPlanProvider,
) -> Result<CodexModelProviderUsageSummary, String> {
    let urls = codex_model_provider_token_plan_urls(base_url, provider)?;
    let mut last_not_found = None;
    let mut last_parse_error = None;
    for url in urls {
        let started = Instant::now();
        let request = client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json");
        let response = match provider {
            CodexTokenPlanProvider::MiniMax => request.bearer_auth(key).send().await,
            CodexTokenPlanProvider::Zhipu => {
                request
                    .header(reqwest::header::AUTHORIZATION, key)
                    .send()
                    .await
            }
        }
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND {
            last_not_found = Some(format!(
                "PROVIDER_USAGE_HTTP_404: {}",
                text.chars().take(300).collect::<String>()
            ));
            continue;
        }
        if !status.is_success() {
            return Err(format!(
                "PROVIDER_USAGE_HTTP_{}: {}",
                status.as_u16(),
                text.chars().take(300).collect::<String>()
            ));
        }
        let parsed = serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
        let summary = match provider {
            CodexTokenPlanProvider::MiniMax => {
                summarize_minimax_token_plan_usage(&parsed, latency_ms)
            }
            CodexTokenPlanProvider::Zhipu => summarize_zhipu_token_plan_usage(&parsed, latency_ms),
        };
        match summary {
            Ok(summary) => return Ok(summary),
            Err(error)
                if provider == CodexTokenPlanProvider::MiniMax
                    && error.starts_with("PROVIDER_USAGE_PARSE_FAILED") =>
            {
                last_parse_error = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_parse_error
        .or(last_not_found)
        .unwrap_or_else(|| "PROVIDER_USAGE_HTTP_404: token plan endpoint not found".to_string()))
}

async fn query_new_api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let subscription_url =
        codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/subscription")?;
    let usage_url = codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/usage")?;
    let token_usage_url = codex_model_provider_new_api_api_url(base_url, "api/usage/token/")?;
    let started = Instant::now();
    let subscription_response = client
        .get(&subscription_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let subscription_status = subscription_response.status();
    let subscription_text = subscription_response.text().await.unwrap_or_default();
    if !subscription_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            subscription_status.as_u16(),
            subscription_text.chars().take(300).collect::<String>()
        ));
    }
    let usage_response = client
        .get(&usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let usage_status = usage_response.status();
    let usage_text = usage_response.text().await.unwrap_or_default();
    if !usage_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            usage_status.as_u16(),
            usage_text.chars().take(300).collect::<String>()
        ));
    }
    let subscription = serde_json::from_str::<serde_json::Value>(&subscription_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let usage = serde_json::from_str::<serde_json::Value>(&usage_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let token_usage = match client
        .get(&token_usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            let text = response.text().await.unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&text).ok()
        }
        _ => None,
    };
    Ok(summarize_new_api_model_provider_usage(
        &subscription,
        &usage,
        token_usage.as_ref(),
        latency_ms,
    ))
}

async fn query_sub2api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let url = codex_model_provider_usage_url(base_url)?;
    let started = Instant::now();
    let response = client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    Ok(summarize_model_provider_usage(&parsed, latency_ms))
}

#[cfg(test)]
mod opencode_go_usage_tests {
    use super::*;

    #[test]
    fn opencode_go_usage_url_only_matches_the_official_go_endpoint() {
        assert_eq!(
            codex_model_provider_opencode_go_usage_url("https://opencode.ai/zen/go/v1")
                .expect("valid URL"),
            Some("https://opencode.ai/zen/go/v1/usage".to_string()),
        );
        assert_eq!(
            codex_model_provider_opencode_go_usage_url("https://example.com/zen/go/v1")
                .expect("valid URL"),
            None,
        );
    }

    #[test]
    fn opencode_go_usage_http_errors_do_not_include_upstream_body_text() {
        assert_eq!(
            provider_usage_http_error(reqwest::StatusCode::UNAUTHORIZED),
            "PROVIDER_USAGE_HTTP_401",
        );
    }

    #[test]
    fn opencode_go_usage_preserves_all_three_quota_windows() {
        let summary = summarize_opencode_go_usage(
            &serde_json::json!({"usage": {
                "rolling": {"status": "ok", "percent": 25.5, "resetsAt": "2026-08-15T04:00:00Z"},
                "weekly": {"status": "ok", "percent": 61, "resetsAt": "2026-08-18T00:00:00+00:00"},
                "monthly": {"status": "ok", "percent": 40, "resetsAt": "2026-09-01T00:00:00Z"}
            }}),
            17,
        )
        .expect("OpenCode Go usage response");
        assert_eq!(summary.mode.as_deref(), Some("opencode_go"));
        assert_eq!(summary.quota_remaining, Some(39.0));
        assert!(summary
            .details
            .iter()
            .any(|detail| detail.key == "rollingRemainingPercent" && detail.value == "74.5"));
    }
}
