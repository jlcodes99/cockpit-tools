// Internal one-shot requests share account installation and transport identity
// with the API engines, without starting or reading the global API service.
static DIRECT_REQUEST_HOST: std::sync::LazyLock<NativeHostBridge> =
    std::sync::LazyLock::new(NativeHostBridge::default);

fn direct_codex_request(mut body: Value) -> Result<(Vec<u8>, HashMap<String, String>), String> {
    let session = uuid::Uuid::now_v7().to_string();
    let turn = uuid::Uuid::now_v7().to_string();
    let request = uuid::Uuid::now_v7().to_string();
    let turn_metadata = build_codex_turn_metadata(&session, &turn);
    let window = format!("{session}:0");
    let object = body.as_object_mut().ok_or("Invalid direct request body")?;
    object.insert("prompt_cache_key".into(), json!(session));
    object.insert(
        "client_metadata".into(),
        json!({
            // Filled by the selected account's direct transport or provider
            // gateway; never derive an installation from a task/account name.
            "x-codex-installation-id": null,
            "x-codex-window-id": window,
            "x-codex-turn-metadata": turn_metadata,
        }),
    );
    let headers = HashMap::from([
        ("accept".into(), "text/event-stream".into()),
        ("content-type".into(), "application/json".into()),
        ("session-id".into(), session.clone()),
        ("thread-id".into(), session.clone()),
        ("conversation_id".into(), session),
        ("x-client-request-id".into(), request),
        ("x-codex-window-id".into(), window),
        ("x-codex-turn-metadata".into(), turn_metadata),
    ]);
    Ok((
        serde_json::to_vec(&body).map_err(|e| e.to_string())?,
        headers,
    ))
}

fn direct_account_identity(
    profile: &crate::modules::codex_api_identity::InstallationProfile,
    body: &[u8],
    headers: &HashMap<String, String>,
    authorization: &str,
) -> Result<(Vec<u8>, HashMap<String, String>, axum::http::HeaderMap), String> {
    let mut mapped = axum::http::HeaderMap::new();
    for (name, value) in headers {
        mapped.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "Invalid direct request header")?,
            axum::http::HeaderValue::from_str(value)
                .map_err(|_| "Invalid direct request header")?,
        );
    }
    mapped.insert(
        AUTHORIZATION,
        axum::http::HeaderValue::from_str(authorization)
            .map_err(|_| "Invalid direct request authorization")?,
    );
    crate::modules::codex_api_identity::apply_headers(profile, &mut mapped)?;
    let mut body: Value =
        serde_json::from_slice(body).map_err(|_| "Invalid direct request body")?;
    let object = body.as_object_mut().ok_or("Invalid direct request body")?;
    let metadata = object.entry("client_metadata").or_insert_with(|| json!({}));
    metadata
        .as_object_mut()
        .ok_or("Invalid direct request metadata")?
        .insert(
            "x-codex-installation-id".into(),
            json!(profile.installation_id),
        );
    crate::modules::codex_api_identity::apply_body(profile, &mut body)?;
    let headers = mapped
        .iter()
        .map(|(name, value)| {
            Ok((
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| "Invalid direct request header")?
                    .to_owned(),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    Ok((
        serde_json::to_vec(&body).map_err(|e| e.to_string())?,
        headers,
        mapped,
    ))
}

async fn prepare_direct_account_transport(
    account: &CodexAccount,
    url: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    authorization: &str,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<(Client, Vec<u8>, HashMap<String, String>), String> {
    let selected = account.clone();
    let permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Account identity worker is unavailable")?;
    let profile = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::modules::codex_api_identity::resolve(&selected)
    })
    .await
    .map_err(|_| "Account identity worker failed")??;
    let (body, headers, mut mapped) =
        direct_account_identity(&profile, body, headers, authorization)?;
    // Pool keys describe credentials and transport, not individual turns. The
    // actual request retains these fields; another account still has its own key.
    for name in [
        "session-id",
        "session_id",
        "thread-id",
        "conversation_id",
        "x-client-request-id",
        "x-codex-turn-metadata",
        "x-codex-window-id",
    ] {
        mapped.remove(name);
    }
    let mut collection: CodexLocalAccessCollection = serde_json::from_value(json!({
        "enabled":false,"port":0,"apiKey":"","accountIds":[],"createdAt":0,"updatedAt":0
    }))
    .map_err(|_| "Invalid direct transport configuration")?;
    collection.upstream_proxy_url = upstream_proxy_url.map(str::to_owned);
    collection.timeouts.legacy_upstream_connect_timeout_ms = duration_to_millis(connect_timeout);
    let owner = native_account_owner(account, url);
    let context = NativeTransportContext {
        owner: &owner,
        profile: &profile,
        base_url: url,
        wire_api: native_contract::WireApi::Responses,
        http1_only: false,
        headers: &mapped,
        compact_endpoint: None,
        websocket_endpoint: None,
    };
    let (client, _, _, _) =
        native_http_client(&DIRECT_REQUEST_HOST, &collection, account, &context)
            .await
            .map_err(|e| e.message)?;
    logger::log_codex_api_info(&format!(
        "[DirectIdentity] prepared account_id={} installation_id={} session_id={} thread_id={}",
        account.id,
        profile.installation_id,
        headers.get("session-id").map(String::as_str).unwrap_or(""),
        headers.get("thread-id").map(String::as_str).unwrap_or(""),
    ));
    Ok((client, body, headers))
}

#[cfg(test)]
#[path = "codex_local_access_direct_identity_tests.rs"]
mod direct_identity_tests;
