// Trae 账号模块：Token expiry windows, refresh decisions and runtime injection payloads。
// 通过 include! 保持原 modules::trae_account 作用域和平台行为。
pub(crate) fn resolve_account_platform_kind(account: &TraeAccount) -> TraePlatformKind {
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let roots = [
        account.trae_auth_raw.as_ref(),
        profile_root,
        account.trae_server_raw.as_ref(),
        account.trae_entitlement_raw.as_ref(),
        account.trae_usage_raw.as_ref(),
    ];
    resolve_platform_from_roots(&roots)
}

fn profile_payload_root(profile_raw: Option<&Value>) -> Option<&Value> {
    let root = profile_raw?;
    root.get("Result")
        .or_else(|| root.get("result"))
        .or_else(|| root.get("data"))
        .or(Some(root))
}

fn to_unix_millis(raw: i64) -> Option<i64> {
    if raw <= 0 {
        return None;
    }
    if raw > 10_000_000_000 {
        return Some(raw);
    }
    raw.checked_mul(1000)
}

fn normalize_iso_from_i64(raw: i64) -> Option<String> {
    let millis = to_unix_millis(raw)?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn normalize_iso_from_text(raw: Option<&str>) -> Option<String> {
    let normalized = normalize_non_empty(raw)?;
    let trimmed = normalized.trim();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(
            parsed
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    if let Ok(parsed) = trimmed.parse::<i64>() {
        return normalize_iso_from_i64(parsed);
    }
    None
}

fn normalize_iso_from_value(raw: Option<&Value>) -> Option<String> {
    let value = raw?;
    if let Some(text) = value.as_str() {
        return normalize_iso_from_text(Some(text));
    }
    if let Some(number) = value.as_i64() {
        return normalize_iso_from_i64(number);
    }
    if let Some(number) = value.as_u64() {
        if number <= i64::MAX as u64 {
            return normalize_iso_from_i64(number as i64);
        }
    }
    None
}

fn resolve_iso_timestamp(
    field_value: Option<i64>,
    roots: &[Option<&Value>],
    value_paths: &[&[&str]],
) -> Option<String> {
    if let Some(value) = field_value.and_then(normalize_iso_from_i64) {
        return Some(value);
    }

    for root in roots {
        for path in value_paths {
            if let Some(value) = extract_json_value(*root, path) {
                if let Some(normalized) = normalize_iso_from_value(Some(&value)) {
                    return Some(normalized);
                }
            }
        }
    }

    if let Some(value) = pick_i64_multi(roots, value_paths) {
        return normalize_iso_from_i64(value);
    }

    None
}

fn parse_iso_timestamp_millis(raw: Option<&str>) -> Option<i64> {
    let value = normalize_non_empty(raw)?;
    chrono::DateTime::parse_from_rfc3339(value.as_str())
        .ok()
        .map(|parsed| parsed.with_timezone(&chrono::Utc).timestamp_millis())
}

fn token_time_roots(account: &TraeAccount) -> [Option<&Value>; 3] {
    [
        account.trae_auth_raw.as_ref(),
        account.trae_server_raw.as_ref(),
        account.trae_profile_raw.as_ref(),
    ]
}

fn resolve_token_expired_at_millis(account: &TraeAccount) -> Option<i64> {
    let roots = token_time_roots(account);
    let expired_at = resolve_iso_timestamp(
        account.expires_at,
        &roots,
        &[
            &["expiredAt"],
            &["expiresAt"],
            &["exchangeResponse", "Result", "TokenExpireAt"],
            &["Result", "TokenExpireAt"],
            &["token", "expiredAt"],
        ],
    )?;
    parse_iso_timestamp_millis(Some(expired_at.as_str()))
}

fn resolve_token_release_at_millis(account: &TraeAccount) -> Option<i64> {
    let roots = token_time_roots(account);
    let token_release_at = resolve_iso_timestamp(
        None,
        &roots,
        &[
            &["tokenReleaseAt"],
            &["exchangeResponse", "Result", "TokenReleaseAt"],
            &["exchangeResponse", "Result", "tokenReleaseAt"],
            &["Result", "TokenReleaseAt"],
            &["Result", "tokenReleaseAt"],
        ],
    )?;
    parse_iso_timestamp_millis(Some(token_release_at.as_str()))
}

pub fn should_refresh_token_by_official_window(account: &TraeAccount) -> bool {
    if normalize_non_empty(Some(account.access_token.as_str())).is_none() {
        return false;
    }

    let Some(expired_at_ms) = resolve_token_expired_at_millis(account) else {
        return true;
    };

    let now = chrono::Utc::now().timestamp_millis();
    let remaining = expired_at_ms - now;
    if remaining <= 0 {
        return true;
    }

    if remaining <= TRAE_NEED_REFRESH_WINDOW_MILLISECONDS {
        return true;
    }

    if let Some(token_release_at_ms) = resolve_token_release_at_millis(account) {
        if expired_at_ms > token_release_at_ms {
            let lifecycle_one_third = (expired_at_ms - token_release_at_ms) / 3;
            if lifecycle_one_third > remaining {
                return true;
            }
        }
    }

    false
}

fn to_store_region(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cn" | "china-north" => "CN".to_string(),
        "sg" | "singapore-central" => "SG".to_string(),
        "us" | "us-east" | "us-east-1" => "US".to_string(),
        "usttp" => "USTTP".to_string(),
        "unknown" => "UNKNOWN".to_string(),
        other if other.is_empty() => "UNKNOWN".to_string(),
        _ => raw.trim().to_string(),
    }
}

fn resolve_user_tag_for_inject(account: &TraeAccount) -> String {
    let user_id = resolve_account_user_id_for_inject(account);

    if let Some(raw_text) = normalize_non_empty(account.trae_usertag_raw.as_deref()) {
        if let Some(map) = decode_usertag_map(raw_text.as_str()) {
            if let Some(uid) = user_id.as_ref() {
                if let Some(value) = map.get(uid) {
                    return value.clone();
                }
            }
            if map.len() == 1 {
                if let Some(value) = map.values().next() {
                    return value.clone();
                }
            }
        }
        if let Some(value) = normalize_usertag_value(Some(raw_text.as_str())) {
            return value;
        }
    }

    normalize_usertag_value(
        pick_string(
            account.trae_auth_raw.as_ref(),
            &[
                &["account", "userTag"],
                &["userTag"],
                &["callbackQuery", "userTag"],
                &["rawQuery", "userTag"],
                &["data", "userTag"],
            ],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_usertag_value(
            pick_string(
                account.trae_server_raw.as_ref(),
                &[&["account", "userTag"], &["userTag"], &["data", "userTag"]],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "row".to_string())
}

fn merge_usertag_map_for_inject(
    root_obj: &Map<String, Value>,
    user_id: Option<&str>,
    user_tag: &str,
) -> Result<Option<String>, String> {
    let mut map = root_obj
        .get(TRAE_STORAGE_USERTAG_KEY)
        .and_then(|value| value.as_str())
        .and_then(decode_usertag_map)
        .unwrap_or_default();

    if let Some(uid) = user_id.and_then(|value| normalize_non_empty(Some(value))) {
        map.insert(uid, user_tag.to_ascii_lowercase());
    }

    if map.is_empty() {
        return Ok(None);
    }

    let encoded = encode_usertag_map(&map)?;
    Ok(Some(encoded))
}

fn resolve_existing_device_key_storage_id(root_obj: &Map<String, Value>) -> Option<String> {
    for key in root_obj.keys() {
        let Some(device_id) = key.strip_prefix(TRAE_STORAGE_DEVICE_KEY_PREFIX) else {
            continue;
        };
        if let Some(normalized) = normalize_non_empty(Some(device_id)) {
            return Some(normalized);
        }
    }
    None
}

fn resolve_device_id_for_inject(
    root_obj: &Map<String, Value>,
    account: &TraeAccount,
) -> Option<String> {
    pick_string_multi(
        &[
            account.trae_auth_raw.as_ref(),
            account.trae_server_raw.as_ref(),
        ],
        &[
            &["deviceInfo", "DeviceID"],
            &["deviceInfo", "deviceId"],
            &["DeviceID"],
            &["deviceId"],
            &["callbackQuery", "device_id"],
            &["callbackQuery", "x_device_id"],
        ],
    )
    .or_else(|| resolve_existing_device_key_storage_id(root_obj))
}

fn normalize_device_key_pair_value(value: &Value) -> Option<Value> {
    let private_key = pick_string(Some(value), &[&["privateKeyPEM"], &["private_key_pem"]])?;
    let public_key = pick_string(Some(value), &[&["publicKeyPEM"], &["public_key_pem"]])?;
    Some(serde_json::json!({
        "privateKeyPEM": private_key,
        "publicKeyPEM": public_key,
    }))
}

fn resolve_device_key_pair_for_inject(account: &TraeAccount) -> Option<Value> {
    let auth_raw = account.trae_auth_raw.as_ref()?;
    auth_raw
        .get("deviceKeyPair")
        .and_then(normalize_device_key_pair_value)
        .or_else(|| normalize_device_key_pair_value(auth_raw))
}

fn write_device_key_pair_for_inject(
    root_obj: &mut Map<String, Value>,
    account: &TraeAccount,
) -> Result<(), String> {
    let Some(device_key_pair) = resolve_device_key_pair_for_inject(account) else {
        return Ok(());
    };
    let Some(device_id) = resolve_device_id_for_inject(root_obj, account) else {
        return Ok(());
    };
    let storage_key = build_device_key_storage_key(device_id.as_str());
    root_obj.insert(storage_key, to_icube_cipher_string_value(&device_key_pair)?);
    Ok(())
}

fn resolve_storage_keys_for_inject(root_obj: &Map<String, Value>) -> (String, String, String) {
    let provider_id = resolve_storage_provider_id(root_obj);
    (
        build_auth_storage_key(provider_id.as_str()),
        build_server_storage_key(provider_id.as_str()),
        build_entitlement_storage_key(provider_id.as_str()),
    )
}

fn resolve_account_user_id_for_auth_object(
    account: &TraeAccount,
    roots: &[Option<&Value>],
) -> String {
    normalize_non_empty(account.user_id.as_deref())
        .or_else(|| {
            pick_string_multi(
                roots,
                &[&["userId"], &["user_id"], &["uid"], &["UserID"], &["id"]],
            )
        })
        .unwrap_or_default()
}

fn ensure_auth_raw_for_inject(account: &TraeAccount, existing_auth_raw: Option<&Value>) -> Value {
    let auth_raw = account.trae_auth_raw.as_ref();
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let server_raw = account.trae_server_raw.as_ref();
    let platform = resolve_account_platform_kind(account);

    let roots = [auth_raw, profile_root, server_raw];
    let user_tag = resolve_user_tag_for_inject(account);

    let user_id = resolve_account_user_id_for_auth_object(account, &roots);

    let username = normalize_non_empty(account.nickname.as_deref())
        .or_else(|| {
            pick_string_multi(
                &roots,
                &[
                    &["ScreenName"],
                    &["nickname"],
                    &["name"],
                    &["displayName"],
                    &["account", "username"],
                ],
            )
        })
        .unwrap_or_else(|| account.email.clone());

    let email = normalize_email(
        pick_string_multi(
            &roots,
            &[
                &["NonPlainTextEmail"],
                &["account", "nonPlainTextEmail"],
                &["account", "email"],
                &["email"],
                &["user", "email"],
            ],
        )
        .as_deref(),
    )
    .unwrap_or_else(|| account.email.clone());

    let avatar_url = pick_string_multi(
        &roots,
        &[&["AvatarUrl"], &["avatar_url"], &["account", "avatar_url"]],
    )
    .unwrap_or_default();
    let description = pick_string_multi(
        &roots,
        &[
            &["Description"],
            &["description"],
            &["account", "description"],
        ],
    )
    .unwrap_or_default();

    let scope = pick_string_multi(&roots, &[&["account", "scope"], &["scope"]])
        .and_then(|value| normalize_non_empty(Some(value.as_str())))
        .map(|value| {
            if value.trim().eq_ignore_ascii_case("trae") {
                "marscode".to_string()
            } else {
                value
            }
        })
        .unwrap_or_else(|| "marscode".to_string());
    let login_scope = pick_string_multi(
        &roots,
        &[
            &["account", "loginScope"],
            &["loginScope"],
            &["callbackQuery", "scope"],
        ],
    )
    .and_then(|value| normalize_non_empty(Some(value.as_str())))
    .unwrap_or_else(|| "trae".to_string());

    let store_country_code = pick_string_multi(
        &roots,
        &[
            &["StoreCountry"],
            &["storeCountry"],
            &["account", "storeCountryCode"],
        ],
    )
    .unwrap_or_default();
    let store_country_src = pick_string_multi(
        &roots,
        &[
            &["StoreCountrySrc"],
            &["storeCountrySrc"],
            &["account", "storeCountrySrc"],
        ],
    )
    .unwrap_or_default();
    let store_region = pick_string_multi(
        &roots,
        &[
            &["account", "storeRegion"],
            &["storeRegion"],
            &["loginRegion"],
            &["callbackQuery", "userRegion"],
            &["userRegion"],
            &["AIRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()))
    .unwrap_or_else(|| "UNKNOWN".to_string());

    let ai_region_roots = [profile_root, server_raw, auth_raw];
    let ai_region = pick_string_multi(
        &ai_region_roots,
        &[
            &["AIRegion"],
            &["userRegion", "_aiRegion"],
            &["userRegion", "region"],
            &["callbackQuery", "userRegion"],
            &["loginRegion"],
        ],
    )
    .map(|value| to_store_region(value.as_str()))
    .unwrap_or_else(|| "UNKNOWN".to_string());

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

    let api_host = resolve_trae_auth_storage_origin(
        platform,
        pick_string_multi(
            &roots,
            &[
                &["callbackQuery", "host"],
                &["data", "host"],
                &["loginHost"],
                &["host"],
                &["Result", "Host"],
                &["Result", "AIPayHost"],
                &["Result", "AIHost"],
            ],
        )
        .as_deref(),
        Some(store_region.as_str()),
        Some(ai_region.as_str()),
        login_region.as_deref(),
    );
    let client_id = resolve_auth_client_id_from_roots(&roots, platform);

    let expires_at = resolve_iso_timestamp(
        account.expires_at,
        &roots,
        &[
            &["expiredAt"],
            &["expiresAt"],
            &["TokenExpireAt"],
            &["exchangeResponse", "Result", "TokenExpireAt"],
        ],
    )
    .unwrap_or_else(|| {
        (chrono::Utc::now() + chrono::Duration::days(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    });

    let refresh_expired_at = resolve_iso_timestamp(
        None,
        &roots,
        &[
            &["refreshExpiredAt"],
            &["RefreshExpireAt"],
            &["Result", "RefreshExpireAt"],
            &["exchangeResponse", "Result", "RefreshExpireAt"],
            &["callbackQuery", "refreshExpireAt"],
        ],
    )
    .unwrap_or_else(|| expires_at.clone());

    let token_release_at = resolve_iso_timestamp(None, &roots, &[&["tokenReleaseAt"]])
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    let refresh_token = normalize_non_empty(account.refresh_token.as_deref())
        .or_else(|| {
            pick_string_multi(
                &roots,
                &[
                    &["refreshToken"],
                    &["refresh_token"],
                    &["RefreshToken"],
                    &["exchangeResponse", "Result", "RefreshToken"],
                ],
            )
        })
        .unwrap_or_default();

    // Merge official auth payloads from both the imported account and current storage so we
    // keep unknown fields across upgrades, switches, and previously "thinned" injections.
    let mut obj = auth_raw
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_obj) = existing_auth_raw.and_then(Value::as_object) {
        for (key, value) in existing_obj {
            obj.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    let had_token_type_key = obj.contains_key("tokenType") || obj.contains_key("token_type");
    let had_region_key = obj.contains_key("region");
    let had_ai_region_key = obj.contains_key("aiRegion");

    let mut account_obj = auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("account"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_account_obj) = existing_auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("account"))
        .and_then(Value::as_object)
    {
        for (key, value) in existing_account_obj {
            account_obj
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }

    account_obj.insert("username".to_string(), Value::String(username));
    account_obj.insert("iss".to_string(), Value::String(String::new()));
    account_obj.insert(
        "iat".to_string(),
        Value::Number(serde_json::Number::from(0)),
    );
    account_obj.insert("organization".to_string(), Value::String(String::new()));
    account_obj.insert("work_country".to_string(), Value::String(String::new()));
    account_obj.insert("email".to_string(), Value::String(email));
    account_obj.insert("avatar_url".to_string(), Value::String(avatar_url));
    account_obj.insert("description".to_string(), Value::String(description));
    account_obj.insert("scope".to_string(), Value::String(scope));
    account_obj.insert("loginScope".to_string(), Value::String(login_scope));
    account_obj.insert(
        "storeCountryCode".to_string(),
        Value::String(store_country_code),
    );
    account_obj.insert(
        "storeCountrySrc".to_string(),
        Value::String(store_country_src),
    );
    account_obj.insert("storeRegion".to_string(), Value::String(store_region));
    account_obj.insert("userTag".to_string(), Value::String(user_tag));

    let mut user_region = auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("userRegion"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_user_region) = existing_auth_raw
        .and_then(Value::as_object)
        .and_then(|value| value.get("userRegion"))
        .and_then(Value::as_object)
    {
        for (key, value) in existing_user_region {
            user_region
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    user_region.insert("region".to_string(), Value::String(ai_region.clone()));
    user_region.insert("_aiRegion".to_string(), Value::String(ai_region.clone()));

    obj.insert(
        "token".to_string(),
        Value::String(account.access_token.clone()),
    );
    obj.insert(
        "accessToken".to_string(),
        Value::String(account.access_token.clone()),
    );
    if !refresh_token.is_empty() {
        obj.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if !user_id.is_empty() {
        obj.insert("userId".to_string(), Value::String(user_id));
    }
    if let Some(token_type) = account
        .token_type
        .clone()
        .or_else(|| pick_string_multi(&roots, &[&["tokenType"], &["token_type"]]))
    {
        if had_token_type_key {
            obj.insert("tokenType".to_string(), Value::String(token_type));
        }
    }

    obj.insert("expiredAt".to_string(), Value::String(expires_at));
    obj.insert(
        "refreshExpiredAt".to_string(),
        Value::String(refresh_expired_at),
    );
    obj.insert(
        "tokenReleaseAt".to_string(),
        Value::String(token_release_at),
    );
    obj.insert("host".to_string(), Value::String(api_host.clone()));
    obj.insert("loginHost".to_string(), Value::String(api_host));
    insert_platform_metadata(&mut obj, platform);
    obj.insert("authClientId".to_string(), Value::String(client_id));
    if had_region_key {
        obj.insert("region".to_string(), Value::String(ai_region.clone()));
    }
    if had_ai_region_key {
        obj.insert("aiRegion".to_string(), Value::String(ai_region.clone()));
    }
    obj.insert("userRegion".to_string(), Value::Object(user_region));
    obj.insert("account".to_string(), Value::Object(account_obj));

    Value::Object(obj)
}

fn should_write_server_data_for_inject(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("entitlementInfo")
        || obj.contains_key("serverTimeInfo")
        || obj.contains_key("commercialActivityInfo")
        || obj.contains_key("soloCnInfo")
        || obj.contains_key("saasEntitlementInfo")
}

fn ensure_server_raw_for_inject(account: &TraeAccount) -> Option<Value> {
    let raw = account.trae_server_raw.clone()?;
    if should_write_server_data_for_inject(&raw) {
        return Some(raw);
    }
    None
}

fn ensure_entitlement_raw_for_inject(account: &TraeAccount) -> Option<Value> {
    account.trae_entitlement_raw.clone()
}

fn read_local_trae_auth_from_storage_path(
    storage_path: &Path,
) -> Result<Option<TraeImportPayload>, String> {
    if !storage_path.exists() {
        return Ok(None);
    }
    let storage_root = read_storage_json(storage_path)?;
    let payload = payload_from_storage_root(&storage_root)?;
    Ok(Some(payload))
}

pub(crate) fn read_local_trae_user_id_from_storage_path(
    storage_path: &Path,
) -> Result<Option<String>, String> {
    Ok(read_local_trae_auth_from_storage_path(storage_path)?
        .and_then(|payload| normalize_non_empty(payload.user_id.as_deref())))
}

pub(crate) fn account_user_id_for_local_session(account: &TraeAccount) -> Option<String> {
    resolve_account_user_id_for_inject(account)
}

/// Persist a resolved official user id onto the saved account when missing.
/// Helps later session-share switches without requiring a full re-import.
pub(crate) fn backfill_account_user_id_if_missing(
    account_id: &str,
    user_id: &str,
) -> Result<bool, String> {
    let Some(mut account) = load_account(account_id) else {
        return Ok(false);
    };
    if normalize_non_empty(account.user_id.as_deref()).is_some() {
        return Ok(false);
    }
    let Some(uid) = normalize_non_empty(Some(user_id)) else {
        return Ok(false);
    };
    account.user_id = Some(uid.clone());
    account.last_used = chrono::Utc::now().timestamp_millis();
    save_account_file(&account)?;
    // Keep index summary in sync for UI/debug.
    if let Ok(mut index) = load_account_index_checked() {
        if let Some(item) = index.accounts.iter_mut().find(|item| item.id == account.id) {
            item.user_id = Some(uid.clone());
            item.last_used = account.last_used;
            let _ = save_account_index(&index);
        }
    }
    logger::log_info(&format!(
        "[Trae Account] 回填官方用户 ID: account_id={}, user_id={}",
        account_id, uid
    ));
    Ok(true)
}

pub fn read_local_trae_auth() -> Result<Option<TraeImportPayload>, String> {
    read_local_trae_auth_for_platform(TraePlatformKind::Trae)
}

pub fn read_local_trae_auth_for_platform(
    platform: TraePlatformKind,
) -> Result<Option<TraeImportPayload>, String> {
    let storage_path = get_default_trae_storage_path_for_platform(platform)?;
    read_local_trae_auth_from_storage_path(&storage_path)
}

pub fn import_from_local() -> Result<Option<TraeAccount>, String> {
    import_from_local_for_platform(TraePlatformKind::Trae)
}

pub fn import_from_local_for_platform(
    platform: TraePlatformKind,
) -> Result<Option<TraeAccount>, String> {
    let mut payload = match read_local_trae_auth_for_platform(platform)? {
        Some(payload) => payload,
        None => return Ok(None),
    };
    attach_platform_metadata_to_payload(&mut payload, platform);
    let account = upsert_account(payload)?;
    logger::log_info(&format!(
        "[Trae Account] 本地导入成功: platform={}, id={}, email={}",
        platform.provider_key(),
        account.id,
        account.email
    ));
    Ok(Some(account))
}

pub(crate) fn resolve_current_account_id(accounts: &[TraeAccount]) -> Option<String> {
    resolve_current_account_id_for_platform(accounts, TraePlatformKind::Trae)
}

pub(crate) fn resolve_current_account_id_for_platform(
    accounts: &[TraeAccount],
    platform: TraePlatformKind,
) -> Option<String> {
    if let Ok(Some(payload)) = read_local_trae_auth_for_platform(platform) {
        let normalized_user_id = normalize_non_empty(payload.user_id.as_deref());
        let normalized_email = normalize_email(Some(payload.email.as_str()));

        if let Some(account_id) = accounts
            .iter()
            .find(|account| {
                if resolve_account_platform_kind(account) != platform {
                    return false;
                }

                if let (Some(existing), Some(incoming)) = (
                    normalize_non_empty(account.user_id.as_deref()),
                    normalized_user_id.clone(),
                ) {
                    if existing == incoming {
                        return true;
                    }
                }

                if let (Some(existing), Some(incoming)) = (
                    normalize_email(Some(account.email.as_str())),
                    normalized_email.clone(),
                ) {
                    return existing == incoming;
                }

                false
            })
            .map(|account| account.id.clone())
        {
            return Some(account_id);
        }
    }

    crate::modules::provider_current_state::resolve_existing_current_account_id(
        platform.provider_key(),
        accounts
            .iter()
            .filter(|account| resolve_account_platform_kind(account) == platform)
            .map(|account| account.id.as_str()),
    )
}

pub(crate) fn resolve_running_account_refresh_protection_map(
    accounts: &[TraeAccount],
) -> BTreeMap<String, Option<PathBuf>> {
    let mut protected = BTreeMap::new();

    for platform in all_trae_platform_kinds() {
        if crate::modules::process::is_trae_running_for_platform(platform) {
            if let Some(current_id) = resolve_current_account_id_for_platform(accounts, platform) {
                let default_storage_path =
                    get_default_trae_storage_path_for_platform(platform).ok();
                protected.insert(current_id, default_storage_path);
            }
        }
    }

    match crate::modules::trae_instance::resolve_running_bound_account_contexts() {
        Ok(contexts) => {
            for context in contexts {
                let account_id = context.account_id;
                let storage_path = context.storage_path;
                protected
                    .entry(account_id)
                    .and_modify(|current| {
                        if current.is_none() {
                            *current = Some(storage_path.clone());
                        }
                    })
                    .or_insert(Some(storage_path));
            }
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 读取运行中实例绑定账号失败，跳过实例保护名单: {}",
                err
            ));
        }
    }

    protected
}

pub fn inject_to_trae(account_id: &str) -> Result<(), String> {
    inject_to_trae_for_platform(TraePlatformKind::Trae, account_id)
}

pub fn inject_to_trae_for_platform(
    platform: TraePlatformKind,
    account_id: &str,
) -> Result<(), String> {
    let storage_path = get_default_trae_storage_path_for_platform(platform)?;
    inject_to_trae_at_path(storage_path.as_path(), account_id)
}

fn storage_paths_equivalent(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => left.to_string_lossy() == right.to_string_lossy(),
    }
}

/// Refuse overwriting a storage.json that is currently owned by a running Trae process.
/// Mutual refresh/inject while the official client holds the session rotates tokens and
/// kicks the live login; switch/start paths close Trae first so this guard stays out of the way.
fn refuse_inject_if_storage_live(storage_path: &Path, account_id: &str) -> Result<(), String> {
    for platform in all_trae_platform_kinds() {
        if !crate::modules::process::is_trae_running_for_platform(platform) {
            continue;
        }
        if let Ok(default_path) = get_default_trae_storage_path_for_platform(platform) {
            if storage_paths_equivalent(default_path.as_path(), storage_path) {
                return Err(format!(
                    "账号所在的 {} 正在运行，已拒绝回写 storage 以免互刷顶号。请先关闭客户端后再切号或注入: account_id={}, path={}",
                    platform.display_name(),
                    account_id,
                    storage_path.display()
                ));
            }
        }
    }

    if let Ok(contexts) = crate::modules::trae_instance::resolve_running_bound_account_contexts() {
        for context in contexts {
            if storage_paths_equivalent(context.storage_path.as_path(), storage_path) {
                return Err(format!(
                    "目标 storage 对应实例正在运行，已拒绝回写以免互刷顶号: account_id={}, running_account_id={}, path={}",
                    account_id,
                    context.account_id,
                    storage_path.display()
                ));
            }
        }
    }

    Ok(())
}

pub fn inject_to_trae_at_path(storage_path: &Path, account_id: &str) -> Result<(), String> {
    refuse_inject_if_storage_live(storage_path, account_id)?;

    let mut account =
        load_account(account_id).ok_or_else(|| format!("Trae 账号不存在: {}", account_id))?;
    // If the target storage already holds this account's fresher rotated tokens,
    // adopt them before rewrite so we never downgrade a live Trae session snapshot.
    if storage_path.exists()
        && sync_account_tokens_from_storage_path(&mut account, storage_path, "注入前本地", true)
    {
        if let Err(err) = save_account_file(&account) {
            logger::log_warn(&format!(
                "[Trae Account] 注入前同步 storage 后落盘失败: account_id={}, error={}",
                account_id, err
            ));
        }
    }

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
    root_obj.insert(auth_storage_key, to_user_auth_storage_value(&auth_raw)?);
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

fn usage_identity_from_product_type(product_type: i64, is_cn: bool) -> Option<&'static str> {
    match product_type {
        // CN 专属套餐（社区 #1281 / 官方 CN 权益 product_type）
        100 if is_cn => Some("CNExpress"),
        6 => Some("Ultra"),
        5 => Some("Pro+"),
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

fn usage_response_payload_root(response: &Value) -> &Value {
    response
        .get("data")
        .or_else(|| response.get("Result"))
        .or_else(|| response.get("result"))
        .or_else(|| response.get("payload"))
        .or_else(|| response.get("user_current_entitlement_list"))
        .or_else(|| response.get("ide_user_ent_usage"))
        .unwrap_or(response)
}

fn mark_trae_usage_source(response: &mut Value, source: &str) {
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "_cockpit_source".to_string(),
            Value::String(source.to_string()),
        );
        for key in [
            "data",
            "Result",
            "result",
            "payload",
            "user_current_entitlement_list",
            "ide_user_ent_usage",
        ] {
            if let Some(inner) = object.get_mut(key).and_then(|value| value.as_object_mut()) {
                inner.insert(
                    "_cockpit_source".to_string(),
                    Value::String(source.to_string()),
                );
            }
        }
    }
}

fn apply_entitlement_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_entitlement_raw = Some(response.clone());

    let entitlement_root = usage_response_payload_root(response);
    let plan_from_root = pick_string(
        Some(entitlement_root),
        &[
            &["user_pay_identity_str"],
            &["identityStr"],
            &["detail", "user_pay_identity_str"],
        ],
    );
    let plan_from_response = pick_string(
        Some(response),
        &[&["user_pay_identity_str"], &["identityStr"]],
    );
    if let Some(plan_type) =
        normalize_non_empty(plan_from_root.as_deref().or(plan_from_response.as_deref()))
    {
        account.plan_type = Some(plan_type);
    }

    let reset_from_root = pick_i64(
        Some(entitlement_root),
        &[
            &["detail", "subscription_renew_time"],
            &["detail", "subscriptionRenewTime"],
        ],
    );
    let reset_from_response = pick_i64(
        Some(response),
        &[
            &["detail", "subscription_renew_time"],
            &["detail", "subscriptionRenewTime"],
        ],
    );
    account.plan_reset_at = normalize_timestamp(reset_from_root.or(reset_from_response));
}

fn apply_usage_response(account: &mut TraeAccount, response: &Value) {
    if let Some(code) = pick_i64(Some(response), &[&["code"]]) {
        if code != 0 {
            return;
        }
    }

    account.trae_usage_raw = Some(response.clone());
    let is_cn = resolve_account_platform_kind(account).is_cn();
    let usage_root = usage_response_payload_root(response);

    if let Some(pack_list) = usage_root
        .get("user_entitlement_pack_list")
        .or_else(|| response.get("user_entitlement_pack_list"))
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

        // CN：CNExpress(100) > Ultra > Pro+(5/4) > Pro > Trial/SoloInvite > Lite > Free
        let pack = if is_cn {
            find_pack(100)
                .or_else(|| find_pack(6))
                .or_else(|| find_pack(5))
                .or_else(|| find_pack(4))
                .or_else(|| find_pack(1))
                .or_else(|| find_pack(9))
                .or_else(|| find_pack(8))
                .or_else(|| find_pack(0))
        } else {
            find_pack(6)
                .or_else(|| find_pack(4))
                .or_else(|| find_pack(1))
                .or_else(|| find_pack(9))
                .or_else(|| find_pack(8))
                .or_else(|| find_pack(0))
        };
        if let Some(pack) = pack {
            if let Some(product_type) = usage_pack_product_type(pack) {
                if let Some(identity) = usage_identity_from_product_type(product_type, is_cn) {
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

fn evaluate_check_login_response(response: &Value) -> TraeCheckLoginVerdict {
    let error_code = normalize_non_empty(
        pick_string(
            Some(response),
            &[
                &["ResponseMetadata", "Error", "Code"],
                &["responseMetadata", "error", "code"],
                &["error", "code"],
            ],
        )
        .as_deref(),
    );
    let is_login = pick_bool(
        Some(response),
        &[&["Result", "IsLogin"], &["result", "isLogin"], &["isLogin"]],
    );
    let invalid_by_code = error_code
        .as_deref()
        .map(|code| {
            TRAE_CHECK_LOGIN_INVALID_ERROR_CODES
                .iter()
                .any(|invalid_code| *invalid_code == code)
        })
        .unwrap_or(false);
    let invalid_by_login = matches!(is_login, Some(false));

    TraeCheckLoginVerdict {
        is_valid: !invalid_by_code && !invalid_by_login,
        error_code,
        is_login,
    }
}

async fn request_check_login_for_account(account: &TraeAccount) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let cookie = pick_cookie_from_account(account);
    let check_login_urls = build_refresh_api_urls(account, TRAE_CHECK_LOGIN_PATH);
    request_trae_json_with_candidates(
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
}

pub async fn check_login_token(account_id: &str) -> Result<TraeCheckLoginVerdict, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    let response = request_check_login_for_account(&existing).await?;
    let verdict = evaluate_check_login_response(&response);

    let mut account = existing.clone();
    let context = build_refresh_routing_context(&account);
    apply_check_login_response(&mut account, &response, &context);
    account.last_used = now_ts();
    if let Err(err) = upsert_account_record(account) {
        logger::log_warn(&format!(
            "[Trae CheckLogin] 同步检查结果到账号存储失败: account_id={}, error={}",
            existing.id, err
        ));
    }

    logger::log_info(&format!(
        "[Trae CheckLogin] 检查完成: account_id={}, valid={}, error_code={}, is_login={}",
        existing.id,
        verdict.is_valid,
        verdict.error_code.as_deref().unwrap_or("-"),
        verdict
            .is_login
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("-")
    ));
    Ok(verdict)
}

pub async fn check_login_then_refresh_if_needed(account_id: &str) -> Result<bool, String> {
    let verdict = check_login_token(account_id).await?;
    if verdict.is_valid {
        return Ok(false);
    }

    if let Ok(accounts) = list_accounts_checked() {
        let protection_map = resolve_running_account_refresh_protection_map(&accounts);
        if let Some(storage_path) = protection_map.get(account_id) {
            logger::log_warn(&format!(
                "[Trae CheckLogin] 账号处于运行中实例，跳过 Token 刷新，改为仅额度刷新: account_id={}, storage_path={}",
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            refresh_account_usage_only_async(account_id, storage_path.as_deref()).await?;
            return Ok(false);
        }
    }

    logger::log_warn(&format!(
        "[Trae CheckLogin] 检测到账号状态异常，开始静默刷新: account_id={}, error_code={}, is_login={}",
        account_id,
        verdict.error_code.as_deref().unwrap_or("-"),
        verdict
            .is_login
            .map(|value| if value { "true" } else { "false" })
            .unwrap_or("-")
    ));
    refresh_account_async(account_id).await?;
    Ok(true)
}

fn apply_runtime_storage_payload_for_usage_refresh(
    account: &mut TraeAccount,
    runtime_storage_path: Option<&Path>,
) {
    let Some(storage_path) = runtime_storage_path else {
        return;
    };
    sync_account_tokens_from_storage_path(account, storage_path, "运行中实例", false);
}

fn runtime_storage_is_newer(
    account_modified: std::time::SystemTime,
    storage_modified: std::time::SystemTime,
) -> bool {
    storage_modified > account_modified
}

fn storage_is_newer_than_saved_account(account: &TraeAccount, storage_path: &Path) -> bool {
    let account_modified = resolve_account_file_path(account.id.as_str())
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok());
    let storage_modified = fs::metadata(storage_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    match (account_modified, storage_modified) {
        (Some(account_modified), Some(storage_modified)) => {
            runtime_storage_is_newer(account_modified, storage_modified)
        }
        _ => false,
    }
}

/// Pull fresher tokens from Trae `storage.json` when identity matches.
/// Critical for avoiding refresh-token races: Trae may have already rotated
/// refresh tokens on disk while Cockpit still holds a stale copy.
fn sync_account_tokens_from_storage_path(
    account: &mut TraeAccount,
    storage_path: &Path,
    source_label: &str,
    require_newer_than_saved_account: bool,
) -> bool {
    if require_newer_than_saved_account
        && !storage_is_newer_than_saved_account(account, storage_path)
    {
        logger::log_info(&format!(
            "[Trae Refresh] {} storage 不晚于账号凭据，跳过本地会话同步: account_id={}, path={}",
            source_label,
            account.id,
            storage_path.display()
        ));
        return false;
    }

    let payload = match read_local_trae_auth_from_storage_path(storage_path) {
        Ok(Some(payload)) => payload,
        Ok(None) => return false,
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Refresh] 读取{} storage 失败，跳过本地会话同步: path={}, error={}",
                source_label,
                storage_path.display(),
                err
            ));
            return false;
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
            "[Trae Refresh] {} storage 与目标账号不匹配，跳过本地会话同步: account_id={}, path={}",
            source_label,
            account.id,
            storage_path.display()
        ));
        return false;
    }

    let account_platform = resolve_account_platform_kind(account);
    let payload_platform = resolve_payload_platform_kind(&payload);
    if !runtime_payload_matches_account_platform(account, &payload) {
        logger::log_warn(&format!(
            "[Trae Refresh] {} storage 平台与目标账号不匹配，拒绝同步 Token 与认证上下文: account_id={}, expected_platform={}, resolved_platform={}, path={}",
            source_label,
            account.id,
            account_platform.provider_key(),
            payload_platform.provider_key(),
            storage_path.display()
        ));
        return false;
    }

    let previous_access_token = account.access_token.clone();
    let previous_refresh_token = account.refresh_token.clone();
    apply_runtime_session_payload(account, payload);
    let token_changed = previous_access_token != account.access_token
        || previous_refresh_token != account.refresh_token;
    logger::log_info(&format!(
        "[Trae Refresh] 已从{}同步会话快照: account_id={}, path={}, token_changed={}",
        source_label,
        account.id,
        storage_path.display(),
        if token_changed { "true" } else { "false" }
    ));
    token_changed
}

/// Collect candidate storage.json paths that may hold a fresher session for this account.
fn collect_storage_paths_for_account_sync(account: &TraeAccount) -> Vec<(String, PathBuf)> {
    let platform = resolve_account_platform_kind(account);
    let mut paths: Vec<(String, PathBuf)> = Vec::new();
    let mut push_unique = |label: String, path: PathBuf| {
        if !path.exists() {
            return;
        }
        if paths
            .iter()
            .any(|(_, existing)| storage_paths_equivalent(existing.as_path(), path.as_path()))
        {
            return;
        }
        paths.push((label, path));
    };

    if let Ok(storage_path) = get_default_trae_storage_path_for_platform(platform) {
        push_unique("本地默认".to_string(), storage_path);
    }

    // Bound multi-open instances may hold a newer rotated refresh token.
    if let Ok(store) = crate::modules::trae_instance::load_instance_store_for_platform(platform) {
        if store
            .default_settings
            .bind_account_id
            .as_deref()
            .map(str::trim)
            == Some(account.id.as_str())
        {
            if let Ok(default_dir) =
                crate::modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)
            {
                push_unique(
                    "默认实例".to_string(),
                    crate::modules::trae_instance::build_storage_json_path(
                        &default_dir.to_string_lossy(),
                    ),
                );
            }
        }
        for instance in store.instances {
            if instance.bind_account_id.as_deref().map(str::trim) != Some(account.id.as_str()) {
                continue;
            }
            let label = if instance.name.trim().is_empty() {
                format!("实例 {}", instance.id)
            } else {
                format!("实例 {}", instance.name.trim())
            };
            push_unique(
                label,
                crate::modules::trae_instance::build_storage_json_path(&instance.user_data_dir),
            );
        }
    }

    paths
}

/// Before ExchangeToken, prefer disk tokens written by the official client.
/// Persist any synced rotation so Cockpit does not keep a stale refresh token
/// even if the subsequent ExchangeToken call fails.
fn prepare_account_tokens_before_exchange(account: &mut TraeAccount) -> bool {
    let mut any_changed = false;
    for (label, storage_path) in collect_storage_paths_for_account_sync(account) {
        if sync_account_tokens_from_storage_path(
            account,
            storage_path.as_path(),
            label.as_str(),
            true,
        ) {
            any_changed = true;
        }
    }

    if any_changed {
        if let Err(err) = save_account_file(account) {
            logger::log_warn(&format!(
                "[Trae Refresh] 同步 storage 后落盘失败: account_id={}, error={}",
                account.id, err
            ));
        } else {
            logger::log_info(&format!(
                "[Trae Refresh] 已落盘 storage 同步后的会话，避免互刷使用过期 RefreshToken: account_id={}",
                account.id
            ));
        }
    }

    any_changed
}

fn format_exchange_token_failure(official_err: &str, legacy_err: &str) -> String {
    let combined = format!(
        "Trae ExchangeToken 失败: official={} | legacy={}",
        official_err, legacy_err
    );
    if combined.contains("会话已过期")
        || combined.contains("未认证")
        || combined.contains("设备密钥缺失")
        || combined.contains("invalid")
        || combined.contains("401")
        || combined.contains("403")
    {
        format!(
            "{}。可能原因：官方客户端已刷新并轮换了 RefreshToken，或设备密钥/区域不一致导致互刷顶号。请在 Trae 重新登录后于 Cockpit「从本地导入」，并避免 Trae 运行中对同一账号做登录刷新。",
            combined
        )
    } else {
        combined
    }
}

/// Whether full ExchangeToken is forbidden because Trae currently owns this account.
pub(crate) fn is_account_protected_from_token_refresh(
    account_id: &str,
    accounts: &[TraeAccount],
) -> Option<Option<PathBuf>> {
    let map = resolve_running_account_refresh_protection_map(accounts);
    map.get(account_id).cloned()
}

async fn refresh_quota_snapshot(
    account: &mut TraeAccount,
    client: &reqwest::Client,
    cookie: Option<&str>,
) {
    let is_cn = resolve_account_platform_kind(account).is_cn();
    // CN 优先 v2（对齐官方 CN 客户端 / 社区 #1281），失败再回退 v1。
    let pay_status_paths: Vec<&str> = if is_cn {
        vec![TRAE_CN_PAY_STATUS_PATH, TRAE_PAY_STATUS_PATH]
    } else {
        vec![TRAE_PAY_STATUS_PATH]
    };
    let ent_usage_paths: Vec<&str> = if is_cn {
        vec![TRAE_CN_ENT_USAGE_PATH, TRAE_ENT_USAGE_PATH]
    } else {
        vec![TRAE_ENT_USAGE_PATH]
    };

    let mut quota_query_errors: Vec<String> = Vec::new();

    let mut entitlement_ok = false;
    for path in pay_status_paths {
        let entitlement_urls = build_refresh_api_urls(account, path);
        match request_trae_pay_json_with_candidates(
            client,
            Method::POST,
            entitlement_urls.as_slice(),
            &account.access_token,
            cookie,
            Some(serde_json::json!({})),
        )
        .await
        {
            Ok(response) => {
                apply_entitlement_response(account, &response);
                entitlement_ok = true;
                break;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae Refresh] ide_user_pay_status 失败 ({}): {}",
                    path, err
                ));
                quota_query_errors.push(format!("{}: {}", path, err));
            }
        }
    }
    if !entitlement_ok {
        // keep errors for later aggregation
    }

    let mut usage_refreshed = false;
    for path in ent_usage_paths {
        let usage_urls = build_refresh_api_urls(account, path);
        match request_trae_pay_json_with_candidates(
            client,
            Method::POST,
            usage_urls.as_slice(),
            &account.access_token,
            cookie,
            Some(serde_json::json!({
                "require_usage": true,
            })),
        )
        .await
        {
            Ok(response) => {
                apply_usage_response(account, &response);
                usage_refreshed = true;
                break;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae Refresh] ide_user_ent_usage 失败 ({}): {}",
                    path, err
                ));
                quota_query_errors.push(format!("{}: {}", path, err));
            }
        }
    }

    // CN 额外拉当前权益列表：部分账号 ide_user_ent_usage 不完整，此接口补 pack / 速通字段。
    if is_cn {
        let list_urls = build_refresh_api_urls(account, TRAE_CN_CURRENT_ENTITLEMENT_LIST_PATH);
        match request_trae_pay_json_with_candidates(
            client,
            Method::POST,
            list_urls.as_slice(),
            &account.access_token,
            cookie,
            Some(serde_json::json!({
                "require_usage": true,
            })),
        )
        .await
        {
            Ok(mut response) => {
                mark_trae_usage_source(&mut response, "user_current_entitlement_list");
                apply_usage_response(account, &response);
                usage_refreshed = true;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae Refresh] user_current_entitlement_list 失败: {}",
                    err
                ));
                // 已有 usage 时仅记录，不把整次刷新判失败
                if !usage_refreshed {
                    quota_query_errors.push(format!(
                        "{}: {}",
                        TRAE_CN_CURRENT_ENTITLEMENT_LIST_PATH, err
                    ));
                }
            }
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
}

async fn refresh_account_async_once(account_id: &str) -> Result<TraeAccount, String> {
    let existing = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;
    logger::log_info(&format!(
        "[Trae Refresh] 开始刷新账号: id={}, email={}",
        existing.id, existing.email
    ));

    // Single-writer rule: if Trae currently owns this account, never ExchangeToken.
    // Doing so rotates refresh tokens and logs the official client out.
    let snapshot = list_accounts_checked().unwrap_or_else(|_| vec![existing.clone()]);
    if let Some(storage_path) =
        is_account_protected_from_token_refresh(account_id, snapshot.as_slice())
    {
        logger::log_warn(&format!(
            "[Trae Refresh] 账号正在官方客户端中使用，跳过 ExchangeToken，改为仅额度刷新: account_id={}, storage_path={}",
            account_id,
            storage_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        return refresh_account_usage_only_async_once(account_id, storage_path.as_deref()).await;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut account = existing.clone();
    // Align with disk before exchange so we use the latest refresh token Trae may have written.
    let _ = prepare_account_tokens_before_exchange(&mut account);

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
        return Err("Trae refresh token 缺失，无法按官方流程刷新登录态。请在 Trae 登录后于 Cockpit「从本地导入」。".to_string());
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
            .map_err(|err| format_exchange_token_failure(official_err.as_str(), err.as_str()))?
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

    refresh_quota_snapshot(&mut account, &client, cookie.as_deref()).await;
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
    if let Some(path) = runtime_storage_path {
        apply_runtime_storage_payload_for_usage_refresh(&mut account, Some(path));
    } else {
        // No explicit runtime path: still pull fresher tokens from local storages.
        let _ = prepare_account_tokens_before_exchange(&mut account);
    }

    let cookie = pick_cookie_from_account(&account);
    let routing_context = build_refresh_routing_context(&account);
    logger::log_info(&format!(
        "[Trae Refresh] 仅额度刷新使用路由: id={}, host={}, login_region={}, store_region={}, ai_region={}",
        account.id,
        routing_context.login_host,
        routing_context.login_region.as_deref().unwrap_or("-"),
        routing_context.store_region.as_deref().unwrap_or("-"),
        routing_context.ai_region.as_deref().unwrap_or("-")
    ));

    refresh_quota_snapshot(&mut account, &client, cookie.as_deref()).await;

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

// ============ 签到功能 API ============

/// 签到状态响应（前端展示用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckinStatusResult {
    pub checked_in: bool,
    pub consecutive_days: i32,
    pub total_credits: i64,
    pub credits_earned_today: i64,
    pub checkin_date: String,
    pub message: String,
}

/// Trae API 签到状态响应
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct TraeCheckinStatusResponse {
    #[serde(default)]
    pub checked_in: bool,
    #[serde(default)]
    pub credits: i64,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub message: String,
}

/// Trae API 签到领取响应
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct TraeCheckinClaimResponse {
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub message: String,
}

/// 获取 Trae 账号的今日签到状态
pub async fn get_trae_checkin_status(
    account_id: &str,
    device_id: &str,
) -> Result<CheckinStatusResult, String> {
    let account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::ACCEPT,
        "application/json, text/plain, */*"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::ORIGIN,
        "https://www.trae.cn"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::REFERER,
        "https://www.trae.cn/"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        "x-app-type",
        "trae"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", account.access_token)
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );

    let mut url = "https://api.trae.cn/trae/api/v2/ug/checkin_credits/status".to_string();
    if !device_id.is_empty() {
        url.push_str(&format!("?did={}", device_id));
        if let Ok(device_header) = reqwest::header::HeaderValue::from_bytes(device_id.as_bytes()) {
            headers.insert("x-device-id", device_header);
        }
    }

    let response = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("签到状态请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("获取签到状态失败 ({}): {}", status, body));
    }

    let data: TraeCheckinStatusResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析签到状态响应失败: {}", e))?;

    if data.code != 0 {
        return Err(format!(
            "获取签到状态失败 (code={}): Token 已过期，请重新登录",
            data.code
        ));
    }

    let message = if data.checked_in {
        format!("今日已签到 · 共 {} 积分", data.credits)
    } else {
        "今日未签到".to_string()
    };

    Ok(CheckinStatusResult {
        checked_in: data.checked_in,
        consecutive_days: 0,
        total_credits: data.credits,
        credits_earned_today: 0,
        checkin_date: String::new(),
        message,
    })
}

/// 领取 Trae 账号的今日签到积分
pub async fn claim_trae_checkin(
    account_id: &str,
    device_id: &str,
) -> Result<CheckinStatusResult, String> {
    let account = load_account(account_id).ok_or_else(|| "账号不存在".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::ACCEPT,
        "application/json, text/plain, */*"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::ORIGIN,
        "https://www.trae.cn"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::REFERER,
        "https://www.trae.cn/"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        "x-app-type",
        "trae"
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", account.access_token)
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|e| e.to_string())?,
    );

    let url = "https://api.trae.cn/trae/api/v2/ug/checkin_credits/claim".to_string();
    if !device_id.is_empty() {
        let device_header = reqwest::header::HeaderValue::from_bytes(device_id.as_bytes())
            .map_err(|e| format!("Device ID 格式错误: {}", e))?;
        headers.insert("x-device-id", device_header);
    }

    let response = client
        .post(&url)
        .headers(headers)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("签到领取请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("签到领取失败 ({}): {}", status, body));
    }

    let claim_data: TraeCheckinClaimResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析签到领取响应失败: {}", e))?;

    if claim_data.code != 0 {
        return Err(format!(
            "签到领取失败 (code={}): Token 已过期，请重新登录",
            claim_data.code
        ));
    }

    // 领取后重新查询状态
    let status_result = get_trae_checkin_status(account_id, device_id).await?;

    Ok(CheckinStatusResult {
        message: format!("签到成功！获得 {} 积分", status_result.total_credits),
        ..status_result
    })
}
