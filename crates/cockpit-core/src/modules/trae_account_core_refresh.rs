// cockpit-core Trae 账号：Device proof, official refresh exchange and profile synchronization。
// 通过 include! 保持原模块作用域和平台调用路径。
pub fn inject_to_trae_for_platform(
    platform: TraePlatformKind,
    account_id: &str,
) -> Result<(), String> {
    let storage_path = get_default_trae_storage_path_for_platform(platform)?;
    inject_to_trae_at_path(storage_path.as_path(), account_id)
}

pub fn inject_to_trae_at_path(storage_path: &Path, account_id: &str) -> Result<(), String> {
    let account =
        load_account(account_id).ok_or_else(|| format!("Trae 账号不存在: {}", account_id))?;
    let mut root = if storage_path.exists() {
        read_storage_json(storage_path)?
    } else {
        Value::Object(Map::new())
    };

    if !root.is_object() {
        root = Value::Object(Map::new());
    }

    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "Trae storage.json 格式非法".to_string())?;

    let (auth_storage_key, server_storage_key, entitlement_storage_key) =
        resolve_storage_keys_for_inject(root_obj);

    let existing_auth_raw = root_obj
        .get(auth_storage_key.as_str())
        .and_then(|value| parse_value_or_json_string_or_icube_cipher(Some(value)));
    let auth_raw = ensure_auth_raw_for_inject(&account, existing_auth_raw.as_ref());
    root_obj.insert(auth_storage_key, to_icube_cipher_string_value(&auth_raw)?);
    write_device_key_pair_for_inject(root_obj, &account)?;

    if let Some(entitlement_raw) = ensure_entitlement_raw_for_inject(&account) {
        root_obj.insert(
            entitlement_storage_key,
            to_json_string_value(&entitlement_raw)?,
        );
    }

    if let Some(server_raw) = ensure_server_raw_for_inject(&account) {
        root_obj.insert(server_storage_key, to_json_string_value(&server_raw)?);
    }

    let user_tag = resolve_user_tag_for_inject(&account);
    let encoded_usertag_map = merge_usertag_map_for_inject(
        root_obj,
        resolve_account_user_id_for_inject(&account).as_deref(),
        user_tag.as_str(),
    )?;
    if let Some(encoded_map) = encoded_usertag_map {
        root_obj.insert(
            TRAE_STORAGE_USERTAG_KEY.to_string(),
            Value::String(encoded_map),
        );
    } else if let Some(usertag_raw) = normalize_non_empty(account.trae_usertag_raw.as_deref()) {
        root_obj.insert(
            TRAE_STORAGE_USERTAG_KEY.to_string(),
            Value::String(usertag_raw),
        );
    }

    write_storage_json(storage_path, &root)?;

    logger::log_info(&format!(
        "[Trae Account] 注入成功: id={}, email={}, path={}",
        account.id,
        account.email,
        storage_path.display()
    ));
    Ok(())
}

fn extract_response_data(raw: &Value) -> Option<&Value> {
    raw.get("data")
        .or_else(|| raw.get("Result"))
        .or_else(|| raw.get("result"))
        .or_else(|| raw.get("payload"))
}

fn pick_cookie_from_account(account: &TraeAccount) -> Option<String> {
    pick_string(
        account.trae_auth_raw.as_ref(),
        &[
            &["cookie"],
            &["Cookie"],
            &["headers", "cookie"],
            &["headers", "Cookie"],
        ],
    )
}

#[derive(Debug, Clone)]
struct TraeDeviceKeyPair {
    private_key_pem: String,
    public_key_pem: String,
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let body = pem
        .lines()
        .filter(|line| !line.trim_start().starts_with("-----"))
        .map(str::trim)
        .collect::<String>();
    BASE64_STANDARD
        .decode(body.as_bytes())
        .map_err(|e| format!("解析 Trae 设备私钥 PEM 失败: {}", e))
}

fn resolve_device_key_pair_for_refresh(account: &TraeAccount) -> Option<TraeDeviceKeyPair> {
    let value = resolve_device_key_pair_for_inject(account)?;
    Some(TraeDeviceKeyPair {
        private_key_pem: pick_string(Some(&value), &[&["privateKeyPEM"]])?,
        public_key_pem: pick_string(Some(&value), &[&["publicKeyPEM"]])?,
    })
}

