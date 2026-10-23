// Included in codex_local_access to reuse its isolated provider test gateway.
pub async fn run_pelican_provider_chat(
    request: CodexModelProviderGatewayChatTestRequest,
    effort: String,
    cancel: watch::Receiver<bool>,
    on_delta: impl Fn(String) + Send + Sync + 'static,
) -> Result<PelicanChatOutput, String> {
    let secret = request.api_key.clone();
    // The worker owns the temporary process through cleanup, even if its caller
    // is dropped. Cancellation closes the stream before stopping the gateway.
    let result = tokio::spawn(async move {
        if *cancel.borrow() {
            return Err("PELICAN_CANCELLED".into());
        }
        let preparation_slot = pelican_with_cancel(cancel.clone(), PELICAN_TOTAL_TIMEOUT, async {
            PELICAN_PREPARATION_SLOTS
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "pelican.error.stateUnavailable".to_string())
        })
        .await?;
        if *cancel.borrow() {
            return Err("PELICAN_CANCELLED".into());
        }
        let prepare_request = request.clone();
        let runtime = tokio::runtime::Handle::current();
        let prepared = tokio::task::spawn_blocking(move || {
            let _slot = preparation_slot;
            runtime.block_on(async move {
                let models = model_provider_gateway_test_models(
                    &prepare_request.model_id,
                    &prepare_request.model_catalog,
                );
                let account = build_model_provider_gateway_test_account(
                    &prepare_request,
                    model_provider_gateway_test_account_id(&prepare_request),
                    models,
                );
                // Both protocols use an explicit provider route. The ordinary
                // connectivity test's Codex alias is not an upstream model.
                let client_model = prepare_request.model_id.clone();
                let provider_gateway = Some(provider_gateway_for_account(&account)?);
                let collection = build_model_provider_gateway_test_collection(
                    &prepare_request,
                    &account,
                    provider_gateway,
                    &client_model,
                )?;
                let dir = provider_gateway_sidecars_dir()?
                    .join(format!("pelican-provider-{}", uuid::Uuid::new_v4()));
                let config = timeout(
                    Duration::from_secs(20),
                    prepare_sidecar_launch_config_in_dir(
                        &collection,
                        dir.clone(),
                        HashMap::new(),
                        None,
                        HashMap::from([(account.id.clone(), account)]),
                    ),
                )
                .await
                .map_err(|_| "PELICAN_TIMEOUT".to_string())
                .and_then(|result| result);
                let result = match config {
                    Ok(config) => spawn_provider_gateway_sidecar(&collection, &config, false).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok((child, task, host)) => {
                        Ok((collection, client_model, dir, child, task, host))
                    }
                    Err(error) => {
                        if dir.exists() {
                            let _ = std::fs::remove_dir_all(&dir);
                        }
                        Err(error)
                    }
                }
            })
        })
        .await
        .map_err(|_| "pelican.error.workerFailed".to_string())??;
        let (collection, client_model, dir, child, task, host) = prepared;
        let result = pelican_with_cancel(cancel, PELICAN_TOTAL_TIMEOUT, async {
            let (body, headers) = build_pelican_request(
                &client_model,
                &effort,
                &request.prompt,
                &format!("{}:{}", request.run_id, request.provider_id),
            )?;
            let client =
                build_localhost_http_client(PELICAN_TOTAL_TIMEOUT, "Pelican provider test")?;
            let mut builder = client
                .post(format!("http://{}:{}/v1/responses", host, collection.port))
                .bearer_auth(&collection.api_key);
            for (name, value) in headers {
                builder = builder.header(name, value);
            }
            let response = timeout(PELICAN_IDLE_TIMEOUT, builder.body(body).send())
                .await
                .map_err(|_| "PELICAN_TIMEOUT".to_string())?
                .map_err(|_| "PELICAN_STREAM_INCOMPLETE".to_string())?;
            if !response.status().is_success() {
                let status = response.status();
                let raw = pelican_read_error_body(response).await?;
                let safe = raw
                    .replace(&request.api_key, "[redacted]")
                    .replace(&collection.api_key, "[redacted]");
                return Err(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    extract_provider_gateway_test_error_message(&safe)
                ));
            }
            pelican_consume_response(response, PELICAN_IDLE_TIMEOUT, &on_delta).await
        })
        .await;
        stop_temporary_provider_gateway_sidecar(child, task, host, collection.port, dir).await;
        result
    })
    .await
    .map_err(|_| "pelican.error.workerFailed".to_string())?;
    result.map_err(|error| {
        if secret.is_empty() {
            error
        } else {
            error.replace(&secret, "[redacted]")
        }
    })
}

#[cfg(test)]
#[path = "codex_pelican_provider_transport_tests.rs"]
mod pelican_provider_transport_tests;
