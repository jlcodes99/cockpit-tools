//! Independent, opt-in ChatGPT plan registrations. These credentials never
//! enter Codex auth.json, account switching, or the existing OAuth client.
#[path = "chatgpt_plan_binding.rs"]
mod binding;
#[path = "chatgpt_plan_http.rs"]
mod http;
#[path = "chatgpt_plan_oauth.rs"]
mod oauth;
#[path = "chatgpt_plan_status.rs"]
mod status;
#[path = "chatgpt_plan_storage.rs"]
mod storage;
#[path = "chatgpt_plan_types.rs"]
mod types;
#[path = "chatgpt_plan_runtime.rs"]
mod runtime;
#[path = "chatgpt_plan_official_health.rs"]
mod official_health;
pub(crate) use official_health::{official_route_snapshot, record_official_route_failure, record_official_route_success, fallback_epoch_matches, OfficialRouteSnapshot};
pub(crate) use runtime::{official_runtime_overlay, authorization_epoch_matches};
pub use status::check_authorization;
pub(crate) use status::{observe_inference_result, token_fingerprint};
pub(crate) use status::{remember_service_mode, remember_service_mode_if_unset};

pub(crate) fn inference_url(path: &str) -> String {
    http::resource_url(path)
}

pub(crate) fn inference_error(status: u16, value: &serde_json::Value) -> String {
    http::response_error(status, value)
}

pub use binding::{
    bind_account, unbind_account, validate_source_association_snapshot, validate_source_binding,
    validate_source_binding_snapshot, SourceAssociationSnapshot, SourceBindingSnapshot,
};
pub(crate) use binding::{
    source_identity_fingerprint,
    validate_source_association_snapshot_for_commit, validate_source_binding_snapshot_for_commit,
};
pub use oauth::{
    cancel_login, open_incognito_window, poll_login, start_login, submit_callback_url,
};
pub use types::{
    ChatgptPlanAccountSummary, ChatgptPlanDisconnectResult, ChatgptPlanLogin,
    ChatgptPlanLoginStatus, ChatgptPlanModel,
};

pub async fn list_accounts() -> Result<Vec<ChatgptPlanAccountSummary>, String> {
    binding::list_accounts().await
}

pub async fn get_account(id: &str) -> Result<ChatgptPlanAccountSummary, String> {
    let record = storage::read(id).await?;
    storage::blocking(move || binding::checked_summary(&record)).await
}

/// Refresh only on use, with cross-process serialization and latest-record
/// re-read. Concurrent forced retries reuse the first refresh's replacement.
pub async fn access_token(id: &str, force: bool) -> Result<String, String> {
    access_snapshot(id, force, false, None)
        .await
        .map(|(token, _)| token)
}

pub async fn inference_token(id: &str, force: bool) -> Result<String, String> {
    access_snapshot(id, force, true, None)
        .await
        .map(|(token, _)| token)
}

pub(crate) async fn inference_snapshot(id: &str, force: bool) -> Result<(String, String), String> {
    let source = get_account(id).await?.source_account_id.ok_or("CHATGPT_PLAN_BINDING_REQUIRED")?;
    access_snapshot(id, force, true, Some(&source)).await
}

pub(crate) async fn inference_snapshot_for_source(id: &str, source: &str, force: bool) -> Result<(String, String), String> {
    access_snapshot(id, force, true, Some(source)).await
}