fn resolve_ide_version_for_account(account: &TraeAccount) -> String {
    pick_string_multi(
        &[
            account.trae_auth_raw.as_ref(),
            account.trae_server_raw.as_ref(),
        ],
        &[
            &["deviceInfo", "ClientVersion"],
            &["ClientVersion"],
            &["x_app_version"],
            &["exchangeResponse", "IDEVersion"],
        ],
    )
    .unwrap_or_else(|| TRAE_IDE_VERSION.to_string())
}

fn build_refresh_device_info(account: &TraeAccount, public_key_pem: &str) -> Value {
    let mut device_info = account
        .trae_auth_raw
        .as_ref()
        .and_then(|value| value.get("deviceInfo"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    device_info.insert(
        "DevicePublicKey".to_string(),
        Value::String(public_key_pem.to_string()),
    );
    device_info
        .entry("PlatformCode".to_string())
        .or_insert_with(|| Value::String("IDE_PC".to_string()));
    device_info
        .entry("DeviceType".to_string())
        .or_insert_with(|| Value::String("PC".to_string()));
    device_info
        .entry("ClientVersion".to_string())
        .or_insert_with(|| Value::String(resolve_ide_version_for_account(account)));
    Value::Object(device_info)
}

fn build_device_proof_message(
    client_id: &str,
    refresh_token: &str,
    timestamp: i64,
    nonce: &str,
) -> String {
    let timestamp_text = timestamp.to_string();
    [
        "POST",
        TRAE_AUTH_CODE_EXCHANGE_TOKEN_PATH,
        client_id,
        refresh_token,
        timestamp_text.as_str(),
        nonce,
    ]
    .join("\n")
}

fn sign_trae_device_proof(
    refresh_token: &str,
    private_key_pem: &str,
    client_id: &str,
) -> Result<Value, String> {
    let mut nonce_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = bytes_to_lower_hex(&nonce_bytes);
    let timestamp = chrono::Utc::now().timestamp();
    let message = build_device_proof_message(client_id, refresh_token, timestamp, nonce.as_str());
    let private_key_der = pem_to_der(private_key_pem)?;
    let rng = SystemRandom::new();
    let key_pair = EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_ASN1_SIGNING,
        private_key_der.as_slice(),
        &rng,
    )
    .map_err(|_| "解析 Trae 设备私钥失败".to_string())?;
    let signature = key_pair
        .sign(&rng, message.as_bytes())
        .map_err(|_| "生成 Trae 设备签名失败".to_string())?;
    Ok(serde_json::json!({
        "Signature": BASE64_STANDARD.encode(signature.as_ref()),
        "Timestamp": timestamp,
        "Nonce": nonce,
    }))
}

async fn request_exchange_token_by_official_refresh(
    client: &reqwest::Client,
    account: &TraeAccount,
    routing_context: &TraeRefreshRoutingContext,
    cookie: Option<&str>,
) -> Result<Value, String> {
    let refresh_token = normalize_non_empty(account.refresh_token.as_deref())
        .ok_or_else(|| "Trae refresh token 缺失，无法按官方流程刷新登录态".to_string())?;
    let device_key_pair = resolve_device_key_pair_for_refresh(account)
        .ok_or_else(|| "Trae 设备密钥缺失，无法按官方新版流程刷新登录态".to_string())?;
    let device_info = build_refresh_device_info(account, device_key_pair.public_key_pem.as_str());
    let client_id = routing_context.client_id.as_str();
    let device_proof = sign_trae_device_proof(
        refresh_token.as_str(),
        device_key_pair.private_key_pem.as_str(),
        client_id,
    )?;
    let body = serde_json::json!({
        "ClientID": client_id,
        "ClientSecret": "",
        "RefreshToken": refresh_token,
        "DeviceInfo": device_info,
        "DeviceProof": device_proof,
        "IDEVersion": resolve_ide_version_for_account(account),
    });
    let urls = build_api_urls(
        routing_context.login_host.as_str(),
        TRAE_AUTH_CODE_EXCHANGE_TOKEN_PATH,
    );
    let response = request_trae_json_with_candidates(
        client,
        Method::POST,
        urls.as_slice(),
        account.access_token.as_str(),
        cookie,
        Some(body),
    )
    .await?;
    let root = extract_response_data(&response).unwrap_or(&response);
    if pick_string(
        Some(root),
        &[&["Token"], &["accessToken"], &["access_token"], &["token"]],
    )
    .is_none()
    {
        return Err("Trae 官方 ExchangeToken 响应缺少 access token".to_string());
    }
    Ok(response)
}

fn normalize_login_region(raw: Option<&str>) -> Option<String> {
    let value = normalize_non_empty(raw)?;
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "china-north" => "cn".to_string(),
        "singapore-central" => "sg".to_string(),
        "us-east" | "us-east-1" => "us".to_string(),
        other => other.to_string(),
    };
    Some(normalized)
}

