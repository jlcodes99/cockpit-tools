// Codex Local Access：Recovery and restart support including rejected-field retry tests。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
#[cfg(test)]
mod openai_responses_rejected_field_retry_tests {
    use super::*;

    #[test]
    fn retries_only_explicit_max_output_tokens_rejection() {
        let body = br#"{"max_output_tokens":128,"input":[]}"#;
        let explicit = br#"{"error":{"code":"unknown_parameter","param":"max_output_tokens","message":"Unknown parameter"}}"#;
        let (retry, reason) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            explicit,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reason, "max_output_tokens parameter rejection");
        assert!(serde_json::from_slice::<Value>(&retry)
            .unwrap()
            .get("max_output_tokens")
            .is_none());

        let ambiguous = br#"{"error":{"message":"invalid max_output_tokens"}}"#;
        assert!(normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            ambiguous,
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn rejects_ambiguous_rejected_field_errors() {
        let cases: &[(&[u8], &[u8])] = &[
            (
                br#"{"input":[{"type":"message","namespace":"keep"}]}"#,
                br#"{"error":{"code":"unknown_parameter","message":"Unknown parameter: 'input[0].namespace'.","param":"input[0].namespace"}}"#,
            ),
            (
                br#"{"max_output_tokens":4096}"#,
                br#"{"error":{"code":"invalid_request_error","message":"max_output_tokens must be positive","param":"max_output_tokens"}}"#,
            ),
            (
                br#"{"input":[{"type":"function_call","namespace":"keep","arguments":"{}"}]}"#,
                br#"{"error":{"code":"unknown_parameter","message":"Unknown parameter: 'input[0].namespace'.","param":"tools"}}"#,
            ),
            (
                br#"{"max_output_tokens":4096,"input":[{"type":"message","content":{"max_output_tokens":"keep"}}]}"#,
                br#"{"error":{"code":"unknown_parameter","message":"Unknown parameter: input[0].content.max_output_tokens","param":"input[0].content.max_output_tokens"}}"#,
            ),
        ];

        for (body, response_body) in cases {
            assert!(normalize_openai_responses_rejected_field_retry_body(
                StatusCode::BAD_REQUEST,
                body,
                response_body,
            )
            .unwrap()
            .is_none());
        }
    }

    #[test]
    fn finds_rejected_namespace_path_in_message() {
        let body = br#"{"input":[{"type":"function_call","namespace":"keep","arguments":"{}"},{"type":"function_call","namespace":"remove","arguments":"{}"}]}"#;
        let response = br#"{"error":{"code":"unknown_parameter","message":"input[0] was accepted; Unknown parameter: 'input[1].namespace'."}}"#;
        let (retry, _) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            response,
        )
        .unwrap()
        .unwrap();
        let retry: Value = serde_json::from_slice(&retry).unwrap();
        assert_eq!(
            retry.pointer("/input/0/namespace").and_then(Value::as_str),
            Some("keep")
        );
        assert!(retry.pointer("/input/1/namespace").is_none());
    }

