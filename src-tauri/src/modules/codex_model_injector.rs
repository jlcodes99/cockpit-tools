use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const MODEL_CATALOG_FILE: &str = "cockpit-provider-model-catalog.json";
const CDP_POLL_ATTEMPTS: usize = 20;
const CDP_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize)]
struct ModelCatalog {
    #[serde(default)]
    models: Vec<ModelCatalogItem>,
}

#[derive(Debug, Deserialize)]
struct ModelCatalogItem {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpTarget {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    target_type: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelDescriptor {
    slug: String,
    display_name: String,
}

pub fn debug_port_for_codex_home(codex_home: &str) -> u16 {
    let digest = md5::compute(codex_home.trim().as_bytes());
    let value = u16::from_be_bytes([digest.0[0], digest.0[1]]);
    47_000 + (value % 10_000)
}

pub fn remote_debugging_arg(codex_home: &str) -> String {
    format!(
        "--remote-debugging-port={}",
        debug_port_for_codex_home(codex_home)
    )
}

pub fn has_remote_debugging_arg(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let trimmed = arg.trim();
        trimmed == "--remote-debugging-port" || trimmed.starts_with("--remote-debugging-port=")
    })
}

pub fn inject_for_codex_home_later(codex_home: PathBuf) {
    crate::modules::logger::log_info(&format!(
        "[Codex Model Injector] schedule injection: codex_home={}",
        codex_home.display()
    ));
    tokio::spawn(async move {
        if let Err(error) = inject_for_codex_home(&codex_home).await {
            crate::modules::logger::log_warn(&format!(
                "[Codex Model Injector] 模型列表注入失败: codex_home={}, error={}",
                codex_home.display(),
                error
            ));
        }
    });
}