async fn access_snapshot(
    id: &str,
    force: bool,
    require_binding: bool,
    expected_source: Option<&str>,
) -> Result<(String, String), String> {
    let observed_revision = storage::read(id).await?.revision;
    let _guard = storage::lock(id).await?;
    let mut record = storage::read(id).await?;
    if !record.summary.connected {
        return Err("CHATGPT_PLAN_DISCONNECTED".into());
    }
    // A stale request's source is contextual evidence, not grant health. Reject
    // before the status writer so it cannot poison a newly bound authorization.
    if let Some(source) = expected_source {
        let snapshot = record.clone(); let source = source.to_owned();
        storage::blocking(move || binding::validate_record_source(&snapshot, &source)).await?;
    }
    let credentials_result = storage::blocking({
        let record = record.clone();
        move || {
            if require_binding {
                binding::validate_bound_record(&record)?;
            }
            record.credentials()
        }
    })
    .await;
    let mut credentials = match credentials_result {
        Ok(credentials) => credentials,
        Err(error) => {
            status::apply_result(&mut record, &Err(error.clone()));
            storage::blocking(move || {
                let _guard = _guard;
                status::save_record(&record)
            })
            .await?;
            status::emit_updated();
            return Err(error);
        }
    };
    if let Err(error) = http::scopes(&credentials.scopes.join(" ")) {
        status::apply_result(&mut record, &Err(error.clone()));
        storage::blocking(move || {
            let _guard = _guard;
            status::save_record(&record)
        })
        .await?;
        status::emit_updated();
        return Err(error);
    }
    if record.summary.needs_reauth {
        return Err("CHATGPT_PLAN_REAUTH_REQUIRED".into());
    }
    let now = chrono::Utc::now().timestamp();
    if !should_refresh(
        credentials.expires_at,
        credentials.earliest_refresh_at,
        force && observed_revision == record.revision,
        now,
    ) {
        return Ok((credentials.access_token, record.authorization_revision));
    }
    let response = http::token(&[
        ("grant_type", "refresh_token"),
        ("client_id", &credentials.client_id),
        ("refresh_token", &credentials.refresh_token),
        ("resource", http::RESOURCE),
    ])
    .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            crate::modules::logger::log_info(&format!(
                "[ChatGPT Plan] refresh failed for {id}: {error}"
            ));
            status::apply_result(&mut record, &Err(error.clone()));
            storage::blocking(move || {
                let _guard = _guard;
                status::save_record(&record)
            })
            .await?;
            status::emit_updated();
            return Err(error);
        }
    };
    let refresh_token = match response.refresh_token.filter(|token| !token.is_empty()) {
        Some(token) => token,
        None => {
            let error = "CHATGPT_PLAN_REAUTH_REQUIRED".to_string();
            status::apply_result(&mut record, &Err(error.clone()));
            storage::blocking(move || {
                let _guard = _guard;
                status::save_record(&record)
            })
            .await?;
            status::emit_updated();
            return Err(error);
        }
    };
    credentials.access_token = response.access_token;
    credentials.refresh_token = refresh_token;
    credentials.expires_at = chrono::Utc::now().timestamp() + response.expires_in;
    credentials.earliest_refresh_at = response.earliest_refresh_at;
    // The original, signature-verified ID token stays bound to this session.
    // An expired ID token remains a supported reauthorization hint; refresh
    // does not need to reinterpret opaque access-token claims or replace it.
    let scope_error = if let Some(scope) = response.scope {
        let missing_permission = http::scopes(&scope).is_err();
        credentials.scopes = scope.split_whitespace().map(str::to_string).collect();
        missing_permission
    } else {
        false
    };
    let token = credentials.access_token.clone();
    let revision = storage::blocking(move || {
        let _guard = _guard;
        record.set_credentials(&credentials)?;
        record.summary.needs_reauth = scope_error;
        status::apply_result(
            &mut record,
            &if scope_error {
                Err("CHATGPT_PLAN_PERMISSION_REQUIRED".into())
            } else {
                Ok(())
            },
        );
        status::save_record(&record)?;
        Ok(record.authorization_revision)
    })
    .await?;
    status::emit_updated();
    if scope_error {
        return Err("CHATGPT_PLAN_PERMISSION_REQUIRED".into());
    }
    Ok((token, revision))
}

fn should_refresh(expires_at: i64, earliest: Option<i64>, force: bool, now: i64) -> bool {
    // Honor server-directed earliest refresh while a token is still valid.
    if expires_at > now && earliest.is_some_and(|earliest| earliest > now) {
        return false;
    }
    force || expires_at <= now + 120
}

