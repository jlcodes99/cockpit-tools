// cockpit-core Trae 账号：Runtime injection payloads and current-account resolution。
// 通过 include! 保持原模块作用域和平台调用路径。
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

pub(crate) fn resolve_current_account_id_for_platform(
    accounts: &[TraeAccount],
    platform: TraePlatformKind,
) -> Option<String> {
    let payload = read_local_trae_auth_for_platform(platform).ok()??;
    let normalized_user_id = normalize_non_empty(payload.user_id.as_deref());
    let normalized_email = normalize_email(Some(payload.email.as_str()));

    accounts
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
