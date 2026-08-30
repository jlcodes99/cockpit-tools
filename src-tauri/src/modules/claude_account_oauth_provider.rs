// Claude 账号模块：OAuth state, API-key/provider configuration and account imports。
// 通过 include! 保持原 modules::claude_account 作用域和私有调用关系。
fn build_account_id(email: &str, account_uuid: Option<&str>, org_uuid: Option<&str>) -> String {
    let identity = format!(
        "{}:{}:{}",
        email.trim().to_ascii_lowercase(),
        account_uuid.unwrap_or_default().trim(),
        org_uuid.unwrap_or_default().trim()
    );
    format!("claude_{:x}", md5::compute(identity.as_bytes()))
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeApiKeyProviderConfig {
    pub api_base_url: Option<String>,
    pub api_provider_id: Option<String>,
    pub api_provider_name: Option<String>,
    pub api_provider_source_tag: Option<String>,
    pub api_provider_website: Option<String>,
    pub api_provider_api_key_url: Option<String>,
    pub api_key_field: Option<String>,
    pub api_model_catalog: Option<Vec<String>>,
    pub api_extra_env: Option<BTreeMap<String, String>>,
}

fn build_api_key_account_id(api_key: &str, api_base_url: Option<&str>) -> String {
    let identity = format!(
        "{}:{}",
        api_base_url.unwrap_or_default().trim().to_ascii_lowercase(),
        api_key
    );
    format!("claude_apikey_{:x}", md5::compute(identity.as_bytes()))
}

fn build_api_key_display_name(
    api_key: &str,
    account_name: Option<&str>,
    provider_name: Option<&str>,
) -> String {
    if let Some(name) = normalize_non_empty(account_name) {
        return name;
    }
    let suffix: String = api_key
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if let Some(provider_name) = normalize_non_empty(provider_name) {
        return format!("{} {}", provider_name, suffix);
    }
    format!("API Key {}", suffix)
}

fn build_desktop_gateway_account_id(api_key: &str, api_base_url: &str) -> String {
    let identity = format!("{}:{}", api_base_url.trim().to_ascii_lowercase(), api_key);
    format!(
        "claude_desktop_gateway_{:x}",
        md5::compute(identity.as_bytes())
    )
}

fn normalize_desktop_gateway_auth_scheme(value: Option<&str>) -> String {
    match value
        .and_then(|item| normalize_non_empty(Some(item)))
        .map(|item| item.to_ascii_lowercase().replace('_', "-"))
        .as_deref()
    {
        Some("auto") => "auto".to_string(),
        Some("x-api-key") => "x-api-key".to_string(),
        Some("sso") => "sso".to_string(),
        _ => "bearer".to_string(),
    }
}

fn normalize_api_provider_base_url(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = raw.and_then(|value| normalize_non_empty(Some(value))) else {
        return Ok(None);
    };
    let parsed = Url::parse(&value).map_err(|_| "供应商 Base URL 不是有效 URL".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("供应商 Base URL 仅支持 http/https".to_string());
    }
    Ok(Some(value.trim_end_matches('/').to_string()))
}

fn claude_desktop_gateway_models_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url = Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let path = url.path().trim_end_matches('/');
    let next_path = if path.is_empty() || path == "/" {
        "/v1/models".to_string()
    } else if path.ends_with("/v1") || path == "/v1" {
        format!("{}/models", path)
    } else {
        format!("{}/v1/models", path)
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn parse_desktop_gateway_models(body: &Value) -> Vec<ClaudeDesktopGatewayModel> {
    let mut seen = BTreeSet::new();
    body.get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?.trim();
                    if id.is_empty() {
                        return None;
                    }
                    let key = id.to_ascii_lowercase();
                    if !seen.insert(key) {
                        return None;
                    }
                    Some(ClaudeDesktopGatewayModel {
                        id: id.to_string(),
                        display_name: item
                            .get("display_name")
                            .or_else(|| item.get("displayName"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn list_desktop_gateway_models_with_scheme(
    base_url: &str,
    api_key: &str,
    auth_scheme: &str,
) -> Result<ClaudeDesktopGatewayModelsResult, String> {
    let url = claude_desktop_gateway_models_url(base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let mut request = client.get(&url).header(ACCEPT, "application/json");
    if auth_scheme == "x-api-key" {
        request = request.header("x-api-key", api_key);
    } else {
        request = request.bearer_auth(api_key);
    }
    let response = request
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
    let parsed = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("PROVIDER_MODELS_PARSE_FAILED: {}", e))?;
    let models = parse_desktop_gateway_models(&parsed);
    let has_claude_models = models
        .iter()
        .any(|model| crate::modules::claude_desktop_gateway::is_claude_desktop_model(&model.id));
    Ok(ClaudeDesktopGatewayModelsResult {
        models,
        latency_ms,
        recommended_mode: Some(
            if has_claude_models {
                "direct"
            } else {
                "local_mapping"
            }
            .to_string(),
        ),
        has_claude_models,
    })
}

pub async fn list_desktop_gateway_models(
    base_url: &str,
    api_key: &str,
    auth_scheme: Option<&str>,
) -> Result<ClaudeDesktopGatewayModelsResult, String> {
    let api_key = normalize_claude_api_key(api_key, false)?;
    let base_url = normalize_api_provider_base_url(Some(base_url))?
        .ok_or_else(|| "请输入 Gateway Base URL".to_string())?;
    let auth_scheme = normalize_desktop_gateway_auth_scheme(auth_scheme);
    if auth_scheme == "auto" {
        match list_desktop_gateway_models_with_scheme(&base_url, &api_key, "bearer").await {
            Ok(result) => Ok(result),
            Err(error)
                if error.starts_with("PROVIDER_MODELS_HTTP_401")
                    || error.starts_with("PROVIDER_MODELS_HTTP_403") =>
            {
                list_desktop_gateway_models_with_scheme(&base_url, &api_key, "x-api-key").await
            }
            Err(error) => Err(error),
        }
    } else {
        list_desktop_gateway_models_with_scheme(&base_url, &api_key, &auth_scheme).await
    }
}

fn normalize_api_key_field(value: Option<&str>, api_base_url: Option<&str>) -> String {
    match value
        .and_then(|item| normalize_non_empty(Some(item)))
        .map(|item| item.to_ascii_uppercase())
        .as_deref()
    {
        Some("ANTHROPIC_API_KEY") => "ANTHROPIC_API_KEY".to_string(),
        Some("ANTHROPIC_AUTH_TOKEN") => "ANTHROPIC_AUTH_TOKEN".to_string(),
        _ if is_official_anthropic_api_base_url(api_base_url) => "ANTHROPIC_API_KEY".to_string(),
        _ => "ANTHROPIC_AUTH_TOKEN".to_string(),
    }
}

fn is_official_anthropic_api_base_url(api_base_url: Option<&str>) -> bool {
    let Some(value) = api_base_url.and_then(|value| normalize_non_empty(Some(value))) else {
        return true;
    };
    Url::parse(&value)
        .ok()
        .map(|url| {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            host == "api.anthropic.com" || host == "api.claude.com"
        })
        .unwrap_or(false)
}

fn normalize_model_catalog(value: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for model in value.into_iter().flatten() {
        let normalized = model.trim();
        if normalized.is_empty() {
            continue;
        }
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            models.push(normalized.to_string());
        }
    }
    (!models.is_empty()).then_some(models)
}

fn normalize_api_extra_env(
    value: Option<BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for (key, value) in value.into_iter().flatten() {
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        if matches!(
            key.as_str(),
            "ANTHROPIC_API_KEY" | "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_BASE_URL"
        ) {
            continue;
        }
        result.insert(key, value.to_string());
    }
    (!result.is_empty()).then_some(result)
}

fn normalize_claude_api_key(raw: &str, require_anthropic_key: bool) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("请输入 API Key".to_string());
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err("API Key 不能填写 URL".to_string());
    }
    if value.chars().any(char::is_whitespace) {
        return Err("API Key 不能包含空白字符".to_string());
    }
    if require_anthropic_key && !value.starts_with("sk-ant-") {
        return Err("请输入以 sk-ant- 开头的 Anthropic API Key".to_string());
    }
    Ok(value.to_string())
}

fn credentials_oauth(credentials: &Value) -> Option<&Value> {
    credentials.get("claudeAiOauth")
}

fn credentials_refresh_token(credentials: &Value) -> Option<String> {
    read_string_path(credentials, &["claudeAiOauth", "refreshToken"])
}

fn credentials_access_token(credentials: &Value) -> Option<String> {
    read_string_path(credentials, &["claudeAiOauth", "accessToken"])
}

fn credentials_expires_at(credentials: &Value) -> Option<i64> {
    read_i64_value(
        credentials
            .get("claudeAiOauth")
            .and_then(|item| item.get("expiresAt")),
    )
}

fn token_is_expired(credentials: &Value) -> bool {
    let Some(expires_at) = credentials_expires_at(credentials) else {
        return false;
    };
    now_ts_ms() + CLAUDE_TOKEN_EXPIRY_BUFFER_MS >= expires_at
}

fn config_oauth_account(config: &Value) -> Option<&Value> {
    config.get("oauthAccount")
}

fn slim_claude_code_config_snapshot(config: &Value) -> Value {
    let mut object = serde_json::Map::new();

    if let Some(oauth_account) = config.get("oauthAccount").cloned() {
        object.insert("oauthAccount".to_string(), oauth_account);
    }
    if let Some(email) = config.get("email").cloned() {
        object.insert("email".to_string(), email);
    }
    if let Some(has_completed_onboarding) = config.get("hasCompletedOnboarding").cloned() {
        object.insert(
            "hasCompletedOnboarding".to_string(),
            has_completed_onboarding,
        );
    } else if object.contains_key("oauthAccount") {
        object.insert("hasCompletedOnboarding".to_string(), Value::Bool(true));
    }

    Value::Object(object)
}

fn slim_claude_account_snapshots(account: &mut ClaudeAccount) -> bool {
    if !matches!(
        account.auth_mode,
        ClaudeAuthMode::OAuth | ClaudeAuthMode::SetupToken
    ) {
        return false;
    }
    let Some(config_raw) = account.claude_config_raw.as_ref() else {
        return false;
    };
    let slimmed = slim_claude_code_config_snapshot(config_raw);
    if &slimmed == config_raw {
        return false;
    }
    account.claude_config_raw = Some(slimmed);
    true
}

fn read_bool_path(value: &Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    read_bool_value(Some(current))
}

fn derive_oauth_snapshot_plan_type(
    credentials_raw: &Value,
    oauth_account: &Value,
) -> Option<String> {
    let credentials_oauth = credentials_oauth(credentials_raw);
    let profile = credentials_oauth.and_then(|value| value.get("profile"));

    for raw in [
        read_string_path(oauth_account, &["subscriptionType"]),
        credentials_oauth.and_then(|value| read_string_path(value, &["subscriptionType"])),
        read_string_path(oauth_account, &["organizationType"]),
        profile.and_then(|value| read_string_path(value, &["organization", "organization_type"])),
        read_string_path(oauth_account, &["rateLimitTier"]),
        credentials_oauth.and_then(|value| read_string_path(value, &["rateLimitTier"])),
        profile.and_then(|value| read_string_path(value, &["organization", "rate_limit_tier"])),
    ] {
        if let Some(plan) = normalize_desktop_plan_value(raw) {
            return Some(plan);
        }
    }

    if profile
        .and_then(|value| read_bool_path(value, &["account", "has_claude_max"]))
        .unwrap_or(false)
    {
        return Some("Max".to_string());
    }
    if profile
        .and_then(|value| read_bool_path(value, &["account", "has_claude_pro"]))
        .unwrap_or(false)
    {
        return Some("Pro".to_string());
    }

    None
}

fn is_claude_billing_source_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "apple_subscription" | "apple subscription" | "stripe_subscription" | "stripe subscription"
    )
}

fn normalize_account_plan_from_snapshots(account: &mut ClaudeAccount) -> bool {
    let Some(config_raw) = account.claude_config_raw.as_ref() else {
        return false;
    };
    let Some(oauth_account) = config_oauth_account(config_raw) else {
        return false;
    };
    let credentials_raw = account
        .claude_credentials_raw
        .as_ref()
        .unwrap_or(&Value::Null);
    let Some(plan_type) = derive_oauth_snapshot_plan_type(credentials_raw, oauth_account) else {
        return false;
    };
    if account.plan_type.as_deref() == Some(plan_type.as_str()) {
        return false;
    }
    let should_replace = account
        .plan_type
        .as_deref()
        .map(|value| is_claude_billing_source_value(value) || is_desktop_plan_placeholder(value))
        .unwrap_or(true);
    if !should_replace {
        return false;
    }
    account.plan_type = Some(plan_type);
    true
}

fn derive_account_from_snapshots(
    credentials_raw: Value,
    config_raw: Value,
    existing: Option<ClaudeAccount>,
) -> Result<ClaudeAccount, String> {
    if credentials_oauth(&credentials_raw).is_none() {
        return Err("Claude credentials 缺少 claudeAiOauth 字段".to_string());
    }
    let oauth_account = config_oauth_account(&config_raw)
        .ok_or_else(|| "Claude config 缺少 oauthAccount 字段".to_string())?;
    let email = read_string_path(oauth_account, &["emailAddress"])
        .or_else(|| read_string_path(&config_raw, &["email"]))
        .ok_or_else(|| "Claude config 缺少账号邮箱".to_string())?;
    let account_uuid = read_string_path(oauth_account, &["accountUuid"]);
    let organization_uuid = read_string_path(oauth_account, &["organizationUuid"]);
    let organization_name = read_string_path(oauth_account, &["organizationName"]);
    let avatar_url = read_string_path(oauth_account, &["avatarUrl"])
        .or_else(|| read_string_path(oauth_account, &["avatar_url"]));
    let plan_type = derive_oauth_snapshot_plan_type(&credentials_raw, oauth_account);
    let id = build_account_id(
        &email,
        account_uuid.as_deref(),
        organization_uuid.as_deref(),
    );
    let now = now_ts_ms();
    let mut account = existing.unwrap_or_else(|| ClaudeAccount {
        id: id.clone(),
        email: email.clone(),
        auth_mode: ClaudeAuthMode::OAuth,
        account_uuid: None,
        organization_uuid: None,
        organization_name: None,
        plan_type: None,
        avatar_url: None,
        profile_updated_at: None,
        quota: None,
        quota_error: None,
        usage_updated_at: None,
        status: None,
        status_reason: None,
        api_key: None,
        api_base_url: None,
        api_provider_id: None,
        api_provider_name: None,
        api_provider_source_tag: None,
        api_provider_website: None,
        api_provider_api_key_url: None,
        api_key_field: None,
        api_model_catalog: None,
        api_extra_env: None,
        desktop_gateway_auth_scheme: None,
        desktop_gateway_credential_kind: None,
        desktop_gateway_config_id: None,
        desktop_gateway_profile_dir: None,
        desktop_gateway_models: None,
        desktop_gateway_connection_mode: None,
        desktop_gateway_upstream_models: None,
        desktop_gateway_model_mappings: None,
        desktop_profile_dir: None,
        desktop_profile_imported_at: None,
        claude_credentials_raw: None,
        claude_config_raw: None,
        claude_usage_raw: None,
        tags: None,
        account_note: None,
        created_at: now,
        last_used: now,
    });
    account.id = id;
    account.email = email;
    account.auth_mode = if credentials_refresh_token(&credentials_raw).is_some() {
        ClaudeAuthMode::OAuth
    } else {
        ClaudeAuthMode::SetupToken
    };
    account.account_uuid = account_uuid;
    account.organization_uuid = organization_uuid;
    account.organization_name = organization_name;
    account.plan_type = plan_type;
    account.avatar_url = avatar_url;
    account.profile_updated_at = Some(now);
    account.api_key = None;
    account.api_base_url = None;
    account.api_provider_id = None;
    account.api_provider_name = None;
    account.api_provider_source_tag = None;
    account.api_provider_website = None;
    account.api_provider_api_key_url = None;
    account.api_key_field = None;
    account.api_model_catalog = None;
    account.api_extra_env = None;
    account.desktop_gateway_auth_scheme = None;
    account.desktop_gateway_credential_kind = None;
    account.desktop_gateway_config_id = None;
    account.desktop_gateway_profile_dir = None;
    account.desktop_gateway_models = None;
    account.desktop_gateway_connection_mode = None;
    account.desktop_gateway_upstream_models = None;
    account.desktop_gateway_model_mappings = None;
    account.claude_credentials_raw = Some(credentials_raw);
    account.claude_config_raw = Some(config_raw);
    account.last_used = now;
    account.status = None;
    account.status_reason = None;
    account.desktop_profile_dir = None;
    account.desktop_profile_imported_at = None;
    Ok(account)
}

pub fn upsert_account_from_snapshots(
    credentials_raw: Value,
    config_raw: Value,
) -> Result<ClaudeAccount, String> {
    let temp = derive_account_from_snapshots(credentials_raw, config_raw, None)?;
    let existing = load_account_file(&temp.id);
    let account = derive_account_from_snapshots(
        temp.claude_credentials_raw.clone().unwrap_or(Value::Null),
        temp.claude_config_raw.clone().unwrap_or(Value::Null),
        existing,
    )?;
    save_desktop_account_with_dedupe(account)
}

pub fn import_api_key(
    api_key: &str,
    account_name: Option<&str>,
    provider_config: ClaudeApiKeyProviderConfig,
) -> Result<ClaudeAccount, String> {
    let api_base_url = normalize_api_provider_base_url(provider_config.api_base_url.as_deref())?;
    let require_anthropic_key = is_official_anthropic_api_base_url(api_base_url.as_deref());
    let api_key = normalize_claude_api_key(api_key, require_anthropic_key)?;
    let api_key_field = normalize_api_key_field(
        provider_config.api_key_field.as_deref(),
        api_base_url.as_deref(),
    );
    let api_provider_name = normalize_non_empty(provider_config.api_provider_name.as_deref())
        .or_else(|| {
            api_base_url.as_deref().and_then(|value| {
                Url::parse(value).ok().and_then(|url| {
                    url.host_str()
                        .map(|host| host.trim_start_matches("www.").to_string())
                })
            })
        })
        .or_else(|| Some("Anthropic Official".to_string()));
    let api_provider_id = normalize_non_empty(provider_config.api_provider_id.as_deref());
    let api_provider_source_tag =
        normalize_non_empty(provider_config.api_provider_source_tag.as_deref());
    let api_provider_website = normalize_non_empty(provider_config.api_provider_website.as_deref());
    let api_provider_api_key_url =
        normalize_non_empty(provider_config.api_provider_api_key_url.as_deref());
    let api_model_catalog = normalize_model_catalog(provider_config.api_model_catalog);
    let api_extra_env = normalize_api_extra_env(provider_config.api_extra_env);
    let id = build_api_key_account_id(&api_key, api_base_url.as_deref());
    let display_name =
        build_api_key_display_name(&api_key, account_name, api_provider_name.as_deref());
    let now = now_ts_ms();
    let mut account = load_account_file(&id).unwrap_or_else(|| ClaudeAccount {
        id: id.clone(),
        email: display_name.clone(),
        auth_mode: ClaudeAuthMode::ApiKey,
        account_uuid: None,
        organization_uuid: None,
        organization_name: None,
        plan_type: None,
        avatar_url: None,
        profile_updated_at: None,
        quota: None,
        quota_error: None,
        usage_updated_at: None,
        status: None,
        status_reason: None,
        api_key: None,
        api_base_url: None,
        api_provider_id: None,
        api_provider_name: None,
        api_provider_source_tag: None,
        api_provider_website: None,
        api_provider_api_key_url: None,
        api_key_field: None,
        api_model_catalog: None,
        api_extra_env: None,
        desktop_gateway_auth_scheme: None,
        desktop_gateway_credential_kind: None,
        desktop_gateway_config_id: None,
        desktop_gateway_profile_dir: None,
        desktop_gateway_models: None,
        desktop_gateway_connection_mode: None,
        desktop_gateway_upstream_models: None,
        desktop_gateway_model_mappings: None,
        desktop_profile_dir: None,
        desktop_profile_imported_at: None,
        claude_credentials_raw: None,
        claude_config_raw: None,
        claude_usage_raw: None,
        tags: None,
        account_note: None,
        created_at: now,
        last_used: now,
    });
    let key_hash = format!("{:x}", md5::compute(api_key.as_bytes()));
    let provider_snapshot = json!({
        "id": api_provider_id.clone(),
        "name": api_provider_name.clone(),
        "baseUrl": api_base_url.clone(),
        "sourceTag": api_provider_source_tag.clone(),
        "website": api_provider_website.clone(),
        "apiKeyUrl": api_provider_api_key_url.clone(),
        "keyField": api_key_field.clone(),
        "modelCatalog": api_model_catalog.clone(),
        "extraEnv": api_extra_env.clone(),
    });
    account.id = id;
    account.email = display_name;
    account.auth_mode = ClaudeAuthMode::ApiKey;
    account.account_uuid = None;
    account.organization_uuid = None;
    account.organization_name = None;
    account.plan_type = api_provider_name
        .clone()
        .or_else(|| Some("API Key".to_string()));
    account.avatar_url = None;
    account.profile_updated_at = None;
    account.quota = None;
    account.quota_error = None;
    account.usage_updated_at = None;
    account.status = None;
    account.status_reason = None;
    account.api_key = Some(api_key.clone());
    account.api_base_url = api_base_url.clone();
    account.api_provider_id = api_provider_id.clone();
    account.api_provider_name = api_provider_name.clone();
    account.api_provider_source_tag = api_provider_source_tag.clone();
    account.api_provider_website = api_provider_website.clone();
    account.api_provider_api_key_url = api_provider_api_key_url.clone();
    account.api_key_field = Some(api_key_field.clone());
    account.api_model_catalog = api_model_catalog.clone();
    account.api_extra_env = api_extra_env.clone();
    account.desktop_gateway_auth_scheme = None;
    account.desktop_gateway_credential_kind = None;
    account.desktop_gateway_config_id = None;
    account.desktop_gateway_profile_dir = None;
    account.desktop_gateway_models = None;
    account.desktop_gateway_connection_mode = None;
    account.desktop_gateway_upstream_models = None;
    account.desktop_gateway_model_mappings = None;
    account.desktop_profile_dir = None;
    account.desktop_profile_imported_at = None;
    account.claude_credentials_raw = Some(json!({
        "authMode": "api_key",
        "anthropicApiKey": api_key,
        "apiKeyField": api_key_field,
        "apiProvider": provider_snapshot.clone(),
    }));
    account.claude_config_raw = Some(json!({
        "apiKeyAccount": {
            "label": account.email.clone(),
            "keyHash": key_hash,
            "provider": provider_snapshot,
        },
        "hasCompletedOnboarding": true,
    }));
    account.last_used = now;
    save_account_and_index(account)
}

pub fn import_desktop_gateway(
    api_key: &str,
    account_name: Option<&str>,
    provider_config: ClaudeApiKeyProviderConfig,
    auth_scheme: Option<&str>,
    desktop_gateway_models: Option<Vec<String>>,
    desktop_gateway_connection_mode: Option<&str>,
    desktop_gateway_upstream_models: Option<Vec<String>>,
    desktop_gateway_model_mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
) -> Result<ClaudeAccount, String> {
    save_desktop_gateway(
        None,
        api_key,
        account_name,
        provider_config,
        auth_scheme,
        desktop_gateway_models,
        desktop_gateway_connection_mode,
        desktop_gateway_upstream_models,
        desktop_gateway_model_mappings,
    )
}

pub fn update_desktop_gateway(
    account_id: &str,
    api_key: &str,
    account_name: Option<&str>,
    provider_config: ClaudeApiKeyProviderConfig,
    auth_scheme: Option<&str>,
    desktop_gateway_models: Option<Vec<String>>,
    desktop_gateway_connection_mode: Option<&str>,
    desktop_gateway_upstream_models: Option<Vec<String>>,
    desktop_gateway_model_mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
) -> Result<ClaudeAccount, String> {
    save_desktop_gateway(
        Some(account_id),
        api_key,
        account_name,
        provider_config,
        auth_scheme,
        desktop_gateway_models,
        desktop_gateway_connection_mode,
        desktop_gateway_upstream_models,
        desktop_gateway_model_mappings,
    )
}

fn save_desktop_gateway(
    account_id_override: Option<&str>,
    api_key: &str,
    account_name: Option<&str>,
    provider_config: ClaudeApiKeyProviderConfig,
    auth_scheme: Option<&str>,
    desktop_gateway_models: Option<Vec<String>>,
    desktop_gateway_connection_mode: Option<&str>,
    desktop_gateway_upstream_models: Option<Vec<String>>,
    desktop_gateway_model_mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
) -> Result<ClaudeAccount, String> {
    let api_base_url = normalize_api_provider_base_url(provider_config.api_base_url.as_deref())?
        .ok_or_else(|| "请输入 Gateway Base URL".to_string())?;
    let api_key = normalize_claude_api_key(api_key, false)?;
    let auth_scheme = normalize_desktop_gateway_auth_scheme(auth_scheme);
    let credential_kind = "static".to_string();
    let api_provider_name = normalize_non_empty(provider_config.api_provider_name.as_deref())
        .or_else(|| {
            Url::parse(&api_base_url).ok().and_then(|url| {
                url.host_str()
                    .map(|host| host.trim_start_matches("www.").to_string())
            })
        })
        .or_else(|| Some("Gateway".to_string()));
    let api_provider_id = normalize_non_empty(provider_config.api_provider_id.as_deref());
    let api_provider_source_tag =
        normalize_non_empty(provider_config.api_provider_source_tag.as_deref());
    let api_provider_website = normalize_non_empty(provider_config.api_provider_website.as_deref());
    let api_provider_api_key_url =
        normalize_non_empty(provider_config.api_provider_api_key_url.as_deref());
    let api_key_field = normalize_api_key_field(
        provider_config.api_key_field.as_deref(),
        Some(api_base_url.as_str()),
    );
    let api_extra_env = normalize_api_extra_env(provider_config.api_extra_env);
    let connection_mode = crate::modules::claude_desktop_gateway::normalize_connection_mode(
        desktop_gateway_connection_mode,
    );
    let desktop_gateway_upstream_models = normalize_model_catalog(desktop_gateway_upstream_models);
    let mut desktop_gateway_model_mappings =
        crate::modules::claude_desktop_gateway::normalize_model_mappings(
            desktop_gateway_model_mappings,
        );
    let mut desktop_gateway_models = normalize_model_catalog(desktop_gateway_models);
    if connection_mode == "local_mapping" {
        if desktop_gateway_model_mappings.is_none() {
            if let (Some(desktop_models), Some(upstream_models)) = (
                desktop_gateway_models.as_ref(),
                desktop_gateway_upstream_models.as_ref(),
            ) {
                desktop_gateway_model_mappings = Some(
                    crate::modules::claude_desktop_gateway::build_default_model_mappings(
                        desktop_models,
                        upstream_models,
                    ),
                );
            }
        }
        let mappings = desktop_gateway_model_mappings
            .as_ref()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| "请配置模型映射".to_string())?;
        if mappings.iter().any(|mapping| {
            !crate::modules::claude_desktop_gateway::is_claude_desktop_model(&mapping.desktop_model)
        }) {
            return Err("映射左侧必须是 Claude 可识别的 Claude 模型名".to_string());
        }
        desktop_gateway_models = normalize_model_catalog(Some(
            mappings
                .iter()
                .map(|mapping| mapping.desktop_model.clone())
                .collect(),
        ));
    } else {
        let models = desktop_gateway_models
            .as_ref()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| "请填写模型目录".to_string())?;
        if models
            .iter()
            .any(|model| !crate::modules::claude_desktop_gateway::is_claude_desktop_model(model))
        {
            return Err("直连模式的模型目录必须使用 Claude 可识别的 Claude 模型名".to_string());
        }
        if let Some(mappings) = desktop_gateway_model_mappings.as_ref() {
            if mappings.iter().any(|mapping| {
                !crate::modules::claude_desktop_gateway::is_claude_desktop_model(
                    &mapping.desktop_model,
                )
            }) {
                return Err("映射左侧必须是 Claude 可识别的 Claude 模型名".to_string());
            }
        }
    }
    if desktop_gateway_models
        .as_ref()
        .map_or(true, |items| items.is_empty())
    {
        return Err("请填写模型目录".to_string());
    }
    let id = account_id_override
        .and_then(|value| normalize_non_empty(Some(value)))
        .unwrap_or_else(|| build_desktop_gateway_account_id(&api_key, &api_base_url));
    let display_name =
        build_api_key_display_name(&api_key, account_name, api_provider_name.as_deref());
    let existing_account = load_account_file(&id);
    if account_id_override.is_some()
        && existing_account
            .as_ref()
            .map(|account| account.auth_mode != ClaudeAuthMode::DesktopGateway)
            .unwrap_or(true)
    {
        return Err("Claude Gateway 账号不存在".to_string());
    }
    let config_id = existing_account
        .as_ref()
        .and_then(|account| account.desktop_gateway_config_id.clone())
        .filter(|value| UUID_RE.is_match(value))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = now_ts_ms();
    let provider_snapshot = json!({
        "id": api_provider_id.clone(),
        "name": api_provider_name.clone(),
        "baseUrl": api_base_url.clone(),
        "sourceTag": api_provider_source_tag.clone(),
        "website": api_provider_website.clone(),
        "apiKeyUrl": api_provider_api_key_url.clone(),
        "apiKeyField": api_key_field.clone(),
        "extraEnv": api_extra_env.clone(),
        "authScheme": auth_scheme.clone(),
        "credentialKind": credential_kind.clone(),
        "configId": config_id.clone(),
        "manualModels": desktop_gateway_models.clone(),
        "connectionMode": connection_mode.clone(),
        "upstreamModels": desktop_gateway_upstream_models.clone(),
        "modelMappings": desktop_gateway_model_mappings.clone(),
    });

    let mut account = existing_account.unwrap_or_else(|| ClaudeAccount {
        id: id.clone(),
        email: display_name.clone(),
        auth_mode: ClaudeAuthMode::DesktopGateway,
        account_uuid: None,
        organization_uuid: None,
        organization_name: None,
        plan_type: None,
        avatar_url: None,
        profile_updated_at: None,
        quota: None,
        quota_error: None,
        usage_updated_at: None,
        status: None,
        status_reason: None,
        api_key: None,
        api_base_url: None,
        api_provider_id: None,
        api_provider_name: None,
        api_provider_source_tag: None,
        api_provider_website: None,
        api_provider_api_key_url: None,
        api_key_field: None,
        api_model_catalog: None,
        api_extra_env: None,
        desktop_gateway_auth_scheme: None,
        desktop_gateway_credential_kind: None,
        desktop_gateway_config_id: None,
        desktop_gateway_profile_dir: None,
        desktop_gateway_models: None,
        desktop_gateway_connection_mode: None,
        desktop_gateway_upstream_models: None,
        desktop_gateway_model_mappings: None,
        desktop_profile_dir: None,
        desktop_profile_imported_at: None,
        claude_credentials_raw: None,
        claude_config_raw: None,
        claude_usage_raw: None,
        tags: None,
        account_note: None,
        created_at: now,
        last_used: now,
    });
    let key_hash = format!("{:x}", md5::compute(api_key.as_bytes()));
    account.id = id;
    account.email = display_name;
    account.auth_mode = ClaudeAuthMode::DesktopGateway;
    account.account_uuid = None;
    account.organization_uuid = None;
    account.organization_name = api_provider_name.clone();
    account.plan_type = Some("Gateway".to_string());
    account.avatar_url = None;
    account.profile_updated_at = None;
    account.quota = None;
    account.quota_error = None;
    account.usage_updated_at = None;
    account.status = None;
    account.status_reason = None;
    account.api_key = Some(api_key.clone());
    account.api_base_url = Some(api_base_url.clone());
    account.api_provider_id = api_provider_id.clone();
    account.api_provider_name = api_provider_name.clone();
    account.api_provider_source_tag = api_provider_source_tag.clone();
    account.api_provider_website = api_provider_website.clone();
    account.api_provider_api_key_url = api_provider_api_key_url.clone();
    account.api_key_field = Some(api_key_field.clone());
    account.api_model_catalog = None;
    account.api_extra_env = api_extra_env.clone();
    account.desktop_gateway_auth_scheme = Some(auth_scheme.clone());
    account.desktop_gateway_credential_kind = Some(credential_kind.clone());
    account.desktop_gateway_config_id = Some(config_id.clone());
    account.desktop_gateway_profile_dir = None;
    account.desktop_gateway_models = desktop_gateway_models.clone();
    account.desktop_gateway_connection_mode = Some(connection_mode.clone());
    account.desktop_gateway_upstream_models = desktop_gateway_upstream_models.clone();
    account.desktop_gateway_model_mappings = desktop_gateway_model_mappings.clone();
    account.desktop_profile_dir = None;
    account.desktop_profile_imported_at = None;
    account.claude_credentials_raw = Some(json!({
        "authMode": "desktop_gateway",
        "gatewayApiKey": api_key,
        "apiKeyField": api_key_field,
        "gatewayAuthScheme": auth_scheme,
        "gatewayCredentialKind": credential_kind,
        "gatewayModels": desktop_gateway_models,
        "gatewayConnectionMode": connection_mode,
        "gatewayUpstreamModels": desktop_gateway_upstream_models,
        "gatewayModelMappings": desktop_gateway_model_mappings,
        "apiProvider": provider_snapshot.clone(),
    }));
    account.claude_config_raw = Some(json!({
        "desktopGateway": {
            "label": account.email.clone(),
            "keyHash": key_hash,
            "provider": provider_snapshot,
        },
        "hasCompletedOnboarding": true,
    }));
    account.claude_usage_raw = None;
    account.last_used = now;
    save_account_and_index(account)
}

