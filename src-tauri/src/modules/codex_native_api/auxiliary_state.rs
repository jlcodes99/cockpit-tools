//! Bounded caller-scoped state for realtime calls and local ephemeral credentials.
use super::super::{
    contract::{Caller, Host, Route, ServiceError},
    server::AppState,
};
use axum::http::HeaderMap;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct AuxiliaryAuth {
    pub caller: Caller,
    pub principal: String,
    pub session: Option<Value>,
    pub account_id: Option<String>,
    pub owner_id: Option<String>,
}

#[derive(Clone)]
pub(super) struct CallBinding {
    host: Weak<dyn Host>,
    caller_id: String,
    principal: String,
    pub account_id: String,
    pub owner_id: String,
    pub model: String,
    expires: Instant,
    // A WebRTC call owns its admission slot until hangup, disconnect or expiry.
    _permit: Option<Arc<dyn Send + Sync>>,
    connected: Arc<std::sync::atomic::AtomicBool>,
    stop: tokio::sync::watch::Sender<bool>,
}

#[derive(Clone)]
struct SecretBinding {
    host: Weak<dyn Host>,
    auth: AuxiliaryAuth,
    expires: Instant,
}
type CallKey = (usize, String, String, String);
static CALLS: LazyLock<Mutex<HashMap<CallKey, CallBinding>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SECRETS: LazyLock<Mutex<HashMap<String, SecretBinding>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn state_error() -> ServiceError {
    ServiceError::new(
        503,
        "realtime_state_unavailable",
        "Realtime state is unavailable",
    )
}
fn same_host(host: &Weak<dyn Host>, state: &AppState) -> bool {
    host.upgrade()
        .is_some_and(|host| Arc::ptr_eq(&host, &state.host))
}
pub(super) fn valid_call_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

pub(super) async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    allow_secret: bool,
) -> Result<AuxiliaryAuth, ServiceError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    if token.starts_with("ek_native_") {
        if !allow_secret {
            return Err(ServiceError::new(
                403,
                "realtime_client_secret_scope_mismatch",
                "Realtime credentials cannot authorize this operation",
            ));
        }
        let mut secrets = SECRETS.lock().map_err(|_| state_error())?;
        secrets.retain(|_, entry| entry.expires > Instant::now() && entry.host.strong_count() > 0);
        return secrets
            .get(token)
            .filter(|entry| same_host(&entry.host, state))
            .map(|entry| entry.auth.clone())
            .ok_or_else(|| {
                ServiceError::new(
                    401,
                    "invalid_realtime_client_secret",
                    "Realtime credential is invalid or expired",
                )
            });
    }
    let caller = state.authorize(headers).await?;
    Ok(AuxiliaryAuth {
        principal: caller.id.clone(),
        caller,
        session: None,
        account_id: None,
        owner_id: None,
    })
}

pub(super) fn create_secret(
    state: &AppState,
    auth: &AuxiliaryAuth,
    body: &Value,
    legacy: bool,
    route: &Route,
) -> Result<Value, ServiceError> {
    if auth.session.is_some() {
        return Err(ServiceError::new(
            403,
            "realtime_client_secret_scope_mismatch",
            "Realtime credentials cannot create more credentials",
        ));
    }
    if !body.is_object() {
        return Err(ServiceError::new(
            400,
            "invalid_session",
            "Session must be a JSON object",
        ));
    }
    let duration = match body.pointer("/expires_after/seconds") {
        Some(value) => value.as_u64().ok_or_else(|| {
            ServiceError::new(
                400,
                "invalid_expires_after",
                "Realtime credential duration must be a positive integer",
            )
        })?,
        None => 600,
    };
    if !(10..=7200).contains(&duration)
        || body
            .pointer("/expires_after/anchor")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty() && v != "created_at")
    {
        return Err(ServiceError::new(
            400,
            "invalid_expires_after",
            "Realtime credential duration must be between 10 and 7200 seconds",
        ));
    }
    let mut session = if legacy {
        body.clone()
    } else {
        body.get("session")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    if !session.is_object() {
        return Err(ServiceError::new(
            400,
            "invalid_session",
            "Session must be a JSON object",
        ));
    }
    if session
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.trim().is_empty() && v != "realtime")
    {
        return Err(ServiceError::new(
            501,
            "realtime_capability_not_supported",
            "This realtime session type is not supported",
        ));
    }
    session["type"] = json!("realtime");
    if session
        .get("model")
        .and_then(Value::as_str)
        .is_none_or(|v| v.trim().is_empty())
    {
        session["model"] = json!("gpt-realtime");
    }
    let token = format!(
        "ek_native_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut secrets = SECRETS.lock().map_err(|_| state_error())?;
    secrets.retain(|_, entry| entry.expires > Instant::now() && entry.host.strong_count() > 0);
    if secrets.len() >= 1024
        || secrets
            .values()
            .filter(|entry| same_host(&entry.host, state) && entry.auth.caller.id == auth.caller.id)
            .count()
            >= 64
    {
        return Err(ServiceError::new(
            429,
            "realtime_client_secret_capacity_exhausted",
            "Too many active realtime credentials",
        )
        .with_retry_after(Some(Duration::from_secs(1))));
    }
    secrets.insert(
        token.clone(),
        SecretBinding {
            host: Arc::downgrade(&state.host),
            auth: AuxiliaryAuth {
                caller: auth.caller.clone(),
                principal: token.clone(),
                session: Some(session.clone()),
                account_id: Some(route.continuity_id().to_owned()),
                owner_id: Some(route.owner_id.clone()),
            },
            expires: Instant::now() + Duration::from_secs(duration),
        },
    );
    let expires = chrono::Utc::now()
        .timestamp()
        .saturating_add(duration as i64);
    session["id"] = json!(format!("sess_{}", uuid::Uuid::new_v4().simple()));
    session["object"] = json!("realtime.session");
    session["expires_at"] = json!(expires);
    if legacy {
        session["client_secret"] = json!({"value":token,"expires_at":expires});
        Ok(session)
    } else {
        Ok(json!({"value":token,"expires_at":expires,"session":session}))
    }
}

