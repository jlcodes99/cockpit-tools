use serde::Serialize;
use tauri::Emitter;
use url::Url;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginEndpointNotice {
    pub provider: String,
    pub endpoint: String,
    pub domain: String,
    pub occurred_at: i64,
}

pub fn notify_login_endpoint(provider: &str, endpoint: &str) {
    let domain = Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(|value| value.to_string()))
        .unwrap_or_else(|| endpoint.to_string());
    let notice = LoginEndpointNotice {
        provider: provider.to_string(),
        endpoint: endpoint.to_string(),
        domain,
        occurred_at: chrono::Utc::now().timestamp(),
    };

    crate::modules::logger::log_warn(&format!(
        "[Security] 登录完成最后调用端点: provider={}, domain={}, endpoint={}",
        notice.provider, notice.domain, notice.endpoint
    ));

    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("security:login-endpoint", notice);
    }
}
