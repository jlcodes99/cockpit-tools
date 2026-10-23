// Native instance gateways share the existing profile transaction, not the global API listener.

impl NativeHostBridge {
    async fn selected_authorization_current(
        &self,
        account: &str,
        registration: Option<&str>,
        revision: Option<&str>,
    ) -> bool {
        let Ok(collection) = self.collection().await else {
            return false;
        };
        if self.profile_dir.is_some() {
            return registration.is_none();
        }
        if !account.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
            && collection
                .official_account_bindings
                .get(account)
                .map(String::as_str)
                != registration
        {
            return false;
        }
        if let (Some(id), Some(revision)) = (registration, revision) {
            crate::modules::chatgpt_plan::authorization_epoch_matches(
                id,
                (!account.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX))
                    .then_some(account),
                revision,
            )
            .await
        } else {
            registration.is_none()
        }
    }
    async fn load_route_account(
        &self,
        route: &native_contract::Route,
    ) -> Result<Option<CodexAccount>, native_contract::ServiceError> {
        if route.official_fallback_registration_id.is_some()
            && !self.selected_route_current(route).await
        {
            return Err(native_error(
                409,
                "account_binding_unavailable",
                "Local fallback authorization changed",
            ));
        }
        if let Some(registration) = route
            .official_registration_id
            .as_ref()
            .filter(|_| route.continuity_account_id.is_some())
        {
            let collection = self.collection().await?;
            if !collection.account_ids.contains(&route.account_id)
                || service_account_paused(&collection, &route.account_id)
                || collection.official_account_bindings.get(&route.account_id) != Some(registration)
            {
                return Err(native_error(
                    409,
                    "account_binding_unavailable",
                    "Selected authorization changed before the request was sent",
                ));
            }
        }
        if route.official_registration_id.is_some()
            && !self
                .selected_authorization_current(
                    &route.account_id,
                    route.official_registration_id.as_deref(),
                    route.official_authorization_revision.as_deref(),
                )
                .await
        {
            return Err(native_error(
                409,
                "account_binding_unavailable",
                "Selected official authorization is no longer valid",
            ));
        }
        let mut accounts = self.load_accounts(vec![route.account_id.clone()]).await?;
        let Some(account) = accounts.pop() else {
            return Ok(None);
        };
        if let Some(registration) = route.official_registration_id.clone().filter(|_| {
            !account
                .id
                .starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
        }) {
            return tokio::task::spawn_blocking(move || {
                Some(crate::modules::chatgpt_plan::official_runtime_overlay(
                    account,
                    &registration,
                ))
            })
            .await
            .map_err(|_| {
                native_error(
                    503,
                    "account_read_failed",
                    "Unable to prepare selected official authorization",
                )
            });
        }
        Ok(Some(account))
    }
    async fn load_routing_accounts(
        &self,
        ids: Vec<String>,
        official: bool,
    ) -> Result<Vec<CodexAccount>, native_contract::ServiceError> {
        let accounts = self.load_accounts(ids).await?;
        let bindings = self.collection().await?.official_account_bindings;
        if !official || self.profile_dir.is_some() || bindings.is_empty() {
            return Ok(accounts);
        }
        tokio::task::spawn_blocking(move || {
            accounts
                .into_iter()
                .map(|account| match bindings.get(&account.id) {
                    Some(id) => crate::modules::chatgpt_plan::official_runtime_overlay(account, id),
                    None => account,
                })
                .collect()
        })
        .await
        .map_err(|_| {
            native_error(
                503,
                "account_read_failed",
                "Unable to read official authorization metadata",
            )
        })
    }

    async fn load_accounts(
        &self,
        ids: Vec<String>,
    ) -> Result<Vec<CodexAccount>, native_contract::ServiceError> {
        let missing = ids
            .iter()
            .filter(|id| !self.account_overrides.contains_key(*id))
            .cloned()
            .collect();
        let mut accounts = native_load_accounts(missing).await?;
        accounts.extend(
            ids.iter()
                .filter_map(|id| self.account_overrides.get(id).cloned()),
        );
        Ok(accounts)
    }
}

