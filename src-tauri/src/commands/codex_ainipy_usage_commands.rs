// Ainipy's web console uses a session bearer credential, not its inference key.
// Keep that credential in memory only and never return it to the app frontend.
static AINIPY_USAGE_SESSIONS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn is_ainipy_usage_base_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value.trim()) else { return false };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(url.host_str(), Some("ainipy.com" | "www.ainipy.com" | "api.ainipy.com"))
}

#[tauri::command]
pub async fn codex_login_ainipy_usage(
    app: AppHandle,
    base_url: String,
    api_key: String,
) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if !is_ainipy_usage_base_url(&base_url) || api_key.trim().is_empty() {
        return Err("AINIPY_PROVIDER_INVALID".into());
    }
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let callback_path = format!("/__cockpit_ainipy_{}", nonce);
    let script = include_str!("ainipy_login.js")
        .replace("__AINIPY_CALLBACK_PATH__", &callback_path);
    let (sender, receiver) = tokio::sync::oneshot::channel::<String>();
    let sender = std::sync::Arc::new(Mutex::new(Some(sender)));
    let navigation_sender = sender.clone();
    let window = WebviewWindowBuilder::new(
        &app,
        format!("ainipy-login-{}", nonce),
        WebviewUrl::External(reqwest::Url::parse("https://www.ainipy.com/").unwrap()),
    )
    .title("Ainipy — Sign in to sync this account's usage")
    .inner_size(920.0, 720.0)
    .incognito(true)
    .initialization_script(&script)
    .on_navigation(move |url| {
        if url.origin().ascii_serialization() == "https://www.ainipy.com"
            && url.path() == callback_path
        {
            if let Some((_, token)) = url::form_urlencoded::parse(url.fragment().unwrap_or("").as_bytes())
                .find(|(name, token)| name == "token" && !token.is_empty() && token.len() <= 16_384)
            {
                if let Ok(mut pending) = navigation_sender.lock() {
                    if let Some(sender) = pending.take() { let _ = sender.send(token.into_owned()); }
                }
            }
            // Never send the credential-bearing callback to the web server.
            return false;
        }
        url.origin().ascii_serialization() == "https://www.ainipy.com" || url.as_str() == "about:blank"
    })
    .build()
    .map_err(|_| "AINIPY_LOGIN_WINDOW_FAILED".to_string())?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Ok(mut pending) = sender.lock() { pending.take(); }
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(300), receiver).await;
    let _ = window.destroy();
    let credential = result.map_err(|_| "AINIPY_LOGIN_TIMEOUT".to_string())?
        .map_err(|_| "AINIPY_LOGIN_CANCELLED".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|_| "AINIPY_NETWORK_FAILED".to_string())?;
    // Validate the session before binding it to the selected card.
    ainipy_usage_get(&client, "/api/client/billing-summary", &credential).await?;
    verify_ainipy_usage_account(&client, &credential, api_key.trim()).await?;
    AINIPY_USAGE_SESSIONS.lock().map_err(|_| "AINIPY_SESSION_FAILED".to_string())?
        .insert(api_key.trim().to_string(), credential);
    Ok(())
}

async fn verify_ainipy_usage_account(
    client: &reqwest::Client,
    credential: &str,
    api_key: &str,
) -> Result<(), String> {
    let payload = ainipy_usage_get(client, "/api/client/api-keys", credential).await?;
    let keys = payload.as_array()
        .or_else(|| payload.get("items").and_then(|v| v.as_array()))
        .or_else(|| payload.pointer("/data/items").and_then(|v| v.as_array()))
        .or_else(|| payload.get("data").and_then(|v| v.as_array()))
        .ok_or_else(|| "AINIPY_USAGE_PARSE_FAILED".to_string())?;
    for key in keys {
        let Some(id) = key.get("id").and_then(|value| value.as_u64()) else { continue };
        let instructions = ainipy_usage_get(client,
            &format!("/api/client/api-keys/{}/usage-instructions?include_secret=1", id), credential).await?;
        if instructions.get("api_key").and_then(|value| value.as_str()) == Some(api_key) {
            return Ok(());
        }
    }
    Err("AINIPY_ACCOUNT_MISMATCH".into())
}

