#[path = "codex_local_access_native_grok.rs"]
mod native_grok;

// Native provider selection, credentials and model catalogs. No legacy wire executor.

fn native_is_chatgpt_plan_account(account: &CodexAccount) -> bool {
    account.api_provider_id.as_deref() == Some("cockpit_chatgpt_plan")
}

fn native_is_oauth_authorization_account(account: &CodexAccount) -> bool {
    // SIWC is independently authorized OAuth even though its service-only
    // reference uses the API-key account shape. Ordinary provider keys stay out.
    !account.is_api_key_auth() || native_is_chatgpt_plan_account(account)
}

fn native_default_codex_model(model: &str) -> bool {
    // The client catalog also contains third-party provider models. It is not
    // evidence that a ChatGPT credential can serve those models.
    // Reserve is an explicit official capability, not a provider-catalog model.
    native_is_reserve_model(model)
        || default_codex_model_ids()
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(model.trim()))
        || (is_gpt_or_codex_namespace_model(model)
            && api_service_routable_codex_model_ids()
                .iter()
                .chain(api_service_supported_codex_model_ids().iter())
                .any(|candidate| candidate.eq_ignore_ascii_case(model.trim())))
}

fn native_chatgpt_plan_model(account: &CodexAccount, model: &str) -> Option<String> {
    if account.api_model_catalog.is_empty() {
        // Cold catalog is not absent consent. Retain the existing public/key
        // model ACL and only route locally known Codex models until GET /models
        // supplies this grant's real catalog; never expose arbitrary models.
        return native_default_codex_model(model).then(|| model.trim().to_owned());
    }
    account
        .api_model_catalog
        .iter()
        .find(|candidate| candidate.trim().eq_ignore_ascii_case(model.trim()))
        .cloned()
}

#[derive(Clone)]
struct NativeModelRoutePlan {
    model: String,
    full_model: String,
    account_ids: Vec<String>,
    oauth_only: bool,
    provider: bool,
    automatic: bool,
    gateway: Option<CodexLocalAccessProviderGateway>,
    xai: bool,
}

fn native_gateway_models(gateway: &CodexLocalAccessProviderGateway) -> Vec<String> {
    let mut models = gateway.upstream_models.clone();
    if !gateway.upstream_model.trim().is_empty() && !models.contains(&gateway.upstream_model) {
        models.push(gateway.upstream_model.clone());
    }
    models.retain(|model| !model.trim().is_empty());
    models
}

fn native_gateway_model(
    gateway: &CodexLocalAccessProviderGateway,
    model: &str,
    shells: bool,
) -> Result<String, native_contract::ServiceError> {
    let models = native_gateway_models(gateway);
    if model.is_empty() {
        return Ok(String::new());
    }
    if models.iter().any(|candidate| candidate == model) {
        return Ok(model.to_owned());
    }
    if shells {
        if let Some(slot) = provider_gateway_model_slots(&models)
            .into_iter()
            .find(|slot| slot.client_model == model)
        {
            return Ok(slot.upstream_model);
        }
    }
    Err(native_error(
        404,
        "model_not_found",
        "The selected provider route does not expose this model",
    ))
}

fn native_model_route_plan(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    model: &str,
) -> Result<NativeModelRoutePlan, native_contract::ServiceError> {
    let mut plan = NativeModelRoutePlan {
        model: model.into(),
        full_model: model.into(),
        account_ids: native_scoped_account_ids(collection, key),
        oauth_only: false,
        provider: false,
        automatic: key
            .is_none_or(|key| key.model_routing.is_none() && key.provider_gateway.is_none()),
        gateway: None,
        xai: false,
    };
    let Some(key) = key else {
        return Ok(plan);
    };
    if let Some(routing) = &key.model_routing {
        if !matches!(routing.failure_policy.as_str(), "" | "strict" | "fail") {
            return Err(native_error(
                400,
                "unsupported_route_policy",
                "Unknown model routing failure policy",
            ));
        }
        let selected = if let Some((namespace, model)) = model.split_once('/') {
            if model.is_empty() {
                return Err(native_error(
                    404,
                    "model_not_found",
                    "Provider model name is empty",
                ));
            }
            let route = routing
                .routes
                .iter()
                .find(|route| route.namespace == namespace)
                .ok_or_else(|| {
                    native_error(404, "model_route_not_found", "Unknown model namespace")
                })?;
            Some((route, model))
        } else if !matches!(routing.default_route.as_str(), "" | "oauth") {
            let route = routing
                .routes
                .iter()
                .find(|route| {
                    route.id == routing.default_route || route.namespace == routing.default_route
                })
                .ok_or_else(|| {
                    native_error(
                        400,
                        "model_route_not_found",
                        "The default model route does not exist",
                    )
                })?;
            Some((route, model))
        } else {
            None
        };
        if let Some((route, model)) = selected {
            if route.provider_account_id.trim().is_empty()
                || route
                    .native_provider
                    .as_deref()
                    .is_some_and(|provider| !matches!(provider, "" | "xai"))
            {
                return Err(native_error(
                    400,
                    "invalid_provider_route",
                    "Provider route has no supported account binding",
                ));
            }
            plan.xai = route.native_provider.as_deref() == Some("xai");
            if !model.is_empty() {
                let base = if plan.xai {
                    native_api::grok_thinking::parse_model(model)?.0
                } else {
                    model.to_owned()
                };
                plan.model = native_gateway_model(&route.provider_gateway, &base, false)?;
            } else {
                plan.model.clear();
            }
            // The explicit route is its own grant. Never merge the default OAuth pool into it.
            plan.account_ids = vec![route.provider_account_id.clone()];
            plan.account_ids = active_service_account_ids(collection, plan.account_ids);
            plan.provider = true;
            plan.gateway = Some(route.provider_gateway.clone());
        } else {
            plan.oauth_only = true;
        }
    } else if let Some(gateway) = &key.provider_gateway {
        plan.model = native_gateway_model(gateway, model, true)?;
        plan.provider = true;
        plan.gateway = Some(gateway.clone());
    }
    Ok(plan)
}

#[cfg(test)]
mod native_provider_routes_tests {
    use super::*;