pub async fn list_models(id: &str) -> Result<Vec<ChatgptPlanModel>, String> {
    for attempt in 0..2 {
        let (token, revision) = access_snapshot(id, attempt == 1, false, None).await?;
        let fingerprint = token_fingerprint(&token);
        let response = http::client(25)?
            .get(http::resource_url("/models"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(http::network_error);
        if response
            .as_ref()
            .is_ok_and(|response| response.status() == reqwest::StatusCode::UNAUTHORIZED)
            && attempt == 0
        {
            continue;
        }
        let result = match response {
            Ok(response) => http::json(response)
                .await
                .and_then(|value| http::parse_models(&value)),
            Err(error) => Err(error),
        };
        status::record_result(
            id,
            Some(&revision),
            Some(&fingerprint),
            result.as_ref().map(|_| ()).map_err(Clone::clone),
            result.as_ref().ok().map(|models| models.iter().map(|model| model.id.clone()).collect()),
        )
        .await?;
        return result;
    }
    Err("CHATGPT_PLAN_REAUTH_REQUIRED".into())
}

pub async fn verify(id: &str, model: &str) -> Result<ChatgptPlanAccountSummary, String> {
    let models = list_models(id).await?;
    if !models.iter().any(|candidate| candidate.id == model) {
        return Err("CHATGPT_PLAN_MODEL_UNAVAILABLE".into());
    }
    let (token, starting_revision) = access_snapshot(id, false, false, None).await?;
    let mut response = http::client(45)?
        .post(http::resource_url("/responses"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "model": model,
            "input": [{"role":"user", "content":"Reply with OK."}],
            "store": false,
            "stream": true
        }))
        .send()
        .await
        .map_err(http::network_error)?;
    if !response.status().is_success() {
        http::json(response).await?;
        return Err("CHATGPT_PLAN_VERIFY_FAILED".into());
    }
    let mut stream = http::VerificationStream::new();
    while let Some(chunk) = response.chunk().await.map_err(http::network_error)? {
        stream.push(&chunk)?;
        if stream.completed {
            break;
        }
    }
    stream.finish()?;
    let _guard = storage::lock(id).await?;
    let mut record = storage::read(id).await?;
    if !record.summary.connected || record.summary.needs_reauth {
        return Err("CHATGPT_PLAN_DISCONNECTED".into());
    }
    // An in-flight validation must not bless a replacement authorization.
    if record.authorization_revision != starting_revision {
        return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
    }
    record.summary.verified_at = Some(chrono::Utc::now().timestamp());
    record.summary.verified_model = Some(model.to_string());
    record.summary.updated_at = chrono::Utc::now().timestamp();
    let summary = record.summary.clone();
    storage::blocking(move || storage::save(&record)).await?;
    Ok(summary)
}

pub async fn disconnect(id: &str) -> Result<ChatgptPlanDisconnectResult, String> {
    disconnect_local(id, false).await
}

pub(crate) async fn disconnect_and_unbind(id: &str) -> Result<ChatgptPlanDisconnectResult, String> {
    disconnect_local(id, true).await
}

async fn disconnect_local(id: &str, detach: bool) -> Result<ChatgptPlanDisconnectResult, String> {
    oauth::cancel_registration(id).await;
    let associations = if detach {
        Some(storage::binding_lock().await?)
    } else {
        None
    };
    let _guard = storage::lock(id).await?;
    let mut record = storage::read(id).await?;
    let credentials = storage::blocking({
        let record = record.clone();
        move || {
            if record.encrypted_credentials.is_some() {
                record.credentials().map(Some)
            } else {
                Ok(None)
            }
        }
    })
    .await
    .ok()
    .flatten();
    // Clear tokens durably before waiting for the remote service. The returned
    // warning accurately distinguishes local disconnect from confirmed revoke.
    record.encrypted_credentials = None;
    record.revision = uuid::Uuid::new_v4().to_string();
    record.authorization_revision = uuid::Uuid::new_v4().to_string();
    record.summary.connected = false;
    record.summary.needs_reauth = false;
    record.summary.check_error = None;
    record.summary.last_checked_at = None;
    record.summary.expires_at = None;
    record.summary.verified_at = None;
    record.summary.verified_model = None;
    if detach {
        record.source_binding_required = true;
        record.source_binding = None;
        record.summary.source_account_id = None;
    }
    record.summary.updated_at = chrono::Utc::now().timestamp();
    let local_record = record.clone();
    storage::blocking(move || {
        let _guard = _guard;
        let _associations = associations;
        status::save_record(&local_record)
    })
    .await?;
    status::emit_updated();
    let Some(credentials) = credentials else {
        return Ok(ChatgptPlanDisconnectResult {
            remote_revoked: record.revocation_confirmed,
        });
    };
    let remote_revoked = revoke(&credentials).await.is_ok();
    if !remote_revoked {
        crate::modules::logger::log_info(&format!("[ChatGPT Plan] local disconnect completed for {id}; remote revocation was not confirmed"));
    }
    if remote_revoked {
        let guard = storage::lock(id).await?;
        let id = id.to_owned();
        storage::blocking(move || {
            let _guard = guard;
            let mut latest = storage::load(&id)?;
            if latest.authorization_revision == record.authorization_revision
                && !latest.summary.connected
            {
                latest.revocation_confirmed = true;
                storage::save(&latest)?;
            }
            Ok::<_, String>(())
        })
        .await?;
    }
    Ok(ChatgptPlanDisconnectResult { remote_revoked })
}

async fn revoke(credentials: &types::Credentials) -> Result<(), String> {
    let discovery = http::discovery().await?;
    let endpoint = discovery
        .revocation_endpoint
        .ok_or("CHATGPT_PLAN_NETWORK")?;
    if !http::trusted_https(&endpoint) {
        return Err("CHATGPT_PLAN_NETWORK".into());
    }
    for attempt in 0..2 {
        let response = http::client(10)?
            .post(&endpoint)
            .form(&[
                ("token", credentials.refresh_token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", credentials.client_id.as_str()),
            ])
            .send()
            .await;
        match response {
            Ok(response) if response.status() == reqwest::StatusCode::OK => return Ok(()),
            Ok(response) if !response.status().is_server_error() => {
                return Err("CHATGPT_PLAN_NETWORK".into())
            }
            _ if attempt == 0 => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            _ => return Err("CHATGPT_PLAN_NETWORK".into()),
        }
    }
    Err("CHATGPT_PLAN_NETWORK".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_respects_expiry_skew_server_earliest_and_force() {
        assert!(!should_refresh(4600, None, false, 1000));
        assert!(should_refresh(1100, None, false, 1000));
        assert!(should_refresh(4600, None, true, 1000));
        assert!(!should_refresh(1100, Some(1050), true, 1000));
        assert!(should_refresh(999, Some(1050), false, 1000));
    }
}

#[cfg(test)]
#[path = "chatgpt_plan_lifecycle_tests.rs"]
mod lifecycle_tests;
