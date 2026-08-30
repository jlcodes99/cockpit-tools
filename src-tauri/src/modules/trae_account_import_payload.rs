// Trae 账号模块：Import/export payload parsing and account mutation APIs。
// 通过 include! 保持原 modules::trae_account 作用域和平台行为。
fn storage_object_value(root: &Value, key: &str) -> Option<Value> {
    root.as_object()
        .and_then(|obj| parse_value_or_json_string(obj.get(key)))
}

fn storage_object_value_auth(root: &Value, key: &str) -> Option<Value> {
    root.as_object()
        .and_then(|obj| parse_value_or_json_string_or_icube_cipher(obj.get(key)))
}

fn payload_from_storage_root(storage_root: &Value) -> Result<TraeImportPayload, String> {
    let root_obj = storage_root.as_object();
    let provider_id = root_obj
        .map(resolve_storage_provider_id)
        .unwrap_or_else(|| TRAE_DEFAULT_AUTH_PROVIDER_ID.to_string());
    let auth_storage_key = build_auth_storage_key(provider_id.as_str());
    let server_storage_key = build_server_storage_key(provider_id.as_str());
    let entitlement_storage_key = build_entitlement_storage_key(provider_id.as_str());

    let auth_raw = storage_object_value_auth(storage_root, auth_storage_key.as_str())
        .or_else(|| storage_object_value_auth(storage_root, TRAE_STORAGE_AUTH_KEY));
    let entitlement_raw = storage_object_value(storage_root, entitlement_storage_key.as_str())
        .or_else(|| storage_object_value(storage_root, TRAE_STORAGE_ENTITLEMENT_KEY));
    let server_raw = storage_object_value(storage_root, server_storage_key.as_str())
        .or_else(|| storage_object_value(storage_root, TRAE_STORAGE_SERVER_KEY));

    let access_token = pick_string(
        auth_raw.as_ref(),
        &[
            &["accessToken"],
            &["access_token"],
            &["token"],
            &["data", "accessToken"],
            &["data", "access_token"],
            &["auth", "accessToken"],
            &["auth", "token"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["accessToken"],
                &["access_token"],
                &["token"],
                &["data", "accessToken"],
                &["data", "token"],
            ],
        )
    })
    .ok_or_else(|| "Trae 本地存储缺少 access token".to_string())?;

    let refresh_token = pick_string(
        auth_raw.as_ref(),
        &[
            &["refreshToken"],
            &["refresh_token"],
            &["RefreshToken"],
            &["exchangeResponse", "Result", "RefreshToken"],
            &["data", "refreshToken"],
            &["data", "refresh_token"],
        ],
    );

    let email = normalize_email(
        pick_string(
            auth_raw.as_ref(),
            &[
                &["email"],
                &["account", "email"],
                &["account", "nonPlainTextEmail"],
                &["NonPlainTextEmail"],
                &["data", "email"],
                &["user", "email"],
                &["userInfo", "email"],
            ],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_email(
            pick_string(
                server_raw.as_ref(),
                &[&["email"], &["data", "email"], &["user", "email"]],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "unknown".to_string());

    let user_id = pick_string(
        auth_raw.as_ref(),
        &[
            &["userId"],
            &["user_id"],
            &["uid"],
            &["id"],
            &["data", "userId"],
            &["data", "uid"],
            &["user", "id"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["userId"],
                &["user_id"],
                &["uid"],
                &["id"],
                &["account", "uid"],
                &["data", "userId"],
                &["data", "uid"],
                &["user", "id"],
            ],
        )
    });

    let nickname = pick_string(
        auth_raw.as_ref(),
        &[
            &["nickname"],
            &["name"],
            &["displayName"],
            &["account", "username"],
            &["data", "nickname"],
            &["user", "nickname"],
            &["user", "name"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["nickname"],
                &["name"],
                &["displayName"],
                &["data", "nickname"],
                &["user", "name"],
            ],
        )
    });

    let token_type = pick_string(
        auth_raw.as_ref(),
        &[
            &["tokenType"],
            &["token_type"],
            &["TokenType"],
            &["data", "tokenType"],
        ],
    );
    let expires_at = normalize_timestamp(
        pick_i64(
            auth_raw.as_ref(),
            &[
                &["expiresAt"],
                &["expiredAt"],
                &["expires_at"],
                &["TokenExpireAt"],
                &["exchangeResponse", "Result", "TokenExpireAt"],
                &["data", "expiresAt"],
            ],
        )
        .or_else(|| {
            pick_i64(
                server_raw.as_ref(),
                &[&["expiresAt"], &["expires_at"], &["data", "expiresAt"]],
            )
        }),
    );

    let plan_type = pick_string(
        entitlement_raw.as_ref(),
        &[
            &["identityStr"],
            &["identity_str"],
            &["user_pay_identity_str"],
            &["entitlementInfo", "identityStr"],
            &["data", "user_pay_identity_str"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[
                &["entitlementInfo", "identityStr"],
                &["identityStr"],
                &["data", "entitlementInfo", "identityStr"],
            ],
        )
    });
    let plan_reset_at = normalize_timestamp(
        pick_i64(
            entitlement_raw.as_ref(),
            &[
                &["detail", "subscription_renew_time"],
                &["detail", "subscriptionRenewTime"],
                &["data", "detail", "subscription_renew_time"],
                &["entitlementInfo", "detail", "subscription_renew_time"],
                &["entitlementInfo", "detail", "subscriptionRenewTime"],
            ],
        )
        .or_else(|| {
            pick_i64(
                server_raw.as_ref(),
                &[
                    &["entitlementInfo", "detail", "subscription_renew_time"],
                    &["entitlementInfo", "detail", "subscriptionRenewTime"],
                    &[
                        "data",
                        "entitlementInfo",
                        "detail",
                        "subscription_renew_time",
                    ],
                ],
            )
        }),
    );

    let status = pick_string(
        auth_raw.as_ref(),
        &[&["status"], &["data", "status"], &["loginStatus"]],
    )
    .or_else(|| pick_string(server_raw.as_ref(), &[&["status"], &["data", "status"]]));
    let status_reason = pick_string(
        auth_raw.as_ref(),
        &[
            &["statusReason"],
            &["status_reason"],
            &["message"],
            &["data", "message"],
        ],
    )
    .or_else(|| {
        pick_string(
            server_raw.as_ref(),
            &[&["statusReason"], &["status_reason"], &["message"]],
        )
    });

    let usertag_raw = resolve_usertag_from_storage(
        root_obj,
        user_id.as_deref(),
        auth_raw.as_ref(),
        server_raw.as_ref(),
    );

    Ok(TraeImportPayload {
        email,
        user_id,
        nickname,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        plan_type,
        plan_reset_at,
        trae_auth_raw: auth_raw,
        trae_profile_raw: None,
        trae_entitlement_raw: entitlement_raw,
        trae_usage_raw: None,
        trae_server_raw: server_raw,
        trae_usertag_raw: usertag_raw,
        status,
        status_reason,
    })
}