    fn collection() -> CodexLocalAccessCollection {
        serde_json::from_value(json!({"enabled":true,"port":1455,"apiKey":"fixture","accountIds":["oauth","other"],"createdAt":1,"updatedAt":1})).unwrap()
    }
    fn key() -> CodexLocalAccessApiKey {
        serde_json::from_value(json!({"id":"key","label":"test","key":"secret","inheritAccountPool":false,"accountIds":["oauth"],"modelRouting":{"defaultRoute":"oauth","failurePolicy":"strict","routes":[{"id":"provider-route","namespace":"vendor","providerAccountId":"provider","providerGateway":{"baseUrl":"https://example.invalid/v1","apiKey":"snapshot","upstreamModel":"model-a","upstreamModels":["model-a","org/model-b"],"wireApi":"chat_completions"}}]}})).unwrap()
    }
    fn provider() -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            "provider".into(),
            "test@example.invalid".into(),
            "live-token".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://example.invalid/v1".into()),
            Some("vendor".into()),
            Some("Vendor".into()),
            vec!["model-a".into()],
        );
        account.api_wire_api = Some("chat_completions".into());
        account
    }
    fn oauth() -> CodexAccount {
        CodexAccount::new(
            "oauth".into(),
            "oauth@example.invalid".into(),
            crate::models::codex::CodexTokens {
                id_token: "id".into(),
                access_token: "token".into(),
                refresh_token: None,
            },
        )
    }

    #[test]
    fn native_provider_routes_namespace_keeps_explicit_account_grant_separate() {
        let collection = collection();
        let key = key();
        let route = native_model_route_plan(&collection, Some(&key), "vendor/org/model-b").unwrap();
        assert_eq!(route.model, "org/model-b");
        assert_eq!(route.account_ids, vec!["provider"]);
        assert!(route.provider && !route.oauth_only);
        let plain = native_model_route_plan(&collection, Some(&key), "gpt-6-astra").unwrap();
        assert_eq!(plain.account_ids, vec!["oauth"]);
        assert!(plain.oauth_only);
        assert!(!native_model_visible(
            &collection,
            Some(&key),
            &plain,
            &provider()
        ));
    }

    #[test]
    fn native_provider_routes_strict_policy_rejects_unknown_namespaces_and_models() {
        let collection = collection();
        let mut key = key();
        for model in ["missing/model-a", "vendor/missing", "vendor/"] {
            assert!(
                native_model_route_plan(&collection, Some(&key), model).is_err(),
                "{model}"
            );
        }
        key.model_routing.as_mut().unwrap().failure_policy = "fall-through-anywhere".into();
        assert_eq!(
            native_model_route_plan(&collection, Some(&key), "vendor/model-a")
                .err()
                .unwrap()
                .code,
            "unsupported_route_policy"
        );
    }

    #[test]
    fn native_provider_routes_default_route_resolves_by_id_or_namespace() {
        let collection = collection();
        let mut key = key();
        for default in ["provider-route", "vendor"] {
            key.model_routing.as_mut().unwrap().default_route = default.into();
            let route = native_model_route_plan(&collection, Some(&key), "model-a").unwrap();
            assert_eq!(route.account_ids, vec!["provider"]);
        }
        key.model_routing.as_mut().unwrap().default_route = "missing".into();
        assert!(native_model_route_plan(&collection, Some(&key), "model-a").is_err());
    }

    #[test]
    fn native_provider_routes_aliases_and_account_denials_apply_after_namespace() {
        let mut collection = collection();
        let key = key();
        collection.model_aliases =
            serde_json::from_value(json!([{"alias":"public","sourceModel":"vendor/model-a"}]))
                .unwrap();
        let model = native_model_for_key(&collection, Some(&key), "public").unwrap();
        let plan = native_model_route_plan(&collection, Some(&key), &model).unwrap();
        assert!(native_model_visible(
            &collection,
            Some(&key),
            &plan,
            &provider()
        ));
        collection.excluded_models = vec!["model-a".into()];
        assert!(!native_model_visible(
            &collection,
            Some(&key),
            &plan,
            &provider()
        ));
    }

    #[test]
    fn native_provider_routes_catalog_preserves_scope_context_and_extra_models() {
        let collection = collection();
        let mut key = key();
        let mut account = provider();
        key.model_prefix = Some("team".into());
        account
            .api_model_context_windows
            .insert("model-a".into(), 123_000);
        account
            .api_model_vision_support
            .insert("model-a".into(), true);
        let catalog = native_catalog(&collection, Some(&key), &[account, oauth()]).unwrap();
        let data = catalog["data"].as_array().unwrap();
        assert!(data.iter().any(|item| item["id"] == "team/vendor/model-a"));
        assert!(data
            .iter()
            .any(|item| item["id"] == "team/vendor/org/model-b"));
        assert!(!data.iter().any(|item| item["id"] == "team/model-a"));
        let entry = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["slug"] == "team/vendor/model-a")
            .unwrap();
        assert_eq!(entry["context_window"], 123_000);
        assert_eq!(entry["auto_compact_token_limit"], 110_700);
        assert_eq!(entry["input_modalities"], json!(["text", "image"]));
        let missing = native_catalog(&collection, Some(&key), &[oauth()]).unwrap();
        assert!(!missing["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "team/vendor/model-a"));
    }

    #[test]
    fn native_provider_routes_plain_catalog_includes_account_and_public_aliases() {
        let mut collection = collection();
        collection.account_ids = vec!["provider".into()];
        collection.model_aliases =
            serde_json::from_value(json!([{"alias":"public","sourceModel":"mapped"}])).unwrap();
        let mut account = provider();
        account.api_model_catalog.push("mapped".into());
        account
            .api_model_mappings
            .push(crate::models::codex::CodexApiModelMapping {
                client_model: "mapped".into(),
                upstream_model: "model-a".into(),
            });
        let catalog = native_catalog(&collection, None, &[account]).unwrap();
        for id in ["public", "mapped", "model-a"] {
            assert!(catalog["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == id));
        }
    }

    #[test]
    fn native_provider_routes_owner_survives_refresh_but_not_destination_change() {
        let mut account = provider();
        let owner = native_account_owner(&account, "https://example.invalid/v1");
        account.token_generation += 1;
        assert_eq!(
            owner,
            native_account_owner(&account, "https://example.invalid/v1/")
        );
        account.openai_api_key = Some("different-principal-key".into());
        assert_ne!(
            owner,
            native_account_owner(&account, "https://example.invalid/v1")
        );
        assert_ne!(
            owner,
            native_account_owner(&account, "https://different.invalid/v1")
        );
        account.upstream_grok_account_id = Some("source".into());
        assert_ne!(
            owner,
            native_account_owner(&account, "https://example.invalid/v1")
        );
    }

    #[tokio::test]
    async fn native_provider_routes_credentials_use_current_account_without_network() {
        let account = provider();
        assert_eq!(
            native_wire_api(&account).unwrap(),
            native_contract::WireApi::Chat
        );
        let (_, headers, base, _) = native_account_credentials(account, false).await.unwrap();
        assert_eq!(headers[AUTHORIZATION], "Bearer live-token");
        assert_eq!(base, "https://example.invalid/v1");
        let mut web = oauth();
        web.token_source_mode = "chatgpt_web_session".into();
        assert!(!native_eligible(&web));
        assert_eq!(
            native_account_credentials(web, false)
                .await
                .err()
                .unwrap()
                .status,
            403
        );
    }
}

fn native_plan_account_model(
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
) -> Option<String> {
    // Explicit provider routes do not broaden this grant's catalog. A cold
    // catalog uses only the pre-existing local Codex model allowlist.
    if native_is_chatgpt_plan_account(account) {
        return native_chatgpt_plan_model(account, &plan.model);
    }
    if plan.provider {
        // An explicit provider catalog can contain additional models not yet in the account catalog.
        account.is_api_key_auth().then(|| plan.model.clone())
    } else {
        let model = if codex_account::is_grok_upstream_provider(account) {
            native_api::grok_thinking::parse_model(&plan.model).ok()?.0
        } else {
            plan.model.clone()
        };
        if plan.automatic {
            let slots = automatic_api_service_account_model_slots(account);
            if !slots.is_empty() {
                return slots
                    .into_iter()
                    .find(|(client, _)| client.trim().eq_ignore_ascii_case(model.trim()))
                    .map(|(_, upstream)| upstream);
            }
            if !account.is_api_key_auth()
                && !codex_account::is_grok_upstream_provider(account)
                && !native_default_codex_model(&model)
            {
                return None;
            }
            // Native/empty Responses pools accept the existing hidden routable models,
            // not names that are declared only by a different provider's route.
            let listed = native_is_reserve_model(&model)
                || api_service_routable_codex_model_ids()
                    .iter()
                    .chain(api_service_supported_codex_model_ids().iter())
                    .any(|candidate| candidate.eq_ignore_ascii_case(&model));
            if !listed {
                return None;
            }
        }
        native_account_model(account, &model)
    }
}

include!("codex_local_access_native_automatic.rs");

fn native_wire_api(
    account: &CodexAccount,
) -> Result<native_contract::WireApi, native_contract::ServiceError> {
    if native_is_chatgpt_plan_account(account)
        || !account.is_api_key_auth()
        || codex_account::is_grok_upstream_provider(account)
    {
        return Ok(native_contract::WireApi::Responses);
    }
    match account.api_wire_api.as_deref().unwrap_or("responses") {
        "responses" => Ok(native_contract::WireApi::Responses),
        "chat_completions" => Ok(native_contract::WireApi::Chat),
        _ => Err(native_error(
            400,
            "unsupported_wire_api",
            "Unsupported provider protocol",
        )),
    }
}

fn native_account_owner(account: &CodexAccount, base_url: &str) -> String {
    if let Some(registration) = account.official_registration_id.as_deref() {
        // Preserve the existing grant owner namespace, never the source Codex
        // workspace owner. The logical account ID is recorded separately.
        let mut identity = account.clone();
        identity.official_registration_id = None;
        identity.id = format!(
            "{}{registration}",
            codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX
        );
        identity.user_id = None;
        identity.account_id = None;
        return native_account_owner(&identity, base_url);
    }
    // Managed OAuth refreshes keep a principal/workspace. An arbitrary API key
    // replacement has no such identity proof, even when the local record is the
    // same, so it must not reuse the old key's upstream continuation state.
    // Keep only a digest; credentials must never enter the identity database.
    let credential_scope =
        if account.is_api_key_auth() && !codex_account::is_grok_upstream_provider(account) {
            format!(
                "{:x}",
                Sha256::digest(account.openai_api_key.as_deref().unwrap_or("").as_bytes())
            )
        } else {
            String::new()
        };
    let agent_scope = account.agent_identity.as_ref().map(|identity| {
        (
            identity.agent_runtime_id.as_str(),
            identity.task_id.as_deref(),
            identity.account_id.as_str(),
            identity.chatgpt_user_id.as_str(),
        )
    });
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{agent_scope:?}",
        account.id,
        if account.is_agent_identity_auth() {
            "agent"
        } else if account.is_api_key_auth() {
            "api"
        } else {
            "oauth"
        },
        account.user_id.as_deref().unwrap_or(""),
        account.account_id.as_deref().unwrap_or(""),
        account.upstream_grok_account_id.as_deref().unwrap_or(""),
        base_url.trim_end_matches('/'),
        credential_scope
    )
}

async fn native_account_credentials(
    account: CodexAccount,
    force: bool,
) -> Result<
    (CodexAccount, axum::http::HeaderMap, String, Option<String>),
    native_contract::ServiceError,
> {
    if account.is_web_session_auth() {
        return Err(native_error(
            403,
            "unsupported_account_kind",
            "Web session accounts only support quota inspection",
        ));
    }
    if native_is_chatgpt_plan_account(&account) {
        let registration_id = account
            .official_registration_id
            .as_deref()
            .or_else(|| account.id.strip_prefix("chatgpt_plan_"))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                native_error(
                    401,
                    "chatgpt_plan_reauthorization_required",
                    "ChatGPT plan authorization has an invalid registration",
                )
            })?;
        // The independent credential store already moves local crypto/I/O to
        // blocking workers. Await its single-flight renewal without occupying
        // the shared Codex credential worker semaphore or a blocking thread.
        let (token, authorization_revision) = timeout(Duration::from_secs(60), async {
            if account.official_registration_id.is_some() {
                crate::modules::chatgpt_plan::inference_snapshot_for_source(
                    registration_id,
                    &account.id,
                    force,
                )
                .await
            } else {
                crate::modules::chatgpt_plan::inference_snapshot(registration_id, force).await
            }
        })
        .await
        .map_err(|_| native_credential_error("CHATGPT_PLAN_TIMEOUT"))?
        .map_err(|error| native_credential_error(&error))?;
        let mut headers = axum::http::HeaderMap::new();
        native_insert_bearer(&mut headers, &token)
            .map_err(|_| native_credential_error("CHATGPT_PLAN_REAUTH_REQUIRED"))?;
        return Ok((
            account,
            headers,
            "https://api.openai.com/v1".to_string(),
            Some(authorization_revision),
        ));
    }
    let permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            native_error(
                503,
                "service_unavailable",
                "Credential worker is unavailable",
            )
        })?;
    let runtime = tokio::runtime::Handle::current();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.block_on(async move {
            let mut account = account;
            let mut headers = axum::http::HeaderMap::new();
            let base_url = if codex_account::is_grok_upstream_provider(&account) {
                let id = account.upstream_grok_account_id.as_deref().unwrap_or("");
                let current = crate::modules::grok_account::load_account(id)
                    .ok_or_else(|| "Grok account is unavailable".to_string())?;
                if current.is_api_key_auth() {
                    return Err("Grok source must be an OAuth account".to_string());
                }
                let current = if force {
                    crate::modules::grok_account::force_refresh_account(id).await?;
                    crate::modules::grok_account::load_account(id)
                        .ok_or_else(|| "Grok account is unavailable".to_string())?
                } else if current.access_token.is_empty()
                    || current
                        .expires_at
                        .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp() + 30)
                {
                    crate::modules::grok_account::prepare_account_for_injection(id).await?
                } else {
                    current
                };
                headers = native_grok::credential_headers(
                    &current.access_token,
                    &native_grok::cached_client_version(),
                )?;
                native_grok::bind_owner(&mut account, &current);
                codex_account::GROK_CLI_CHAT_PROXY_BASE_URL.to_string()
            } else if account.is_agent_identity_auth() {
                let expected = if force {
                    account
                        .agent_identity
                        .as_ref()
                        .and_then(|identity| identity.task_id.as_deref())
                } else {
                    None
                };
                let (updated, auth, _) =
                    codex_agent_identity::build_authentication_headers_with_base_url(
                        &account,
                        expected,
                        codex_agent_identity::AGENT_IDENTITY_AUTH_API_BASE_URL,
                    )
                    .await?;
                account = updated;
                headers = auth;
                "https://chatgpt.com/backend-api/codex".to_string()
            } else if account.is_api_key_auth() {
                native_insert_bearer(
                    &mut headers,
                    account.openai_api_key.as_deref().unwrap_or(""),
                )?;
                account
                    .api_base_url
                    .clone()
                    .filter(|url| !url.trim().is_empty())
                    .unwrap_or_else(|| "https://api.openai.com/v1".into())
            } else {
                native_insert_bearer(&mut headers, &account.tokens.access_token)?;
                "https://chatgpt.com/backend-api/codex".to_string()
            };
            if !account.is_api_key_auth() {
                if let Some(id) = account.account_id.as_deref().filter(|id| !id.is_empty()) {
                    headers.insert(
                        "chatgpt-account-id",
                        HeaderValue::from_str(id).map_err(|_| "Invalid account identity")?,
                    );
                }
                if account
                    .agent_identity
                    .as_ref()
                    .is_some_and(|identity| identity.chatgpt_account_is_fedramp)
                {
                    headers.insert("x-openai-fedramp", HeaderValue::from_static("true"));
                }
            }
            Ok::<_, String>((account, headers, base_url, None))
        })
    });
    timeout(Duration::from_secs(60), task)
        .await
        .map_err(|_| {
            native_error(
                504,
                "credential_refresh_timeout",
                "Credential preparation timed out",
            )
        })?
        .map_err(|_| native_error(500, "credential_refresh_failed", "Credential worker failed"))?
        .map_err(|error| native_credential_error(&error))
}