fn build_refresh_routing_context(account: &TraeAccount) -> TraeRefreshRoutingContext {
    let platform = resolve_account_platform_kind(account);
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let roots = [
        account.trae_auth_raw.as_ref(),
        profile_root,
        account.trae_server_raw.as_ref(),
        account.trae_entitlement_raw.as_ref(),
        account.trae_usage_raw.as_ref(),
    ];

    let login_region = normalize_login_region(
        pick_string_multi(
            &roots,
            &[
                &["loginRegion"],
                &["callbackQuery", "userRegion"],
                &["userRegion", "region"],
                &["userRegion", "_aiRegion"],
                &["storeRegion"],
                &["AIRegion"],
            ],
        )
        .as_deref(),
    );

    let store_region = pick_string_multi(
        &roots,
        &[
            &["storeRegion"],
            &["account", "storeRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["AIRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()));

    let ai_region = pick_string_multi(
        &roots,
        &[
            &["AIRegion"],
            &["userRegion", "_aiRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["loginRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()));

    let client_id = resolve_auth_client_id_from_roots(&roots, platform);
    let login_host = resolve_trae_account_api_origin(
        platform,
        pick_string_multi(
            &roots,
            &[
                &["loginHost"],
                &["host"],
                &["account", "host"],
                &["callbackQuery", "host"],
                &["data", "host"],
                &["Result", "Host"],
                &["Result", "AIPayHost"],
                &["Result", "AIHost"],
                &["result", "loginHost"],
                &["data", "loginHost"],
                &["exchangeResponse", "Result", "loginHost"],
            ],
        )
        .as_deref(),
        store_region.as_deref(),
        ai_region.as_deref(),
        login_region.as_deref(),
    );

    TraeRefreshRoutingContext {
        platform,
        client_id,
        login_host,
        login_region,
        store_region,
        ai_region,
    }
}

fn build_refresh_api_urls(account: &TraeAccount, path: &str) -> Vec<String> {
    let context = build_refresh_routing_context(account);
    build_api_urls(context.login_host.as_str(), path)
}

fn merge_refresh_routing_context(response: &Value, context: &TraeRefreshRoutingContext) -> Value {
    let Some(response_obj) = response.as_object() else {
        return response.clone();
    };

    let mut merged = response_obj.clone();

    if !context.login_host.is_empty() {
        merged
            .entry("loginHost".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
        merged
            .entry("host".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
    }

    if let Some(login_region) = context.login_region.as_ref() {
        merged
            .entry("loginRegion".to_string())
            .or_insert_with(|| Value::String(login_region.clone()));
    }

    if let Some(store_region) = context.store_region.as_ref() {
        merged
            .entry("storeRegion".to_string())
            .or_insert_with(|| Value::String(store_region.clone()));
    }

    if let Some(ai_region) = context.ai_region.as_ref() {
        merged
            .entry("AIRegion".to_string())
            .or_insert_with(|| Value::String(ai_region.clone()));
    }
    merged
        .entry("authClientId".to_string())
        .or_insert_with(|| Value::String(context.client_id.clone()));

    Value::Object(merged)
}

fn merge_exchange_auth_raw(
    existing_auth_raw: Option<&Value>,
    exchange_response: &Value,
    context: &TraeRefreshRoutingContext,
    access_token: &str,
    refresh_token: Option<&str>,
    token_type: Option<&str>,
    expires_at: Option<i64>,
) -> Value {
    let mut merged = match existing_auth_raw {
        Some(Value::Object(obj)) => obj.clone(),
        _ => Map::new(),
    };

    merged.insert("exchangeResponse".to_string(), exchange_response.clone());
    merged.insert("token".to_string(), Value::String(access_token.to_string()));
    merged.insert(
        "accessToken".to_string(),
        Value::String(access_token.to_string()),
    );

    if let Some(refresh) = normalize_non_empty(refresh_token) {
        merged.insert("refreshToken".to_string(), Value::String(refresh));
    }

    if let Some(kind) = normalize_non_empty(token_type) {
        merged.insert("tokenType".to_string(), Value::String(kind));
    }

    let response_roots = [Some(exchange_response)];

    if let Some(expired_at) = resolve_iso_timestamp(
        expires_at,
        &response_roots,
        &[
            &["expiredAt"],
            &["expiresAt"],
            &["TokenExpireAt"],
            &["Result", "TokenExpireAt"],
        ],
    ) {
        merged.insert("expiredAt".to_string(), Value::String(expired_at));
    }

    if let Some(refresh_expired_at) = resolve_iso_timestamp(
        None,
        &response_roots,
        &[
            &["refreshExpiredAt"],
            &["RefreshExpireAt"],
            &["Result", "RefreshExpireAt"],
        ],
    ) {
        merged.insert(
            "refreshExpiredAt".to_string(),
            Value::String(refresh_expired_at),
        );
    }

    if let Some(token_release_at) =
        resolve_iso_timestamp(None, &response_roots, &[&["tokenReleaseAt"]])
    {
        merged.insert(
            "tokenReleaseAt".to_string(),
            Value::String(token_release_at),
        );
    }

    if !context.login_host.is_empty() {
        merged
            .entry("host".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
        merged
            .entry("loginHost".to_string())
            .or_insert_with(|| Value::String(context.login_host.clone()));
    }

    if let Some(login_region) = context.login_region.as_ref() {
        merged
            .entry("loginRegion".to_string())
            .or_insert_with(|| Value::String(login_region.clone()));
    }

    if let Some(store_region) = context.store_region.as_ref() {
        merged
            .entry("storeRegion".to_string())
            .or_insert_with(|| Value::String(store_region.clone()));
    }

    if let Some(ai_region) = context.ai_region.as_ref() {
        merged
            .entry("AIRegion".to_string())
            .or_insert_with(|| Value::String(ai_region.clone()));
    }

    Value::Object(merged)
}

fn header_value_or_dash(headers: &reqwest::header::HeaderMap, key: &str) -> String {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn build_body_preview(body_text: &str, max_chars: usize) -> String {
    let mut preview = String::new();
    let mut count = 0usize;
    for ch in body_text.chars() {
        if count >= max_chars {
            preview.push_str("...[truncated]");
            break;
        }
        match ch {
            '\n' => preview.push_str("\\n"),
            '\r' => preview.push_str("\\r"),
            '\t' => preview.push_str("\\t"),
            _ => preview.push(ch),
        }
        count += 1;
    }
    if preview.is_empty() {
        "<empty>".to_string()
    } else {
        preview
    }
}

async fn parse_trae_response_body(response: reqwest::Response, url: &str) -> Result<Value, String> {
    let status = response.status();
    let status_code = status.as_u16();
    if status_code == 401 || status_code == 403 {
        return Err("Trae 会话已过期或未认证，请重新登录".to_string());
    }

    let headers = response.headers();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let x_request_id = header_value_or_dash(headers, "x-request-id");
    let request_id = header_value_or_dash(headers, "request-id");
    let cf_ray = header_value_or_dash(headers, "cf-ray");

    let body_text = response
        .text()
        .await
        .map_err(|e| format!("读取 Trae 响应失败({}): {}", url, e))?;
    let body_trimmed = body_text.trim();
    if body_trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    serde_json::from_str::<Value>(&body_text).map_err(|e| {
        let body_preview = build_body_preview(body_trimmed, 200);
        format!(
            "解析 Trae 响应 JSON 失败({}): {} | status={} | content-type={} | x-request-id={} | request-id={} | cf-ray={} | body_preview={}",
            url,
            e,
            status_code,
            content_type,
            x_request_id,
            request_id,
            cf_ray,
            body_preview
        )
    })
}

async fn request_trae_json(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut request = client
        .request(method, url)
        .header("Accept", "application/json")
        .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("x-cloudide-token", access_token);

    if let Some(cookie_header) = cookie.and_then(|value| normalize_non_empty(Some(value))) {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(payload) = body {
        request = request
            .header("Content-Type", "application/json")
            .json(&payload);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("请求 Trae 接口失败({}): {}", url, e))?;

    parse_trae_response_body(response, url).await
}

async fn request_trae_json_with_candidates(
    client: &reqwest::Client,
    method: Method,
    urls: &[String],
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    for url in urls {
        match request_trae_json(
            client,
            method.clone(),
            url.as_str(),
            access_token,
            cookie,
            body.clone(),
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(err) => errors.push(format!("{} => {}", url, err)),
        }
    }

    if errors.is_empty() {
        return Err("Trae 请求地址为空".to_string());
    }

    Err(errors.join(" | "))
}

async fn request_trae_pay_json(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut request = client
        .request(method, url)
        .header("Accept", "application/json")
        .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
        .header("Authorization", format!("Cloud-IDE-JWT {}", access_token));

    if let Some(cookie_header) = cookie.and_then(|value| normalize_non_empty(Some(value))) {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(payload) = body {
        request = request
            .header("Content-Type", "application/json")
            .json(&payload);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("请求 Trae 接口失败({}): {}", url, e))?;

    parse_trae_response_body(response, url).await
}

async fn request_trae_pay_json_with_candidates(
    client: &reqwest::Client,
    method: Method,
    urls: &[String],
    access_token: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    for url in urls {
        match request_trae_pay_json(
            client,
            method.clone(),
            url.as_str(),
            access_token,
            cookie,
            body.clone(),
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(err) => errors.push(format!("{} => {}", url, err)),
        }
    }

    if errors.is_empty() {
        return Err("Trae 请求地址为空".to_string());
    }

    Err(errors.join(" | "))
}

fn apply_profile_response(account: &mut TraeAccount, response: &Value) {
    let profile_root = extract_response_data(response).unwrap_or(response);
    account.trae_profile_raw = Some(response.clone());

    if let Some(email) = normalize_email(
        pick_string(
            Some(profile_root),
            &[
                &["NonPlainTextEmail"],
                &["Email"],
                &["email"],
                &["user", "email"],
                &["userInfo", "email"],
                &["profile", "email"],
            ],
        )
        .as_deref(),
    ) {
        account.email = email;
    }

    if let Some(user_id) = normalize_non_empty(
        pick_string(
            Some(profile_root),
            &[
                &["UserID"],
                &["userId"],
                &["user_id"],
                &["uid"],
                &["id"],
                &["user", "id"],
            ],
        )
        .as_deref(),
    ) {
        account.user_id = Some(user_id);
    }

    if let Some(nickname) = normalize_non_empty(
        pick_string(
            Some(profile_root),
            &[
                &["ScreenName"],
                &["Nickname"],
                &["nickname"],
                &["name"],
                &["displayName"],
                &["user", "name"],
            ],
        )
        .as_deref(),
    ) {
        account.nickname = Some(nickname);
    }
}

fn usage_identity_from_product_type(product_type: i64) -> Option<&'static str> {
    match product_type {
        6 => Some("Ultra"),
        4 => Some("Pro+"),
        1 => Some("Pro"),
        9 => Some("Pro"),
        8 => Some("Lite"),
        0 => Some("Free"),
        _ => None,
    }
}

fn usage_pack_product_type(pack: &Value) -> Option<i64> {
    pick_i64(
        Some(pack),
        &[
            &["entitlement_base_info", "product_type"],
            &["product_type"],
        ],
    )
}

fn apply_entitlement_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_entitlement_raw = Some(response.clone());

    if let Some(plan_type) =
        normalize_non_empty(pick_string(Some(response), &[&["user_pay_identity_str"]]).as_deref())
    {
        account.plan_type = Some(plan_type);
    }

    account.plan_reset_at = normalize_timestamp(pick_i64(
        Some(response),
        &[&["detail", "subscription_renew_time"]],
    ));
}

fn apply_usage_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_usage_raw = Some(response.clone());

    if let Some(pack_list) = response
        .get("user_entitlement_pack_list")
        .and_then(|value| value.as_array())
    {
        let filtered_packs: Vec<&Value> = pack_list
            .iter()
            .filter(|pack| usage_pack_product_type(pack) != Some(3))
            .collect();

        let find_pack = |product_type: i64| {
            filtered_packs
                .iter()
                .copied()
                .find(|pack| usage_pack_product_type(pack) == Some(product_type))
        };

        let pack = find_pack(6)
            .or_else(|| find_pack(4))
            .or_else(|| find_pack(1))
            .or_else(|| find_pack(9))
            .or_else(|| find_pack(8))
            .or_else(|| find_pack(0));
        if let Some(pack) = pack {
            if let Some(product_type) = usage_pack_product_type(pack) {
                if let Some(identity) = usage_identity_from_product_type(product_type) {
                    account.plan_type = Some(identity.to_string());
                }
            }

            let reset_at = pick_i64(Some(pack), &[&["entitlement_base_info", "end_time"]])
                .and_then(|value| if value > 0 { Some(value + 1) } else { None });

            account.plan_reset_at = normalize_timestamp(reset_at);
        }
    }
}

fn apply_exchange_response(
    account: &mut TraeAccount,
    response: &Value,
    context: &TraeRefreshRoutingContext,
) {
    let merged_response = merge_refresh_routing_context(response, context);
    let exchange_root = extract_response_data(&merged_response).unwrap_or(&merged_response);

    let access_token = pick_string(
        Some(exchange_root),
        &[&["Token"], &["accessToken"], &["access_token"], &["token"]],
    );
    let refresh_token = pick_string(
        Some(exchange_root),
        &[&["RefreshToken"], &["refreshToken"], &["refresh_token"]],
    );
    let token_type = pick_string(
        Some(exchange_root),
        &[&["TokenType"], &["tokenType"], &["token_type"]],
    );
    let expires_at = normalize_timestamp(pick_i64(
        Some(exchange_root),
        &[
            &["TokenExpireAt"],
            &["expiresAt"],
            &["expires_at"],
            &["expired_at"],
        ],
    ));

    if let Some(token) = access_token {
        account.access_token = token;
    }
    if let Some(refresh) = refresh_token {
        account.refresh_token = Some(refresh);
    }
    if let Some(kind) = token_type {
        account.token_type = Some(kind);
    }
    if expires_at.is_some() {
        account.expires_at = expires_at;
    }

    account.trae_auth_raw = Some(merge_exchange_auth_raw(
        account.trae_auth_raw.as_ref(),
        &merged_response,
        context,
        account.access_token.as_str(),
        account.refresh_token.as_deref(),
        account.token_type.as_deref(),
        account.expires_at,
    ));
}

fn apply_check_login_response(
    account: &mut TraeAccount,
    response: &Value,
    context: &TraeRefreshRoutingContext,
) {
    let merged_response = merge_refresh_routing_context(response, context);
    let root = extract_response_data(&merged_response).unwrap_or(&merged_response);

    if let Some(status) = normalize_non_empty(
        pick_string(
            Some(root),
            &[&["status"], &["loginStatus"], &["authStatus"], &["result"]],
        )
        .as_deref(),
    ) {
        account.status = Some(status);
    }
    if let Some(reason) = normalize_non_empty(
        pick_string(
            Some(root),
            &[
                &["statusReason"],
                &["status_reason"],
                &["message"],
                &["error"],
            ],
        )
        .as_deref(),
    ) {
        account.status_reason = Some(reason);
    }

    account.trae_server_raw = Some(merged_response);
}

async fn refresh_account_async_once(account_id: &str) -> Result<TraeAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Trae Refresh] 开始刷新账号: id={}, email={}",
        existing.id, existing.email
    ));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut account = existing.clone();

    let cookie = pick_cookie_from_account(&account);
    let routing_context = build_refresh_routing_context(&account);
    logger::log_info(&format!(
        "[Trae Refresh] 使用路由: id={}, platform={}, host={}, login_region={}, store_region={}, ai_region={}",
        account.id,
        routing_context.platform.provider_key(),
        routing_context.login_host,
        routing_context.login_region.as_deref().unwrap_or("-"),
        routing_context.store_region.as_deref().unwrap_or("-"),
        routing_context.ai_region.as_deref().unwrap_or("-")
    ));

    if normalize_non_empty(account.refresh_token.as_deref()).is_none() {
        return Err("Trae refresh token 缺失，无法按官方流程刷新登录态".to_string());
    }

    let exchange_response = match request_exchange_token_by_official_refresh(
        &client,
        &account,
        &routing_context,
        cookie.as_deref(),
    )
    .await
    {
        Ok(response) => response,
        Err(official_err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 官方新版 ExchangeToken 失败，尝试旧接口 fallback: {}",
                official_err
            ));
            let exchange_body = serde_json::json!({
                "ClientID": routing_context.client_id.as_str(),
                "RefreshToken": account.refresh_token.clone().unwrap_or_default(),
                "ClientSecret": TRAE_EXCHANGE_CLIENT_SECRET,
                "UserID": "",
                "refreshToken": account.refresh_token.clone().unwrap_or_default(),
                "refresh_token": account.refresh_token.clone().unwrap_or_default(),
                "token": account.access_token.clone(),
            });
            let exchange_urls = build_refresh_api_urls(&account, TRAE_EXCHANGE_TOKEN_PATH);
            request_trae_json_with_candidates(
                &client,
                Method::POST,
                exchange_urls.as_slice(),
                &account.access_token,
                cookie.as_deref(),
                Some(exchange_body),
            )
            .await
            .map_err(|err| {
                format!(
                    "Trae ExchangeToken 失败: official={} | legacy={}",
                    official_err, err
                )
            })?
        }
    };
    let exchange_context = build_refresh_routing_context(&account);
    apply_exchange_response(&mut account, &exchange_response, &exchange_context);

    let profile_urls = build_refresh_api_urls(&account, TRAE_GET_USER_INFO_PATH);
    match request_trae_json_with_candidates(
        &client,
        Method::POST,
        profile_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await
    {
        Ok(response) => apply_profile_response(&mut account, &response),
        Err(err) => logger::log_warn(&format!("[Trae Refresh] GetUserInfo 失败: {}", err)),
    }

    let check_login_urls = build_refresh_api_urls(&account, TRAE_CHECK_LOGIN_PATH);
    match request_trae_json_with_candidates(
        &client,
        Method::POST,
        check_login_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "IDEVersion": TRAE_IDE_VERSION,
        })),
    )
    .await
    {
        Ok(response) => {
            let check_login_context = build_refresh_routing_context(&account);
            apply_check_login_response(&mut account, &response, &check_login_context);
        }
        Err(err) => logger::log_warn(&format!("[Trae Refresh] CheckLogin 失败: {}", err)),
    }

    let entitlement_urls = build_refresh_api_urls(&account, TRAE_PAY_STATUS_PATH);
    let entitlement_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        entitlement_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await;

    let mut quota_query_errors: Vec<String> = Vec::new();
    match entitlement_response {
        Ok(response) => apply_entitlement_response(&mut account, &response),
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_pay_status 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let usage_urls = build_refresh_api_urls(&account, TRAE_ENT_USAGE_PATH);
    let usage_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        usage_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "require_usage": true,
        })),
    )
    .await;

    let mut usage_refreshed = false;
    match usage_response {
        Ok(response) => {
            apply_usage_response(&mut account, &response);
            usage_refreshed = true;
        }
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_ent_usage 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let refreshed_at = now_ts();
    if usage_refreshed {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else if !quota_query_errors.is_empty() {
        account.quota_query_last_error = Some(quota_query_errors.join(" | "));
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;
    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae Refresh] 刷新完成: id={}, email={}",
        updated.id, updated.email
    ));
    Ok(updated)
}