fn payload_from_import_value(raw: Value) -> Result<TraeImportPayload, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "Trae 导入项必须是对象".to_string())?;

    if obj.contains_key(TRAE_STORAGE_AUTH_KEY) || has_trae_auth_storage_key(obj) {
        return payload_from_storage_root(&raw);
    }

    let auth_raw = obj
        .get("trae_auth_raw")
        .cloned()
        .or_else(|| obj.get("auth_raw").cloned())
        .or_else(|| obj.get("auth").cloned());
    let entitlement_raw = obj
        .get("trae_entitlement_raw")
        .cloned()
        .or_else(|| obj.get("entitlement_raw").cloned())
        .or_else(|| obj.get("usage_raw").cloned())
        .or_else(|| obj.get("quota_raw").cloned());
    let usage_raw = obj
        .get("trae_usage_raw")
        .cloned()
        .or_else(|| obj.get("usage_status_raw").cloned())
        .or_else(|| obj.get("ent_usage_raw").cloned());
    let profile_raw = obj
        .get("trae_profile_raw")
        .cloned()
        .or_else(|| obj.get("profile_raw").cloned())
        .or_else(|| obj.get("profile").cloned());
    let server_raw = obj
        .get("trae_server_raw")
        .cloned()
        .or_else(|| obj.get("server_raw").cloned())
        .or_else(|| obj.get("server").cloned());
    let usertag_raw = obj
        .get("trae_usertag_raw")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_non_empty(Some(value)));

    let access_token = pick_string(
        Some(&raw),
        &[
            &["access_token"],
            &["accessToken"],
            &["token"],
            &["trae_access_token"],
        ],
    )
    .or_else(|| {
        pick_string(
            auth_raw.as_ref(),
            &[&["accessToken"], &["access_token"], &["token"]],
        )
    })
    .ok_or_else(|| "缺少 access_token 字段".to_string())?;

    let refresh_token = pick_string(
        Some(&raw),
        &[
            &["refresh_token"],
            &["refreshToken"],
            &["trae_refresh_token"],
        ],
    )
    .or_else(|| pick_string(auth_raw.as_ref(), &[&["refreshToken"], &["refresh_token"]]));

    let email = normalize_email(
        pick_string(
            Some(&raw),
            &[&["email"], &["trae_email"], &["user", "email"]],
        )
        .as_deref(),
    )
    .or_else(|| {
        normalize_email(
            pick_string(
                auth_raw.as_ref(),
                &[
                    &["email"],
                    &["account", "email"],
                    &["account", "nonPlainTextEmail"],
                    &["NonPlainTextEmail"],
                    &["user", "email"],
                ],
            )
            .as_deref(),
        )
    })
    .unwrap_or_else(|| "unknown".to_string());

    let user_id =
        pick_string(Some(&raw), &[&["user_id"], &["userId"], &["uid"], &["id"]]).or_else(|| {
            pick_string(
                auth_raw.as_ref(),
                &[&["userId"], &["uid"], &["id"], &["account", "uid"]],
            )
        });
    let nickname = pick_string(
        Some(&raw),
        &[
            &["nickname"],
            &["name"],
            &["displayName"],
            &["user", "name"],
        ],
    )
    .or_else(|| {
        pick_string(
            profile_raw.as_ref(),
            &[
                &["nickname"],
                &["name"],
                &["displayName"],
                &["Result", "ScreenName"],
                &["Result", "Nickname"],
            ],
        )
    })
    .or_else(|| pick_string(auth_raw.as_ref(), &[&["account", "username"]]));
    let token_type = pick_string(Some(&raw), &[&["token_type"], &["tokenType"]]).or_else(|| {
        pick_string(
            auth_raw.as_ref(),
            &[&["tokenType"], &["token_type"], &["TokenType"]],
        )
    });
    let expires_at = normalize_timestamp(
        pick_i64(
            Some(&raw),
            &[&["expires_at"], &["expiresAt"], &["expiredAt"]],
        )
        .or_else(|| {
            pick_i64(
                auth_raw.as_ref(),
                &[
                    &["expiresAt"],
                    &["expiredAt"],
                    &["TokenExpireAt"],
                    &["exchangeResponse", "Result", "TokenExpireAt"],
                    &["expires_at"],
                ],
            )
        }),
    );
    let plan_type = pick_string(
        Some(&raw),
        &[
            &["identityStr"],
            &["identity_str"],
            &["user_pay_identity_str"],
        ],
    )
    .or_else(|| {
        pick_string(
            entitlement_raw.as_ref(),
            &[
                &["identityStr"],
                &["identity_str"],
                &["user_pay_identity_str"],
                &["entitlementInfo", "identityStr"],
                &["data", "user_pay_identity_str"],
            ],
        )
    })
    .or_else(|| {
        pick_string(
            usage_raw.as_ref(),
            &[
                &["identityStr"],
                &["identity_str"],
                &["data", "identityStr"],
                &["entitlementInfo", "identityStr"],
            ],
        )
    });
    let plan_reset_at = normalize_timestamp(
        pick_i64(
            Some(&raw),
            &[&["plan_reset_at"], &["detail", "subscription_renew_time"]],
        )
        .or_else(|| {
            pick_i64(
                entitlement_raw.as_ref(),
                &[
                    &["detail", "subscription_renew_time"],
                    &["entitlementInfo", "detail", "subscription_renew_time"],
                ],
            )
        })
        .or_else(|| {
            pick_i64(
                usage_raw.as_ref(),
                &[
                    &["currentPlan", "timeInfo", "nextResetTime"],
                    &["data", "currentPlan", "timeInfo", "nextResetTime"],
                    &["nextResetTime"],
                ],
            )
        }),
    );

    let status = pick_string(Some(&raw), &[&["status"]])
        .or_else(|| pick_string(auth_raw.as_ref(), &[&["status"], &["loginStatus"]]));
    let status_reason = pick_string(Some(&raw), &[&["status_reason"], &["statusReason"]])
        .or_else(|| pick_string(auth_raw.as_ref(), &[&["statusReason"], &["message"]]));

    Ok(TraeImportPayload {
        email,
        user_id,
        nickname,
        access_token,
        refresh_token,
        token_type,
        expires_at,
        plan_type,
        plan_reset_at,
        trae_auth_raw: auth_raw,
        trae_profile_raw: profile_raw,
        trae_entitlement_raw: entitlement_raw,
        trae_usage_raw: usage_raw,
        trae_server_raw: server_raw,
        trae_usertag_raw: usertag_raw,
        status,
        status_reason,
    })
}