fn native_credential_error(error: &str) -> native_contract::ServiceError {
    match error {
        "CHATGPT_PLAN_REAUTH_REQUIRED" | "CHATGPT_PLAN_DISCONNECTED" | "CHATGPT_PLAN_NOT_FOUND" => {
            native_error(
                401,
                "chatgpt_plan_reauthorization_required",
                "ChatGPT plan authorization is unavailable; sign in again",
            )
            .with_credential_scope(true)
        }
        "CHATGPT_PLAN_PERMISSION_REQUIRED" => native_error(
            403,
            "chatgpt_plan_permission_required",
            "ChatGPT plan usage permission is required",
        )
        .with_credential_scope(true),
        "CHATGPT_PLAN_NOT_VERIFIED" => native_error(
            403,
            "chatgpt_plan_verification_required",
            "Verify the ChatGPT plan authorization before using it in the API service",
        ),
        "CHATGPT_PLAN_TIMEOUT" => native_error(
            504,
            "chatgpt_plan_temporarily_unavailable",
            "ChatGPT plan credential renewal timed out; retry later",
        ),
        error if error.starts_with("CHATGPT_PLAN_") => native_error(
            503,
            "chatgpt_plan_temporarily_unavailable",
            "ChatGPT plan credentials are temporarily unavailable; retry later",
        ),
        _ => native_error(
            401,
            "credentials_unavailable",
            "Unable to prepare account credentials; check authorization",
        ),
    }
}

fn native_insert_bearer(headers: &mut axum::http::HeaderMap, token: &str) -> Result<(), String> {
    if token.trim().is_empty() {
        return Err("Account token is empty".into());
    }
    let mut value =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "Invalid account token")?;
    value.set_sensitive(true);
    headers.insert(AUTHORIZATION, value);
    Ok(())
}

fn native_model_visible(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
) -> bool {
    plan.account_ids.contains(&account.id)
        && native_eligible(account)
        && (!plan.oauth_only || native_is_oauth_authorization_account(account))
        && !native_xai_base_denied(collection, key, plan, account)
        && native_plan_account_model(plan, account).is_some_and(|upstream| {
            !native_excluded(&collection.excluded_models, &upstream)
                && !key.is_some_and(|key| native_excluded(&key.excluded_models, &upstream))
                && !collection.account_model_rules.iter().any(|rule| {
                    rule.account_id == account.id
                        && (native_excluded(&rule.excluded_models, &plan.model)
                            || native_excluded(&rule.excluded_models, &upstream))
                })
        })
}