    #[test]
    fn binds_rejected_namespace_path_to_rejection_phrase() {
        let body = br#"{"input":[{"type":"function_call","namespace":"keep","arguments":"{}"},{"type":"function_call","namespace":"remove","arguments":"{}"}]}"#;
        let response = br#"{"error":{"code":"unknown_parameter","message":"input[0].namespace is supported; Unknown parameter: input[1].namespace."}}"#;
        let (retry, _) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            response,
        )
        .unwrap()
        .unwrap();
        let retry: Value = serde_json::from_slice(&retry).unwrap();
        assert_eq!(
            retry.pointer("/input/0/namespace").and_then(Value::as_str),
            Some("keep")
        );
        assert!(retry.pointer("/input/1/namespace").is_none());
    }

    #[test]
    fn does_not_treat_max_output_tokens_suggestion_as_rejection() {
        let body = br#"{"max_tokens":4096,"max_output_tokens":2048}"#;
        let response = br#"{"error":{"code":"unknown_parameter","message":"Unknown parameter: max_tokens. Use max_output_tokens instead."}}"#;
        assert!(normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            response,
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn composes_distinct_rejected_field_retries() {
        let initial = br#"{"max_output_tokens":2048,"input":[{"type":"function_call","namespace":"keep","arguments":"{}"},{"type":"custom_tool_call","namespace":"remove","input":"{}"}]}"#;
        let namespace_response = br#"{"error":{"code":"unknown_parameter","message":"Unknown parameter: 'input[1].namespace'.","param":"input[1].namespace"}}"#;
        let max_tokens_response = br#"{"error":{"code":"unsupported_parameter","message":"Unsupported parameter: max_output_tokens","param":"max_output_tokens"}}"#;
        let mut state = OpenAIResponsesRejectedFieldRetryState::new(initial);

        let (without_namespace, _) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            initial,
            namespace_response,
        )
        .unwrap()
        .unwrap();
        assert!(state.allow(&without_namespace));
        let first_retry: Value = serde_json::from_slice(&without_namespace).unwrap();
        assert!(first_retry.pointer("/input/1/namespace").is_none());
        assert_eq!(
            first_retry.get("max_output_tokens").and_then(Value::as_u64),
            Some(2048)
        );

        let (without_both, _) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            &without_namespace,
            max_tokens_response,
        )
        .unwrap()
        .unwrap();
        assert!(state.allow(&without_both));
        let second_retry: Value = serde_json::from_slice(&without_both).unwrap();
        assert!(second_retry.pointer("/input/1/namespace").is_none());
        assert!(second_retry.get("max_output_tokens").is_none());
    }

    #[test]
    fn removes_only_rejected_tool_call_namespace() {
        let body = br#"{"input":[{"type":"function_call","namespace":"collaboration"},{"type":"message","namespace":"keep"}]}"#;
        let response = br#"{"error":{"code":"unsupported_parameter","param":"input[0].namespace","message":"Unsupported parameter"}}"#;
        let (retry, _) = normalize_openai_responses_rejected_field_retry_body(
            StatusCode::BAD_REQUEST,
            body,
            response,
        )
        .unwrap()
        .unwrap();
        let retry: Value = serde_json::from_slice(&retry).unwrap();
        assert!(retry.pointer("/input/0/namespace").is_none());
        assert_eq!(
            retry.pointer("/input/1/namespace").and_then(Value::as_str),
            Some("keep")
        );
    }

    #[test]
    fn retry_state_rejects_duplicate_and_seventh_mutation() {
        let initial = br#"{"input":[]}"#;
        let mut state = OpenAIResponsesRejectedFieldRetryState::new(initial);
        assert!(!state.allow(initial));
        for attempt in 0..MAX_OPENAI_RESPONSES_REJECTED_FIELD_RETRIES {
            assert!(state.allow(format!(r#"{{"attempt":{attempt}}}"#).as_bytes()));
        }
        assert!(!state.allow(br#"{"attempt":99}"#));
    }
}

async fn proxy_request_with_account_pool(
    request: &ParsedRequest,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<ProxyDispatchSuccess, ProxyDispatchError> {
    let unfiltered_scoped_account_ids = scoped_collection_account_ids(collection, api_key);
    let strategy = effective_routing_strategy(collection, &unfiltered_scoped_account_ids);
    let scoped_account_ids =
        apply_bound_oauth_quota_reserve(collection, unfiltered_scoped_account_ids.clone());
    if scoped_account_ids.is_empty() {
        let quota_reserved = !unfiltered_scoped_account_ids.is_empty();
        return Err(ProxyDispatchError {
            status: 503,
            message: if quota_reserved {
                "绑定 OAuth 账号已达到保留额度，当前没有可路由账号".to_string()
            } else {
                "本地接入集合暂无账号".to_string()
            },
            account_id: None,
            account_email: None,
            error_category: Some(
                if quota_reserved {
                    "quota_reserved"
                } else {
                    "no_accounts"
                }
                .to_string(),
            ),
        });
    }

    let upstream_target =
        resolve_upstream_target(&request.target).map_err(|err| ProxyDispatchError {
            status: 400,
            message: err,
            account_id: None,
            account_email: None,
            error_category: Some("bad_request".to_string()),
        })?;
    let timeouts = collection_timeouts(collection);
    let upstream_connect_timeout = duration_from_millis(
        timeouts.legacy_upstream_connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    let image_generation_mode =
        request_image_generation_mode(collection.image_generation_mode, &request.headers);
    let routing_hint = build_request_routing_hint(request);
    let total = scoped_account_ids.len();
    let max_credential_attempts = max_credential_attempts_for_strategy(collection, total, strategy);
    let session_affinity_key = routing_hint
        .session_affinity_key
        .as_deref()
        .filter(|_| collection.session_affinity)
        .map(session_affinity_binding_key);
    let affinity_account_id = if let Some(session_key) = session_affinity_key.as_deref() {
        resolve_affinity_account(session_key).await
    } else {
        match routing_hint.previous_response_id.as_deref() {
            Some(previous_response_id) => resolve_affinity_account(previous_response_id).await,
            None => None,
        }
    };
    let priority_account_ids = affinity_account_id
        .as_ref()
        .map(|account_id| vec![account_id.clone()])
        .unwrap_or_else(|| api_key_priority_account_ids(collection, api_key));
    let mut last_status = 503u16;
    let mut last_error = "本地接入集合暂无可用账号".to_string();
    let mut last_error_category: Option<String> = None;
    let mut last_account_id: Option<String> = None;
    let mut last_account_email: Option<String> = None;
    let mut attempts = 0usize;
    let mut retry_round = 0usize;
    let mut earliest_cooldown_wait: Option<Duration>;

    loop {
        let start = GATEWAY_ROUND_ROBIN_CURSOR.fetch_add(1, Ordering::Relaxed);
        let ordered_account_ids = request_ordered_account_ids(
            collection,
            &scoped_account_ids,
            strategy,
            start,
            &priority_account_ids,
        );
        let strategy_account_ids = pin_account_to_front_for_strategy(
            apply_routing_strategy(
                &ordered_account_ids,
                strategy,
                &collection.custom_routing_rules,
                start,
            ),
            &priority_account_ids,
            strategy,
            &collection.custom_routing_rules,
        );
        let mut attempted_in_round = false;
        let mut round_cooldown_wait: Option<Duration> = None;

        for account_id in strategy_account_ids {
            if attempts >= max_credential_attempts {
                break;
            }

            if account_model_rule_blocks_model(collection, &account_id, &routing_hint.model_key) {
                last_error = if routing_hint.model_key.trim().is_empty() {
                    "账号模型规则已跳过该账号".to_string()
                } else {
                    format!(
                        "模型 {} 在部分账号上已被禁用，已跳过这些账号",
                        routing_hint.model_key
                    )
                };
                last_error_category = Some("account_model_disabled".to_string());
                continue;
            }

            if account_id_blocked_by_health(&account_id).await {
                last_error = "账号连续鉴权或预处理失败，已暂时跳过".to_string();
                last_error_category = Some("account_unhealthy".to_string());
                continue;
            }

            if !collection.disable_cooling {
                if let Some(wait) =
                    get_model_cooldown_wait(&account_id, &routing_hint.model_key).await
                {
                    round_cooldown_wait = Some(match round_cooldown_wait {
                        Some(current) if current <= wait => current,
                        _ => wait,
                    });
                    continue;
                }
            }

            attempted_in_round = true;
            attempts += 1;

            let mut account = match get_prepared_account(&account_id).await {
                Ok(account) => account,
                Err(err) => {
                    invalidate_prepared_account(&account_id).await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        None,
                        Some(account_id.as_str()),
                        None,
                        None,
                        format!("账号预处理失败: {}", err).as_str(),
                    );
                    last_error = err;
                    last_error_category = Some("account_prepare_failed".to_string());
                    continue;
                }
            };

            if collection.restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref())
            {
                mark_account_failure(
                    &account,
                    None,
                    Some("free_account_restricted"),
                    "Free 账号不支持加入本地接入",
                    request_kind,
                )
                .await;
                log_codex_api_failure(
                    None,
                    Some(request),
                    None,
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    "Free 账号不支持加入本地接入",
                );
                last_error = "Free 账号不支持加入本地接入".to_string();
                last_error_category = Some("free_account_restricted".to_string());
                continue;
            }

            last_account_id = Some(account.id.clone());
            last_account_email = Some(account.email.clone());
            legacy_debug_log(
                collection.debug_logs,
                format!(
                    "account_selected method={} target={} request_kind={} account_id={} account_email={} attempt={}/{}",
                    request.method,
                    request.target,
                    request_kind_log_label(request_kind),
                    account.id,
                    account.email,
                    attempts,
                    max_credential_attempts
                ),
            );

            let mut single_account_status_retry_attempt = 0usize;
            let mut upstream_request_body = request.body.clone();
            let mut rejected_field_retry_state = is_responses_request(&request.target)
                .then(|| OpenAIResponsesRejectedFieldRetryState::new(&upstream_request_body));
            loop {
                let upstream_send_started_at = Instant::now();
                legacy_debug_log(
                    collection.debug_logs,
                    format!(
                        "upstream_send_started method={} target={} request_kind={} account_id={} account_email={} retry_attempt={}",
                        request.method,
                        request.target,
                        request_kind_log_label(request_kind),
                        account.id,
                        account.email,
                        single_account_status_retry_attempt
                    ),
                );
                let first_response = send_upstream_request(
                    &request.method,
                    &upstream_target,
                    &request.headers,
                    &upstream_request_body,
                    &account,
                    collection.upstream_proxy_url.as_deref(),
                    upstream_connect_timeout,
                    &timeouts,
                    image_generation_mode,
                    request_kind,
                )
                .await;

                let mut response = match first_response {
                    Ok(response) => {
                        legacy_debug_log(
                            collection.debug_logs,
                            format!(
                                "upstream_response_headers method={} target={} status={} account_id={} account_email={} upstream_latency_ms={}",
                                request.method,
                                request.target,
                                response.status().as_u16(),
                                account.id,
                                account.email,
                                upstream_send_started_at.elapsed().as_millis()
                            ),
                        );
                        response
                    }
                    Err(err) => {
                        legacy_debug_log(
                            collection.debug_logs,
                            format!(
                                "upstream_send_failed method={} target={} account_id={} account_email={} upstream_latency_ms={} detail={}",
                                request.method,
                                request.target,
                                account.id,
                                account.email,
                                upstream_send_started_at.elapsed().as_millis(),
                                escape_failure_detail(&err)
                            ),
                        );
                        last_status = StatusCode::BAD_GATEWAY.as_u16();
                        mark_account_failure(
                            &account,
                            Some(last_status),
                            Some("upstream_network"),
                            &err,
                            request_kind,
                        )
                        .await;
                        log_codex_api_failure(
                            None,
                            Some(request),
                            Some(last_status),
                            Some(account.id.as_str()),
                            Some(account.email.as_str()),
                            None,
                            format!("上游请求失败: {}", err).as_str(),
                        );
                        last_error = err;
                        last_error_category = Some("upstream_network".to_string());
                        break;
                    }
                };

                if response.status() == StatusCode::UNAUTHORIZED && account.is_api_key_auth() {
                    last_status = StatusCode::UNAUTHORIZED.as_u16();
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(last_status),
                        Some("auth_unavailable"),
                        "API Key 账号上游鉴权失败",
                        request_kind,
                    )
                    .await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        Some(last_status),
                        Some(account.id.as_str()),
                        Some(account.email.as_str()),
                        None,
                        format!("API Key 账号 {} 上游鉴权失败", account.email).as_str(),
                    );
                    last_error = format!("API Key 账号 {} 上游鉴权失败", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    break;
                }

                if response.status() == StatusCode::UNAUTHORIZED
                    && !account_has_refresh_token(&account)
                {
                    last_status = StatusCode::UNAUTHORIZED.as_u16();
                    invalidate_prepared_account(&account_id).await;
                    if let Err(err) =
                        codex_account::mark_access_token_only_account_requires_reauth(&account.id)
                    {
                        logger::log_codex_api_warn(&format!(
                            "[CodexLocalAccess] 标记 access-token-only 账号需重新登录失败: account_id={}, error={}",
                            account.id, err
                        ));
                    }
                    mark_account_failure(
                        &account,
                        Some(last_status),
                        Some("auth_unavailable"),
                        "access-token-only 账号的 access_token 已被上游拒绝",
                        request_kind,
                    )
                    .await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        Some(last_status),
                        Some(account.id.as_str()),
                        Some(account.email.as_str()),
                        None,
                        format!(
                            "上游返回 401，access-token-only 账号的 access_token 已不可用，按普通账号路径轮转: {}",
                            account.email
                        )
                        .as_str(),
                    );
                    last_error = format!("账号 {} 当前 access_token 已被上游拒绝", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    break;
                }

                if response.status() == StatusCode::UNAUTHORIZED {
                    match force_refresh_gateway_account(&account_id, account.token_generation).await
                    {
                        Ok(refreshed_account) => {
                            account = refreshed_account;
                            response = match send_upstream_request(
                                &request.method,
                                &upstream_target,
                                &request.headers,
                                &upstream_request_body,
                                &account,
                                collection.upstream_proxy_url.as_deref(),
                                upstream_connect_timeout,
                                &timeouts,
                                image_generation_mode,
                                request_kind,
                            )
                            .await
                            {
                                Ok(response) => response,
                                Err(err) => {
                                    last_status = StatusCode::BAD_GATEWAY.as_u16();
                                    log_codex_api_failure(
                                        None,
                                        Some(request),
                                        Some(last_status),
                                        Some(account.id.as_str()),
                                        Some(account.email.as_str()),
                                        None,
                                        format!("刷新后重试上游失败: {}", err).as_str(),
                                    );
                                    last_error = err;
                                    last_error_category = Some("upstream_network".to_string());
                                    break;
                                }
                            };

                            if response.status() == StatusCode::UNAUTHORIZED {
                                last_status = StatusCode::UNAUTHORIZED.as_u16();
                                invalidate_prepared_account(&account_id).await;
                                mark_account_failure(
                                    &account,
                                    Some(last_status),
                                    Some("auth_unavailable"),
                                    "账号鉴权失败",
                                    request_kind,
                                )
                                .await;
                                log_codex_api_failure(
                                    None,
                                    Some(request),
                                    Some(last_status),
                                    Some(account.id.as_str()),
                                    Some(account.email.as_str()),
                                    None,
                                    format!("账号 {} 鉴权失败", account.email).as_str(),
                                );
                                last_error = format!("账号 {} 鉴权失败", account.email);
                                last_error_category = Some("auth_unavailable".to_string());
                                break;
                            }
                        }
                        Err(err) => {
                            last_status = StatusCode::UNAUTHORIZED.as_u16();
                            invalidate_prepared_account(&account_id).await;
                            mark_account_failure(
                                &account,
                                Some(last_status),
                                Some("auth_refresh_failed"),
                                &err,
                                request_kind,
                            )
                            .await;
                            log_codex_api_failure(
                                None,
                                Some(request),
                                Some(StatusCode::UNAUTHORIZED.as_u16()),
                                Some(account.id.as_str()),
                                Some(account.email.as_str()),
                                None,
                                format!("账号刷新失败: {}", err).as_str(),
                            );
                            last_error = err;
                            last_error_category = Some("auth_refresh_failed".to_string());
                            break;
                        }
                    }
                }

                if response.status().is_success() {
                    clear_model_cooldown(&account.id, &routing_hint.model_key).await;
                    mark_account_success(&account, request_kind).await;
                    return Ok(ProxyDispatchSuccess {
                        upstream: response,
                        account_id: account.id.clone(),
                        account_email: account.email.clone(),
                    });
                }

                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if let Some(state) = rejected_field_retry_state.as_mut() {
                    if let Some((next_body, reason)) =
                        normalize_openai_responses_rejected_field_retry_body(
                            status,
                            &upstream_request_body,
                            body.as_bytes(),
                        )
                        .map_err(|message| ProxyDispatchError {
                            status: StatusCode::BAD_REQUEST.as_u16(),
                            message,
                            account_id: Some(account.id.clone()),
                            account_email: Some(account.email.clone()),
                            error_category: Some("bad_request".to_string()),
                        })?
                    {
                        if state.allow(&next_body) {
                            legacy_debug_log(
                                collection.debug_logs,
                                format!(
                                    "responses_rejected_field_retry account_id={} attempt={} reason={}",
                                    account.id, state.attempts, reason
                                ),
                            );
                            upstream_request_body = next_body;
                            continue;
                        }
                    }
                }
                let category = classify_upstream_error_category(status, &body);
                let message = if category == Some("image_generation_not_enabled") {
                    friendly_image_generation_capability_error(&account.email)
                } else {
                    summarize_upstream_error(status, &body)
                };
                mark_account_failure(
                    &account,
                    Some(status.as_u16()),
                    category,
                    &message,
                    request_kind,
                )
                .await;
                log_codex_api_failure(
                    None,
                    Some(request),
                    Some(status.as_u16()),
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    format!("上游返回失败: {}", message).as_str(),
                );

                if !collection.disable_cooling {
                    if let Some(retry_after) = parse_codex_retry_after(status, &body) {
                        set_model_cooldown(
                            &account.id,
                            &routing_hint.model_key,
                            retry_after,
                            "usage_limit_reached",
                        )
                        .await;
                        round_cooldown_wait = Some(match round_cooldown_wait {
                            Some(current) if current <= retry_after => current,
                            _ => retry_after,
                        });
                    }
                }

                let can_retry_single_account = (total == 1
                    || strategy == CodexLocalAccessRoutingStrategy::SingleAccount)
                    && single_account_status_retry_attempt
                        < timeouts.single_account_status_retry_attempts as usize
                    && should_retry_single_account_upstream_status(status);
                if can_retry_single_account {
                    single_account_status_retry_attempt += 1;
                    tokio::time::sleep(backoff_retry_delay(
                        single_account_status_retry_attempt,
                        timeouts.single_account_status_retry_base_delay_ms,
                        timeouts.single_account_status_retry_max_delay_ms,
                    ))
                    .await;
                    continue;
                }

                if should_try_next_account(status, &body) {
                    last_status = status.as_u16();
                    last_error = if category == Some("image_generation_not_enabled") {
                        message.clone()
                    } else {
                        format!("账号 {} 当前不可用，已尝试轮转: {}", account.email, message)
                    };
                    last_error_category = category.map(str::to_string);
                    break;
                }

                return Err(ProxyDispatchError {
                    status: status.as_u16(),
                    message,
                    account_id: Some(account.id.clone()),
                    account_email: Some(account.email.clone()),
                    error_category: category.map(str::to_string),
                });
            }
        }

        earliest_cooldown_wait = round_cooldown_wait;
        let Some(wait) = earliest_cooldown_wait else {
            break;
        };
        let max_retry_wait = Duration::from_millis(
            collection
                .max_retry_interval_ms
                .clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS),
        );
        if attempts >= max_credential_attempts
            || retry_round >= MAX_REQUEST_RETRY_ATTEMPTS
            || wait > max_retry_wait
        {
            if !attempted_in_round {
                return Err(ProxyDispatchError {
                    status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
                    message: build_cooldown_unavailable_message(&routing_hint.model_key, wait),
                    account_id: affinity_account_id.clone(),
                    account_email: None,
                    error_category: Some("cooldown".to_string()),
                });
            }
            break;
        }

        tokio::time::sleep(wait).await;
        retry_round += 1;
    }

    Err(ProxyDispatchError {
        status: if last_status == 503 {
            earliest_cooldown_wait
                .map(|_| StatusCode::TOO_MANY_REQUESTS.as_u16())
                .unwrap_or(last_status)
        } else {
            last_status
        },
        message: if matches!(last_status, 429 | 503) {
            earliest_cooldown_wait
                .map(|wait| build_cooldown_unavailable_message(&routing_hint.model_key, wait))
                .unwrap_or(last_error)
        } else {
            last_error
        },
        account_id: last_account_id,
        account_email: last_account_email,
        error_category: last_error_category,
    })
}

fn is_websocket_upgrade_request(request: &ParsedRequest) -> bool {
    let upgrade = header_value(&request.headers, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection = header_value(&request.headers, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    upgrade && connection && header_value(&request.headers, "sec-websocket-key").is_some()
}

fn websocket_accept_value(sec_websocket_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(sec_websocket_key.trim().as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    general_purpose::STANDARD.encode(hasher.finalize())
}

async fn accept_downstream_websocket(
    mut stream: TcpStream,
    request: &ParsedRequest,
) -> Result<WebSocketStream<TcpStream>, String> {
    let sec_key = header_value(&request.headers, "sec-websocket-key")
        .ok_or_else(|| "WebSocket 握手缺少 Sec-WebSocket-Key".to_string())?;
    let accept_value = websocket_accept_value(sec_key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        accept_value
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("写入 WebSocket 握手响应失败: {}", e))?;
    Ok(WebSocketStream::from_raw_socket(stream, Role::Server, None).await)
}

async fn read_initial_websocket_payload(
    downstream: &mut WebSocketStream<TcpStream>,
    initial_message_timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + initial_message_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("等待 WebSocket 首个 response.create 消息超时".to_string());
        }
        let message = timeout(remaining, downstream.next())
            .await
            .map_err(|_| "等待 WebSocket 首个 response.create 消息超时".to_string())?
            .ok_or_else(|| "客户端在发送首个 WebSocket 消息前已断开".to_string())?
            .map_err(|e| format!("读取 WebSocket 首个消息失败: {}", e))?;

        match message {
            Message::Text(text) => return Ok(text.to_string().into_bytes()),
            Message::Binary(bytes) => return Ok(bytes.to_vec()),
            Message::Ping(bytes) => {
                downstream
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|e| format!("回复 WebSocket Ping 失败: {}", e))?;
            }
            Message::Pong(_) => {}
            Message::Close(frame) => {
                let _ = downstream.send(Message::Close(frame)).await;
                return Err("客户端在发送首个 WebSocket 消息前已关闭连接".to_string());
            }
            _ => {}
        }
    }
}

fn prepare_websocket_initial_request(
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
    default_service_tier: Option<&str>,
) -> Result<(), String> {
    let mut body_value = parse_request_body_json(&request.body)
        .ok_or_else(|| "WebSocket response.create 消息必须是合法 JSON".to_string())?;
    let request_has_service_tier = request_body_has_service_tier(&body_value);
    rewrite_request_model_alias_value(&mut body_value);
    codex_protocol::normalize_responses_body_for_codex_with_lite(
        &mut body_value,
        request_uses_responses_lite(request),
    );
    if !request_has_service_tier {
        apply_default_service_tier_if_missing(&mut body_value, default_service_tier);
    }
    let body_obj = body_value
        .as_object_mut()
        .ok_or_else(|| "WebSocket response.create 消息必须是 JSON 对象".to_string())?;
    body_obj.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    request.body = serde_json::to_vec(&body_value)
        .map_err(|e| format!("序列化 WebSocket response.create 消息失败: {}", e))?;
    request
        .headers
        .insert("content-type".to_string(), "application/json".to_string());
    align_codex_prompt_cache(request, api_key)?;
    apply_codex_official_headers(request);
    Ok(())
}

fn build_upstream_websocket_url(account: &CodexAccount, target: &str) -> Result<String, String> {
    let http_url = build_upstream_url(account, target)?;
    let mut parsed =
        Url::parse(&http_url).map_err(|e| format!("上游 WebSocket URL 无效: {}", e))?;
    let next_scheme = match parsed.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => return Err(format!("上游 WebSocket 不支持 {} 协议", other)),
    };
    parsed
        .set_scheme(next_scheme)
        .map_err(|_| "切换上游 WebSocket 协议失败".to_string())?;
    Ok(parsed.to_string())
}

fn should_skip_websocket_upstream_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "host"
            | "content-length"
            | "connection"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-protocol"
            | "sec-websocket-extensions"
            | "accept-encoding"
            | "proxy-connection"
            | "x-api-key"
            | "x-agtools-local-request-kind"
    )
}