async fn native_run_model_provider_chat_test(
    request: CodexModelProviderGatewayChatTestRequest,
    collection: CodexLocalAccessCollection,
    account: CodexAccount,
    client_model: String,
) -> Result<CodexModelProviderGatewayChatTestResult, String> {
    let directory = std::env::temp_dir().join(format!(
        "cockpit-native-provider-test-{}",
        uuid::Uuid::new_v4()
    ));
    let mut host = NativeHostBridge::for_collection(collection.clone());
    host.account_overrides.insert(account.id.clone(), account);
    let address = format!(
        "{}:{}",
        CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST, collection.port
    )
    .parse()
    .map_err(|_| "NATIVE_API_INVALID_ADDRESS".to_string())?;
    let started_handle = native_api::start(
        address,
        Arc::new(host),
        directory.join("identity.sqlite"),
        native_api::Options::default(),
    )
    .await;
    let mut handle = match started_handle {
        Ok(handle) => handle,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&directory).await;
            return Err(error);
        }
    };
    let client_request_id = format!(
        "{}:{}:{}:{}",
        request.run_id.trim(),
        request.provider_id.trim(),
        request.api_key_id.as_deref().unwrap_or_default().trim(),
        request.model_id.trim()
    );
    let started = Instant::now();
    let operation = async {
        let client = build_localhost_http_client(Duration::from_secs(90), "Native provider test")?;
        let mut builder = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth(&collection.api_key)
            .header(ACCEPT, "application/json");
        for (name, value) in [
            ("x-client-request-id", client_request_id.as_str()),
            ("x-agtools-test-run-id", request.run_id.as_str()),
            ("x-agtools-provider-id", request.provider_id.as_str()),
            ("x-agtools-wire-api", request.wire_api.as_str()),
        ] {
            builder = add_model_provider_test_header(builder, name, value);
        }
        let response = builder.json(&json!({"model":client_model,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":request.prompt}]}],"instructions":"","stream":false,"store":false,"max_output_tokens":256,
            "metadata":{"agtools_source":"codex_model_provider_batch_test","agtools_test_run_id":request.run_id,"agtools_provider_id":request.provider_id,"agtools_provider_name":request.provider_name,"agtools_provider_api_key_id":request.api_key_id,"agtools_provider_api_key_name":request.api_key_name,"agtools_wire_api":request.wire_api,"agtools_test_model":request.model_id,"agtools_client_model":client_model,"agtools_client_request_id":client_request_id}}))
            .send().await.map_err(|error|format!("Native provider test failed: {error}"))?;
        let status = response.status();
        let text = response.text().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "Native provider test failed ({}): {}",
                status.as_u16(),
                extract_provider_gateway_test_error_message(&text)
            ));
        }
        let parsed: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let reply = extract_provider_gateway_test_output_text(&parsed)
            .ok_or_else(|| "Native provider test returned no text".to_string())?;
        Ok(CodexModelProviderGatewayChatTestResult {
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            reply,
        })
    };
    let result = tokio::select! {
        result=operation=>result,
        _=wait_for_model_provider_chat_test_cancellation(request.run_id.trim())=>Err(MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR.to_string()),
    };
    let stopped = handle.shutdown(GATEWAY_SHUTDOWN_TIMEOUT).await;
    drop(handle);
    let cleanup = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(directory)).await;
    if let Err(error) = stopped {
        logger::log_codex_api_warn(&format!("[NativeAPI] Temporary provider shutdown: {error}"));
    }
    if let Ok(Err(error)) = cleanup {
        logger::log_codex_api_warn(&format!("[NativeAPI] Temporary provider cleanup: {error}"));
    }
    result
}

fn native_provider_runtime_alive(runtime: &mut ProviderGatewayRuntime) -> bool {
    runtime
        .native_handle
        .as_ref()
        .is_some_and(|handle| !handle.is_finished())
        || runtime
            .sidecar_child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
}

fn native_profile_identity_path(profile: &Path, runtime_id: &str) -> Result<PathBuf, String> {
    Ok(provider_gateway_sidecar_dir(profile, runtime_id)?.join("native-identity.sqlite"))
}