async fn ainipy_usage_get(
    client: &reqwest::Client,
    path: &str,
    credential: &str,
) -> Result<serde_json::Value, String> {
    let response = client.get(format!("https://www.ainipy.com{}", path))
        .bearer_auth(credential).send().await
        .map_err(|_| "AINIPY_NETWORK_FAILED".to_string())?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("AINIPY_LOGIN_REQUIRED".into());
    }
    if !status.is_success() { return Err(format!("AINIPY_HTTP_{}", status.as_u16())); }
    response.json().await.map_err(|_| "AINIPY_USAGE_PARSE_FAILED".to_string())
}

async fn query_ainipy_usage(api_key: &str) -> Result<CodexModelProviderUsageSummary, String> {
    let credential = AINIPY_USAGE_SESSIONS.lock()
        .map_err(|_| "AINIPY_SESSION_FAILED".to_string())?
        .get(api_key).cloned().ok_or_else(|| "AINIPY_LOGIN_REQUIRED".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|_| "AINIPY_NETWORK_FAILED".to_string())?;
    let started = Instant::now();
    let billing = ainipy_usage_get(&client, "/api/client/billing-summary", &credential).await?;
    // A temporary statistics failure must not hide a successfully fetched balance.
    let usage = ainipy_usage_get(&client, "/api/client/usage-summary?range=today", &credential).await.ok();
    let normalized = normalize_ainipy_usage(&billing, usage.as_ref())?;
    Ok(summarize_model_provider_usage(&normalized, started.elapsed().as_millis() as u64))
}

fn normalize_ainipy_usage(
    billing: &serde_json::Value,
    usage: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let balance = json_f64_at(billing, &["balance_milli_tokens"])
        .filter(|value| value.is_finite()).ok_or_else(|| "AINIPY_USAGE_PARSE_FAILED".to_string())? / 1000.0;
    let total = json_f64_at(billing, &["total_consumed_milli_tokens"])
        .filter(|value| value.is_finite()).map(|value| (value / 1000.0).floor() as i64);
    let today = usage.filter(|value| value.get("range").and_then(|v| v.as_str()) == Some("today"))
        .and_then(|value| json_f64_at(value, &["usage", "milli_tokens"]))
        .filter(|value| value.is_finite()).map(|value| (value / 1000.0).floor() as i64);
    Ok(serde_json::json!({
        "mode": "ainipy", "balance": balance, "remaining": balance, "unit": "tokens",
        "usage": { "today": { "total_tokens": today }, "total": { "total_tokens": total } }
    }))
}

#[cfg(test)]
mod ainipy_usage_tests {
    use super::*;

    #[test]
    fn only_trust_known_https_ainipy_hosts() {
        assert!(is_ainipy_usage_base_url("https://www.ainipy.com/api/desktop/v1"));
        for value in ["https://www.ainipy.com.evil.test", "http://ainipy.com", "https://user@ainipy.com", "https://ainipy.com:444"] {
            assert!(!is_ainipy_usage_base_url(value));
        }
    }

    #[test]
    fn convert_milli_tokens_without_inventing_missing_usage() {
        let billing = serde_json::json!({"balance_milli_tokens": 1234567, "total_consumed_milli_tokens": 123999});
        let usage = serde_json::json!({"range": "today", "usage": {"milli_tokens": 789123}});
        let summary = summarize_model_provider_usage(&normalize_ainipy_usage(&billing, Some(&usage)).unwrap(), 1);
        assert_eq!(summary.balance, Some(1234.567));
        assert_eq!(summary.today_total_tokens, Some(789));
        assert_eq!(summary.total_total_tokens, Some(123));
        assert_eq!(summary.unit.as_deref(), Some("tokens"));
        let partial = normalize_ainipy_usage(&billing, None).unwrap();
        assert!(partial["usage"]["today"]["total_tokens"].is_null());
        assert!(normalize_ainipy_usage(&serde_json::json!({"error": "unauthorized"}), None).is_err());
    }
}
