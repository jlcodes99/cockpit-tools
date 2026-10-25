use serde::{Deserialize, Serialize};

const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const DEFAULT_OAUTH_CLIENT_KEY: &str = "antigravity_enterprise";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 900;
// Match Antigravity-Manager's native OAuth UA and known-stable fallback.
// Do not run version probes or remote discovery on the authorization path.
const NATIVE_OAUTH_USER_AGENT: &str = "vscode/1.X.X (Antigravity/4.3.0)";

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(skip)]
    pub oauth_client_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub id: Option<String>,
    pub email: String,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
}

impl UserInfo {
    pub fn get_display_name(&self) -> Option<String> {
        if let Some(name) = &self.name {
            if !name.trim().is_empty() {
                return Some(name.clone());
            }
        }

        match (&self.given_name, &self.family_name) {
            (Some(given), Some(family)) => Some(format!("{} {}", given, family)),
            (Some(given), None) => Some(given.clone()),
            (None, Some(family)) => Some(family.clone()),
            (None, None) => None,
        }
    }
}

fn normalize_oauth_client_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn resolve_oauth_client_key(preferred: Option<&str>) -> Result<String, String> {
    match preferred.map(normalize_oauth_client_key) {
        Some(key) if key == DEFAULT_OAUTH_CLIENT_KEY => Ok(key),
        Some(key) if key.is_empty() => Ok(DEFAULT_OAUTH_CLIENT_KEY.to_string()),
        Some(key) => Err(format!(
            "未知 OAuth client: {}，当前版本仅支持 {}",
            key, DEFAULT_OAUTH_CLIENT_KEY
        )),
        None => Ok(DEFAULT_OAUTH_CLIENT_KEY.to_string()),
    }
}

fn oauth_client_config(
    preferred: Option<&str>,
) -> Result<(&'static str, &'static str, String), String> {
    let key = resolve_oauth_client_key(preferred)?;
    Ok((CLIENT_ID, CLIENT_SECRET, key))
}

/// 生成 OAuth 授权 URL
pub fn get_auth_url(redirect_uri: &str, state: Option<&str>) -> String {
    let (client_id, _, _) = oauth_client_config(None).expect("默认 OAuth client 配置无效");
    let scopes = vec![
        "openid",
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        "https://www.googleapis.com/auth/cclog",
        "https://www.googleapis.com/auth/experimentsandconfigs",
        // Required by Antigravity voice transcription (Cockpit #2361).
        "https://www.googleapis.com/auth/aicode",
    ]
    .join(" ");

    let mut params = vec![
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scopes),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
    ];

    if let Some(state) = state.filter(|value| !value.trim().is_empty()) {
        params.push(("state", state));
    }

    let url = url::Url::parse_with_params(AUTH_URL, &params).expect("无效的 Auth URL");
    url.to_string()
}

/// 使用 Authorization Code 交换 Token
pub async fn exchange_code(code: &str, redirect_uri: &str) -> Result<TokenResponse, String> {
    crate::modules::logger::log_info(&format!("开始 Token 交换, redirect_uri: {}", redirect_uri));
    let client = crate::utils::http::create_client(15);
    let (client_id, client_secret, client_key) = oauth_client_config(None)?;

    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];

    let response = client
        .post(TOKEN_URL)
        .header(reqwest::header::USER_AGENT, NATIVE_OAUTH_USER_AGENT)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("Token 交换请求失败: {}", e);
            crate::modules::logger::log_error(&msg);
            msg
        })?;

    let status = response.status();
    crate::modules::logger::log_info(&format!("Token 交换响应状态: {}", status));

    if status.is_success() {
        let mut token_res = response.json::<TokenResponse>().await.map_err(|e| {
            let msg = format!("Token 解析失败: {}", e);
            crate::modules::logger::log_error(&msg);
            msg
        })?;
        token_res.oauth_client_key = Some(client_key);

        if token_res.refresh_token.is_some() {
            crate::modules::logger::log_info("Token 交换成功, 获取到 refresh_token");
        } else {
            crate::modules::logger::log_warn(
                "警告: Google 未返回 refresh_token, 可能之前已授权过此应用",
            );
        }

        Ok(token_res)
    } else {
        let error_text = response.text().await.unwrap_or_default();
        let msg = format!("Token 交换失败 ({})，body_len={}", status, error_text.len());
        crate::modules::logger::log_error(&msg);
        Err(msg)
    }
}