include!("codex_local_access_native_catalog_tools.rs");

fn native_catalog(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    accounts: &[CodexAccount],
) -> Result<Value, native_contract::ServiceError> {
    let mut candidates = Vec::new();
    let automatic =
        key.is_none_or(|key| key.provider_gateway.is_none() && key.model_routing.is_none());
    if let Some(gateway) = key.and_then(|key| key.provider_gateway.as_ref()) {
        let models = native_gateway_models(gateway);
        candidates.extend(models.iter().cloned());
        candidates.extend(
            provider_gateway_model_slots(&models)
                .into_iter()
                .map(|slot| slot.client_model),
        );
    } else if automatic {
        let scope = native_scoped_account_ids(collection, key);
        let scoped = accounts
            .iter()
            .filter(|account| scope.contains(&account.id) && native_eligible(account))
            .cloned()
            .collect::<Vec<_>>();
        let mut fallback = api_service_supported_codex_model_ids();
        fallback.push(CODEX_GPT_RESERVE_MODEL_ID.into());
        candidates.extend(automatic_api_service_pool_model_ids(&scoped, fallback));
        // Server-provided plan models are already filtered for visibility.
        // Do not reduce them to the older Codex client's recommended set.
        for account in scoped
            .iter()
            .filter(|account| native_is_chatgpt_plan_account(account))
        {
            candidates.extend(account.api_model_catalog.iter().cloned());
        }
    } else {
        for account in accounts {
            if account.is_api_key_auth() {
                if key.is_some_and(|key| key.model_routing.is_some())
                    && !native_is_chatgpt_plan_account(account)
                {
                    continue;
                }
                candidates.extend(account.api_model_catalog.iter().cloned());
                if native_is_chatgpt_plan_account(account) && account.api_model_catalog.is_empty() {
                    candidates.extend(api_service_supported_codex_model_ids());
                }
                candidates.extend(
                    account
                        .api_model_mappings
                        .iter()
                        .map(|mapping| mapping.client_model.clone()),
                );
            } else {
                candidates.extend(codex_protocol::managed_codex_model_ids());
            }
        }
    }
    if !automatic && key.is_none_or(|key| key.provider_gateway.is_none()) {
        candidates.push("gpt-reserve".into());
    }
    if let Some(routing) = key.and_then(|key| key.model_routing.as_ref()) {
        for route in &routing.routes {
            candidates.extend(
                native_gateway_models(&route.provider_gateway)
                    .into_iter()
                    .map(|model| format!("{}/{model}", route.namespace)),
            );
        }
    }
    candidates.extend(
        collection
            .model_aliases
            .iter()
            .map(|alias| alias.alias.clone()),
    );
    let mut ids = Vec::new();
    let mut selected = Vec::new();
    for candidate in candidates {
        let exposed = key
            .and_then(|key| key.model_prefix.as_deref())
            .filter(|prefix| !prefix.is_empty())
            .map(|prefix| format!("{}/{candidate}", prefix.trim_end_matches('/')))
            .unwrap_or(candidate);
        if ids.contains(&exposed) {
            continue;
        }
        let Ok(model) = native_model_for_key(collection, key, &exposed) else {
            continue;
        };
        let Ok(plan) = native_model_route_plan(collection, key, &model) else {
            continue;
        };
        if let Some(account) = accounts.iter().find(|account| {
            (!plan.provider || account.official_registration_id.is_none())
                && (plan.provider
                    || !accounts.iter().any(|other| {
                        other.id == account.id && other.official_registration_id.is_some()
                    })
                    || account.official_registration_id.is_some())
                && native_model_visible(collection, key, &plan, account)
        }) {
            ids.push(exposed.clone());
            selected.push((
                exposed,
                native_plan_account_model(&plan, account).unwrap_or_default(),
                account,
                plan,
            ));
        }
    }
    let mut value = codex_protocol::build_codex_client_models_response(&ids);
    if let Some(models) = value["models"].as_array_mut() {
        for model in models {
            if let Some((_, upstream, account, plan)) = selected
                .iter()
                .find(|(id, _, _, _)| model["slug"].as_str() == Some(id.as_str()))
            {
                native_apply_catalog_tool_capabilities(model, collection, key, plan, accounts);
                model["prefer_websockets"] = json!(native_catalog_prefers_websockets(
                    collection, key, plan, account
                ));
                if let Some(window) = account
                    .api_model_context_windows
                    .get(upstream)
                    .filter(|window| **window > 0)
                {
                    model["context_window"] = json!(window);
                    model["max_context_window"] = json!(window);
                    model["auto_compact_token_limit"] =
                        json!(codex_protocol::derived_auto_compact_token_limit(*window));
                }
                if account.is_api_key_auth() {
                    let vision = native_catalog_vision(collection, plan, account, upstream);
                    model["input_modalities"] = if vision {
                        json!(["text", "image"])
                    } else {
                        json!(["text"])
                    };
                    if automatic {
                        let display = codex_account::provider_model_display_name(&plan.model);
                        if display.trim() != plan.model.trim() {
                            model["display_name"] = json!(display);
                        }
                        if let Some(template) = account_model_template(account, &plan.model)
                            .or_else(|| account_model_template(account, upstream))
                        {
                            for field in ["supported_reasoning_levels", "default_reasoning_level"] {
                                if let Some(value) = template.get(field) {
                                    model[field] = value.clone();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    value["object"] = json!("list");
    value["data"] = json!(selected
        .iter()
        .map(|(id, upstream, account, plan)| native_public_model_entry(
            collection, plan, account, id, upstream
        ))
        .collect::<Vec<_>>());
    Ok(value)
}

async fn native_models(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
) -> Result<Value, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller)?;
    let mut ids = native_scoped_account_ids(&collection, key);
    if let Some(routing) = key.and_then(|key| key.model_routing.as_ref()) {
        ids.extend(
            routing
                .routes
                .iter()
                .map(|route| route.provider_account_id.clone()),
        );
    }
    ids.extend(collection.image_generation_account_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    let raw_accounts = host.load_accounts(ids.clone()).await?;
    let mut accounts = host
        .load_routing_accounts(
            ids,
            caller.id != NATIVE_INTERNAL_CALLER
                && key.is_none_or(|key| key.provider_gateway.is_none()),
        )
        .await?;
    if key.is_some_and(|key| key.model_routing.is_some()) {
        accounts.extend(
            raw_accounts
                .iter()
                .filter(|account| {
                    collection
                        .official_account_bindings
                        .contains_key(&account.id)
                })
                .cloned(),
        );
    }
    let mut catalog = native_catalog(&collection, key, &accounts)?;
    native_append_image_models(
        &mut catalog,
        &collection,
        key,
        if collection.image_generation_account_ids.is_empty() {
            &accounts
        } else {
            &raw_accounts
        },
    );
    Ok(catalog)
}

// Some(ZERO) is immediately eligible; Some(delay) is a known quota reset;
// None means quota policy cannot predict when this account becomes eligible.
// In particular, reservation/entitlement denials are not reset-time promises.
fn native_model_quota_retry_wait(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    model: &str,
) -> Option<Duration> {
    if native_model_quota_allows(collection, account, model) {
        return Some(Duration::ZERO);
    }
    if (native_is_reserve_model(model) && !account.is_api_key_auth())
        || !native_bound_quota_reserve_allows(collection, account)
    {
        return None;
    }
    let now = now_ms();
    let snapshot = account_quota_cooldown(account, now)?;
    if !snapshot.active(now) {
        // A reset may have elapsed between the policy check and this snapshot.
        // Recheck the original policy instead of inventing a zero-delay retry.
        return native_model_quota_allows(collection, account, model)
            .then_some(Duration::ZERO);
    }
    snapshot
        .reset_at_ms
        .map(|reset| Duration::from_millis(reset.saturating_sub(now).max(0) as u64))
}

async fn native_select_route(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: native_contract::RouteRequest,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    // Route preparation contains credential, proxy and installation futures.
    // Keep its state on the heap when nested inside an HTTP cooldown retry;
    // otherwise repeated polling/moves approach the default thread stack limit.
    let first = Box::pin(native_select_route_once(host, caller, request.clone())).await;
    let Err(error) = &first else {
        return first;
    };
    let max_wait = Duration::from_millis(host.collection().await?.max_retry_interval_ms);
    if let Some(wait) = error
        .retry_after
        .filter(|wait| !wait.is_zero() && *wait <= max_wait)
    {
        if matches!(error.code, "no_available_account" | "account_cooling") {
            tokio::time::sleep(wait).await;
            return Box::pin(native_select_route_once(host, caller, request)).await;
        }
    }
    first
}

async fn native_select_route_once(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    mut request: native_contract::RouteRequest,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller)?;
    let public_model = native_model_for_key(&collection, key, &request.model)?;
    let plan = native_model_route_plan(&collection, key, &public_model)?;
    // A continuation pins the credential namespace, never an account's currently
    // selected mode. Only the current explicit binding can translate that pin.
    if let Some(grant) = request
        .preferred_account_id
        .as_deref()
        .and_then(|id| id.strip_prefix(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX))
    {
        if !plan
            .account_ids
            .iter()
            .any(|id| Some(id) == request.preferred_account_id.as_ref())
        {
            let matches = collection
                .official_account_bindings
                .iter()
                .filter(|(source, id)| id.as_str() == grant && plan.account_ids.contains(source))
                .collect::<Vec<_>>();
            if caller.id == NATIVE_INTERNAL_CALLER || matches.len() != 1 {
                return Err(native_error(
                    409,
                    "account_binding_unavailable",
                    "The original authorization is no longer selected",
                ));
            }
            let (source, _) = matches[0];
            let association_valid = if !plan.provider
                && native_official_fallback_operation(host, caller, &request)
            {
                // A paused/expired official credential does not revoke the
                // independently valid source association. Mapping still owns
                // the mandatory safe-history rebuild before using local OAuth.
                crate::modules::chatgpt_plan::official_route_snapshot(grant, source, &plan.model)
                    .await
                    .map(|_| ())
            } else {
                crate::modules::chatgpt_plan::validate_source_binding(grant, source)
                    .await
                    .map(|_| ())
            };
            association_valid.map_err(|_| {
                native_error(
                    409,
                    "account_binding_unavailable",
                    "The original authorization binding is unavailable",
                )
            })?;
            request.preferred_account_id = Some(source.clone());
        }
    }
    if caller.id != NATIVE_INTERNAL_CALLER
        && request
            .preferred_account_id
            .as_deref()
            .is_some_and(|id| service_account_paused(&collection, id))
    {
        return Err(native_error(
            503,
            "service_member_paused",
            "This API service member is paused",
        ));
    }
    let model = plan.model.clone();
    let ids = if caller.id == NATIVE_INTERNAL_CALLER {
        let target = request.target_account_id.as_ref().ok_or_else(|| {
            native_error(
                400,
                "missing_account",
                "Internal request requires a target account",
            )
        })?;
        if !internal_api_account_ids().contains(target) {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account is not registered",
            ));
        }
        vec![target.clone()]
    } else {
        if request.target_account_id.is_some() {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account header is reserved for internal callers",
            ));
        }
        plan.account_ids.clone()
    };
    let _selection_guard = native_acquire_selection_guard(
        host,
        caller,
        &request,
        native_selection_wait_budget(
            &collection,
            native_api::execution::remaining_prepare_budget(),
        ),
    )
    .await?;
    let mut accounts = host
        .load_routing_accounts(ids, caller.id != NATIVE_INTERNAL_CALLER && !plan.provider)
        .await?;
    let scope_fingerprint = native_official_scope_fingerprint(&collection, caller);
    let mut local_fallbacks = native_official_preflight_fallbacks(
        host,
        caller,
        &collection,
        key,
        &plan,
        &request,
        &mut accounts,
    )
    .await;
    if request
        .auxiliary
        .as_deref()
        .is_some_and(|operation| operation != "responses.websocket")
        && !accounts.is_empty()
        && accounts.iter().all(native_is_chatgpt_plan_account)
    {
        return Err(native_chatgpt_plan_auxiliary_error(
            request
                .auxiliary
                .as_deref()
                .expect("auxiliary operation checked"),
        ));
    }
    accounts.retain(|account| {
        native_eligible(account)
            && native_auxiliary_eligible(account, request.auxiliary.as_deref())
            && !request.excluded_account_ids.contains(&account.id)
            && !account.official_registration_id.as_ref().is_some_and(|id| {
                request.excluded_account_ids.contains(&format!(
                    "{}{id}",
                    codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX
                ))
            })
            && (!plan.oauth_only || native_is_oauth_authorization_account(account))
            && !native_xai_base_denied(&collection, key, &plan, account)
            && !(collection.restrict_free_accounts
                && account.plan_type.as_deref() == Some("free")
                && (!account.is_api_key_auth() || native_is_chatgpt_plan_account(account))
                && !account.is_agent_identity_auth())
            && native_operation_model(
                &collection,
                &plan,
                account,
                request.auxiliary.as_deref(),
                request.image_input,
            )
            .is_some_and(|upstream| {
                !native_excluded(&collection.excluded_models, &upstream)
                    && !key.is_some_and(|key| native_excluded(&key.excluded_models, &upstream))
                    && !collection.account_model_rules.iter().any(|rule| {
                        rule.account_id == account.id
                            && (native_excluded(&rule.excluded_models, &model)
                                || native_excluded(&rule.excluded_models, &plan.model)
                                || native_excluded(&rule.excluded_models, &upstream))
                    })
            })
    });
    native_filter_capability_backups(&collection, &plan, &request, &mut accounts)?;
    // Reservation policy has its own coordinator lock. Read quota readiness
    // before acquiring the cooldown lock, then combine the two wait barriers.
    let quota_candidates: HashMap<_, _> = accounts
        .iter()
        .filter_map(|account| {
            let upstream = native_operation_model(
                &collection,
                &plan,
                account,
                request.auxiliary.as_deref(),
                request.image_input,
            )?;
            let wait = native_model_quota_retry_wait(&collection, account, &upstream)?;
            Some((account.id.clone(), (upstream, wait)))
        })
        .collect();
    let mut closest_cooldown = None;
    {
        let cooldowns = host.cooldowns.lock().map_err(|_| {
            native_error(503, "service_unavailable", "Account state is unavailable")
        })?;
        accounts.retain(|account| {
            let Some((upstream, quota_wait)) = quota_candidates.get(&account.id) else {
                return false;
            };
            let memory_wait = if native_is_chatgpt_plan_account(account) {
                Duration::ZERO
            } else {
                native_cooldown_wait(
                    &cooldowns,
                    &account.id,
                    upstream,
                    collection.disable_cooling,
                    Instant::now(),
                )
                .unwrap_or(Duration::ZERO)
            };
            // Both barriers must expire. Never shorten a real quota window by
            // using an unrelated transient cooldown (or the disable switch).
            let wait = (*quota_wait).max(memory_wait);
            if request
                .preferred_account_id
                .as_ref()
                .is_none_or(|id| id == &account.id)
            {
                if !wait.is_zero() {
                    closest_cooldown =
                        Some(closest_cooldown.map_or(wait, |current: Duration| current.min(wait)));
                }
            }
            wait.is_zero()
        });
    }
    // A known continuation owner can be temporarily cooling even while another
    // account is ready. Keep its ownership pin, but expose a retryable failure
    // instead of treating that temporary absence as a broken binding.
    if request.preferred_account_id.is_some() && closest_cooldown.is_some() {
        return Err(native_error(
            503,
            "account_cooling",
            "The original account is temporarily cooling; retry this conversation after the indicated delay",
        )
        .with_retry_after(closest_cooldown));
    }
    if accounts.is_empty() {
        if request.capability_fallback_from.is_some() {
            return Err(native_error(
                503,
                "capability_fallback_no_compatible_account",
                "No compatible account is available within this API key's model scope",
            )
            .with_retry_after(closest_cooldown));
        }
        return Err(native_error(
            503,
            "no_available_account",
            "No compatible account is available for this model",
        )
        .with_retry_after(closest_cooldown));
    }
    let routing_namespace = native_routing_namespace(
        caller,
        &request.model,
        &plan.account_ids,
        request.target_account_id.as_deref(),
    );
    native_sort_accounts_scoped(host, &mut accounts, &collection, key, &routing_namespace);
    if plan.automatic {
        // The old automatic gateway exhausts native candidates before provider routes.
        // Preserve the configured selection order within each group.
        accounts.sort_by_key(|account| {
            automatic_api_service_provider_route_eligible(
                account,
                &native_automatic_upstream_models(&collection, account),
            ) && !codex_account::is_grok_upstream_provider(account)
        });
    }
    if request.capability_fallback_from.is_some() {
        accounts.sort_by_key(|account| {
            collection
                .capability_fallback
                .account_ids
                .iter()
                .position(|id| id == &account.id)
                .unwrap_or(usize::MAX)
        });
    }
    if host.collection_override.is_some() {
        native_contract::Host::set_session_affinity_enabled(host, collection.session_affinity);
    }
    let binding_epoch = host.binding_epoch.load(Ordering::Acquire);
    let (local_ids, parent_id) = native_soft_binding_ids(
        request.session_id.as_deref(),
        request.thread_id.as_deref(),
        request.parent_thread_id.as_deref(),
    );
    let binding_keys: Vec<String> = if collection.session_affinity {
        local_ids
            .into_iter()
            .map(|id| native_soft_binding_key(&caller.id, &request.model, id))
            .collect()
    } else {
        vec![]
    };
    let parent_key = collection
        .session_affinity
        .then(|| parent_id.map(|id| native_soft_binding_key(&caller.id, &request.model, id)))
        .flatten();
    let mut bindings = host.bindings.lock().await;
    native_prune_soft_bindings(
        host,
        &mut bindings,
        binding_epoch,
        collection.session_affinity,
        collection.session_affinity_ttl_ms.max(60_000),
    );
    let preferred = request.preferred_account_id.clone().or_else(|| {
        collection
            .session_affinity
            .then(|| {
                binding_keys
                    .iter()
                    .chain(parent_key.iter())
                    .find_map(|key| bindings.get(key).map(|(id, _)| id.clone()))
            })
            .flatten()
    });
    if let Some(preferred) = preferred.as_ref() {
        if let Some(position) = accounts.iter().position(|account| &account.id == preferred) {
            // A hard opaque/recovery pin overrides scheduling. A soft cached
            // account does not bypass the preferred/regular/backup tier policy.
            if request.preferred_account_id.is_some()
                || native_account_rank(&accounts[position], &collection, key).0
                    == native_account_rank(&accounts[0], &collection, key).0
            {
                accounts.swap(0, position);
            }
        } else if request.preferred_account_id.is_some() {
            return Err(native_error(
                409,
                "account_binding_unavailable",
                "The connection's account is no longer available; reconnect with full input",
            )
            .with_retry_after(closest_cooldown));
        }
    }
    let affinity_selected = preferred
        .as_ref()
        .is_some_and(|id| accounts.first().is_some_and(|account| &account.id == id));
    if affinity_selected {
        accounts.truncate(1);
    }
    drop(bindings);
    let (position, permit) =
        native_acquire_account_permit(host, &collection, &accounts, request.requires_permit)
            .await?;
    let account = accounts.swap_remove(position);
    let upstream_model = native_operation_model(
        &collection,
        &plan,
        &account,
        request.auxiliary.as_deref(),
        request.image_input,
    )
    .ok_or_else(|| native_error(403, "model_not_allowed", "Selected model is unavailable"))?;
    let preflight_fallback = local_fallbacks.remove(&account.id);
    let selected_official = account.official_registration_id.clone();
    let source_id = account.id.clone();
    // Capture the grant before credential refresh or any other preparation await.
    // Re-reading a new grant after an old preparation failed would bless stale work.
    let official_snapshot = if native_official_fallback_operation(host, caller, &request) {
        match selected_official.as_deref() {
            Some(id) if !source_id.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX) => Some(
                crate::modules::chatgpt_plan::official_route_snapshot(
                    id,
                    &source_id,
                    &upstream_model,
                )
                .await
                .map_err(|_| {
                    native_error(
                        409,
                        "official_fallback_authorization_changed",
                        "Selected official authorization changed before preparation",
                    )
                })?,
            ),
            _ => None,
        }
    } else {
        None
    };
    let route_result = native_route_for_reserved_account(
        host,
        &collection,
        account,
        upstream_model,
        request.force_refresh,
        permit,
        matches!(
            request.auxiliary.as_deref(),
            Some("responses.websocket" | "realtime")
        ),
    )
    .await;
    let mut route = match route_result {
        Ok(route) => route,
        Err(error)
            if official_snapshot.is_some() && native_official_prepare_recoverable(&error) =>
        {
            let snapshot = official_snapshot.expect("official snapshot checked");
            let failure = native_contract::OfficialFallbackFailure {
                status: error.status,
                code: error.code.to_string(),
                message: Some(error.message.clone()),
                retry_after: error.retry_after,
                trigger_stage: "preflight",
            };
            native_prepare_official_local_fallback(
                host,
                caller,
                request.clone(),
                NativeOfficialLocalIntent { snapshot, failure },
                &scope_fingerprint,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    if let Some(intent) = preflight_fallback {
        route = native_attach_official_fallback(host, caller, route, intent, &scope_fingerprint)
            .await?;
    } else if route.chatgpt_plan {
        route.authorization_scope_fingerprint = Some(scope_fingerprint);
    }
    native_commit_routing_cursor(host, &routing_namespace, &route.account_id);
    // A settings transition invalidates old in-flight preparations too. Soft
    // state has only TTL retention; capacity never blocks new conversations.
    if collection.session_affinity && host.binding_epoch.load(Ordering::Acquire) == binding_epoch {
        let mut bindings = host.bindings.lock().await;
        if host.binding_epoch.load(Ordering::Acquire) == binding_epoch {
            native_publish_soft_bindings(
                host,
                &mut bindings,
                binding_keys,
                &route.account_id,
                now_ms(),
            );
        }
    }
    Ok(route)
}

async fn native_acquire_selection_guard(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: &native_contract::RouteRequest,
    wait: Duration,
) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, native_contract::ServiceError> {
    let Some(identity) = request
        .session_id
        .as_ref()
        .or(request.parent_thread_id.as_ref())
        .or(request.thread_id.as_ref())
    else {
        return Ok(None);
    };
    let key = format!("{}\0{}", caller.id, identity);
    let gate = {
        let mut gates = host.selection_gates.lock().map_err(|_| {
            native_error(
                503,
                "service_unavailable",
                "Session selector is unavailable",
            )
        })?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(&key).and_then(std::sync::Weak::upgrade) {
            gate
        } else {
            if gates.len() >= NATIVE_SELECTION_GATE_LIMIT {
                return Err(native_error(
                    503,
                    "session_capacity_reached",
                    "Session selector capacity reached",
                ));
            }
            let gate = Arc::new(TokioMutex::new(()));
            gates.insert(key, Arc::downgrade(&gate));
            gate
        }
    };
    // Serialise only preparation of the same logical session. The global map
    // lock is released before any wait, IO, credential refresh, or stream work.
    timeout(wait, gate.lock_owned())
        .await
        .map(Some)
        .map_err(|_| {
            native_error(
                503,
                "session_selection_busy",
                "Session account selection timed out",
            )
        })
}

fn native_selection_wait_budget(
    collection: &CodexLocalAccessCollection,
    remaining_prepare: Option<Duration>,
) -> Duration {
    remaining_prepare.unwrap_or_else(|| {
        // Direct Host callers have no request scope. Preserve both persisted
        // budgets within the same host validation bounds rather than applying
        // a shorter body-read cap to account preparation.
        Duration::from_millis(
            collection
                .timeouts
                .legacy_request_read_timeout_ms
                .min(LOCAL_ACCESS_TIMEOUT_MAX_MS),
        )
        .max(native_account_concurrency_wait(collection))
    })
}

fn native_account_concurrency_wait(collection: &CodexLocalAccessCollection) -> Duration {
    // Match the host's persisted setting range; the execution deadline may stop
    // preparation sooner, but this layer must not silently replace 120s with 60s.
    Duration::from_millis(
        collection
            .account_concurrency_wait_ms
            .min(ACCOUNT_CONCURRENCY_WAIT_MAX_MS),
    )
}

// Keep the additional admission state on the heap. This function sits inside
// deeply nested HTTP/WS retry futures, including the unprotected fast path.
fn native_acquire_account_permit<'a>(
    host: &'a NativeHostBridge,
    collection: &'a CodexLocalAccessCollection,
    accounts: &'a [CodexAccount],
    reserve_permit: bool,
) -> futures_util::future::BoxFuture<
    'a,
    Result<(usize, Option<Arc<dyn Send + Sync>>), native_contract::ServiceError>,
> {
    Box::pin(native_acquire_account_permit_impl(
        host,
        collection,
        accounts,
        reserve_permit,
    ))
}

async fn native_acquire_account_permit_impl(
    host: &NativeHostBridge,
    collection: &CodexLocalAccessCollection,
    accounts: &[CodexAccount],
    reserve_permit: bool,
) -> Result<(usize, Option<Arc<dyn Send + Sync>>), native_contract::ServiceError> {
    if !reserve_permit {
        return native_acquire_account_concurrency_permit(host, collection, accounts, false).await;
    }
    let local_reserved = collection.bound_oauth_quota_reserve.is_some()
        && accounts.iter().any(|account| {
            !native_is_chatgpt_plan_account(account)
                && collection.bound_oauth_account_id.as_deref() == Some(account.id.as_str())
        });
    let shared_reserved = quota_reserve_coordinator()
        .lock()
        .ok()
        .is_some_and(|runtime| {
            runtime.strict_policy.as_ref().is_some_and(|(id, _)| {
                accounts
                    .iter()
                    .any(|account| !native_is_chatgpt_plan_account(account) && &account.id == id)
            })
        });
    if !local_reserved && !shared_reserved {
        return native_acquire_account_concurrency_permit(host, collection, accounts, true).await;
    }
    let deadline = Instant::now() + native_account_concurrency_wait(collection);
    let mut candidates: Vec<usize> = (0..accounts.len()).collect();
    let mut last_error = None;
    while !candidates.is_empty() {
        let available: Vec<_> = candidates
            .iter()
            .map(|index| accounts[*index].clone())
            .collect();
        let mut settings = collection.clone();
        settings.account_concurrency_wait_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let (position, concurrency) =
            native_acquire_account_concurrency_permit(host, &settings, &available, true).await?;
        let index = candidates[position];
        // This check runs after the semaphore wait, against the latest shared evidence.
        let quota = if native_is_chatgpt_plan_account(&accounts[index]) {
            Ok(None)
        } else {
            acquire_quota_admission(collection, &accounts[index])
        };
        match quota {
            Ok(quota) => {
                let permit = if concurrency.is_none() && quota.is_none() {
                    None
                } else {
                    Some(Arc::new((concurrency, quota)) as Arc<dyn Send + Sync>)
                };
                return Ok((index, permit));
            }
            Err(error) if error.code == "quota_reserved" => {
                drop(concurrency);
                last_error = Some(error);
                candidates.remove(position);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        native_error(
            503,
            "no_available_account",
            "No compatible account is available for this model",
        )
    }))
}

// Match Go's account wait queue and release the slot on success, timeout or cancellation.
const NATIVE_ACCOUNT_MAX_WAITING: u64 = 100;
struct NativeAccountWaitGuard<'a>(&'a AtomicU64);
impl Drop for NativeAccountWaitGuard<'_> {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}
fn native_account_wait_slot(host: &NativeHostBridge) -> Result<NativeAccountWaitGuard<'_>, native_contract::ServiceError> {
    host.account_waiters.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < NATIVE_ACCOUNT_MAX_WAITING).then_some(count + 1)
    }).map_err(|_| native_error(429, "account_busy", "Account concurrency wait queue is full"))?;
    Ok(NativeAccountWaitGuard(&host.account_waiters))
}

