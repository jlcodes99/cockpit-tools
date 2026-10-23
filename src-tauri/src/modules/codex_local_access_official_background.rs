// Account-scoped background inference. Service membership, engine selection and
// collection routing must not affect a benchmark or wakeup for a chosen account.
struct OfficialBackgroundAuthorization {
    account: CodexAccount,
    registration_id: String,
    snapshot: crate::modules::chatgpt_plan::SourceAssociationSnapshot,
}

async fn prepare_official_background_authorization(
    account_id: &str,
) -> Result<Option<OfficialBackgroundAuthorization>, String> {
    timeout(Duration::from_secs(90), async {
        let mut registrations = crate::modules::chatgpt_plan::list_accounts()
            .await?
            .into_iter()
            .filter(|registration| registration.source_account_id.as_deref() == Some(account_id));
        let Some(registration) = registrations.next() else {
            return Ok(None);
        };
        if registrations.next().is_some() {
            return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
        }
        // Revalidate the actual binding rather than the asynchronous display
        // cache. Never fall back on invalid or revoked grants.
        crate::modules::chatgpt_plan::validate_source_binding(&registration.id, account_id).await?;
        let snapshot = crate::modules::chatgpt_plan::validate_source_association_snapshot(
            &registration.id,
            account_id,
        )
        .await?;
        let source_id = account_id.to_owned();
        let account = tokio::task::spawn_blocking(move || {
            codex_account::load_account(&source_id)
                .ok_or("CHATGPT_PLAN_SOURCE_NOT_FOUND".to_string())
        })
        .await
        .map_err(|_| "CHATGPT_PLAN_STORAGE")??;
        // In particular, do not refresh or inject the original Codex tokens:
        // an independent official grant can work when those tokens have expired.
        Ok(Some(OfficialBackgroundAuthorization {
            account,
            registration_id: registration.id,
            snapshot,
        }))
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_TIMEOUT".to_string())?
}

async fn official_background_http_client(
    authorization: &OfficialBackgroundAuthorization,
    url: &str,
    token: &str,
) -> Result<Client, String> {
    let account = authorization.account.clone();
    let permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "CHATGPT_PLAN_STORAGE")?;
    let registration = authorization.registration_id.clone();
    let (profile, transport_account) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let profile = crate::modules::codex_api_identity::resolve(&account)?;
        let overlay =
            crate::modules::chatgpt_plan::official_runtime_overlay(account, &registration);
        Ok::<_, String>((profile, overlay))
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_STORAGE")??;
    let collection: CodexLocalAccessCollection = serde_json::from_value(json!({
        "enabled":false,"port":0,"apiKey":"","accountIds":[],"createdAt":0,"updatedAt":0
    }))
    .map_err(|_| "CHATGPT_PLAN_CONFIGURATION")?;
    let owner = native_account_owner(&transport_account, url);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "CHATGPT_PLAN_REAUTH_REQUIRED")?,
    );
    let context = NativeTransportContext {
        owner: &owner,
        profile: &profile,
        base_url: url,
        wire_api: native_contract::WireApi::Responses,
        http1_only: false,
        headers: &headers,
        compact_endpoint: None,
        websocket_endpoint: None,
    };
    // Reuse account/global/system proxy policy and isolated pools only. None of
    // the private Codex identity headers or request transformations are sent.
    native_http_client(
        &DIRECT_REQUEST_HOST,
        &collection,
        &transport_account,
        &context,
    )
    .await
    .map(|(client, _, _, _)| client)
    .map_err(|error| error.message)
}

async fn run_official_background_chat(
    authorization: &OfficialBackgroundAuthorization,
    model: &str,
    effort: &str,
    prompt: &str,
    instructions: &str,
    purpose: &str,
    on_delta: impl Fn(String),
) -> Result<PelicanChatOutput, String> {
    timeout(PELICAN_TOTAL_TIMEOUT, async {
        let (token, authorization_revision) =
            crate::modules::chatgpt_plan::inference_snapshot_for_source(
                &authorization.registration_id,
                &authorization.account.id,
                false,
            )
            .await?;
        let url = crate::modules::chatgpt_plan::inference_url("/responses");
        let client = official_background_http_client(authorization, &url, &token).await?;
        crate::modules::chatgpt_plan::validate_source_binding(
            &authorization.registration_id,
            &authorization.account.id,
        )
        .await?;
        let snapshot = authorization.snapshot.clone();
        tokio::task::spawn_blocking(move || {
            crate::modules::chatgpt_plan::validate_source_association_snapshot_for_commit(&snapshot)
        })
        .await
        .map_err(|_| "CHATGPT_PLAN_STORAGE")??;
        if !crate::modules::chatgpt_plan::authorization_epoch_matches(
            &authorization.registration_id,
            Some(&authorization.account.id),
            &authorization_revision,
        )
        .await
        {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        logger::log_codex_api_info(&format!(
            "[OfficialBackground] purpose={purpose} account_id={} registration_id={} model={model}",
            authorization.account.id, authorization.registration_id,
        ));
        let body = json!({
            "model":model,
            "input":[{"role":"user","content":[{"type":"input_text","text":prompt}]}],
            "instructions":instructions,"reasoning":{"effort":effort},"store":false,"stream":true,
        });
        // Send once. A failed/uncertain generation must never be replayed using
        // the original credential, another model, or another account.
        let response = timeout(
            PELICAN_IDLE_TIMEOUT,
            client
                .post(url)
                .bearer_auth(&token)
                .header(ACCEPT, "text/event-stream")
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| "CHATGPT_PLAN_TIMEOUT")?
        .map_err(|_| "CHATGPT_PLAN_NETWORK")?;
        let status = response.status();
        if !status.is_success() {
            let raw = pelican_read_error_body(response).await?;
            let value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            let error = crate::modules::chatgpt_plan::inference_error(status.as_u16(), &value);
            crate::modules::chatgpt_plan::observe_inference_result(
                &authorization.registration_id,
                &authorization_revision,
                &crate::modules::chatgpt_plan::token_fingerprint(&token),
                status.as_u16(),
                Some(&error),
            )
            .await;
            return Err(error);
        }
        let mut output = pelican_consume_response(response, PELICAN_IDLE_TIMEOUT, &on_delta)
            .await
            .map_err(|error| error.replace(&token, "[redacted]"))?;
        output.uses_official_authorization = true;
        output.quota = None;
        Ok(output)
    })
    .await
    .map_err(|_| "CHATGPT_PLAN_TIMEOUT".to_string())?
}

async fn run_bound_official_wakeup(
    authorization: OfficialBackgroundAuthorization,
    model: Option<&str>,
    effort: Option<&str>,
    prompt: &str,
) -> Result<CodexOfficialWakeupChatResult, String> {
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(crate::modules::codex_wakeup::DEFAULT_WAKEUP_MODEL);
    let effort = effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("medium");
    let started_at = Instant::now();
    let output =
        run_official_background_chat(&authorization, model, effort, prompt, "", "wakeup", |_| {})
            .await?;
    if output.reply.trim().is_empty() {
        return Err("CHATGPT_PLAN_STREAM_INCOMPLETE".into());
    }
    Ok(CodexOfficialWakeupChatResult {
        account: authorization.account,
        reply: output.reply,
        duration_ms: started_at.elapsed().as_millis() as u64,
    })
}