/// 使用 refresh_token 刷新 access_token
pub async fn refresh_access_token(refresh_token: &str) -> Result<TokenResponse, String> {
    refresh_access_token_with_client(refresh_token, None).await
}

/// 使用指定 OAuth client 刷新 access_token。
pub async fn refresh_access_token_with_client(
    refresh_token: &str,
    oauth_client_key: Option<&str>,
) -> Result<TokenResponse, String> {
    let client = crate::utils::http::create_client(15);
    refresh_access_token_at(&client, TOKEN_URL, refresh_token, oauth_client_key).await
}

async fn refresh_access_token_at(
    client: &reqwest::Client,
    token_url: &str,
    refresh_token: &str,
    oauth_client_key: Option<&str>,
) -> Result<TokenResponse, String> {
    let (client_id, client_secret, client_key) = oauth_client_config(oauth_client_key)?;
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    for attempt in 0..2 {
        let response = client
            .post(token_url)
            .header(reqwest::header::USER_AGENT, NATIVE_OAUTH_USER_AGENT)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("刷新请求失败: {}", e))?;

        if response.status().is_success() {
            let mut token_data = response
                .json::<TokenResponse>()
                .await
                .map_err(|e| format!("刷新数据解析失败: {}", e))?;
            token_data.oauth_client_key = Some(client_key);
            return Ok(token_data);
        }

        let error_text = response.text().await.unwrap_or_default();
        let invalid_grant = serde_json::from_str::<serde_json::Value>(&error_text)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|e| e.as_str())
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("invalid_grant");
        if attempt == 0 && invalid_grant {
            crate::modules::logger::log_warn("OAuth invalid_grant，短暂退避后再次确认");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }
        return Err(format!("刷新失败: {}", error_text));
    }
    unreachable!("OAuth refresh attempts are bounded")
}

/// 获取用户信息
pub async fn get_user_info(access_token: &str) -> Result<UserInfo, String> {
    let client = crate::utils::http::create_client(15);

    let response = client
        .get(USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("用户信息请求失败: {}", e))?;

    if response.status().is_success() {
        response
            .json::<UserInfo>()
            .await
            .map_err(|e| format!("用户信息解析失败: {}", e))
    } else {
        let error_text = response.text().await.unwrap_or_default();
        Err(format!("获取用户信息失败: {}", error_text))
    }
}

/// 检查并在需要时刷新 Token
pub async fn ensure_fresh_token(
    current_token: &crate::models::TokenData,
) -> Result<crate::models::TokenData, String> {
    let now = chrono::Local::now().timestamp();

    if !token_needs_refresh(current_token.expiry_timestamp, now) {
        return Ok(current_token.clone());
    }

    crate::modules::logger::log_info("Token 即将过期，正在刷新...");
    let response = refresh_access_token_with_client(
        &current_token.refresh_token,
        current_token.oauth_client_key.as_deref(),
    )
    .await?;

    let mut token = crate::models::TokenData::new(
        response.access_token,
        current_token.refresh_token.clone(),
        response.expires_in,
        current_token.email.clone(),
        current_token.project_id.clone(),
        current_token.session_id.clone(),
    )
    .with_oauth_metadata(
        response.oauth_client_key,
        response.id_token.or_else(|| current_token.id_token.clone()),
    );
    token.is_gcp_tos = current_token.is_gcp_tos;
    Ok(token)
}

fn token_needs_refresh(expiry_timestamp: i64, now: i64) -> bool {
    expiry_timestamp <= now + TOKEN_REFRESH_SKEW_SECONDS
}

#[cfg(test)]
#[path = "../../../../src-tauri/src/modules/oauth_tests.rs"]
mod tests;