fn websocket_header_value(value: impl Into<String>) -> Result<WsHeaderValue, String> {
    WsHeaderValue::from_str(&value.into()).map_err(|e| format!("无效 WebSocket 请求头值: {}", e))
}

fn websocket_target_host_port(request: &WsClientRequest) -> Result<(String, u16), String> {
    let uri = request.uri();
    let host = uri
        .host()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "上游 WebSocket URL 缺少 Host".to_string())?
        .to_string();
    let port = uri
        .port_u16()
        .or_else(|| match uri.scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or_else(|| "上游 WebSocket URL 缺少端口".to_string())?;
    Ok((host, port))
}

async fn tcp_connect_with_timeout(
    addr: &str,
    label: &str,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("连接 {} 超时", label))?
        .map_err(|e| format!("连接 {} 失败: {}", label, e))
}

fn decode_proxy_credential(value: &str) -> String {
    urlencoding::decode(value)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| value.to_string())
}

fn proxy_authorization_header(proxy_url: &Url) -> Option<String> {
    if proxy_url.username().is_empty() {
        return None;
    }
    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let credential = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    Some(format!("Proxy-Authorization: Basic {}\r\n", credential))
}

async fn connect_http_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket 上游代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket 上游代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket HTTP 代理", connect_timeout).await?;
    let target_addr = format!("{}:{}", target_host, target_port);
    let auth_header = proxy_authorization_header(proxy_url).unwrap_or_default();
    let request = format!(
        "CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\nProxy-Connection: Keep-Alive\r\n{auth_header}\r\n"
    );
    timeout(connect_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "发送 WebSocket 代理 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket 代理 CONNECT 请求失败: {}", e))?;

    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if response.len() > CODEX_WEBSOCKET_PROXY_CONNECT_MAX_BYTES {
            return Err("WebSocket 代理 CONNECT 响应过大".to_string());
        }
        let read = timeout(connect_timeout, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取 WebSocket 代理 CONNECT 响应超时".to_string())?
            .map_err(|e| format!("读取 WebSocket 代理 CONNECT 响应失败: {}", e))?;
        if read == 0 {
            return Err("WebSocket 代理在 CONNECT 完成前关闭连接".to_string());
        }
        response.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&response) {
            let header_text = String::from_utf8_lossy(&response[..header_end]);
            let status_line = header_text
                .lines()
                .next()
                .ok_or_else(|| "WebSocket 代理 CONNECT 响应为空".to_string())?;
            let status = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| format!("WebSocket 代理 CONNECT 响应状态无效: {}", status_line))?;
            if (200..300).contains(&status) {
                return Ok(stream);
            }
            return Err(format!("WebSocket 代理 CONNECT 失败: HTTP {}", status));
        }
    }
}