async fn native_acquire_account_concurrency_permit(
    host: &NativeHostBridge,
    collection: &CodexLocalAccessCollection,
    accounts: &[CodexAccount],
    reserve_permit: bool,
) -> Result<(usize, Option<Arc<dyn Send + Sync>>), native_contract::ServiceError> {
    if accounts.is_empty() {
        return Err(native_error(
            503,
            "no_available_account",
            "No compatible account is available for this model",
        ));
    }
    if !reserve_permit || collection.max_account_concurrency == 0 {
        return Ok((0, None));
    }
    let gates: Vec<_> = {
        let mut gates = host.gates.lock().map_err(|_| {
            native_error(503, "service_unavailable", "Account limiter is unavailable")
        })?;
        gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        accounts
            .iter()
            .map(|account| {
                gates
                    .entry(format!(
                        "{}:{}",
                        account.id, collection.max_account_concurrency
                    ))
                    .or_insert_with(|| {
                        Arc::new(Semaphore::new(collection.max_account_concurrency as usize))
                    })
                    .clone()
            })
            .collect()
    };
    for (index, gate) in gates.iter().enumerate() {
        if let Ok(permit) = gate.clone().try_acquire_owned() {
            return Ok((index, Some(Arc::new(permit))));
        }
    }
    if collection.account_concurrency_wait_ms == 0 {
        return Err(native_error(
            429,
            "account_busy",
            "Account concurrency limit reached",
        ));
    }
    let _wait_slot = native_account_wait_slot(host)?;
    // Wait once for any candidate, without multiplying the configured wait by
    // the pool size. Dropping the other futures cancels their reservations.
    let mut waiting: futures_util::stream::FuturesUnordered<_> = gates
        .into_iter()
        .enumerate()
        .map(|(index, gate)| async move { (index, gate.acquire_owned().await) })
        .collect();
    let (index, acquired) = timeout(native_account_concurrency_wait(collection), waiting.next())
        .await
        .map_err(|_| native_error(429, "account_busy", "Account concurrency wait timed out"))?
        .ok_or_else(|| native_error(503, "service_unavailable", "Account limiter stopped"))?;
    let permit = acquired
        .map_err(|_| native_error(503, "service_unavailable", "Account limiter stopped"))?;
    Ok((index, Some(Arc::new(permit))))
}

