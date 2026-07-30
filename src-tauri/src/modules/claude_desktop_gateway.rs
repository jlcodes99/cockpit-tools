use crate::models::claude::{ClaudeAccount, ClaudeDesktopGatewayModelMapping};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

#[derive(Debug, Clone)]
pub struct ClaudeDesktopLocalGatewayEndpoint {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone)]
struct GatewayDesktopModel {
    name: String,
    label_override: Option<String>,
    supports_1m: Option<bool>,
}

#[derive(Clone)]
struct GatewayConfig {
    account_id: String,
    upstream_base_url: String,
    upstream_api_key: String,
    upstream_auth_scheme: String,
    mappings: BTreeMap<String, String>,
    desktop_models: Vec<GatewayDesktopModel>,
    fingerprint: String,
}

struct GatewayRuntime {
    endpoint: ClaudeDesktopLocalGatewayEndpoint,
    fingerprint: String,
    stop_tx: mpsc::Sender<()>,
    handle: Option<JoinHandle<()>>,
}

static GATEWAY_RUNTIMES: LazyLock<Mutex<HashMap<String, GatewayRuntime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn is_claude_desktop_model(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.starts_with("claude-") || normalized.starts_with("anthropic/claude-")
}

pub fn infer_anthropic_family_tier(model: &str) -> Option<&'static str> {
    let normalized = model.trim().to_ascii_lowercase();
    ["haiku", "sonnet", "opus", "fable", "mythos"]
        .into_iter()
        .find(|tier| {
            normalized
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .any(|part| part == *tier)
        })
}

fn normalize_anthropic_family_tier(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "haiku" | "sonnet" | "opus" | "fable" | "mythos"
            )
        })
        .map(|value| value.to_ascii_lowercase())
}

pub fn normalize_connection_mode(value: Option<&str>) -> String {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace('-', "_"))
        .as_deref()
    {
        Some("local_mapping") | Some("mapping") | Some("local") => "local_mapping".to_string(),
        _ => "direct".to_string(),
    }
}

pub fn normalize_model_mappings(
    mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
) -> Option<Vec<ClaudeDesktopGatewayModelMapping>> {
    let mut seen = BTreeMap::<String, ClaudeDesktopGatewayModelMapping>::new();
    for mapping in mappings.into_iter().flatten() {
        let desktop_model = mapping.desktop_model.trim().to_string();
        let upstream_model = mapping.upstream_model.trim().to_string();
        if desktop_model.is_empty() || upstream_model.is_empty() {
            continue;
        }
        let key = desktop_model.to_ascii_lowercase();
        let label_override = mapping
            .label_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let supports_1m = mapping.supports_1m.filter(|value| *value);
        let anthropic_family_tier = normalize_anthropic_family_tier(
            mapping.anthropic_family_tier.as_deref(),
        )
        .or_else(|| infer_anthropic_family_tier(&desktop_model).map(str::to_string));
        seen.entry(key)
            .or_insert_with(|| ClaudeDesktopGatewayModelMapping {
                desktop_model,
                upstream_model,
                label_override,
                anthropic_family_tier,
                supports_1m,
            });
    }
    (!seen.is_empty()).then(|| seen.into_values().collect())
}

pub fn build_default_model_mappings(
    desktop_models: &[String],
    upstream_models: &[String],
) -> Vec<ClaudeDesktopGatewayModelMapping> {
    let fallback = upstream_models
        .iter()
        .find(|model| !model.trim().is_empty())
        .cloned()
        .unwrap_or_default();
    desktop_models
        .iter()
        .filter_map(|desktop_model| {
            let desktop_model = desktop_model.trim();
            if desktop_model.is_empty() || fallback.is_empty() {
                return None;
            }
            Some(ClaudeDesktopGatewayModelMapping {
                desktop_model: desktop_model.to_string(),
                upstream_model: fallback.clone(),
                label_override: None,
                anthropic_family_tier: infer_anthropic_family_tier(desktop_model)
                    .map(str::to_string),
                supports_1m: None,
            })
        })
        .collect()
}