async fn socks5_read_exact(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    connect_timeout: Duration,
) -> Result<(), String> {
    timeout(connect_timeout, stream.read_exact(buffer))
        .await
        .map_err(|_| "读取 WebSocket SOCKS5 代理响应超时".to_string())?
        .map_err(|e| format!("读取 WebSocket SOCKS5 代理响应失败: {}", e))?;
    Ok(())
}

async fn connect_socks5_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket SOCKS5 代理", connect_timeout).await?;

    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let use_auth = !username.is_empty();
    let greeting: &[u8] = if use_auth {
        &[0x05, 0x02, 0x00, 0x02]
    } else {
        &[0x05, 0x01, 0x00]
    };
    timeout(connect_timeout, stream.write_all(greeting))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 握手超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 握手失败: {}", e))?;

    let mut method_response = [0u8; 2];
    socks5_read_exact(&mut stream, &mut method_response, connect_timeout).await?;
    if method_response[0] != 0x05 {
        return Err("WebSocket SOCKS5 代理响应版本无效".to_string());
    }
    if method_response[1] == 0xff {
        return Err("WebSocket SOCKS5 代理不接受当前认证方式".to_string());
    }
    if method_response[1] == 0x02 {
        let username_bytes = username.as_bytes();
        let password_bytes = password.as_bytes();
        if username_bytes.len() > u8::MAX as usize || password_bytes.len() > u8::MAX as usize {
            return Err("WebSocket SOCKS5 代理用户名或密码过长".to_string());
        }
        let mut auth_request = Vec::with_capacity(3 + username_bytes.len() + password_bytes.len());
        auth_request.push(0x01);
        auth_request.push(username_bytes.len() as u8);
        auth_request.extend_from_slice(username_bytes);
        auth_request.push(password_bytes.len() as u8);
        auth_request.extend_from_slice(password_bytes);
        timeout(connect_timeout, stream.write_all(&auth_request))
            .await
            .map_err(|_| "发送 WebSocket SOCKS5 认证超时".to_string())?
            .map_err(|e| format!("发送 WebSocket SOCKS5 认证失败: {}", e))?;
        let mut auth_response = [0u8; 2];
        socks5_read_exact(&mut stream, &mut auth_response, connect_timeout).await?;
        if auth_response != [0x01, 0x00] {
            return Err("WebSocket SOCKS5 代理认证失败".to_string());
        }
    } else if method_response[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 代理返回不支持的认证方式: {}",
            method_response[1]
        ));
    }

    let target_host_bytes = target_host.as_bytes();
    if target_host_bytes.len() > u8::MAX as usize {
        return Err("WebSocket SOCKS5 目标 Host 过长".to_string());
    }
    let mut connect_request = Vec::with_capacity(7 + target_host_bytes.len());
    connect_request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, target_host_bytes.len() as u8]);
    connect_request.extend_from_slice(target_host_bytes);
    connect_request.extend_from_slice(&target_port.to_be_bytes());
    timeout(connect_timeout, stream.write_all(&connect_request))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 CONNECT 请求失败: {}", e))?;

    let mut reply_header = [0u8; 4];
    socks5_read_exact(&mut stream, &mut reply_header, connect_timeout).await?;
    if reply_header[0] != 0x05 {
        return Err("WebSocket SOCKS5 CONNECT 响应版本无效".to_string());
    }
    if reply_header[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 CONNECT 失败，状态码 {}",
            reply_header[1]
        ));
    }
    let addr_len = match reply_header[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0u8; 1];
            socks5_read_exact(&mut stream, &mut len, connect_timeout).await?;
            len[0] as usize
        }
        0x04 => 16,
        other => return Err(format!("WebSocket SOCKS5 CONNECT 地址类型无效: {}", other)),
    };
    let mut bound_addr = vec![0u8; addr_len + 2];
    socks5_read_exact(&mut stream, &mut bound_addr, connect_timeout).await?;
    Ok(stream)
}

async fn connect_upstream_websocket_socket(
    request: &WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let (target_host, target_port) = websocket_target_host_port(request)?;
    let signature = current_upstream_http_client_signature(upstream_proxy_url, connect_timeout);
    let Some(proxy_url) = signature.proxy_url.as_deref() else {
        return tcp_connect_with_timeout(
            &format!("{}:{}", target_host, target_port),
            "Codex 上游 WebSocket",
            connect_timeout,
        )
        .await;
    };
    let proxy_url =
        Url::parse(proxy_url).map_err(|e| format!("WebSocket 上游代理地址无效: {}", e))?;
    match proxy_url.scheme() {
        "http" => {
            connect_http_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout).await
        }
        "socks5" | "socks5h" => {
            connect_socks5_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout)
                .await
        }
        "https" => {
            Err("WebSocket 上游代理暂不支持 https 代理，请改用 http 或 socks5 代理地址".to_string())
        }
        other => Err(format!("WebSocket 上游代理不支持 {} 协议", other)),
    }
}

impl WebSocketConnectError {
    fn upstream(message: String) -> Self {
        Self {
            status: None,
            message,
            category: "upstream_websocket".to_string(),
        }
    }
}

fn websocket_connect_error_from_http_response(
    status: StatusCode,
    body: String,
) -> WebSocketConnectError {
    let category = classify_upstream_error_category(status, &body)
        .unwrap_or("upstream_websocket")
        .to_string();
    let message = if body.trim().is_empty() {
        format!("Codex 上游 WebSocket 握手失败: HTTP {}", status.as_u16())
    } else {
        format!(
            "Codex 上游 WebSocket 握手失败: {}",
            summarize_upstream_error(status, &body)
        )
    };
    WebSocketConnectError {
        status: Some(status.as_u16()),
        message,
        category,
    }
}

fn websocket_connect_error_from_tungstenite(error: WsError) -> WebSocketConnectError {
    match error {
        WsError::Http(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = response
                .body()
                .as_deref()
                .map(String::from_utf8_lossy)
                .map(Cow::into_owned)
                .unwrap_or_default();
            websocket_connect_error_from_http_response(status, body)
        }
        other => {
            WebSocketConnectError::upstream(format!("连接 Codex 上游 WebSocket 失败: {}", other))
        }
    }
}

async fn connect_upstream_websocket_request(
    request: WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let socket = connect_upstream_websocket_socket(&request, upstream_proxy_url, connect_timeout)
        .await
        .map_err(WebSocketConnectError::upstream)?;
    let (upstream, _) = client_async_tls_with_config(request, socket, None, None)
        .await
        .map_err(websocket_connect_error_from_tungstenite)?;
    Ok(upstream)
}