async fn native_route_for_reserved_account(
    host: &NativeHostBridge,
    collection: &CodexLocalAccessCollection,
    account: CodexAccount,
    upstream_model: String,
    force_refresh: bool,
    permit: Option<Arc<dyn Send + Sync>>,
    http1_only: bool,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    let account = native_prepare_account(account, force_refresh).await?;
    let (account, mut headers, base_url, official_authorization_revision) =
        native_account_credentials(account, force_refresh).await?;
    if account.official_registration_id.is_some()
        && !host
            .selected_authorization_current(
                &account.id,
                account.official_registration_id.as_deref(),
                official_authorization_revision.as_deref(),
            )
            .await
    {
        return Err(native_error(
            409,
            "account_binding_unavailable",
            "Selected official authorization changed",
        ));
    }
    if let Some(reserve) = quota_reserve_policy(collection, &account.id)
        .filter(|_| !native_is_chatgpt_plan_account(&account))
    {
        let runtime = quota_reserve_coordinator().lock().map_err(|_| {
            native_error(503, "quota_unavailable", "Quota admission is unavailable")
        })?;
        if quota_reserve_decision(&reserve, &account, runtime.accounts.get(&account.id)).0 {
            return Err(
                native_error(429, "quota_reserved", "OAuth quota is reserved")
                    .with_retry_after(Some(Duration::from_secs(5))),
            );
        }
    }
    let chatgpt_plan = native_is_chatgpt_plan_account(&account);
    let chatgpt = !account.is_api_key_auth() && !chatgpt_plan;
    for value in headers.values_mut() {
        value.set_sensitive(true);
    }
    let parsed = Url::parse(&base_url)
        .map_err(|_| native_error(400, "invalid_upstream", "Upstream URL is invalid"))?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(native_error(
            400,
            "invalid_upstream",
            "Upstream URL must be an HTTP(S) base URL without credentials or query",
        ));
    }
    let profile_account = account.clone();
    let profile_permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            native_error(
                503,
                "identity_profile_unavailable",
                "Installation identity worker is unavailable",
            )
        })?;
    #[cfg(test)]
    let identity_fixture = host.identity_fixture.clone();
    let installation_profile = tokio::task::spawn_blocking(move || {
        let _permit = profile_permit;
        #[cfg(test)]
        return identity_fixture.resolve(&profile_account);
        #[cfg(not(test))]
        crate::modules::codex_api_identity::resolve(&profile_account)
    })
    .await
    .map_err(|_| {
        native_error(
            503,
            "identity_profile_unavailable",
            "Unable to prepare installation identity",
        )
    })?
    .map_err(|_| {
        native_error(
            503,
            "identity_profile_unavailable",
            "Unable to prepare installation identity",
        )
    })?;
    let wire_api = native_wire_api(&account)?;
    if wire_api == native_contract::WireApi::Responses
        && !chatgpt_plan
        && !headers.contains_key("x-xai-token-auth")
    {
        crate::modules::codex_api_identity::apply_headers(&installation_profile, &mut headers)
            .map_err(|_| {
                native_error(
                    503,
                    "identity_profile_unavailable",
                    "Invalid installation identity",
                )
            })?;
    }
    let profile_dir = host.profile_dir.clone();
    let default_service_tier = if chatgpt {
        tokio::task::spawn_blocking(move || match profile_dir {
            Some(profile) => crate::modules::codex_speed::get_app_speed_config_for_dir(&profile),
            None => crate::modules::codex_speed::get_api_service_app_speed_config(),
        })
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|config| config.speed.service_tier().map(str::to_string))
    } else {
        None
    };
    let grok_endpoint = codex_account::is_grok_upstream_provider(&account)
        .then(|| native_grok::official_endpoint(&headers));
    let owner_id = native_account_owner(&account, &base_url);
    let transport_context = NativeTransportContext {
        owner: &owner_id,
        profile: &installation_profile,
        base_url: &base_url,
        wire_api,
        http1_only,
        headers: &headers,
        compact_endpoint: grok_endpoint.as_ref(),
        websocket_endpoint: grok_endpoint.as_ref(),
    };
    // Credentials and the principal may change while preparing Grok/Agent
    // Identity. Build the pool key only from this final route snapshot.
    let (client, proxy, proxy_route, transport_identity) =
        native_http_client(host, collection, &account, &transport_context).await?;
    if headers.contains_key("x-xai-token-auth") {
        crate::modules::grok_client_version::refresh_in_background(client.clone());
    }
    Ok(native_contract::Route {
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
        official_fallback_failure: None,
        continuity_account_id: account
            .official_registration_id
            .as_ref()
            .map(|id| format!("{}{id}", codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)),
        official_registration_id: account.official_registration_id.clone().or_else(|| {
            account
                .id
                .strip_prefix(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                .map(str::to_owned)
        }),
        account_id: account.id.clone(),
        owner_id,
        installation_profile: (!chatgpt_plan).then_some(installation_profile),
        official_authorization_revision,
        transport_identity: Some(transport_identity),
        base_url,
        model: upstream_model,
        headers,
        client,
        proxy_url: proxy,
        proxy_route,
        chatgpt,
        chatgpt_plan,
        wire_api,
        compact_endpoint: grok_endpoint.clone(),
        websocket_endpoint: grok_endpoint,
        permit,
        default_service_tier,
    })
}
fn native_chatgpt_plan_auxiliary_error(operation: &str) -> native_contract::ServiceError {
    let message = if operation == "search" {
        "This gateway's official-authorization route does not implement the standalone Codex /alpha/search endpoint. Use the original local Codex gateway for this endpoint. Responses web_search is a separate capability."
            .to_string()
    } else {
        format!("This gateway's official-authorization route does not implement standalone operation '{operation}'. Select a compatible service account for this endpoint.")
    };
    native_error(400, "subscription_sharing_unsupported_capability", &message)
        .with_error_details(json!({"code":"subscription_sharing_unsupported_capability","message":message,"param":"endpoint"}))
}

