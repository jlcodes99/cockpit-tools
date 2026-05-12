use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use url::Url;

use crate::modules::{floating_card_window, logger};

pub const EXTERNAL_PROVIDER_IMPORT_EVENT: &str = "external:provider-import";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderImportPayload {
    pub provider_id: String,
    pub page: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    pub auto_import: bool,
    #[serde(default)]
    pub activate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
}

static PENDING_EXTERNAL_PROVIDER_IMPORT: LazyLock<Mutex<Option<ExternalProviderImportPayload>>> =
    LazyLock::new(|| Mutex::new(None));

fn normalize_lookup_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn parse_boolean_like(value: Option<&String>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

fn resolve_provider_and_page(value: &str) -> Option<(&'static str, &'static str)> {
    let normalized = normalize_lookup_key(value);
    match normalized.as_str() {
        "antigravity" | "overview" | "accounts" => Some(("antigravity", "overview")),
        "codex" => Some(("codex", "codex")),
        "github_copilot" | "githubcopilot" | "ghcp" => Some(("github-copilot", "github-copilot")),
        "windsurf" => Some(("windsurf", "windsurf")),
        "kiro" => Some(("kiro", "kiro")),
        "cursor" => Some(("cursor", "cursor")),
        "gemini" => Some(("gemini", "gemini")),
        "codebuddy" => Some(("codebuddy", "codebuddy")),
        "codebuddy_cn" | "codebuddycn" => Some(("codebuddy_cn", "codebuddy-cn")),
        "qoder" => Some(("qoder", "qoder")),
        "trae" => Some(("trae", "trae")),
        "workbuddy" => Some(("workbuddy", "workbuddy")),
        "zed" => Some(("zed", "zed")),
        _ => None,
    }
}

fn is_supported_scheme(scheme: &str) -> bool {
    matches!(scheme, "cockpit-tools" | "cockpittools")
}

fn is_import_action(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if matches!(
        host.as_str(),
        "import" | "provider-import" | "account-import"
    ) {
        return true;
    }

    let path = url.path().trim_matches('/').to_ascii_lowercase();
    matches!(
        path.as_str(),
        "import" | "provider-import" | "account-import"
    )
}

fn parse_query_map(url: &Url) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in url.query_pairs() {
        let normalized_key = normalize_lookup_key(key.as_ref());
        if normalized_key.is_empty() {
            continue;
        }
        map.entry(normalized_key)
            .or_insert_with(|| value.into_owned());
    }
    map
}

fn summarize_candidate(candidate: &str) -> String {
    let Ok(parsed) = Url::parse(candidate) else {
        let preview = candidate.chars().take(140).collect::<String>();
        return format!("raw='{}'", preview);
    };

    let mut keys: Vec<String> = Vec::new();
    let mut token_len: Option<usize> = None;
    for (key, value) in parsed.query_pairs() {
        let normalized_key = normalize_lookup_key(key.as_ref());
        if normalized_key.is_empty() {
            continue;
        }
        if matches!(
            normalized_key.as_str(),
            "token"
                | "import_token"
                | "importtoken"
                | "payload"
                | "import_payload"
                | "importpayload"
                | "import_url"
                | "importurl"
        ) && token_len.is_none()
        {
            token_len = Some(value.trim().len());
        }
        keys.push(normalized_key);
    }

    format!(
        "{}://{}{}?keys={:?},token_len={}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default(),
        parsed.path(),
        keys,
        token_len
            .map(|len| len.to_string())
            .unwrap_or_else(|| "-".to_string())
    )
}