async fn native_start_profile_listener(
    profile: &Path,
    runtime_id: &str,
    collection: &CodexLocalAccessCollection,
) -> Result<native_api::Handle, String> {
    let address = format!(
        "{}:{}",
        CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST, collection.port
    )
    .parse()
    .map_err(|_| "NATIVE_API_INVALID_ADDRESS".to_string())?;
    let options = native_api::Options {
        request_timeout: Duration::from_millis(
            collection
                .timeouts
                .legacy_request_read_timeout_ms
                .clamp(1000, 120_000),
        ),
        idle_timeout: Duration::from_millis(
            collection
                .timeouts
                .legacy_stream_idle_timeout_ms
                .clamp(1000, 1_800_000),
        ),
        ..Default::default()
    };
    let mut host = NativeHostBridge::for_collection(collection.clone());
    host.profile_dir = Some(profile.to_path_buf());
    native_api::start(
        address,
        Arc::new(host),
        native_profile_identity_path(profile, runtime_id)?,
        options,
    )
    .await
}

async fn native_activate_profile<P, F>(
    profile: &Path,
    runtime_id: &str,
    collection: CodexLocalAccessCollection,
    key: &str,
    oauth_account_ids: Vec<String>,
    publish: P,
) -> Result<(), String>
where
    P: FnOnce() -> F,
    F: std::future::Future<Output = Result<(), String>>,
{
    let handle = activate_provider_gateway_profile(
        profile,
        key,
        || native_start_profile_listener(profile, runtime_id, &collection),
        publish,
        |mut handle| async move {
            let _ = handle.shutdown(GATEWAY_SHUTDOWN_TIMEOUT).await;
        },
    )
    .await?;
    let runtime_key = provider_gateway_runtime_key(profile, runtime_id);
    let runtime = ProviderGatewayRuntime {
        actual_port: Some(handle.addr.port()),
        actual_bind_host: Some(handle.addr.ip().to_string()),
        native_handle: Some(handle),
        collection: Some(collection),
        oauth_account_ids,
        ..Default::default()
    };
    let mut runtimes = provider_gateway_runtime_store().lock().await;
    runtimes.insert(runtime_key, runtime);
    NATIVE_PROFILE_RUNNING.store(
        runtimes
            .values()
            .filter(|runtime| runtime.native_handle.is_some())
            .count(),
        Ordering::Release,
    );
    Ok(())
}

async fn native_ensure_provider_profile(
    profile: &Path,
    account: &CodexAccount,
    collection: CodexLocalAccessCollection,
    key: &str,
    slots: &[ProviderGatewayModelSlot],
) -> Result<(), String> {
    let images = image_generation_accounts_for_account(account, &collection);
    let oauth_ids = images
        .iter()
        .filter(|account| !account.is_api_key_auth())
        .map(|account| account.id.clone())
        .collect();
    let profile_owned = profile.to_path_buf();
    let account_owned = account.clone();
    let published = collection.clone();
    let key_owned = key.to_string();
    let slots = slots.to_vec();
    native_activate_profile(
        profile,
        &account.id,
        collection,
        key,
        oauth_ids,
        || async move {
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let profile = profile_owned.as_path();
                let account = &account_owned;
                save_profile_takeover_backup(profile, &key_owned)?;
                runtime.block_on(write_local_access_profile_takeover(
                    profile,
                    &published,
                    Some(&key_owned),
                    false,
                ))?;
                cleanup_provider_gateway_profile_model_overrides(profile)?;
                backup_current_profile_model_before_provider_gateway(
                    profile,
                    &slots
                        .iter()
                        .map(|slot| slot.client_model.clone())
                        .collect::<Vec<_>>(),
                )?;
                if let Some(slot) = preferred_provider_gateway_slot(account, &slots) {
                    write_local_access_profile_model_override(profile, &slot.client_model)?;
                }
                if !slots.is_empty() {
                    write_provider_gateway_model_catalog_with_templates(
                        profile,
                        &slots,
                        official_catalog_json_for_provider_gateway(account)?.as_deref(),
                        Some(account),
                    )?;
                }
                codex_account::reapply_experimental_model_policy_if_enabled(profile)?;
                reapply_deepseek_profile_config_overrides(profile, account)?;
                runtime.block_on(native_sync_profile_catalog_tool_capabilities(
                    profile,
                    &published,
                    &key_owned,
                    std::slice::from_ref(account),
                ))
            })
            .await
            .map_err(|_| "NATIVE_PROFILE_WRITE_FAILED".to_string())?
        },
    )
    .await
}