pub fn ensure_gateway_for_account(
    account: &ClaudeAccount,
) -> Result<ClaudeDesktopLocalGatewayEndpoint, String> {
    let config = GatewayConfig::from_account(account)?;
    let mut runtimes = GATEWAY_RUNTIMES
        .lock()
        .map_err(|_| "Claude 本地网关锁已损坏".to_string())?;
    if let Some(runtime) = runtimes.get(&config.account_id) {
        if runtime.fingerprint == config.fingerprint {
            return Ok(runtime.endpoint.clone());
        }
    }
    if let Some(runtime) = runtimes.remove(&config.account_id) {
        stop_runtime(runtime);
    }

    let server =
        Server::http("127.0.0.1:0").map_err(|e| format!("启动 Claude 本地网关失败: {}", e))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "Claude 本地网关监听地址不可用".to_string())?
        .port();
    let local_api_key = generate_local_api_key();
    let endpoint = ClaudeDesktopLocalGatewayEndpoint {
        base_url: format!("http://127.0.0.1:{}", port),
        api_key: local_api_key.clone(),
    };
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread_config = Arc::new(config.clone());
    let handle = thread::Builder::new()
        .name(format!("claude-desktop-gateway-{}", config.account_id))
        .spawn(move || run_gateway(server, thread_config, local_api_key, stop_rx))
        .map_err(|e| format!("启动 Claude 本地网关线程失败: {}", e))?;

    runtimes.insert(
        config.account_id.clone(),
        GatewayRuntime {
            endpoint: endpoint.clone(),
            fingerprint: config.fingerprint,
            stop_tx,
            handle: Some(handle),
        },
    );
    Ok(endpoint)
}

fn stop_runtime(mut runtime: GatewayRuntime) {
    let _ = runtime.stop_tx.send(());
    if let Some(handle) = runtime.handle.take() {
        let _ = handle.join();
    }
}

fn generate_local_api_key() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("ag-claude-gw-{}", URL_SAFE_NO_PAD.encode(bytes))
}

impl GatewayConfig {
    fn from_account(account: &ClaudeAccount) -> Result<Self, String> {
        let account_id = account.id.trim().to_string();
        if account_id.is_empty() {
            return Err("Claude Gateway 账号 ID 为空".to_string());
        }
        let upstream_base_url = account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Claude Gateway 账号缺少 Base URL".to_string())?
            .trim_end_matches('/')
            .to_string();
        let upstream_api_key = account
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Claude Gateway 账号缺少 API Key".to_string())?
            .to_string();
        let upstream_auth_scheme = account
            .desktop_gateway_auth_scheme
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("bearer")
            .to_ascii_lowercase();
        let mappings = normalize_model_mappings(account.desktop_gateway_model_mappings.clone())
            .ok_or_else(|| "Claude Gateway 映射关系为空".to_string())?;
        let desktop_models = mappings
            .iter()
            .map(|mapping| GatewayDesktopModel {
                name: mapping.desktop_model.clone(),
                label_override: mapping.label_override.clone(),
                supports_1m: mapping.supports_1m,
            })
            .collect::<Vec<_>>();
        let mappings = mappings
            .into_iter()
            .map(|mapping| {
                (
                    mapping.desktop_model.to_ascii_lowercase(),
                    mapping.upstream_model,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let fingerprint = json!({
            "baseUrl": upstream_base_url,
            "authScheme": upstream_auth_scheme,
            "apiKeyHash": format!("{:x}", md5::compute(upstream_api_key.as_bytes())),
            "mappings": mappings,
            "desktopModels": desktop_models.iter().map(|model| {
                json!({
                    "name": model.name.clone(),
                    "labelOverride": model.label_override.clone(),
                    "supports1m": model.supports_1m,
                })
            }).collect::<Vec<_>>(),
        })
        .to_string();
        Ok(Self {
            account_id,
            upstream_base_url,
            upstream_api_key,
            upstream_auth_scheme,
            mappings,
            desktop_models,
            fingerprint,
        })
    }
}

fn run_gateway(
    server: Server,
    config: Arc<GatewayConfig>,
    local_api_key: String,
    stop_rx: mpsc::Receiver<()>,
) {
    let client = match Client::builder()
        .timeout(Duration::from_secs(300))
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(request)) => handle_request(request, &client, &config, &local_api_key),
            Ok(None) => {}
            Err(_) => break,
        }
    }
}