fn parse_external_import_url_with_reason(
    raw_url: &str,
) -> Result<ExternalProviderImportPayload, String> {
    let parsed = Url::parse(raw_url).map_err(|err| format!("URL 解析失败: {}", err))?;
    if !is_supported_scheme(parsed.scheme()) {
        return Err(format!("协议不支持: {}", parsed.scheme()));
    }
    if !is_import_action(&parsed) {
        return Err(format!(
            "动作不支持: host='{}', path='{}'",
            parsed.host_str().unwrap_or_default(),
            parsed.path()
        ));
    }

    let query = parse_query_map(&parsed);
    let provider_raw = query
        .get("provider")
        .or_else(|| query.get("provider_id"))
        .or_else(|| query.get("providerid"))
        .or_else(|| query.get("platform"))
        .or_else(|| query.get("platform_id"))
        .or_else(|| query.get("platformid"))
        .or_else(|| query.get("target"))
        .or_else(|| query.get("page"))
        .ok_or_else(|| "缺少平台参数（provider/platform/target/page）".to_string())?;
    let (provider_id, page) = resolve_provider_and_page(provider_raw)
        .ok_or_else(|| format!("平台值不支持: {}", provider_raw))?;

    let token = query
        .get("token")
        .or_else(|| query.get("import_token"))
        .or_else(|| query.get("importtoken"))
        .or_else(|| query.get("payload"))
        .or_else(|| query.get("import_payload"))
        .or_else(|| query.get("importpayload"))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let import_url = query
        .get("import_url")
        .or_else(|| query.get("importurl"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if token.is_empty() && import_url.is_none() {
        return Err(
            "缺少内容参数（token/import_token/payload/import_payload/import_url）".to_string(),
        );
    }
    if !token.is_empty() && import_url.is_some() {
        return Err("内容参数不能同时包含 token/payload 与 import_url".to_string());
    }
    let token = token.trim().to_string();
    let min_app_version = query
        .get("min_app_version")
        .or_else(|| query.get("minappversion"))
        .map(|value| {
            value
                .trim()
                .trim_start_matches(|ch| ch == 'v' || ch == 'V')
                .to_string()
        })
        .filter(|value| !value.is_empty());

    let auto_import = parse_boolean_like(
        query
            .get("auto_import")
            .or_else(|| query.get("autoimport"))
            .or_else(|| query.get("auto_submit"))
            .or_else(|| query.get("autosubmit")),
    );
    let activate = parse_boolean_like(
        query
            .get("activate")
            .or_else(|| query.get("auto_activate"))
            .or_else(|| query.get("autoactivate")),
    );

    Ok(ExternalProviderImportPayload {
        provider_id: provider_id.to_string(),
        page: page.to_string(),
        token,
        import_url,
        min_app_version,
        auto_import,
        activate,
        source: None,
        raw_url: None,
    })
}

#[cfg(test)]
fn parse_external_import_url(raw_url: &str) -> Option<ExternalProviderImportPayload> {
    parse_external_import_url_with_reason(raw_url).ok()
}

fn set_pending(payload: ExternalProviderImportPayload) {
    if let Ok(mut guard) = PENDING_EXTERNAL_PROVIDER_IMPORT.lock() {
        logger::log_info(&format!(
            "[ExternalImport] 写入待处理导入: provider={}, page={}, auto_import={}, token_len={}",
            payload.provider_id,
            payload.page,
            payload.auto_import,
            payload.token.len()
        ));
        *guard = Some(payload);
    }
}

pub fn take_pending_external_import() -> Option<ExternalProviderImportPayload> {
    let Ok(mut guard) = PENDING_EXTERNAL_PROVIDER_IMPORT.lock() else {
        return None;
    };
    let payload = guard.take();
    if let Some(item) = payload.as_ref() {
        logger::log_info(&format!(
            "[ExternalImport] 读取待处理导入: provider={}, page={}, auto_import={}, token_len={}",
            item.provider_id,
            item.page,
            item.auto_import,
            item.token.len()
        ));
    } else {
        logger::log_info("[ExternalImport] 读取待处理导入: empty");
    }
    payload
}

fn emit_external_import_payload<R: Runtime>(
    app: &AppHandle<R>,
    payload: &ExternalProviderImportPayload,
) {
    if let Err(err) = app.emit(EXTERNAL_PROVIDER_IMPORT_EVENT, payload.clone()) {
        logger::log_warn(&format!("[ExternalImport] 发送外部导入事件失败: {}", err));
        return;
    }
    logger::log_info(&format!(
        "[ExternalImport] 已发送外部导入事件: provider={}, page={}, auto_import={}, token_len={}",
        payload.provider_id,
        payload.page,
        payload.auto_import,
        payload.token.len()
    ));
}

pub fn handle_external_import_args<R: Runtime>(
    app: &AppHandle<R>,
    args: &[String],
    source: &str,
) -> bool {
    logger::log_info(&format!(
        "[ExternalImport] 开始处理外部导入参数: source={}, arg_count={}",
        source,
        args.len()
    ));
    let mut saw_deep_link = false;
    for arg in args {
        let candidate = arg.trim();
        if candidate.is_empty() {
            continue;
        }
        let candidate_is_deep_link =
            candidate.starts_with("cockpit-tools://") || candidate.starts_with("cockpittools://");
        if candidate_is_deep_link {
            saw_deep_link = true;
        }
        let candidate_summary = summarize_candidate(candidate);
        logger::log_info(&format!(
            "[ExternalImport] 检查参数: source={}, is_deep_link={}, candidate={}",
            source, candidate_is_deep_link, candidate_summary
        ));

        let mut payload = match parse_external_import_url_with_reason(candidate) {
            Ok(payload) => payload,
            Err(reason) => {
                if candidate_is_deep_link {
                    logger::log_warn(&format!(
                        "[ExternalImport] 参数未通过解析: source={}, candidate={}, reason={}",
                        source, candidate_summary, reason
                    ));
                }
                continue;
            }
        };
        payload.source = Some(source.to_string());
        payload.raw_url = Some(candidate.to_string());

        set_pending(payload.clone());

        if let Err(err) = floating_card_window::show_main_window_and_navigate(app, &payload.page) {
            logger::log_warn(&format!("[ExternalImport] 唤醒主窗口并导航失败: {}", err));
        }
        emit_external_import_payload(app, &payload);

        logger::log_info(&format!(
            "[ExternalImport] 已接收外部导入请求: provider={}, page={}, auto_import={}, source={}, candidate={}",
            payload.provider_id, payload.page, payload.auto_import, source, candidate_summary
        ));
        return true;
    }
    if saw_deep_link {
        logger::log_warn(&format!(
            "[ExternalImport] 未匹配到可处理的 Deep Link: source={}",
            source
        ));
    } else {
        logger::log_info(&format!(
            "[ExternalImport] 未发现 Deep Link 参数: source={}",
            source
        ));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::parse_external_import_url;

    #[test]
    fn parse_basic_import_link() {
        let raw = "cockpit-tools://import?provider=codex&token=abc123";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "codex");
        assert_eq!(payload.page, "codex");
        assert_eq!(payload.token, "abc123");
        assert_eq!(payload.import_url, None);
        assert_eq!(payload.min_app_version, None);
        assert!(!payload.auto_import);
    }

    #[test]
    fn parse_alias_and_boolean() {
        let raw =
            "cockpit-tools://provider-import?platform=codebuddy-cn&payload=%7B%7D&auto_import=true";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "codebuddy_cn");
        assert_eq!(payload.page, "codebuddy-cn");
        assert_eq!(payload.token, "{}");
        assert_eq!(payload.import_url, None);
        assert_eq!(payload.min_app_version, None);
        assert!(payload.auto_import);
    }

    #[test]
    fn parse_antigravity_overview_alias() {
        let raw = "cockpittools://account-import?page=overview&token=1%2F%2F0gTokenDemo";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "antigravity");
        assert_eq!(payload.page, "overview");
        assert_eq!(payload.token, "1//0gTokenDemo");
        assert_eq!(payload.import_url, None);
        assert_eq!(payload.min_app_version, None);
        assert!(!payload.auto_import);
    }

    #[test]
    fn parse_import_url_link() {
        let raw = "cockpit-tools://import?provider=codex&import_url=https%3A%2F%2Fexample.com%2Fuser%2Fapi%2FtoolsImport%2Ffetch%3Fid%3Dabc%26token%3Ddef&auto_import=true";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "codex");
        assert_eq!(payload.page, "codex");
        assert_eq!(payload.token, "");
        assert_eq!(
            payload.import_url,
            Some("https://example.com/user/api/toolsImport/fetch?id=abc&token=def".to_string())
        );
        assert_eq!(payload.min_app_version, None);
        assert!(payload.auto_import);
    }

    #[test]
    fn parse_activate_flag() {
        let raw = "cockpit-tools://provider-import?platform=codex&import_url=https%3A%2F%2Fexample.com%2Fbundle&auto_import=true&activate=true";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "codex");
        assert!(payload.auto_import);
        assert!(payload.activate);
    }

    #[test]
    fn parse_min_app_version() {
        let raw = "cockpit-tools://import?provider=codex&token=abc123&min_app_version=v0.22.21";
        let payload = parse_external_import_url(raw).expect("payload");
        assert_eq!(payload.provider_id, "codex");
        assert_eq!(payload.min_app_version, Some("0.22.21".to_string()));
    }
}