async fn inject_for_codex_home(codex_home: &Path) -> Result<(), String> {
    let models = load_model_descriptors(codex_home)?;
    if models.is_empty() {
        crate::modules::logger::log_info(&format!(
            "[Codex Model Injector] skip injection: codex_home={}, reason=no_visible_models",
            codex_home.display()
        ));
        return Ok(());
    }

    let codex_home_text = codex_home.to_string_lossy().to_string();
    let port = debug_port_for_codex_home(&codex_home_text);
    let script = build_injection_script(&models)?;
    crate::modules::logger::log_info(&format!(
        "[Codex Model Injector] start injection: codex_home={}, port={}, models={}",
        codex_home.display(),
        port,
        models
            .iter()
            .map(|item| item.slug.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));

    let ws_urls = wait_for_page_websockets(port).await?;
    for (index, ws_url) in ws_urls.iter().enumerate() {
        crate::modules::logger::log_info(&format!(
            "[Codex Model Injector] installing script: target_index={}, ws={}",
            index, ws_url
        ));
        install_script(ws_url, &script).await?;
        crate::modules::logger::log_info(&format!(
            "[Codex Model Injector] script installed: target_index={}, ws={}",
            index, ws_url
        ));
    }
    crate::modules::logger::log_info(&format!(
        "[Codex Model Injector] 模型列表注入完成: codex_home={}, port={}, targets={}, models={}",
        codex_home.display(),
        port,
        ws_urls.len(),
        models
            .iter()
            .map(|item| item.slug.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    Ok(())
}

fn load_model_descriptors(codex_home: &Path) -> Result<Vec<ModelDescriptor>, String> {
    let path = codex_home.join(MODEL_CATALOG_FILE);
    if !path.exists() {
        crate::modules::logger::log_info(&format!(
            "[Codex Model Injector] model catalog missing: codex_home={}, path={}",
            codex_home.display(),
            path.display()
        ));
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取模型 catalog 失败 ({}): {}", path.display(), error))?;
    let catalog: ModelCatalog = serde_json::from_str(&content)
        .map_err(|error| format!("解析模型 catalog 失败 ({}): {}", path.display(), error))?;
    let mut models = Vec::new();
    let mut hidden_count = 0usize;
    let mut blank_count = 0usize;
    for item in catalog.models {
        let slug = item.slug.trim();
        if slug.is_empty() {
            blank_count += 1;
            continue;
        }
        if item.visibility.as_deref() == Some("hide") {
            hidden_count += 1;
            continue;
        }
        models.push(ModelDescriptor {
            slug: slug.to_string(),
            display_name: item
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(slug)
                .to_string(),
        });
    }
    crate::modules::logger::log_info(&format!(
        "[Codex Model Injector] model catalog loaded: path={}, bytes={}, visible={}, hidden={}, blank={}, models={}",
        path.display(),
        content.len(),
        models.len(),
        hidden_count,
        blank_count,
        models
            .iter()
            .map(|item| item.slug.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    Ok(models)
}

async fn wait_for_page_websockets(port: u16) -> Result<Vec<String>, String> {
    let mut last_error = String::new();
    for attempt in 1..=CDP_POLL_ATTEMPTS {
        match fetch_page_websockets(port).await {
            Ok(urls) => {
                crate::modules::logger::log_info(&format!(
                    "[Codex Model Injector] CDP page targets ready: port={}, attempt={}, targets={}",
                    port,
                    attempt,
                    urls.len()
                ));
                return Ok(urls);
            }
            Err(error) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Model Injector] CDP poll failed: port={}, attempt={}/{}, error={}",
                    port, attempt, CDP_POLL_ATTEMPTS, error
                ));
                last_error = error;
                sleep(CDP_POLL_INTERVAL).await;
            }
        }
    }
    Err(last_error)
}

async fn fetch_page_websockets(port: u16) -> Result<Vec<String>, String> {
    let url = format!("http://127.0.0.1:{}/json/list", port);
    let response = reqwest::get(&url)
        .await
        .map_err(|error| format!("连接 CDP 失败 ({}): {}", url, error))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("<read body failed: {}>", error));
        return Err(format!(
            "CDP 返回非成功状态 ({}): {}",
            status,
            summarize_for_log(&body, 300)
        ));
    }
    let targets = response
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|error| format!("解析 CDP target 失败: {}", error))?;
    crate::modules::logger::log_info(&format!(
        "[Codex Model Injector] CDP targets: port={}, targets={}",
        port,
        targets
            .iter()
            .map(|target| {
                let kind = target
                    .target_type
                    .as_deref()
                    .or(target.r#type.as_deref())
                    .unwrap_or_default();
                format!(
                    "{{type={},title={},url={},has_ws={}}}",
                    kind,
                    target.title.as_deref().unwrap_or_default(),
                    target.url.as_deref().unwrap_or_default(),
                    target
                        .web_socket_debugger_url
                        .as_deref()
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    ));
    let mut urls = targets
        .iter()
        .filter(|target| {
            let kind = target
                .target_type
                .as_deref()
                .or(target.r#type.as_deref())
                .unwrap_or_default();
            kind == "page"
        })
        .filter_map(|target| {
            target
                .web_socket_debugger_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    if urls.is_empty() {
        Err("未找到可注入的 Codex CDP 页面 target".to_string())
    } else {
        Ok(urls)
    }
}

async fn install_script(ws_url: &str, script: &str) -> Result<(), String> {
    let (mut ws, _) = connect_async(ws_url)
        .await
        .map_err(|error| format!("连接 CDP websocket 失败: {}", error))?;
    send_cdp(&mut ws, 1, "Runtime.enable", json!({})).await?;
    send_cdp(
        &mut ws,
        2,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": script }),
    )
    .await?;
    send_cdp(
        &mut ws,
        3,
        "Runtime.evaluate",
        json!({ "expression": script, "awaitPromise": true }),
    )
    .await?;
    observe_cdp_events(&mut ws, ws_url, Duration::from_secs(180)).await;
    let _ = ws.close(None).await;
    Ok(())
}

async fn send_cdp<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let payload = json!({
        "id": id,
        "method": method,
        "params": params,
    });
    ws.send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|error| format!("发送 CDP 请求失败: {}", error))?;
    while let Some(message) = ws.next().await {
        let message = message.map_err(|error| format!("读取 CDP 响应失败: {}", error))?;
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            _ => continue,
        };
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        log_cdp_event(&parsed, method);
        if parsed.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = parsed.get("error") {
            return Err(format!("CDP 请求失败 ({}): {}", method, error));
        }
        return Ok(());
    }
    Err(format!("CDP 连接已关闭: {}", method))
}

async fn observe_cdp_events<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    ws_url: &str,
    duration: Duration,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let started = std::time::Instant::now();
    while started.elapsed() < duration {
        let remaining = duration.saturating_sub(started.elapsed());
        match timeout(remaining.min(Duration::from_millis(500)), ws.next()).await {
            Ok(Some(Ok(message))) => {
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    _ => continue,
                };
                let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                log_cdp_event(&parsed, "observe");
            }
            Ok(Some(Err(error))) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Model Injector] CDP observe failed: ws={}, error={}",
                    ws_url, error
                ));
                return;
            }
            Ok(None) => return,
            Err(_) => {}
        }
    }
}

fn log_cdp_event(value: &Value, context: &str) {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match method {
        "Runtime.consoleAPICalled" => {
            let params = value.get("params").unwrap_or(&Value::Null);
            let level = params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("console");
            let text = params
                .get("args")
                .and_then(Value::as_array)
                .map(|args| {
                    args.iter()
                        .filter_map(|arg| {
                            arg.get("value")
                                .and_then(Value::as_str)
                                .or_else(|| arg.get("description").and_then(Value::as_str))
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if text.contains("Cockpit") || text.contains("features") || text.contains("model") {
                crate::modules::logger::log_info(&format!(
                    "[Codex Model Injector][console] context={}, level={}, text={}",
                    context,
                    level,
                    summarize_for_log(&text, 800)
                ));
            }
        }
        "Runtime.exceptionThrown" => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Model Injector][exception] context={}, payload={}",
                context,
                summarize_for_log(&value.to_string(), 1200)
            ));
        }
        _ => {}
    }
}