pub(super) fn bind_call(
    state: &AppState,
    auth: &AuxiliaryAuth,
    route: &Route,
    id: &str,
    model: &str,
) -> Result<(), ServiceError> {
    if !valid_call_id(id) {
        return Err(ServiceError::new(
            502,
            "invalid_call_id",
            "Upstream returned an invalid call identifier",
        ));
    }
    let mut calls = CALLS.lock().map_err(|_| state_error())?;
    calls.retain(|_, entry| {
        let active = entry.expires > Instant::now() && entry.host.strong_count() > 0;
        if !active {
            entry.stop.send_replace(true);
        }
        active
    });
    if calls.len() >= 1024 {
        return Err(ServiceError::new(
            503,
            "realtime_capacity_exhausted",
            "Too many active realtime calls",
        ));
    }
    let key = call_key(state, auth, id);
    if calls.contains_key(&key) {
        return Err(ServiceError::new(
            409,
            "realtime_call_conflict",
            "Realtime call identifier is already in use",
        ));
    }
    let (stop, mut revoked) = tokio::sync::watch::channel(false);
    let expires = Instant::now() + Duration::from_secs(3600);
    calls.insert(
        key.clone(),
        CallBinding {
            host: Arc::downgrade(&state.host),
            caller_id: auth.caller.id.clone(),
            principal: auth.principal.clone(),
            account_id: route.continuity_id().to_owned(),
            owner_id: route.owner_id.clone(),
            model: model.into(),
            expires,
            _permit: route.permit.clone(),
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop,
        },
    );
    drop(calls);
    // This task holds neither the host nor the account permit. Listener shutdown
    // must revoke calls even when no client sends another request.
    let mut shutdown = state.stop.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = revoked.changed() => return,
            _ = shutdown.wait() => {},
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(expires)) => {},
        }
        remove_key(&key);
    });
    Ok(())
}

fn call_key(state: &AppState, auth: &AuxiliaryAuth, id: &str) -> CallKey {
    (
        Arc::as_ptr(&state.host) as *const () as usize,
        auth.caller.id.clone(),
        auth.principal.clone(),
        id.into(),
    )
}

pub(super) fn call(
    state: &AppState,
    auth: &AuxiliaryAuth,
    id: &str,
) -> Result<CallBinding, ServiceError> {
    if !valid_call_id(id) {
        return Err(ServiceError::new(
            400,
            "invalid_call_id",
            "Invalid realtime call identifier",
        ));
    }
    let calls = CALLS.lock().map_err(|_| state_error())?;
    let entry = calls
        .get(&call_key(state, auth, id))
        .filter(|entry| entry.expires > Instant::now() && same_host(&entry.host, state))
        .ok_or_else(|| {
            ServiceError::new(
                404,
                "realtime_call_not_found",
                "Realtime call was not found",
            )
        })?;
    if entry.caller_id != auth.caller.id || entry.principal != auth.principal {
        return Err(ServiceError::new(
            403,
            "realtime_call_scope_mismatch",
            "Realtime call belongs to another caller",
        ));
    }
    Ok(entry.clone())
}

pub(super) fn remove_call(state: &AppState, auth: &AuxiliaryAuth, id: &str) {
    remove_key(&call_key(state, auth, id));
}
fn remove_key(key: &CallKey) {
    if let Ok(mut calls) = CALLS.lock() {
        if let Some(binding) = calls.remove(key) {
            binding.stop.send_replace(true);
        }
    }
}

pub(super) struct CallConnection {
    key: CallKey,
    binding: CallBinding,
    active: bool,
}
impl CallConnection {
    pub fn activate(mut self) -> Self {
        self.active = true;
        self
    }
    pub fn stop(&self) -> tokio::sync::watch::Receiver<bool> {
        self.binding.stop.subscribe()
    }
}
impl Drop for CallConnection {
    fn drop(&mut self) {
        if let Ok(mut calls) = CALLS.lock() {
            if calls
                .get(&self.key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.connected, &self.binding.connected))
            {
                if self.active {
                    if let Some(binding) = calls.remove(&self.key) {
                        binding.stop.send_replace(true);
                    }
                } else {
                    self.binding
                        .connected
                        .store(false, std::sync::atomic::Ordering::Release);
                }
            }
        }
    }
}
pub(super) fn connect_call(
    state: &AppState,
    auth: &AuxiliaryAuth,
    id: &str,
) -> Result<CallConnection, ServiceError> {
    let binding = call(state, auth, id)?;
    if binding
        .connected
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return Err(ServiceError::new(
            409,
            "realtime_call_already_connected",
            "Realtime call already has an active connection",
        ));
    }
    Ok(CallConnection {
        key: call_key(state, auth, id),
        binding,
        active: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn call_identifiers_cannot_form_paths_or_urls() {
        for id in ["rtc_valid-1", "A"] {
            assert!(valid_call_id(id));
        }
        for id in ["", "../other", "https://host", "a?b", "a/b", "a%2fb"] {
            assert!(!valid_call_id(id));
        }
    }
}