fn payloads_from_import_json_value(raw: Value) -> Result<Vec<TraeImportPayload>, String> {
    match raw {
        Value::Array(items) => {
            if items.is_empty() {
                return Err("导入数组为空".to_string());
            }
            let mut payloads = Vec::with_capacity(items.len());
            for (idx, item) in items.into_iter().enumerate() {
                let payload = payload_from_import_value(item)
                    .map_err(|e| format!("第 {} 条 Trae 账号解析失败: {}", idx + 1, e))?;
                payloads.push(payload);
            }
            Ok(payloads)
        }
        Value::Object(obj) => {
            if let Some(accounts_raw) = obj.get("accounts") {
                if let Some(accounts) = accounts_raw.as_array() {
                    if accounts.is_empty() {
                        return Err("导入数组为空".to_string());
                    }
                    let mut payloads = Vec::with_capacity(accounts.len());
                    for (idx, item) in accounts.iter().enumerate() {
                        let payload = payload_from_import_value(item.clone())
                            .map_err(|e| format!("第 {} 条 Trae 账号解析失败: {}", idx + 1, e))?;
                        payloads.push(payload);
                    }
                    return Ok(payloads);
                }
            }
            Ok(vec![payload_from_import_value(Value::Object(obj))?])
        }
        _ => Err("Trae 导入 JSON 必须是对象或数组".to_string()),
    }
}

pub fn import_from_json(json_content: &str) -> Result<Vec<TraeAccount>, String> {
    if let Ok(account) = serde_json::from_str::<TraeAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<TraeAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    let value = serde_json::from_str::<Value>(json_content)
        .map_err(|e| format!("解析 JSON 失败: {}", e))?;
    let payloads = payloads_from_import_json_value(value)?;
    let mut result = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let saved = upsert_account(payload)?;
        result.push(saved);
    }
    Ok(result)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<TraeAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