fn summarize_for_log(value: &str, max_len: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_len {
        return normalized;
    }
    let mut output = normalized.chars().take(max_len).collect::<String>();
    output.push_str("...");
    output
}

fn build_injection_script(models: &[ModelDescriptor]) -> Result<String, String> {
    let model_values = models
        .iter()
        .map(|model| json!({ "slug": model.slug, "displayName": model.display_name }))
        .collect::<Vec<_>>();
    let model_json = serde_json::to_string(&model_values)
        .map_err(|error| format!("序列化模型失败: {}", error))?;
    Ok(format!(
        r#"
(() => {{
  const modelItems = {model_json};
  const modelNames = Array.from(new Set(modelItems.map((item) => item.slug).filter(Boolean)));
  const defaultModel = modelNames[0] || "";
  if (!modelNames.length) return;
  if (window.__cockpitCodexModelInjectorVersion === "1") return;
  window.__cockpitCodexModelInjectorVersion = "1";
  try {{ console.info("[Cockpit Codex] model injector active", modelNames); }} catch {{}}

  function descriptor(name) {{
    const item = modelItems.find((entry) => entry.slug === name) || {{ slug: name, displayName: name }};
    return {{
      model: name,
      id: name,
      slug: name,
      name: item.displayName || name,
      displayName: item.displayName || name,
      display_name: item.displayName || name,
      description: "Custom provider model",
      hidden: false,
      isDefault: name === defaultModel,
      defaultReasoningEffort: "medium",
      supportedReasoningEfforts: ["minimal", "low", "medium", "high", "xhigh"].map((reasoningEffort) => ({{ reasoningEffort, description: reasoningEffort + " effort" }})),
    }};
  }}

  function patchModelArray(value, allowEmpty) {{
    if (!Array.isArray(value)) return false;
    if (!allowEmpty && value.length === 0) return false;
    if (value.length > 0 && !value.every((item) => item && typeof item === "object" && typeof item.model === "string")) return false;
    const existing = new Set(value.map((item) => item.model));
    let changed = false;
    for (const item of value) {{
      if (modelNames.includes(item.model) && item.hidden !== false) {{
        item.hidden = false;
        changed = true;
      }}
    }}
    for (const name of modelNames) {{
      if (!existing.has(name)) {{
        value.push(descriptor(name));
        changed = true;
      }}
    }}
    return changed;
  }}

  function patchContainer(value) {{
    if (!value || typeof value !== "object") return false;
    let changed = false;
    if (patchModelArray(value, true)) changed = true;
    if (patchModelArray(value.models, "defaultModel" in value || "availableModels" in value)) changed = true;
    if (patchModelArray(value.data, false)) changed = true;
    if (patchModelArray(value.result, false)) changed = true;
    if (patchModelArray(value.result?.data, false)) changed = true;
    if (patchModelArray(value.result?.models, false)) changed = true;
    if (patchModelArray(value.message?.result?.data, false)) changed = true;
    if (patchModelArray(value.message?.result?.models, false)) changed = true;
    for (const key of ["availableModels", "available_models"]) {{
      const current = value[key];
      if (Array.isArray(current)) {{
        if (!current.every((item) => typeof item === "string")) continue;
        for (const name of modelNames) {{
          if (!current.includes(name)) {{
            current.push(name);
            changed = true;
          }}
        }}
      }} else if (current instanceof Set) {{
        for (const name of modelNames) {{
          if (!current.has(name)) {{
            current.add(name);
            changed = true;
          }}
        }}
      }}
    }}
    for (const key of ["hiddenModels", "hidden_models"]) {{
      if (Array.isArray(value[key])) {{
        const before = value[key].length;
        value[key] = value[key].filter((name) => !modelNames.includes(name));
        if (value[key].length !== before) changed = true;
      }}
    }}
    if (("models" in value || "availableModels" in value || "available_models" in value) && value.defaultModel == null && defaultModel) {{
      value.defaultModel = descriptor(defaultModel);
      changed = true;
    }}
    return changed;
  }}

  function summarizeForLog(value, depth = 0) {{
    if (value == null) return value;
    if (typeof value === "string") return value.length > 160 ? value.slice(0, 160) + "..." : value;
    if (typeof value === "number" || typeof value === "boolean") return value;
    if (Array.isArray(value)) return depth >= 2 ? "Array(" + value.length + ")" : value.slice(0, 8).map((item) => summarizeForLog(item, depth + 1));
    if (typeof value === "object") {{
      if (depth >= 2) return "Object(" + Object.keys(value).join(",") + ")";
      const output = {{}};
      for (const key of Object.keys(value).slice(0, 20)) {{
        if (/token|key|authorization|secret|password/i.test(key)) {{
          output[key] = "<redacted>";
        }} else {{
          output[key] = summarizeForLog(value[key], depth + 1);
        }}
      }}
      return output;
    }}
    return typeof value;
  }}
  function stringifyForLog(value) {{
    try {{ return JSON.stringify(summarizeForLog(value)); }} catch (error) {{ return String(value); }}
  }}

  function patchStatsigConfig(config) {{
    const value = config?.value;
    if (!value || typeof value !== "object") return config;
    const available = Array.isArray(value.available_models) ? [...value.available_models] : [];
    for (const name of modelNames) if (!available.includes(name)) available.push(name);
    const nextValue = {{ ...value, available_models: available, default_model: defaultModel || value.default_model }};
    try {{ config.value = nextValue; return config; }} catch {{ return {{ ...config, value: nextValue }}; }}
  }}

  function patchStatsig() {{
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return;
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null, ...(root.instances && typeof root.instances === "object" ? Object.values(root.instances) : [])].filter(Boolean);
    for (const client of clients) {{
      if (typeof client.getDynamicConfig !== "function" || client.__cockpitModelPatched) continue;
      const original = client.getDynamicConfig.bind(client);
      client.getDynamicConfig = (name, options) => patchStatsigConfig(original(name, options));
      client.__cockpitModelPatched = true;
      try {{ patchStatsigConfig(client.getDynamicConfig("107580212", {{ disableExposureLog: true }})); }} catch {{}}
    }}
  }}

  const modulePromises = new Map();
  function assetUrl(namePart) {{
    const urls = [
      ...Array.from(document.scripts || []).map((script) => script.src),
      ...Array.from(document.querySelectorAll("link[href]") || []).map((link) => link.href),
      ...performance.getEntriesByType("resource").map((entry) => entry.name),
    ].filter(Boolean);
    return urls.find((url) => url.includes("/assets/") && url.includes(namePart) && url.split("?")[0].endsWith(".js")) || "";
  }}
  async function loadModule(namePart) {{
    if (!modulePromises.has(namePart)) {{
      modulePromises.set(namePart, Promise.resolve().then(async () => {{
        const url = assetUrl(namePart);
        if (!url) throw new Error("missing asset " + namePart);
        return await import(url);
      }}).catch((error) => {{ modulePromises.delete(namePart); throw error; }}));
    }}
    return await modulePromises.get(namePart);
  }}
  function patchClient(client) {{
    if (!client || typeof client.sendRequest !== "function" || client.__cockpitModelRequestPatched) return false;
    const original = client.sendRequest.bind(client);
    client.sendRequest = async function patchedSendRequest(method, params, options) {{
      const actualMethod = method === "send-cli-request-for-host" && params?.method ? String(params.method) : String(method || "");
      const shouldTrace = actualMethod === "list-models-for-host" || /thread|session|conversation|task|prompt/i.test(actualMethod);
      if (shouldTrace) {{
        try {{ console.info("[Cockpit Codex] app-server request", actualMethod, stringifyForLog(params)); }} catch {{}}
      }}
      try {{
        const result = await original(method, params, options);
        if (actualMethod === "list-models-for-host") {{
          patchContainer(result);
        }}
        if (shouldTrace) {{
          try {{ console.info("[Cockpit Codex] app-server response", actualMethod, stringifyForLog(result)); }} catch {{}}
        }}
        return result;
      }} catch (error) {{
        if (shouldTrace || String(error?.message || error || "").includes("features")) {{
          try {{ console.warn("[Cockpit Codex] app-server error", actualMethod, stringifyForLog(params), error?.message || String(error), error?.stack || ""); }} catch {{}}
        }}
        throw error;
      }}
    }};
    client.__cockpitModelRequestPatched = true;
    return true;
  }}
  async function patchAppServerClient() {{
    try {{
      const module = await loadModule("app-server-manager-signals-");
      for (const candidate of Object.values(module).filter((value) => value && typeof value === "object")) {{
        patchClient(candidate);
        if (typeof candidate.sendRequest !== "function" && typeof candidate.get === "function") {{
          try {{ patchClient(candidate.get()); }} catch {{}}
        }}
      }}
    }} catch {{}}
  }}

  function tick() {{
    patchStatsig();
    patchAppServerClient();
  }}
  tick();
  setTimeout(tick, 300);
  setTimeout(tick, 1000);
  setInterval(tick, 2500);
}})();
"#
    ))
}