fn native_auxiliary_eligible(account: &CodexAccount, operation: Option<&str>) -> bool {
    if native_is_chatgpt_plan_account(account) {
        // A client Responses socket is bridged to public HTTP/SSE. It does not
        // grant access to any additional upstream endpoint or hosted tool.
        return operation.is_none() || operation == Some("responses.websocket");
    }
    let ordinary_oauth = !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
        && !codex_account::is_grok_upstream_provider(account);
    match operation {
        None => true,
        Some("responses.websocket") => true,
        Some("audio.transcriptions") => ordinary_oauth || account.is_api_key_auth(),
        Some("audio.translations" | "audio.speech") => account.is_api_key_auth(),
        Some("search" | "realtime") => ordinary_oauth,
        Some(_) => false,
    }
}
fn native_operation_model(
    collection: &CodexLocalAccessCollection,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
    operation: Option<&str>,
    image_input: bool,
) -> Option<String> {
    if operation.is_none() || operation == Some("responses.websocket") {
        let model = native_plan_account_model(plan, account)?;
        if !plan.provider
            && native_is_reserve_model(&model)
            && !account_has_gpt_reserve_entitlement(account)
        {
            return None;
        }
        let model = if image_input {
            native_plan_vision_model(collection, plan, account, &model)
        } else {
            model
        };
        return if native_is_chatgpt_plan_account(account) {
            native_chatgpt_plan_model(account, &model)
        } else {
            Some(model)
        };
    }
    let model = account
        .api_model_mappings
        .iter()
        .find(|mapping| mapping.client_model == plan.model)
        .map(|mapping| mapping.upstream_model.clone())
        .unwrap_or_else(|| plan.model.clone());
    Some(model)
}