async fn connect_upstream_websocket(
    request: &ParsedRequest,
    account: &CodexAccount,
    upstream_target: &str,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let ws_url = build_upstream_websocket_url(account, upstream_target)
        .map_err(WebSocketConnectError::upstream)?;
    let upstream_token =
        account_upstream_token(account).map_err(WebSocketConnectError::upstream)?;
    let mut upstream_request = ws_url.as_str().into_client_request().map_err(|e| {
        WebSocketConnectError::upstream(format!("创建上游 WebSocket 请求失败: {}", e))
    })?;

    let session_id = header_value(&request.headers, "session-id")
        .or_else(|| header_value(&request.headers, "session_id"));
    for (name, value) in &request.headers {
        if should_skip_websocket_upstream_header(name.as_str()) {
            continue;
        }
        if matches!(name.as_str(), "session_id" | "session-id") {
            continue;
        }
        if !account.is_api_key_auth() && matches!(name.as_str(), "user-agent" | "originator") {
            continue;
        }
        let header_name = WsHeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            WebSocketConnectError::upstream(format!("无效 WebSocket 请求头 {}: {}", name, e))
        })?;
        let header_value =
            websocket_header_value(value.clone()).map_err(WebSocketConnectError::upstream)?;
        upstream_request
            .headers_mut()
            .insert(header_name, header_value);
    }

    upstream_request.headers_mut().insert(
        "Authorization",
        websocket_header_value(format!("Bearer {}", upstream_token))
            .map_err(WebSocketConnectError::upstream)?,
    );
    if !account.is_api_key_auth() {
        upstream_request.headers_mut().insert(
            "User-Agent",
            websocket_header_value(DEFAULT_CODEX_USER_AGENT)
                .map_err(WebSocketConnectError::upstream)?,
        );
        upstream_request.headers_mut().insert(
            "Originator",
            websocket_header_value(DEFAULT_CODEX_ORIGINATOR)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    if let Some(session_id) = session_id {
        upstream_request.headers_mut().insert(
            "Session-Id",
            websocket_header_value(session_id).map_err(WebSocketConnectError::upstream)?,
        );
    }
    if !account.is_api_key_auth() {
        if let Some(account_id) = resolve_upstream_account_id(account) {
            upstream_request.headers_mut().insert(
                "ChatGPT-Account-Id",
                websocket_header_value(account_id).map_err(WebSocketConnectError::upstream)?,
            );
        }
    }
    let beta_header = header_value(&request.headers, "openai-beta").unwrap_or_default();
    if !beta_header.contains("responses_websockets=") {
        upstream_request.headers_mut().insert(
            "OpenAI-Beta",
            websocket_header_value(CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    connect_upstream_websocket_request(upstream_request, upstream_proxy_url, connect_timeout).await
}

async fn proxy_websocket_with_account_pool(
    request: &ParsedRequest,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<WebSocketDispatchSuccess, ProxyDispatchError> {
    let unfiltered_scoped_account_ids = scoped_collection_account_ids(collection, api_key);
    let strategy = effective_routing_strategy(collection, &unfiltered_scoped_account_ids);
    let scoped_account_ids =
        apply_bound_oauth_quota_reserve(collection, unfiltered_scoped_account_ids.clone());
    if scoped_account_ids.is_empty() {
        let quota_reserved = !unfiltered_scoped_account_ids.is_empty();
        return Err(ProxyDispatchError {
            status: 503,
            message: if quota_reserved {
                "绑定 OAuth 账号已达到保留额度，当前没有可路由账号".to_string()
            } else {
                "本地接入集合暂无账号".to_string()
            },
            account_id: None,
            account_email: None,
            error_category: Some(
                if quota_reserved {
                    "quota_reserved"
                } else {
                    "no_accounts"
                }
                .to_string(),
            ),
        });
    }

    let upstream_target =
        resolve_upstream_target(&request.target).map_err(|err| ProxyDispatchError {
            status: 400,
            message: err,
            account_id: None,
            account_email: None,
            error_category: Some("bad_request".to_string()),
        })?;
    let timeouts = collection_timeouts(collection);
    let websocket_connect_timeout = duration_from_millis(
        timeouts.websocket_connect_timeout_ms,
        CODEX_WEBSOCKET_CONNECT_TIMEOUT,
    );
    let routing_hint = build_request_routing_hint(request);
    let total = scoped_account_ids.len();
    let max_credential_attempts = max_credential_attempts_for_strategy(collection, total, strategy);
    let start = GATEWAY_ROUND_ROBIN_CURSOR.fetch_add(1, Ordering::Relaxed);
    let session_affinity_key = routing_hint
        .session_affinity_key
        .as_deref()
        .filter(|_| collection.session_affinity)
        .map(session_affinity_binding_key);
    let affinity_account_id = if let Some(session_key) = session_affinity_key.as_deref() {
        resolve_affinity_account(session_key).await
    } else {
        None
    };
    let priority_account_ids = affinity_account_id
        .as_ref()
        .map(|account_id| vec![account_id.clone()])
        .unwrap_or_else(|| api_key_priority_account_ids(collection, api_key));
    let ordered_account_ids = request_ordered_account_ids(
        collection,
        &scoped_account_ids,
        strategy,
        start,
        &priority_account_ids,
    );
    let strategy_account_ids = pin_account_to_front_for_strategy(
        apply_routing_strategy(
            &ordered_account_ids,
            strategy,
            &collection.custom_routing_rules,
            start,
        ),
        &priority_account_ids,
        strategy,
        &collection.custom_routing_rules,
    );

    let mut attempts = 0usize;
    let mut last_status = StatusCode::BAD_GATEWAY.as_u16();
    let mut last_error = "本地接入集合暂无可用账号".to_string();
    let mut last_error_category: Option<String> = None;
    let mut last_account_id: Option<String> = None;
    let mut last_account_email: Option<String> = None;

    for account_id in strategy_account_ids {
        if attempts >= max_credential_attempts {
            break;
        }
        if account_model_rule_blocks_model(collection, &account_id, &routing_hint.model_key) {
            last_status = StatusCode::SERVICE_UNAVAILABLE.as_u16();
            last_error = if routing_hint.model_key.trim().is_empty() {
                "账号模型规则已跳过该账号".to_string()
            } else {
                format!(
                    "模型 {} 在部分账号上已被禁用，已跳过这些账号",
                    routing_hint.model_key
                )
            };
            last_error_category = Some("account_model_disabled".to_string());
            continue;
        }
        if account_id_blocked_by_health(&account_id).await {
            last_error = "账号连续鉴权或预处理失败，已暂时跳过".to_string();
            last_error_category = Some("account_unhealthy".to_string());
            continue;
        }
        if !collection.disable_cooling {
            if get_model_cooldown_wait(&account_id, &routing_hint.model_key)
                .await
                .is_some()
            {
                continue;
            }
        }
        attempts += 1;

        let mut account = match get_prepared_account(&account_id).await {
            Ok(account) => account,
            Err(err) => {
                invalidate_prepared_account(&account_id).await;
                last_status = StatusCode::BAD_GATEWAY.as_u16();
                last_error = err;
                last_error_category = Some("account_prepare_failed".to_string());
                continue;
            }
        };
        if collection.restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref()) {
            mark_account_failure(
                &account,
                None,
                Some("free_account_restricted"),
                "Free 账号不支持加入本地接入",
                request_kind,
            )
            .await;
            last_error = "Free 账号不支持加入本地接入".to_string();
            last_error_category = Some("free_account_restricted".to_string());
            continue;
        }

        last_account_id = Some(account.id.clone());
        last_account_email = Some(account.email.clone());

        match connect_upstream_websocket(
            request,
            &account,
            &upstream_target,
            collection.upstream_proxy_url.as_deref(),
            websocket_connect_timeout,
        )
        .await
        {
            Ok(upstream) => {
                return Ok(WebSocketDispatchSuccess {
                    upstream,
                    account_id: account.id.clone(),
                    account_email: account.email.clone(),
                    account,
                });
            }
            Err(err) => {
                let status = err.status.unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
                if status == StatusCode::UNAUTHORIZED.as_u16() && account.is_api_key_auth() {
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(status),
                        Some("auth_unavailable"),
                        "API Key 账号上游 WebSocket 鉴权失败",
                        request_kind,
                    )
                    .await;
                    last_status = status;
                    last_error = format!("API Key 账号 {} 上游 WebSocket 鉴权失败", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    continue;
                }

                if status == StatusCode::UNAUTHORIZED.as_u16()
                    && !account_has_refresh_token(&account)
                {
                    invalidate_prepared_account(&account_id).await;
                    if let Err(err) =
                        codex_account::mark_access_token_only_account_requires_reauth(&account.id)
                    {
                        logger::log_codex_api_warn(&format!(
                            "[CodexLocalAccess] 标记 access-token-only WebSocket 账号需重新登录失败: account_id={}, error={}",
                            account.id, err
                        ));
                    }
                    mark_account_failure(
                        &account,
                        Some(status),
                        Some("auth_unavailable"),
                        "access-token-only 账号的 WebSocket access_token 已被上游拒绝",
                        request_kind,
                    )
                    .await;
                    last_status = status;
                    last_error = format!(
                        "账号 {} 当前 WebSocket access_token 已被上游拒绝",
                        account.email
                    );
                    last_error_category = Some("auth_unavailable".to_string());
                    continue;
                }

                if status == StatusCode::UNAUTHORIZED.as_u16() {
                    match force_refresh_gateway_account(&account_id, account.token_generation).await
                    {
                        Ok(refreshed_account) => {
                            account = refreshed_account;
                            match connect_upstream_websocket(
                                request,
                                &account,
                                &upstream_target,
                                collection.upstream_proxy_url.as_deref(),
                                websocket_connect_timeout,
                            )
                            .await
                            {
                                Ok(upstream) => {
                                    return Ok(WebSocketDispatchSuccess {
                                        upstream,
                                        account_id: account.id.clone(),
                                        account_email: account.email.clone(),
                                        account,
                                    });
                                }
                                Err(retry_err) => {
                                    let retry_status = retry_err
                                        .status
                                        .unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
                                    let retry_category =
                                        if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                            "auth_unavailable"
                                        } else {
                                            retry_err.category.as_str()
                                        };
                                    if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                        invalidate_prepared_account(&account_id).await;
                                    }
                                    mark_account_failure(
                                        &account,
                                        Some(retry_status),
                                        Some(retry_category),
                                        &retry_err.message,
                                        request_kind,
                                    )
                                    .await;
                                    last_status = retry_status;
                                    last_error =
                                        if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                            format!("账号 {} WebSocket 鉴权失败", account.email)
                                        } else {
                                            retry_err.message
                                        };
                                    last_error_category = Some(retry_category.to_string());
                                }
                            }
                        }
                        Err(refresh_err) => {
                            invalidate_prepared_account(&account_id).await;
                            mark_account_failure(
                                &account,
                                Some(status),
                                Some("auth_refresh_failed"),
                                &refresh_err,
                                request_kind,
                            )
                            .await;
                            last_status = status;
                            last_error = refresh_err;
                            last_error_category = Some("auth_refresh_failed".to_string());
                        }
                    }
                    continue;
                }

                mark_account_failure(
                    &account,
                    Some(status),
                    Some(err.category.as_str()),
                    &err.message,
                    request_kind,
                )
                .await;
                last_status = status;
                last_error = err.message;
                last_error_category = Some(err.category);
            }
        }
    }

    Err(ProxyDispatchError {
        status: last_status,
        message: last_error,
        account_id: last_account_id,
        account_email: last_account_email,
        error_category: last_error_category,
    })
}

fn websocket_capture_from_message(message: &Message, capture: &mut ResponseCapture) {
    let parsed = match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    };
    let Some(value) = parsed else {
        return;
    };
    if let Some(usage) = extract_usage_capture(&value) {
        capture.usage = Some(usage);
    }
    if capture.response_id.is_none() {
        capture.response_id = extract_response_id(&value);
    }
    if capture.response_model.is_none() {
        capture.response_model = extract_response_model(&value);
    }
}

fn websocket_message_value(message: &Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    }
}