pub async fn refresh_account_async(account_id: &str) -> Result<TraeAccount, String> {
    let result = refresh_account_async_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

fn apply_runtime_storage_payload_for_usage_refresh(
    account: &mut TraeAccount,
    runtime_storage_path: Option<&Path>,
) {
    let Some(storage_path) = runtime_storage_path else {
        return;
    };

    let payload = match read_local_trae_auth_from_storage_path(storage_path) {
        Ok(Some(payload)) => payload,
        Ok(None) => return,
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 读取运行中实例 storage 失败，跳过本地会话同步: path={}, error={}",
                storage_path.display(),
                err
            ));
            return;
        }
    };

    let payload_user_id = normalize_non_empty(payload.user_id.as_deref());
    let payload_email = normalize_identity_email(Some(payload.email.as_str()));
    if !account_matches_import_identity(
        account,
        payload_user_id.as_deref(),
        payload_email.as_deref(),
    ) {
        logger::log_warn(&format!(
            "[Trae Refresh] 运行中实例 storage 与目标账号不匹配，跳过本地会话同步: account_id={}, path={}",
            account.id,
            storage_path.display()
        ));
        return;
    }

    let account_platform = resolve_account_platform_kind(account);
    let payload_platform = resolve_payload_platform_kind(&payload);
    if !runtime_payload_matches_account_platform(account, &payload) {
        logger::log_warn(&format!(
            "[Trae Refresh] 运行中实例 storage 平台与目标账号不匹配，拒绝同步 Token 与认证上下文: account_id={}, expected_platform={}, resolved_platform={}, path={}",
            account.id,
            account_platform.provider_key(),
            payload_platform.provider_key(),
            storage_path.display()
        ));
        return;
    }

    let previous_access_token = account.access_token.clone();
    apply_runtime_session_payload(account, payload);
    logger::log_info(&format!(
        "[Trae Refresh] 已同步运行中实例会话快照: account_id={}, path={}, token_changed={}",
        account.id,
        storage_path.display(),
        if previous_access_token == account.access_token {
            "false"
        } else {
            "true"
        }
    ));
}