fn native_append_image_models(
    catalog: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    accounts: &[CodexAccount],
) {
    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return;
    }
    let image_ids = if collection.image_generation_account_ids.is_empty() {
        native_scoped_account_ids(collection, key)
    } else {
        collection.image_generation_account_ids.clone()
    };
    let Some(data) = catalog.get_mut("data").and_then(Value::as_array_mut) else {
        return;
    };
    for account in accounts
        .iter()
        .filter(|account| !service_account_paused(collection, &account.id))
        .filter(|account| image_ids.contains(&account.id) && native_eligible(account))
    {
        let family = &collection.image_generation_model;
        if !native_image_source_allowed(collection, account, family)
            || !native_image_model_allowed(collection, key, account, family)
        {
            continue;
        }
        let Some(actual) = native_image_source_model(collection, account, family) else {
            continue;
        };
        for model in [family.clone(), actual] {
            let exposed = key
                .and_then(|key| key.model_prefix.as_ref())
                .filter(|v| !v.is_empty())
                .map(|prefix| format!("{}/{model}", prefix.trim_end_matches('/')))
                .unwrap_or(model);
            if native_model_for_key(collection, key, &exposed).is_ok()
                && !data.iter().any(|entry| entry["id"] == exposed)
            {
                data.push(
                    json!({"id":exposed,"object":"model","owned_by":"configured-image-source"}),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_local_access_native_chatgpt_plan_tests.rs"]
mod native_chatgpt_plan_tests;

#[cfg(test)]
pub(crate) use native_chatgpt_plan_tests::assert_chatgpt_plan_cold_catalog_real_http;
#[cfg(test)]
pub(crate) use native_chatgpt_plan_tests::assert_chatgpt_plan_mixed_key_real_host;

#[cfg(test)]
mod native_image_catalog_tests {
    use super::*;
    #[test]
    fn native_image_catalog_exposes_only_selected_allowed_full_model_ids() {
        let mut collection:CodexLocalAccessCollection=serde_json::from_value(json!({"enabled":true,"port":1455,"apiKey":"fixture","accountIds":[],"createdAt":1,"updatedAt":1})).unwrap();
        collection.image_generation_account_ids = vec!["image".into()];
        collection.image_generation_model = "gpt-image-2.5".into();
        collection
            .image_generation_account_models
            .insert("image".into(), "gpt-image-2.5-flare".into());
        let account = CodexAccount::new_api_key(
            "image".into(),
            "test@example.invalid".into(),
            "token".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://example.invalid/v1".into()),
            None,
            None,
            vec!["gpt-image-2.5-flare".into()],
        );
        let mut catalog = json!({"object":"list","data":[]});
        native_append_image_models(&mut catalog, &collection, None, &[account.clone()]);
        assert_eq!(catalog["data"].as_array().unwrap().len(), 2);
        collection.excluded_models.push("gpt-image-*".into());
        let mut denied = json!({"data":[]});
        native_append_image_models(&mut denied, &collection, None, &[account]);
        assert_eq!(denied["data"], json!([]));
    }
}