fn handle_request(
    mut request: Request,
    client: &Client,
    config: &GatewayConfig,
    local_api_key: &str,
) {
    if *request.method() == Method::Options {
        let _ = request.respond(empty_response(204));
        return;
    }
    if !is_authorized(&request, local_api_key) {
        let _ = request.respond(json_response(
            401,
            json!({ "error": { "message": "Unauthorized" } }),
        ));
        return;
    }
    let url = request.url().to_string();
    if *request.method() == Method::Get && normalize_path_without_query(&url) == "/v1/models" {
        let data = config
            .desktop_models
            .iter()
            .map(|model| {
                let mut value = json!({
                    "id": model.name.clone(),
                    "object": "model",
                    "created": 0,
                    "owned_by": "anthropic",
                });
                if let Some(label_override) = model
                    .label_override
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    value["display_name"] = Value::String(label_override.to_string());
                }
                if model.supports_1m.unwrap_or(false) {
                    value["supports1m"] = Value::Bool(true);
                }
                value
            })
            .collect::<Vec<_>>();
        let _ = request.respond(json_response(
            200,
            json!({ "object": "list", "data": data }),
        ));
        return;
    }
    let mut body = Vec::new();
    if request.as_reader().read_to_end(&mut body).is_err() {
        let _ = request.respond(json_response(
            400,
            json!({ "error": { "message": "Invalid request body" } }),
        ));
        return;
    }
    let mapped_body = map_request_body(&body, config).unwrap_or(body);
    match forward_request(request, client, config, mapped_body) {
        Ok(response) => {
            let _ = response;
        }
        Err(error) => {
            let _ = error.respond(json_response(
                502,
                json!({ "error": { "message": "Claude local gateway upstream request failed" } }),
            ));
        }
    }
}

fn forward_request(
    request: Request,
    client: &Client,
    config: &GatewayConfig,
    body: Vec<u8>,
) -> Result<(), Request> {
    let method = match *request.method() {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Patch => reqwest::Method::PATCH,
        Method::Delete => reqwest::Method::DELETE,
        _ => reqwest::Method::POST,
    };
    let url = match build_upstream_url(&config.upstream_base_url, request.url()) {
        Ok(url) => url,
        Err(_) => {
            let _ = request.respond(json_response(
                400,
                json!({ "error": { "message": "Invalid upstream URL" } }),
            ));
            return Ok(());
        }
    };
    let headers = request.headers().to_vec();
    let mut builder = client.request(method, url);
    for header in headers {
        let name = header.field.as_str().as_str().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "host"
                | "authorization"
                | "x-api-key"
                | "content-length"
                | "connection"
                | "proxy-authorization"
        ) {
            continue;
        }
        builder = builder.header(header.field.as_str().as_str(), header.value.as_str());
    }
    if config
        .upstream_auth_scheme
        .eq_ignore_ascii_case("x-api-key")
    {
        builder = builder.header("x-api-key", &config.upstream_api_key);
    } else {
        builder = builder.bearer_auth(&config.upstream_api_key);
    }
    let upstream_response = match builder.body(body).send() {
        Ok(response) => response,
        Err(_) => return Err(request),
    };
    let status = upstream_response.status().as_u16();
    let content_length = upstream_response
        .content_length()
        .and_then(|value| value.try_into().ok());
    let response_headers = upstream_response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.as_str().to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "content-length" | "connection" | "transfer-encoding" | "content-encoding"
            ) {
                return None;
            }
            Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()).ok()
        })
        .collect::<Vec<_>>();
    let response = Response::new(
        StatusCode(status),
        response_headers,
        upstream_response,
        content_length,
        None,
    )
    .with_chunked_threshold(1024);
    let _ = request.respond(response);
    Ok(())
}