fn websocket_error_status(value: &Value) -> Option<u16> {
    for key in ["status", "status_code"] {
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .and_then(|status| status.parse::<u16>().ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
    }

    None
}

fn build_websocket_error_body(value: &Value, status: u16) -> Value {
    let mut out = Map::new();
    out.insert("status".to_string(), json!(status));

    if let Some(body) = value.get("body") {
        out.insert("body".to_string(), body.clone());
        if let Some(error) = body.get("error") {
            out.insert("error".to_string(), error.clone());
            return Value::Object(out);
        }
    }

    if let Some(error) = value.get("error") {
        out.insert("error".to_string(), error.clone());
        return Value::Object(out);
    }

    out.insert(
        "error".to_string(),
        json!({
            "type": "server_error",
            "message": format!("HTTP {}", status),
        }),
    );
    Value::Object(out)
}

fn retry_after_duration_from_value(value: &Value) -> Option<Duration> {
    if let Some(seconds) = value.as_u64() {
        return Some(Duration::from_secs(seconds));
    }
    value
        .as_str()
        .map(str::trim)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn parse_websocket_retry_after_header(value: &Value) -> Option<Duration> {
    let headers = value.get("headers")?.as_object()?;
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("retry-after") {
            retry_after_duration_from_value(value)
        } else {
            None
        }
    })
}

fn websocket_error_matches(value: &Value, expected: &str) -> bool {
    for path in [
        &["error", "code"][..],
        &["error", "type"][..],
        &["body", "error", "code"][..],
        &["body", "error", "type"][..],
        &["code"][..],
        &["error"][..],
    ] {
        if extract_body_string_path(value, path).as_deref() == Some(expected) {
            return true;
        }
    }
    false
}

fn parse_websocket_upstream_error(message: &Message) -> Option<WebSocketUpstreamError> {
    let value = websocket_message_value(message)?;
    if value.get("type").and_then(Value::as_str).map(str::trim) != Some("error") {
        return None;
    }

    let status = websocket_error_status(&value)?;
    let body_value = build_websocket_error_body(&value, status);
    let body = serde_json::to_string(&body_value).unwrap_or_else(|_| value.to_string());
    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let usage_retry_after = parse_codex_retry_after(status_code, &body);
    let is_connection_limit = websocket_error_matches(&value, "websocket_connection_limit_reached");
    let category = if is_connection_limit {
        "websocket_connection_limit_reached"
    } else if usage_retry_after.is_some() || websocket_error_matches(&value, "usage_limit_reached")
    {
        "usage_limit_reached"
    } else {
        classify_upstream_error_category(status_code, &body).unwrap_or("upstream_websocket_error")
    }
    .to_string();
    let retry_after = usage_retry_after
        .or_else(|| parse_websocket_retry_after_header(&value))
        .or_else(|| is_connection_limit.then_some(Duration::ZERO));

    Some(WebSocketUpstreamError {
        status,
        body,
        category,
        retry_after,
    })
}

#[derive(Clone)]
struct WebSocketImageGenerationFilter {
    account: CodexAccount,
    fallback_mode: CodexLocalAccessImageGenerationMode,
    request_headers: HashMap<String, String>,
    responses_lite: bool,
}

async fn current_websocket_image_generation_mode(
    filter: &WebSocketImageGenerationFilter,
) -> CodexLocalAccessImageGenerationMode {
    let collection_mode = gateway_runtime()
        .lock()
        .await
        .collection
        .as_ref()
        .map(|collection| collection.image_generation_mode)
        .unwrap_or(filter.fallback_mode);
    request_image_generation_mode(collection_mode, &filter.request_headers)
}

fn filter_websocket_client_message(
    message: Message,
    account: &CodexAccount,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    responses_lite: bool,
) -> Result<Message, String> {
    fn filter_payload(
        body: &[u8],
        account: &CodexAccount,
        image_generation_mode: CodexLocalAccessImageGenerationMode,
        responses_lite: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut body_value = parse_request_body_json(body);
        let message_uses_responses_lite = responses_lite
            || body_value
                .as_ref()
                .and_then(|value| {
                    value
                        .get("model")
                        .or_else(|| value.pointer("/response/model"))
                })
                .and_then(Value::as_str)
                .is_some_and(codex_protocol::codex_model_uses_responses_lite);
        let lite_filtered = if message_uses_responses_lite {
            match body_value.as_mut() {
                Some(body_value) => {
                    if codex_protocol::filter_responses_lite_tools(body_value) {
                        Some(serde_json::to_vec(body_value).map_err(|error| {
                            format!(
                                "序列化 WebSocket Responses Lite 工具过滤结果失败: {}",
                                error
                            )
                        })?)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let source = lite_filtered.as_deref().unwrap_or(body);
        let effective_image_generation_mode = if message_uses_responses_lite
            && image_generation_mode == CodexLocalAccessImageGenerationMode::Enabled
        {
            CodexLocalAccessImageGenerationMode::ImagesOnly
        } else {
            image_generation_mode
        };
        let account_filtered = build_account_scoped_upstream_body(
            "/responses",
            source,
            account,
            effective_image_generation_mode,
            CodexLocalAccessRequestKind::Text,
        )?;
        match account_filtered {
            Cow::Borrowed(_) => Ok(lite_filtered),
            Cow::Owned(filtered) => Ok(Some(filtered)),
        }
    }

    match message {
        Message::Text(text) => {
            let body = text.to_string().into_bytes();
            let Some(filtered) =
                filter_payload(&body, account, image_generation_mode, responses_lite)?
            else {
                return Ok(Message::Text(text));
            };
            let filtered = String::from_utf8(filtered)
                .map_err(|error| format!("过滤 WebSocket 文本图片工具后不是 UTF-8: {}", error))?;
            Ok(Message::Text(filtered.into()))
        }
        Message::Binary(bytes) => {
            let Some(filtered) = filter_payload(
                bytes.as_ref(),
                account,
                image_generation_mode,
                responses_lite,
            )?
            else {
                return Ok(Message::Binary(bytes));
            };
            Ok(Message::Binary(filtered.into()))
        }
        other => Ok(other),
    }
}

async fn bridge_websocket_streams(
    downstream: WebSocketStream<TcpStream>,
    mut upstream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    first_payload: Vec<u8>,
    timeouts: CodexLocalAccessTimeouts,
    image_filter: Option<WebSocketImageGenerationFilter>,
) -> Result<WebSocketBridgeResult, String> {
    let first_payload = if let Some(filter) = image_filter.as_ref() {
        let mode = current_websocket_image_generation_mode(filter).await;
        build_account_scoped_upstream_body(
            "/responses",
            &first_payload,
            &filter.account,
            mode,
            CodexLocalAccessRequestKind::Text,
        )?
        .into_owned()
    } else {
        first_payload
    };
    let first_text = String::from_utf8(first_payload)
        .map_err(|e| format!("WebSocket response.create 不是合法 UTF-8: {}", e))?;
    upstream
        .send(Message::Text(first_text.into()))
        .await
        .map_err(|e| format!("发送首个 WebSocket 上游消息失败: {}", e))?;

    let (mut downstream_write, mut downstream_read) = downstream.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();
    let mut capture = ResponseCapture::default();
    let mut upstream_error = None;
    let heartbeat_interval = duration_from_millis(
        timeouts.websocket_heartbeat_interval_ms,
        CODEX_WEBSOCKET_HEARTBEAT_INTERVAL,
    );
    let idle_timeout = duration_from_millis(
        timeouts.websocket_idle_timeout_ms,
        CODEX_WEBSOCKET_IDLE_TIMEOUT,
    );
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                upstream_write
                    .send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|e| format!("发送 Codex 上游 WebSocket 心跳失败: {}", e))?;
                upstream_write
                    .flush()
                    .await
                    .map_err(|e| format!("刷新 Codex 上游 WebSocket 心跳失败: {}", e))?;
            }
            downstream_next = timeout(idle_timeout, downstream_read.next()) => {
                let downstream_next = downstream_next
                    .map_err(|_| "WebSocket 客户端空闲超时".to_string())?;
                let Some(message_result) = downstream_next else {
                    break;
                };
                let mut message = message_result
                    .map_err(|e| format!("读取 WebSocket 客户端消息失败: {}", e))?;
                if let Some(filter) = image_filter.as_ref() {
                    let mode = current_websocket_image_generation_mode(filter).await;
                    message = filter_websocket_client_message(
                        message,
                        &filter.account,
                        mode,
                        filter.responses_lite,
                    )?;
                }
                let should_close = matches!(message, Message::Close(_));
                upstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 WebSocket 客户端消息失败: {}", e))?;
                if should_close {
                    break;
                }
            }
            upstream_next = timeout(idle_timeout, upstream_read.next()) => {
                let upstream_next = upstream_next
                    .map_err(|_| "Codex 上游 WebSocket 空闲超时".to_string())?;
                let Some(message_result) = upstream_next else {
                    break;
                };
                let message = message_result
                    .map_err(|e| format!("读取 Codex 上游 WebSocket 消息失败: {}", e))?;
                websocket_capture_from_message(&message, &mut capture);
                let parsed_upstream_error = parse_websocket_upstream_error(&message);
                let should_close = matches!(message, Message::Close(_));
                downstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 Codex 上游 WebSocket 消息失败: {}", e))?;
                if let Some(error) = parsed_upstream_error {
                    upstream_error = Some(error);
                    break;
                }
                if should_close {
                    break;
                }
            }
        }
    }

    Ok(WebSocketBridgeResult {
        capture,
        upstream_error,
    })
}