async fn native_ensure_bound_profile(
    profile: &Path,
    account: &CodexAccount,
    collection: CodexLocalAccessCollection,
    key: &str,
) -> Result<(), String> {
    let profile_owned = profile.to_path_buf();
    let published = collection.clone();
    let key_owned = key.to_string();
    native_activate_profile(
        profile,
        &account.id,
        collection,
        key,
        vec![account.id.clone()],
        || async move {
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let profile = profile_owned.as_path();
                save_profile_takeover_backup(profile, &key_owned)?;
                runtime.block_on(write_local_access_profile_takeover(
                    profile,
                    &published,
                    Some(&key_owned),
                    false,
                ))?;
                cleanup_provider_gateway_profile_model_overrides(profile)?;
                codex_account::reapply_experimental_model_policy_if_enabled(profile)?;
                runtime.block_on(native_sync_profile_catalog_tool_capabilities(
                    profile,
                    &published,
                    &key_owned,
                    &[],
                ))
            })
            .await
            .map_err(|_| "NATIVE_PROFILE_WRITE_FAILED".to_string())?
        },
    )
    .await
}

async fn native_ensure_mixed_profile(
    profile: &Path,
    oauth_id: &str,
    collection: CodexLocalAccessCollection,
    key: &str,
    routing: &CodexInstanceModelRouting,
    is_current: impl Fn() -> bool,
) -> Result<(), String> {
    if !is_current() {
        return Ok(());
    }
    let profile_owned = profile.to_path_buf();
    let published = collection.clone();
    let key_owned = key.to_string();
    let routing = routing.clone();
    let result = native_activate_profile(
        profile,
        MIXED_MODEL_ROUTING_RUNTIME_ID,
        collection,
        key,
        vec![oauth_id.to_string()],
        || async {
            if !is_current() {
                return Err("NATIVE_PROFILE_START_CANCELLED".into());
            }
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let profile = profile_owned.as_path();
                let definitions = mixed_model_catalog_definitions(&routing)?;
                save_profile_takeover_backup(profile, &key_owned)?;
                cleanup_provider_gateway_profile_model_overrides(profile)?;
                if !definitions.is_empty() {
                    backup_current_profile_model_before_provider_gateway(
                        profile,
                        &definitions
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>(),
                    )?;
                }
                runtime.block_on(write_local_access_profile_takeover(
                    profile,
                    &published,
                    Some(&key_owned),
                    false,
                ))?;
                if !definitions.is_empty() {
                    write_local_access_profile_model_catalog_with_definitions(
                        profile,
                        false,
                        Some(definitions),
                    )?;
                }
                codex_account::reapply_experimental_model_policy_if_enabled(profile)?;
                runtime.block_on(native_sync_profile_catalog_tool_capabilities(
                    profile,
                    &published,
                    &key_owned,
                    &[],
                ))
            })
            .await
            .map_err(|_| "NATIVE_PROFILE_WRITE_FAILED".to_string())??;
            if !is_current() {
                return Err("NATIVE_PROFILE_START_CANCELLED".into());
            }
            Ok(())
        },
    )
    .await;
    match result {
        Err(error) if error == "NATIVE_PROFILE_START_CANCELLED" => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod native_profile_tests {
    use super::*;
    fn fixture_collection(key: &str) -> CodexLocalAccessCollection {
        serde_json::from_value(json!({"enabled":true,"port":0,"apiKey":key,"accountIds":[],"createdAt":1,"updatedAt":1})).unwrap()
    }
    async fn fixture_listener(root: &Path, key: &str) -> native_api::Handle {
        native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(NativeHostBridge::for_collection(fixture_collection(key))),
            root.join("identity.sqlite"),
            native_api::Options::default(),
        )
        .await
        .unwrap()
    }
    #[test]
    fn native_profiles_keep_runtime_keys_separate() {
        let one = provider_gateway_runtime_key(Path::new("/fixture/profile-one"), "account");
        let two = provider_gateway_runtime_key(Path::new("/fixture/profile-two"), "account");
        assert_ne!(one, two);
        assert_ne!(
            one,
            provider_gateway_runtime_key(
                Path::new("/fixture/profile-one"),
                MIXED_MODEL_ROUTING_RUNTIME_ID
            )
        );
    }
    #[test]
    fn native_profiles_empty_runtime_is_not_healthy() {
        assert!(!native_provider_runtime_alive(
            &mut ProviderGatewayRuntime::default()
        ));
    }

    #[tokio::test]
    async fn native_profiles_listeners_keep_keys_and_shutdown_isolated() {
        let root =
            std::env::temp_dir().join(format!("native-profile-isolation-{}", uuid::Uuid::new_v4()));
        let mut first = fixture_listener(&root.join("one"), "profile-one-key").await;
        let mut second = fixture_listener(&root.join("two"), "profile-two-key").await;
        assert_ne!(first.addr, second.addr);
        let client = Client::builder().no_proxy().build().unwrap();
        let first_url = format!("http://{}/v1/models", first.addr);
        let second_url = format!("http://{}/v1/models", second.addr);
        assert_eq!(
            client
                .get(&first_url)
                .bearer_auth("profile-one-key")
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .get(&second_url)
                .bearer_auth("profile-one-key")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        first.shutdown(Duration::from_secs(2)).await.unwrap();
        assert!(TcpStream::connect(first.addr).await.is_err());
        assert_eq!(
            client
                .get(&second_url)
                .bearer_auth("profile-two-key")
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        second.shutdown(Duration::from_secs(2)).await.unwrap();
        drop(first);
        drop(second);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn native_profiles_failed_publish_stops_unpublished_listener() {
        let root =
            std::env::temp_dir().join(format!("native-profile-rollback-{}", uuid::Uuid::new_v4()));
        let profile = root.join("profile");
        tokio::fs::create_dir_all(&profile).await.unwrap();
        let original_auth = br#"{"tokens":{"access_token":"original-token"},"extra":"preserve"}"#;
        let original_config = b"model = \"original-model\"\n";
        tokio::fs::write(profile_auth_path(&profile), original_auth)
            .await
            .unwrap();
        tokio::fs::write(profile_config_path(&profile), original_config)
            .await
            .unwrap();
        let observed = Arc::new(std::sync::atomic::AtomicU16::new(0));
        let listener_dir = root.join("listener");
        let result = activate_provider_gateway_profile(
            &profile,
            "rollback-key",
            || async {
                let handle = fixture_listener(&listener_dir, "rollback-key").await;
                observed.store(handle.addr.port(), Ordering::SeqCst);
                Ok(handle)
            },
            || async {
                tokio::fs::write(profile_auth_path(&profile), b"temporary-auth")
                    .await
                    .unwrap();
                tokio::fs::write(profile_config_path(&profile), b"temporary-config")
                    .await
                    .unwrap();
                tokio::fs::write(
                    profile.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE),
                    b"temporary-models",
                )
                .await
                .unwrap();
                Err("fixture publish failure".to_string())
            },
            |mut handle| async move {
                handle.shutdown(Duration::from_secs(2)).await.unwrap();
            },
        )
        .await;
        assert!(matches!(result,Err(ref error) if error=="fixture publish failure"));
        let port = observed.load(Ordering::SeqCst);
        assert_ne!(port, 0);
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        assert_eq!(
            tokio::fs::read(profile_auth_path(&profile)).await.unwrap(),
            original_auth
        );
        assert_eq!(
            tokio::fs::read(profile_config_path(&profile))
                .await
                .unwrap(),
            original_config
        );
        assert!(!profile.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE).exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
