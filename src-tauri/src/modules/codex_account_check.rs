// Codex 账号模块：Official account-check request and response validation。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexAccountCheckErrorKind {
    Unauthorized,
    Forbidden,
    Network,
    InvalidResponse,
}

#[derive(Debug)]
struct CodexAccountCheckError {
    kind: CodexAccountCheckErrorKind,
    message: String,
}

fn account_check_candidate_ids(payload: &serde_json::Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Some(ordering) = payload
        .get("account_ordering")
        .and_then(|value| value.as_array())
    {
        for value in ordering {
            if let Some(id) = value
                .as_str()
                .and_then(|value| normalize_optional_ref(Some(value)))
            {
                ids.insert(id);
            }
        }
    }
    if let Some(accounts) = payload.get("accounts").and_then(|value| value.as_object()) {
        for (key, value) in accounts {
            let key_looks_like_account_id = key.starts_with("org-")
                || key.starts_with("account-")
                || key.starts_with("acct_")
                || (key.len() == 36 && key.chars().filter(|ch| *ch == '-').count() == 4);
            if key_looks_like_account_id {
                if let Some(id) = normalize_optional_ref(Some(key)) {
                    ids.insert(id);
                }
            }
            if let Some(record) = value.as_object() {
                if let Some(id) = extract_account_record_field(
                    record,
                    &["id", "account_id", "chatgpt_account_id", "workspace_id"],
                )
                .and_then(|value| normalize_optional_ref(Some(&value)))
                {
                    ids.insert(id);
                }
            }
        }
    }
    for value in collect_account_records(payload) {
        let Some(record) = value.as_object() else {
            continue;
        };
        if let Some(id) = extract_account_record_field(
            record,
            &["id", "account_id", "chatgpt_account_id", "workspace_id"],
        )
        .and_then(|value| normalize_optional_ref(Some(&value)))
        {
            ids.insert(id);
        }
    }
    ids
}

fn validate_account_check_payload(
    payload: &serde_json::Value,
    account: &CodexAccount,
) -> Result<(), CodexAccountCheckError> {
    let records = collect_account_records(payload);
    let candidate_ids = account_check_candidate_ids(payload);
    if records.is_empty() && candidate_ids.is_empty() {
        return Err(CodexAccountCheckError {
            kind: CodexAccountCheckErrorKind::InvalidResponse,
            message: "官方账号检查接口未返回可用账号信息".to_string(),
        });
    }

    let expected_account_id =
        extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
            .or_else(|| normalize_optional_ref(account.account_id.as_deref()));
    if let Some(expected_account_id) = expected_account_id {
        if !candidate_ids.is_empty() && !candidate_ids.contains(&expected_account_id) {
            return Err(CodexAccountCheckError {
                kind: CodexAccountCheckErrorKind::Unauthorized,
                message: format!(
                    "官方账号检查结果与目标账号不一致: expected_account_id={}, returned_account_count={}",
                    expected_account_id,
                    candidate_ids.len()
                ),
            });
        }
        if let Some(record) = payload
            .get("accounts")
            .and_then(serde_json::Value::as_object)
            .and_then(|accounts| accounts.get(&expected_account_id))
            .and_then(serde_json::Value::as_object)
        {
            if record
                .get("can_access_with_session")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
            {
                return Err(CodexAccountCheckError {
                    kind: CodexAccountCheckErrorKind::Forbidden,
                    message: format!(
                        "官方账号检查结果不允许当前登录态访问目标账号: account_id={}",
                        expected_account_id
                    ),
                });
            }
            if let Some(returned_account_id) = record
                .get("account")
                .and_then(serde_json::Value::as_object)
                .and_then(|account| account.get("account_id"))
                .and_then(serde_json::Value::as_str)
                .and_then(|value| normalize_optional_ref(Some(value)))
            {
                if returned_account_id != expected_account_id {
                    return Err(CodexAccountCheckError {
                        kind: CodexAccountCheckErrorKind::Unauthorized,
                        message: format!(
                            "官方账号检查结果与目标账号不一致: expected_account_id={}, returned_account_id={}",
                            expected_account_id, returned_account_id
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

async fn request_remote_account_check(
    account: &CodexAccount,
) -> Result<serde_json::Value, CodexAccountCheckError> {
    let access_token = account.tokens.access_token.trim();
    if access_token.is_empty() {
        return Err(CodexAccountCheckError {
            kind: CodexAccountCheckErrorKind::Unauthorized,
            message: "access_token 为空，无法执行官方账号检查".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| CodexAccountCheckError {
            kind: CodexAccountCheckErrorKind::Network,
            message: format!("创建官方账号检查客户端失败: {}", error),
        })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", access_token)).map_err(|error| {
            CodexAccountCheckError {
                kind: CodexAccountCheckErrorKind::InvalidResponse,
                message: format!("构建 Authorization 头失败: {}", error),
            }
        })?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    if let Some(account_id) = normalize_optional_ref(account.account_id.as_deref())
        .or_else(|| extract_chatgpt_account_id_from_access_token(access_token))
    {
        headers.insert(
            "ChatGPT-Account-Id",
            HeaderValue::from_str(&account_id).map_err(|error| CodexAccountCheckError {
                kind: CodexAccountCheckErrorKind::InvalidResponse,
                message: format!("构建 ChatGPT-Account-Id 头失败: {}", error),
            })?,
        );
    }

    let response = client
        .get(ACCOUNT_CHECK_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|error| CodexAccountCheckError {
            kind: CodexAccountCheckErrorKind::Network,
            message: format!("官方账号检查请求失败: {}", error),
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| CodexAccountCheckError {
            kind: CodexAccountCheckErrorKind::Network,
            message: format!("读取官方账号检查响应失败: {}", error),
        })?;

    if !status.is_success() {
        let kind = match status.as_u16() {
            401 => CodexAccountCheckErrorKind::Unauthorized,
            403 => CodexAccountCheckErrorKind::Forbidden,
            _ => CodexAccountCheckErrorKind::InvalidResponse,
        };
        return Err(CodexAccountCheckError {
            kind,
            message: format!(
                "官方账号检查接口返回错误: status={}, body_len={}",
                status,
                body.len()
            ),
        });
    }

    serde_json::from_str(&body).map_err(|error| CodexAccountCheckError {
        kind: CodexAccountCheckErrorKind::InvalidResponse,
        message: format!("官方账号检查响应 JSON 解析失败: {}", error),
    })
}

async fn fetch_remote_account_profile(
    account: &CodexAccount,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持刷新远端资料".to_string());
    }

    let payload = request_remote_account_check(account)
        .await
        .map_err(|error| error.message)?;
    Ok(parse_account_profile_from_check_response(&payload, account))
}