async fn handle_websocket_connection(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    mut parsed: ParsedRequest,
    collection: CodexLocalAccessCollection,
    resolved_api_key: ResolvedLocalApiKey,
) -> Result<(), String> {
    let started_at = Instant::now();
    let timeouts = collection_timeouts(&collection);
    let mut downstream = accept_downstream_websocket(stream, &parsed).await?;
    let initial_message_timeout = duration_from_millis(
        timeouts.websocket_initial_message_timeout_ms,
        CODEX_WEBSOCKET_INITIAL_MESSAGE_TIMEOUT,
    );
    let initial_payload =
        match read_initial_websocket_payload(&mut downstream, initial_message_timeout).await {
            Ok(payload) => payload,
            Err(err) => {
                let _ = downstream.send(Message::Close(None)).await;
                return Err(err);
            }
        };
    parsed.body = initial_payload;
    let default_service_tier = api_service_default_service_tier()?;
    prepare_websocket_initial_request(&mut parsed, &resolved_api_key, default_service_tier)?;
    let stats_context = RequestStatsContext {
        request_kind: CodexLocalAccessRequestKind::Text,
        model_id: stats_model_id_for_request_kind(&parsed.body, CodexLocalAccessRequestKind::Text),
        api_key_id: resolved_api_key.id.clone(),
        api_key_label: resolved_api_key.label.clone(),
    };
    let stats_service_tier = service_tier_from_request_body(&parsed.body);
    let stats_reasoning_effort = reasoning_effort_from_request_body(&parsed.body);
    let routing_hint = build_request_routing_hint(&parsed);

    match proxy_websocket_with_account_pool(
        &parsed,
        &collection,
        &resolved_api_key,
        stats_context.request_kind,
    )
    .await
    {
        Ok(success) => {
            let account_id = success.account_id.clone();
            let account_email = success.account_email.clone();
            let account = success.account.clone();
            let bridge_result = bridge_websocket_streams(
                downstream,
                success.upstream,
                parsed.body.clone(),
                timeouts.clone(),
                Some(WebSocketImageGenerationFilter {
                    account: success.account.clone(),
                    fallback_mode: collection.image_generation_mode,
                    request_headers: parsed.headers.clone(),
                    responses_lite: request_body_uses_responses_lite(&parsed),
                }),
            )
            .await?;
            let response_model_id = stats_model_id_from_response_capture(
                stats_context.model_id.as_str(),
                &bridge_result.capture,
            );
            if let Some(upstream_error) = bridge_result.upstream_error {
                mark_account_failure(
                    &account,
                    Some(upstream_error.status),
                    Some(upstream_error.category.as_str()),
                    upstream_error.body.as_str(),
                    stats_context.request_kind,
                )
                .await;
                if !collection.disable_cooling {
                    if let Some(retry_after) = upstream_error.retry_after {
                        set_model_cooldown(
                            &account_id,
                            &routing_hint.model_key,
                            retry_after,
                            upstream_error.category.as_str(),
                        )
                        .await;
                    }
                }

                let latency_ms = started_at.elapsed().as_millis() as u64;
                log_codex_api_failure(
                    Some(&addr),
                    Some(&parsed),
                    Some(upstream_error.status),
                    Some(account_id.as_str()),
                    Some(account_email.as_str()),
                    Some(latency_ms),
                    upstream_error.body.as_str(),
                );
                if let Err(err) = record_request_stats_with_meta(
                    Some(account_id.as_str()),
                    Some(account_email.as_str()),
                    Some(stats_context.api_key_id.as_str()),
                    Some(stats_context.api_key_label.as_str()),
                    Some(response_model_id.as_str()),
                    stats_context.request_kind,
                    false,
                    Some(upstream_error.category.as_str()),
                    latency_ms,
                    bridge_result.capture.usage,
                    RequestStatsMeta {
                        service_tier: stats_service_tier.as_deref(),
                        reasoning_effort: stats_reasoning_effort.as_deref(),
                        ..RequestStatsMeta::default()
                    },
                )
                .await
                {
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess] 写入 WebSocket 上游失败统计失败: {}",
                        err
                    ));
                }
                return Ok(());
            }

            clear_model_cooldown(&account_id, &routing_hint.model_key).await;
            mark_account_success(&account, stats_context.request_kind).await;
            if let Some(response_id) = bridge_result.capture.response_id.as_deref() {
                bind_response_affinity(response_id, &account_id).await;
            }
            if collection.session_affinity {
                let session_key = routing_hint
                    .session_affinity_key
                    .clone()
                    .map(|key| session_affinity_binding_key(&key));
                if let Some(session_key) = session_key.as_deref() {
                    bind_response_affinity(session_key, &account_id).await;
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u64;
            if let Err(err) = record_request_stats_with_meta(
                Some(account_id.as_str()),
                Some(account_email.as_str()),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(response_model_id.as_str()),
                stats_context.request_kind,
                true,
                None,
                latency_ms,
                bridge_result.capture.usage,
                RequestStatsMeta {
                    service_tier: stats_service_tier.as_deref(),
                    reasoning_effort: stats_reasoning_effort.as_deref(),
                    ..RequestStatsMeta::default()
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入 WebSocket 请求统计失败: {}",
                    err
                ));
            }
            Ok(())
        }
        Err(error) => {
            let latency_ms = started_at.elapsed().as_millis() as u64;
            log_codex_api_failure(
                Some(&addr),
                Some(&parsed),
                Some(error.status),
                error.account_id.as_deref(),
                error.account_email.as_deref(),
                Some(latency_ms),
                error.message.as_str(),
            );
            let _ = downstream.send(Message::Close(None)).await;
            if let Err(err) = record_request_stats_with_meta(
                error.account_id.as_deref(),
                error.account_email.as_deref(),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                false,
                error.error_category.as_deref(),
                latency_ms,
                None,
                RequestStatsMeta {
                    service_tier: stats_service_tier.as_deref(),
                    reasoning_effort: stats_reasoning_effort.as_deref(),
                    ..RequestStatsMeta::default()
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入 WebSocket 失败统计失败: {}",
                    err
                ));
            }
            Err(error.message)
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    addr: std::net::SocketAddr,
) -> Result<(), String> {
    let request_read_timeout = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(collection_timeouts)
            .map(|timeouts| {
                duration_from_millis(
                    timeouts.legacy_request_read_timeout_ms,
                    DEFAULT_REQUEST_READ_TIMEOUT,
                )
            })
            .unwrap_or(DEFAULT_REQUEST_READ_TIMEOUT)
    };
    let raw_request = match read_http_request(&mut stream, request_read_timeout).await {
        Ok(raw_request) => raw_request,
        Err(err) => {
            let message = format!("读取本地 API 请求失败: {}", err);
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                400,
                "Bad Request",
                message.as_str(),
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
    };
    let mut parsed = match parse_http_request(&raw_request) {
        Ok(parsed) => parsed,
        Err(err) => {
            let message = format!("解析本地 API 请求失败: {}", err);
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                400,
                "Bad Request",
                message.as_str(),
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
    };

    if parsed.method.eq_ignore_ascii_case("OPTIONS") {
        stream
            .write_all(&options_response())
            .await
            .map_err(|e| format!("写入 OPTIONS 响应失败: {}", e))?;
        return Ok(());
    }

    if !parsed.method.eq_ignore_ascii_case("GET") && !parsed.method.eq_ignore_ascii_case("POST") {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            405,
            "Method Not Allowed",
            "Only GET and POST are allowed",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    parsed.target = normalize_proxy_target(&parsed.target)?;
    if !is_supported_proxy_target(&parsed.target) {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            "Not Found",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(api_key) = extract_local_api_key(&parsed.headers) else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            401,
            "Unauthorized",
            "缺少 Authorization Bearer 或 X-API-Key",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    let state = {
        let runtime = gateway_runtime().lock().await;
        build_request_state_snapshot(&runtime)
    };
    let Some(collection) = state.collection else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            503,
            "Service Unavailable",
            "本地接入集合尚未创建",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    if !collection.enabled || !state.running {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            503,
            "Service Unavailable",
            "本地接入服务未启用",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(resolved_api_key) = resolve_collection_api_key(&collection, &api_key) else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            401,
            "Unauthorized",
            "本地访问秘钥无效",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };
    touch_local_access_api_key(&resolved_api_key.id).await;
    let started_at = Instant::now();

    if !is_local_models_request(&parsed.target) {
        if let Some((token_used, token_limit)) = api_key_token_limit_exceeded(&resolved_api_key) {
            let request_kind = request_kind_from_target(&parsed.target);
            let model_id = stats_model_id_for_request_kind(&parsed.body, request_kind);
            let message = format!(
                "API key token limit exceeded ({} of {} tokens used)",
                token_used, token_limit
            );
            let latency_ms = started_at.elapsed().as_millis() as u64;
            write_json_error_response(
                &mut stream,
                Some(&addr),
                Some(&parsed),
                429,
                "Too Many Requests",
                message.as_str(),
                None,
                None,
                Some(latency_ms),
            )
            .await?;
            let stats_service_tier = service_tier_from_request_body(&parsed.body);
            if let Err(err) = record_request_stats_with_meta(
                None,
                None,
                Some(resolved_api_key.id.as_str()),
                Some(resolved_api_key.label.as_str()),
                Some(model_id.as_str()),
                request_kind,
                false,
                Some("token_limit_exceeded"),
                latency_ms,
                None,
                RequestStatsMeta {
                    http_status: Some(429),
                    error_message: Some(message.as_str()),
                    service_tier: stats_service_tier.as_deref(),
                    ..RequestStatsMeta::default()
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] failed to record token-limit rejection: {}",
                    err
                ));
            }
            return Ok(());
        }
    }

    if is_websocket_upgrade_request(&parsed) {
        if !is_backend_codex_responses_websocket_request(&parsed.target)
            && !is_responses_request(&parsed.target)
        {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                Some(&parsed),
                404,
                "Not Found",
                "WebSocket 仅支持 /backend-api/codex/responses",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
        return handle_websocket_connection(stream, addr, parsed, collection, resolved_api_key)
            .await;
    }

    if is_local_models_request(&parsed.target) {
        if scoped_collection_account_ids(&collection, &resolved_api_key).is_empty() {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                Some(&parsed),
                503,
                "Service Unavailable",
                "本地接入集合暂无账号",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }

        let model_ids = visible_codex_model_ids_for_api_key(&collection, &resolved_api_key, None);
        let response_body = if codex_protocol::is_codex_client_models_request(&parsed.target) {
            let windows = model_context_windows_for_account_ids(&scoped_collection_account_ids(
                &collection,
                &resolved_api_key,
            ));
            apply_explicit_context_windows_to_client_models(
                build_codex_client_models_response(&model_ids),
                &windows,
            )
        } else {
            build_local_models_response(&model_ids)
        };
        let response = json_response(200, "OK", &response_body);
        stream
            .write_all(&response)
            .await
            .map_err(|e| format!("写入模型响应失败: {}", e))?;
        return Ok(());
    }

    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled
        && (is_images_generations_request(&parsed.target)
            || is_images_edits_request(&parsed.target))
    {
        let request_kind = request_kind_from_target(&parsed.target);
        let model_id = stats_model_id_for_request_kind(&parsed.body, request_kind);
        let message = "API 服务已禁用 image_generation，图片生成和图片编辑接口不可用。";
        let latency_ms = started_at.elapsed().as_millis() as u64;
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            message,
            None,
            None,
            Some(latency_ms),
        )
        .await?;
        let stats_service_tier = service_tier_from_request_body(&parsed.body);
        let stats_reasoning_effort = reasoning_effort_from_request_body(&parsed.body);
        if let Err(err) = record_request_stats_with_meta(
            None,
            None,
            Some(resolved_api_key.id.as_str()),
            Some(resolved_api_key.label.as_str()),
            Some(model_id.as_str()),
            request_kind,
            false,
            Some("image_generation_disabled"),
            latency_ms,
            None,
            RequestStatsMeta {
                service_tier: stats_service_tier.as_deref(),
                reasoning_effort: stats_reasoning_effort.as_deref(),
                ..RequestStatsMeta::default()
            },
        )
        .await
        {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 写入禁用图片请求统计失败: {}",
                err
            ));
        }
        return Ok(());
    }
    let health_snapshot = {
        let runtime = gateway_runtime().lock().await;
        runtime.account_health.clone()
    };
    if let Err(err) = rewrite_request_model_for_access_policy(
        &mut parsed,
        &collection,
        &resolved_api_key,
        Some(&health_snapshot),
    ) {
        let latency_ms = started_at.elapsed().as_millis() as u64;
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            err.as_str(),
            None,
            None,
            Some(latency_ms),
        )
        .await?;
        let stats_service_tier = service_tier_from_request_body(&parsed.body);
        let stats_reasoning_effort = reasoning_effort_from_request_body(&parsed.body);
        if let Err(stats_err) = record_request_stats_with_meta(
            None,
            None,
            Some(resolved_api_key.id.as_str()),
            Some(resolved_api_key.label.as_str()),
            extract_request_model_id(&parsed.body).as_deref(),
            request_kind_from_target(&parsed.target),
            false,
            Some("model_not_available"),
            latency_ms,
            None,
            RequestStatsMeta {
                service_tier: stats_service_tier.as_deref(),
                reasoning_effort: stats_reasoning_effort.as_deref(),
                ..RequestStatsMeta::default()
            },
        )
        .await
        {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 写入模型规则拦截统计失败: {}",
                stats_err
            ));
        }
        return Ok(());
    }
    let default_service_tier = match api_service_default_service_tier() {
        Ok(service_tier) => service_tier,
        Err(err) => {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                500,
                "Internal Server Error",
                err.as_str(),
                None,
                None,
                Some(started_at.elapsed().as_millis() as u64),
            )
            .await?;
            return Ok(());
        }
    };
    let (mut prepared_request, response_adapter) =
        match prepare_gateway_request_with_default_service_tier(parsed, default_service_tier) {
            Ok(prepared) => prepared,
            Err(err) => {
                write_json_error_response(
                    &mut stream,
                    Some(&addr),
                    None,
                    400,
                    "Bad Request",
                    err.as_str(),
                    None,
                    None,
                    Some(started_at.elapsed().as_millis() as u64),
                )
                .await?;
                return Ok(());
            }
        };
    if let Err(err) = align_codex_prompt_cache(&mut prepared_request, &resolved_api_key) {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&prepared_request),
            400,
            "Bad Request",
            err.as_str(),
            None,
            None,
            Some(started_at.elapsed().as_millis() as u64),
        )
        .await?;
        return Ok(());
    }
    apply_codex_official_headers(&mut prepared_request);
    let stats_context =
        build_request_stats_context(&prepared_request, &response_adapter, &resolved_api_key);
    let stats_service_tier = service_tier_from_request_body(&prepared_request.body);
    let stats_reasoning_effort = reasoning_effort_from_request_body(&prepared_request.body);
    legacy_debug_log(
        collection.debug_logs,
        format!(
            "request_started addr={} method={} target={} request_kind={} model={} api_key_id={} api_key_label={}",
            addr,
            prepared_request.method,
            prepared_request.target,
            request_kind_log_label(stats_context.request_kind),
            stats_context.model_id,
            stats_context.api_key_id,
            stats_context.api_key_label
        ),
    );

    match proxy_request_with_account_pool(
        &prepared_request,
        &collection,
        &resolved_api_key,
        stats_context.request_kind,
    )
    .await
    {
        Ok(success) => {
            let ProxyDispatchSuccess {
                upstream,
                account_id,
                account_email,
            } = success;
            let timeouts = collection_timeouts(&collection);
            let response_capture = match write_gateway_response(
                &mut stream,
                upstream,
                response_adapter,
                collection.debug_logs,
                &prepared_request,
                started_at,
                &timeouts,
            )
            .await
            {
                Ok(response_capture) => response_capture,
                Err(err) => {
                    if !is_client_disconnect_error_message(&err) {
                        let latency_ms = started_at.elapsed().as_millis() as u64;
                        let error_category = legacy_stream_error_category(&err);
                        let status = if error_category == "upstream_stream_timeout" {
                            StatusCode::GATEWAY_TIMEOUT.as_u16()
                        } else {
                            StatusCode::BAD_GATEWAY.as_u16()
                        };
                        log_codex_api_failure(
                            Some(&addr),
                            Some(&prepared_request),
                            Some(status),
                            Some(account_id.as_str()),
                            Some(account_email.as_str()),
                            Some(latency_ms),
                            err.as_str(),
                        );
                        if let Err(stats_err) = record_request_stats_with_meta(
                            Some(account_id.as_str()),
                            Some(account_email.as_str()),
                            Some(stats_context.api_key_id.as_str()),
                            Some(stats_context.api_key_label.as_str()),
                            Some(stats_context.model_id.as_str()),
                            stats_context.request_kind,
                            false,
                            Some(error_category),
                            latency_ms,
                            None,
                            RequestStatsMeta {
                                service_tier: stats_service_tier.as_deref(),
                                reasoning_effort: stats_reasoning_effort.as_deref(),
                                ..RequestStatsMeta::default()
                            },
                        )
                        .await
                        {
                            logger::log_codex_api_warn(&format!(
                                "[CodexLocalAccess] 写入流式失败统计失败: {}",
                                stats_err
                            ));
                        }
                    }
                    return Err(err);
                }
            };
            let response_model_id = stats_model_id_from_response_capture(
                stats_context.model_id.as_str(),
                &response_capture,
            );
            if let Some(response_id) = response_capture.response_id.as_deref() {
                bind_response_affinity(response_id, &account_id).await;
            }
            if collection.session_affinity {
                let session_key = build_request_routing_hint(&prepared_request)
                    .session_affinity_key
                    .map(|key| session_affinity_binding_key(&key));
                if let Some(session_key) = session_key.as_deref() {
                    bind_response_affinity(session_key, &account_id).await;
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u64;
            if let Err(err) = record_request_stats_with_meta(
                Some(account_id.as_str()),
                Some(account_email.as_str()),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(response_model_id.as_str()),
                stats_context.request_kind,
                true,
                None,
                latency_ms,
                response_capture.usage,
                RequestStatsMeta {
                    service_tier: stats_service_tier.as_deref(),
                    reasoning_effort: stats_reasoning_effort.as_deref(),
                    ..RequestStatsMeta::default()
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入请求统计失败: {}",
                    err
                ));
            }
            legacy_debug_log(
                collection.debug_logs,
                format!(
                    "request_completed addr={} method={} target={} status=200 account_id={} account_email={} latency_ms={}",
                    addr,
                    prepared_request.method,
                    prepared_request.target,
                    account_id,
                    account_email,
                    latency_ms
                ),
            );
            Ok(())
        }
        Err(error) => {
            let ProxyDispatchError {
                status,
                message,
                account_id,
                account_email,
                error_category,
            } = error;
            let latency_ms = started_at.elapsed().as_millis() as u64;
            log_codex_api_failure(
                Some(&addr),
                Some(&prepared_request),
                Some(status),
                account_id.as_deref(),
                account_email.as_deref(),
                Some(latency_ms),
                message.as_str(),
            );
            let status_text = match status {
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                405 => "Method Not Allowed",
                429 => "Too Many Requests",
                502 => "Bad Gateway",
                422 => "Unprocessable Entity",
                _ => "Service Unavailable",
            };
            let proxy_diagnostics = (status == StatusCode::BAD_GATEWAY.as_u16()).then(|| {
                current_upstream_proxy_diagnostics(collection.upstream_proxy_url.as_deref())
            });
            let response = json_response(
                status,
                status_text,
                &gateway_error_body(status, &message, proxy_diagnostics.as_ref()),
            );
            let write_result = stream
                .write_all(&response)
                .await
                .map_err(|e| format!("写入错误响应失败: {}", e));
            if let Err(err) = record_request_stats_with_meta(
                account_id.as_deref(),
                account_email.as_deref(),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                false,
                error_category.as_deref(),
                latency_ms,
                None,
                RequestStatsMeta {
                    service_tier: stats_service_tier.as_deref(),
                    reasoning_effort: stats_reasoning_effort.as_deref(),
                    ..RequestStatsMeta::default()
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入失败统计失败: {}",
                    err
                ));
            }
            write_result
        }
    }
}

