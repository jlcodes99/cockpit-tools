//! Import schema compatibility, based on KiroAccountManager exports. This only
//! normalizes supplied credentials; it never guesses identity from an email.
use base64::Engine;
use serde_json::{Map, Value};

fn present(value: &Value) -> bool {
    !value.is_null() && !value.as_str().is_some_and(|s| s.trim().is_empty())
}

pub(crate) fn normalize(raw: &Value) -> Result<Value, String> {
    let root = raw.as_object().ok_or("KIRO_IMPORT_INVALID_OBJECT")?;
    let mut sources = Vec::new();
    for key in [
        "kiro_auth_token_raw",
        "credentials",
        "authToken",
        "token",
        "auth",
    ] {
        if let Some(value) = root.get(key).and_then(Value::as_object) {
            sources.push(value);
        }
    }
    sources.push(root);
    let mut auth = Map::new();
    for (target, aliases) in [
        (
            "accessToken",
            &["accessToken", "access_token", "accessTokenJwt", "token"][..],
        ),
        (
            "refreshToken",
            &["refreshToken", "refresh_token", "refreshTokenJwt"],
        ),
        ("idToken", &["idToken", "id_token"]),
        (
            "expiresAt",
            &[
                "expiresAt",
                "expires_at",
                "expiresAtMs",
                "expiry",
                "expiration",
            ],
        ),
        ("tokenType", &["tokenType", "token_type"]),
        (
            "provider",
            &[
                "provider",
                "loginProvider",
                "login_provider",
                "idp",
                "login_option",
            ],
        ),
        ("authMethod", &["authMethod", "auth_method"]),
        (
            "region",
            &[
                "authRegion",
                "auth_region",
                "idc_region",
                "idcRegion",
                "region",
            ],
        ),
        ("clientId", &["clientId", "client_id"]),
        ("clientSecret", &["clientSecret", "client_secret"]),
        ("clientIdHash", &["clientIdHash", "client_id_hash"]),
        (
            "clientRegistration",
            &[
                "clientRegistration",
                "client_registration",
                "registration",
                "oidcClient",
            ],
        ),
        ("startUrl", &["startUrl", "start_url"]),
        ("issuerUrl", &["issuerUrl", "issuer_url", "issuer"]),
        ("tokenEndpoint", &["tokenEndpoint", "token_endpoint"]),
        ("scopes", &["scopes", "scope"]),
        ("profileArn", &["profileArn", "profile_arn", "arn"]),
        ("email", &["email", "userEmail"]),
        ("userId", &["userId", "user_id", "sub", "accountId"]),
        ("loginHint", &["loginHint", "login_hint"]),
    ] {
        if let Some(value) = sources.iter().find_map(|source| {
            aliases
                .iter()
                .find_map(|key| source.get(*key).filter(|v| present(v)))
        }) {
            // A nested token object is not an access token.
            if target != "accessToken" || value.is_string() {
                auth.insert(target.into(), value.clone());
            }
        }
    }
    if let Some(registration) = auth
        .get("clientRegistration")
        .and_then(Value::as_object)
        .cloned()
    {
        for key in ["clientId", "clientSecret"] {
            if let Some(value) = registration.get(key).filter(|v| present(v)) {
                auth.entry(key).or_insert_with(|| value.clone());
            }
        }
    }
    if let Some(values) = auth.get("scopes").and_then(Value::as_array) {
        let text = values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        auth.insert("scopes".into(), Value::String(text));
    }
    let method = text(&auth, "authMethod").to_ascii_lowercase();
    let provider = text(&auth, "provider").to_ascii_lowercase();
    let external = method == "external_idp"
        || provider == "external_idp"
        || microsoft_token_endpoint(text(&auth, "tokenEndpoint"));
    let idc = method == "idc"
        || matches!(
            provider.as_str(),
            "builderid" | "builder_id" | "awsidc" | "iam_sso" | "enterprise" | "internal"
        )
        || (!text(&auth, "clientId").is_empty() && !text(&auth, "clientSecret").is_empty());
    if external {
        auth.insert("provider".into(), Value::String("ExternalIdp".into()));
        auth.insert("authMethod".into(), Value::String("external_idp".into()));
    } else if idc {
        auth.insert("authMethod".into(), Value::String("IdC".into()));
        auth.entry("region")
            .or_insert_with(|| Value::String("us-east-1".into()));
        if !auth.contains_key("startUrl") {
            if let Some(url) = start_url_from_registration(text(&auth, "clientSecret")) {
                auth.insert("startUrl".into(), Value::String(url));
            }
        }
        let start = text(&auth, "startUrl");
        let enterprise = url::Url::parse(start).ok().is_some_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some_and(|host| {
                    host.ends_with(".awsapps.com") && host != "view.awsapps.com"
                })
        });
        let canonical = if enterprise || matches!(provider.as_str(), "enterprise" | "iam_sso") {
            Some("Enterprise")
        } else if matches!(provider.as_str(), "builderid" | "builder_id" | "awsidc") {
            Some("BuilderId")
        } else if provider == "internal" {
            Some("Internal")
        } else {
            None
        };
        if let Some(provider) = canonical {
            auth.insert("provider".into(), Value::String(provider.into()));
        }
    } else if method == "social" || matches!(provider.as_str(), "google" | "github") {
        auth.insert("authMethod".into(), Value::String("social".into()));
        if matches!(provider.as_str(), "google" | "github") {
            auth.insert(
                "provider".into(),
                Value::String(
                    if provider == "google" {
                        "Google"
                    } else {
                        "Github"
                    }
                    .into(),
                ),
            );
        }
    }
    Ok(Value::Object(auth))
}

fn text<'a>(auth: &'a Map<String, Value>, key: &str) -> &'a str {
    auth.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

fn start_url_from_registration(secret: &str) -> Option<String> {
    let encoded = secret.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let value = claims.get("initiateLoginUri")?.as_str()?;
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https" && url.host_str()?.ends_with(".awsapps.com"))
        .then(|| value.to_string())
}

pub(crate) fn microsoft_token_endpoint(endpoint: &str) -> bool {
    url::Url::parse(endpoint).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && matches!(
                url.host_str(),
                Some(
                    "login.microsoftonline.com"
                        | "login.microsoftonline.us"
                        | "login.chinacloudapi.cn"
                )
            )
            && url.path().ends_with("/oauth2/v2.0/token")
    })
}

pub(crate) fn merge_refreshed(token: &Value, context: &Value) -> Result<Value, String> {
    let mut token = normalize(token)?;
    if token
        .get("accessToken")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err("KIRO_REFRESH_MISSING_ACCESS_TOKEN".into());
    }
    if let (Some(target), Some(context)) = (token.as_object_mut(), context.as_object()) {
        for (key, value) in context {
            // A previous expiry is not the lifetime of the new access token.
            if key != "expiresAt" {
                target.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    Ok(token)
}