async fn refresh_account_usage_only_async_once(
    account_id: &str,
    runtime_storage_path: Option<&Path>,
) -> Result<TraeAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Trae Refresh] 开始仅额度刷新: id={}, email={}",
        existing.id, existing.email
    ));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut account = existing.clone();
    apply_runtime_storage_payload_for_usage_refresh(&mut account, runtime_storage_path);

    let cookie = pick_cookie_from_account(&account);
    let routing_context = build_refresh_routing_context(&account);
    logger::log_info(&format!(
        "[Trae Refresh] 仅额度刷新使用路由: id={}, platform={}, host={}, login_region={}, store_region={}, ai_region={}",
        account.id,
        routing_context.platform.provider_key(),
        routing_context.login_host,
        routing_context.login_region.as_deref().unwrap_or("-"),
        routing_context.store_region.as_deref().unwrap_or("-"),
        routing_context.ai_region.as_deref().unwrap_or("-")
    ));

    let entitlement_urls = build_refresh_api_urls(&account, TRAE_PAY_STATUS_PATH);
    let entitlement_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        entitlement_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({})),
    )
    .await;

    let mut quota_query_errors: Vec<String> = Vec::new();
    match entitlement_response {
        Ok(response) => apply_entitlement_response(&mut account, &response),
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_pay_status 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let usage_urls = build_refresh_api_urls(&account, TRAE_ENT_USAGE_PATH);
    let usage_response = request_trae_pay_json_with_candidates(
        &client,
        Method::POST,
        usage_urls.as_slice(),
        &account.access_token,
        cookie.as_deref(),
        Some(serde_json::json!({
            "require_usage": true,
        })),
    )
    .await;

    let mut usage_refreshed = false;
    match usage_response {
        Ok(response) => {
            apply_usage_response(&mut account, &response);
            usage_refreshed = true;
        }
        Err(err) => {
            logger::log_warn(&format!("[Trae Refresh] ide_user_ent_usage 失败: {}", err));
            quota_query_errors.push(err);
        }
    }

    let refreshed_at = now_ts();
    if usage_refreshed {
        account.quota_query_last_error = None;
        account.quota_query_last_error_at = None;
        account.usage_updated_at = Some(refreshed_at);
    } else if !quota_query_errors.is_empty() {
        account.quota_query_last_error = Some(quota_query_errors.join(" | "));
        account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    }
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    logger::log_info(&format!(
        "[Trae Refresh] 仅额度刷新完成: id={}, email={}",
        updated.id, updated.email
    ));
    Ok(updated)
}