fn map_request_body(body: &[u8], config: &GatewayConfig) -> Option<Vec<u8>> {
    if body.is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let model = value.get("model").and_then(Value::as_str)?.trim();
    let upstream_model = config.mappings.get(&model.to_ascii_lowercase())?;
    value["model"] = Value::String(upstream_model.clone());
    serde_json::to_vec(&value).ok()
}

fn is_authorized(request: &Request, local_api_key: &str) -> bool {
    request.headers().iter().any(|header| {
        let name = header.field.as_str().as_str().to_ascii_lowercase();
        let value = header.value.as_str().trim();
        (name == "x-api-key" && value == local_api_key)
            || (name == "authorization"
                && value
                    .strip_prefix("Bearer ")
                    .is_some_and(|token| token.trim() == local_api_key))
    })
}

fn normalize_path_without_query(url: &str) -> String {
    url.split('?')
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

fn build_upstream_url(base_url: &str, request_url: &str) -> Result<String, String> {
    let mut url = Url::parse(base_url.trim_end_matches('/')).map_err(|e| e.to_string())?;
    let base_path = url.path().trim_end_matches('/');
    let request_path = request_url
        .split('?')
        .next()
        .unwrap_or(request_url)
        .trim_start_matches('/');
    let request_path = if base_path.ends_with("/v1") && request_path.starts_with("v1/") {
        &request_path[3..]
    } else {
        request_path
    };
    let next_path = if base_path.is_empty() || base_path == "/" {
        format!("/{}", request_path)
    } else {
        format!("{}/{}", base_path, request_path)
    };
    url.set_path(&next_path);
    url.set_query(request_url.split_once('?').map(|(_, query)| query));
    Ok(url.to_string())
}

fn json_response(status: u16, value: Value) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let headers = vec![
        Header::from_bytes("content-type", "application/json").unwrap(),
        Header::from_bytes("access-control-allow-origin", "*").unwrap(),
        Header::from_bytes(
            "access-control-allow-headers",
            "authorization,x-api-key,content-type,anthropic-version,anthropic-beta",
        )
        .unwrap(),
    ];
    Response::new(StatusCode(status), headers, Cursor::new(body), None, None)
}

fn empty_response(status: u16) -> Response<Cursor<Vec<u8>>> {
    let headers = vec![
        Header::from_bytes("access-control-allow-origin", "*").unwrap(),
        Header::from_bytes(
            "access-control-allow-headers",
            "authorization,x-api-key,content-type,anthropic-version,anthropic-beta",
        )
        .unwrap(),
    ];
    Response::new(
        StatusCode(status),
        headers,
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_base_url_appends_messages_path() {
        let server = Server::http("127.0.0.1:0").expect("mock upstream should start");
        let port = server
            .server_addr()
            .to_ip()
            .expect("mock upstream should use an IP address")
            .port();
        let captured = thread::spawn(move || {
            let request = server
                .recv()
                .expect("mock upstream should receive a request");
            let path = request.url().to_string();
            request
                .respond(json_response(200, json!({ "type": "message" })))
                .expect("mock upstream should send a response");
            path
        });

        let base_url = format!("http://127.0.0.1:{}/anthropic", port);
        let url = build_upstream_url(&base_url, "/v1/messages")
            .expect("upstream messages URL should be valid");
        let response = Client::builder()
            .no_proxy()
            .build()
            .expect("HTTP client should build")
            .post(url)
            .body("{}")
            .send()
            .expect("captured request should succeed");

        assert!(response.status().is_success());
        assert_eq!(
            captured.join().expect("capture thread should finish"),
            "/anthropic/v1/messages"
        );
    }
}
