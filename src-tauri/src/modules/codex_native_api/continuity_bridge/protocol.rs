//! Private loopback DTOs. Headers and events retain their original wire values.
use std::collections::BTreeMap;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::contract::ServiceError;

pub type Headers = BTreeMap<String, Vec<String>>;

fn enabled() -> bool {
    true
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PrepareRequest {
    pub request_id: String,
    pub caller: String,
    #[serde(default)]
    pub headers: Headers,
    pub body: Value,
    #[serde(default)]
    pub compact: bool,
    #[serde(default = "enabled")]
    pub allow_rebuild: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PrepareResponse {
    pub ticket: String,
    pub preferred_account_id: Option<String>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub self_contained: bool,
    pub headers: Headers,
    pub body: Value,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct BindRequest {
    pub ticket: String,
    pub attempt_id: String,
    pub account_id: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    /// Explicit selected-transport capability; absent for old workers and WS/API keys.
    #[serde(default)]
    pub expand_http_previous: bool,
    #[serde(default)]
    pub headers: Headers,
    pub body: Value,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct BindResponse {
    pub ticket: String,
    pub attempt_id: String,
    pub headers: Headers,
    pub body: Value,
    pub rebuild_applied: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ObserveRequest {
    pub ticket: String,
    pub attempt_id: String,
    pub sequence: u64,
    #[serde(default)]
    pub headers: Option<Headers>,
    #[serde(default)]
    pub events: Vec<Value>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ObserveResponse {
    pub next_sequence: u64,
    pub terminal: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct FinishRequest {
    pub ticket: String,
    /// Empty only when preparation failed before an account could be bound.
    #[serde(default)]
    pub attempt_id: String,
    pub sequence: u64,
    pub status: FinishStatus,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct FinishResponse {}

#[derive(Clone, Deserialize, Serialize)]
pub struct ReleaseRequest {
    pub ticket: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ReleaseResponse {}

pub(super) fn decode_headers(values: &Headers) -> Result<HeaderMap, ServiceError> {
    let mut result = HeaderMap::new();
    for (name, values) in values {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            ServiceError::new(
                400,
                "invalid_continuity_headers",
                "Invalid protocol header name",
            )
        })?;
        for value in values {
            result.append(
                name.clone(),
                HeaderValue::from_str(value).map_err(|_| {
                    ServiceError::new(
                        400,
                        "invalid_continuity_headers",
                        "Invalid protocol header value",
                    )
                })?,
            );
        }
    }
    Ok(result)
}

pub(super) fn encode_headers(values: &HeaderMap) -> Result<Headers, ServiceError> {
    let mut result = Headers::new();
    for (name, value) in values {
        result.entry(name.to_string()).or_default().push(
            value
                .to_str()
                .map_err(|_| {
                    ServiceError::new(
                        400,
                        "invalid_continuity_headers",
                        "Non-text protocol header",
                    )
                })?
                .to_owned(),
        );
    }
    Ok(result)
}