pub async fn refresh_account_usage_only_async(
    account_id: &str,
    runtime_storage_path: Option<&Path>,
) -> Result<TraeAccount, String> {
    let result = refresh_account_usage_only_async_once(account_id, runtime_storage_path).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

async fn refresh_accounts(
    accounts: Vec<TraeAccount>,
) -> Result<Vec<(String, Result<TraeAccount, String>)>, String> {
    let protection_map = resolve_running_account_refresh_protection_map(&accounts);
    let mut results = Vec::with_capacity(accounts.len());
    for account in accounts {
        let account_id = account.id.clone();
        if let Some(storage_path) = protection_map.get(account_id.as_str()) {
            logger::log_info(&format!(
                "[Trae Refresh] 运行中实例账号走仅额度刷新: account_id={}, storage_path={}",
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            let result =
                refresh_account_usage_only_async(account_id.as_str(), storage_path.as_deref())
                    .await;
            results.push((account_id, result));
            continue;
        }
        let result = refresh_account_async(account_id.as_str()).await;
        results.push((account_id, result));
    }
    Ok(results)
}

pub async fn refresh_tokens_for_platform(
    platform: TraePlatformKind,
) -> Result<Vec<(String, Result<TraeAccount, String>)>, String> {
    let accounts = list_accounts_checked()?
        .into_iter()
        .filter(|account| resolve_account_platform_kind(account) == platform)
        .collect();
    refresh_accounts(accounts).await
}
