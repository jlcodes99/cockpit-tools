//! Transport-neutral diagnostics and the authenticated local management surface.
use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::Method,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{contract::ServiceError, server::AppState};

#[path = "diagnostic_error.rs"]
mod error;
pub(crate) use error::error_object;
pub(crate) use error::is_cloudflare_challenge;
pub(crate) use error::upstream_error_metadata_status;
pub(super) use error::{
    failure_phase, sanitize_error_text, transport_failure, upstream_error_metadata,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticContext {
    pub request_id: String,
    #[serde(default)]
    pub client_identity_metadata_captured: bool,
    #[serde(default)]
    pub client_installation_id: Option<String>,
    #[serde(default)]
    pub client_chatgpt_account_id: Option<String>,
    pub client_instance_id: Option<String>,
    #[serde(default)]
    pub client_identity_captured: bool,
    #[serde(default)]
    pub client_session_id: Option<String>,
    #[serde(default)]
    pub client_thread_id: Option<String>,
    /// Internal routing feedback only; displayed header identities stay verbatim.
    #[serde(skip)]
    pub routing_identity: Option<super::contract::RoutingIdentity>,
    /// Internal failure classification, independent of optional payload logging.
    #[serde(skip)]
    pub upstream_request_scoped: bool,
    pub endpoint: String,
    pub method: String,
    pub started_at_ms: i64,
    pub first_response_ms: Option<u64>,
    pub output_generation_ms: Option<u64>,
    pub proxy_route: Option<Value>,
    pub failure_phase: Option<String>,
    pub attempts: Vec<DiagnosticAttempt>,
    pub payloads: Vec<DiagnosticPayload>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticAttempt {
    pub authorization_route: Option<String>,
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_pool_retired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_http_version: Option<String>,
    #[serde(default)]
    pub upstream_identity_metadata_captured: bool,
    #[serde(default)]
    pub upstream_installation_id: Option<String>,
    #[serde(default)]
    pub upstream_chatgpt_account_id: Option<String>,
    #[serde(default)]
    pub account_identity: Option<crate::modules::codex_api_identity::InstallationProfile>,
    pub account_id: String,
    pub account_email: Option<String>,
    pub model_id: String,
    pub transport: String,
    #[serde(default)]
    pub upstream_identity_captured: bool,
    #[serde(default)]
    pub upstream_session_id: Option<String>,
    #[serde(default)]
    pub upstream_thread_id: Option<String>,
    pub started_at_ms: i64,
    pub latency_ms: u64,
    pub status: Option<u16>,
    pub success: bool,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
    pub failure_phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_fallback: Option<CapabilityFallbackTrace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_fallback: Option<AuthorizationFallbackTrace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_adaptation: Option<crate::models::codex_local_access::CodexLocalAccessServiceTierAdaptation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_request_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationFallbackTrace {
    #[serde(default)]
    pub trigger_stage: String,
    pub reason: String,
    pub from_account_id: String,
    pub to_account_id: String,
    pub from_route: String,
    pub to_route: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityFallbackTrace {
    pub reason: String,
    pub capabilities: Vec<String>,
    pub from_account_id: String,
    pub from_model: String,
    pub to_account_id: String,
    pub to_model: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct HeaderIdentity {
    pub captured: bool,
    pub session: Option<String>,
    pub thread: Option<String>,
    pub metadata_captured: bool,
    pub installation: Option<String>,
    pub account: Option<String>,
}

pub(super) fn capture_header_identity(headers: &axum::http::HeaderMap) -> HeaderIdentity {
    fn identity(value: Option<&axum::http::HeaderValue>) -> Result<Option<String>, ()> {
        let Some(value) = value else {
            return Ok(None);
        };
        let value = std::str::from_utf8(value.as_bytes()).map_err(|_| ())?;
        if value.is_empty() {
            return Ok(None);
        }
        if value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(());
        }
        Ok(Some(value.to_owned()))
    }
    let session_header = headers
        .get("session-id")
        .filter(|value| !value.as_bytes().is_empty())
        .or_else(|| headers.get("session_id"));
    let session = identity(session_header);
    let thread = identity(headers.get("thread-id"));
    let mut snapshot = match (session, thread) {
        (Ok(session), Ok(thread)) => HeaderIdentity {
            captured: true,
            session,
            thread,
            ..Default::default()
        },
        _ => HeaderIdentity::default(),
    };
    if let (Ok(installation), Ok(account)) = (
        identity(headers.get("x-codex-installation-id")),
        identity(headers.get("chatgpt-account-id")),
    ) {
        snapshot.metadata_captured = true;
        snapshot.installation = installation;
        snapshot.account = account;
    }
    snapshot
}

#[cfg(test)]
#[path = "diagnostic_identity_tests.rs"]
mod identity_tests;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPayload {
    pub stage: String,
    pub attempt_sequence: Option<u32>,
    pub transport: String,
    pub content_type: String,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub truncated: bool,
    pub original_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagementRequest {
    DiagnosticsConfig {
        enabled: bool,
    },
    DiagnosticsEvents,
    ResetAuth {
        account_ids: Vec<String>,
    },
    ResetScheduler {
        account_ids: Vec<String>,
    },
    Quota,
    ServiceTier {
        speed: String,
        revision: u64,
        loopback: bool,
    },
}

impl ManagementRequest {
    pub fn requires_internal_key(&self) -> bool {
        matches!(
            self,
            Self::DiagnosticsConfig { .. } | Self::DiagnosticsEvents | Self::ServiceTier { .. }
        )
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticsConfig {
    request_payload_logging: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetAccounts {
    account_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceTier {
    speed: String,
    revision: u64,
}

fn invalid_request() -> ServiceError {
    ServiceError::new(400, "invalid_request", "Invalid management request")
}

fn parse_management(
    method: &Method,
    path: &str,
    body: &[u8],
    loopback: bool,
) -> Result<ManagementRequest, ServiceError> {
    let post = *method == Method::POST;
    match path {
        "/v1/cockpit/diagnostics/config" if post => {
            let input: DiagnosticsConfig =
                serde_json::from_slice(body).map_err(|_| invalid_request())?;
            Ok(ManagementRequest::DiagnosticsConfig {
                enabled: input.request_payload_logging,
            })
        }
        "/v1/cockpit/diagnostics/events" if *method == Method::GET => {
            Ok(ManagementRequest::DiagnosticsEvents)
        }
        "/v1/cockpit/quota" if *method == Method::GET => Ok(ManagementRequest::Quota),
        "/v1/cockpit/auth/reset" | "/v1/cockpit/accounts/reset-scheduler" if post => {
            let input: ResetAccounts =
                serde_json::from_slice(body).map_err(|_| invalid_request())?;
            if input.account_ids.is_empty() || input.account_ids.len() > 1024 {
                return Err(invalid_request());
            }
            let mut account_ids = Vec::new();
            for id in input.account_ids {
                let id = id.trim();
                if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                    return Err(invalid_request());
                }
                if !account_ids.iter().any(|existing| existing == id) {
                    account_ids.push(id.to_string());
                }
            }
            if path.ends_with("auth/reset") {
                Ok(ManagementRequest::ResetAuth { account_ids })
            } else {
                Ok(ManagementRequest::ResetScheduler { account_ids })
            }
        }
        "/v1/cockpit/service-tier-default" if post => {
            let input: ServiceTier = serde_json::from_slice(body).map_err(|_| invalid_request())?;
            if input.revision == 0
                || !matches!(input.speed.as_str(), "standard" | "fast" | "ultrafast")
            {
                return Err(invalid_request());
            }
            Ok(ManagementRequest::ServiceTier {
                speed: input.speed,
                revision: input.revision,
                loopback,
            })
        }
        _ => Err(ServiceError::new(
            404,
            "not_found",
            "Management endpoint not found",
        )),
    }
}

/// Register the six exact management paths with their corresponding GET/POST methods.
/// ConnectInfo comes from the accepted socket, never Forwarded/X-Forwarded-For.
pub(super) async fn management(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Result<Json<Value>, ServiceError> {
    let _tracker = state.begin()?;
    let caller = state.authorize(request.headers()).await?;
    let loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback());
    let (parts, body) = request.into_parts();
    let mut stop = state.stop.clone();
    let body = tokio::select! {
        _ = stop.wait() => return Err(super::lifecycle::stopping_error()),
        result = tokio::time::timeout(state.options.request_timeout, to_bytes(body, 16 * 1024)) => {
            result.map_err(|_| ServiceError::new(408, "request_body_timeout", "Management request timed out"))?
                .map_err(|_| ServiceError::new(413, "request_too_large", "Management request is too large"))?
        }
    };
    let request = parse_management(&parts.method, parts.uri.path(), &body, loopback)?;
    tokio::select! {
        _ = stop.wait() => Err(super::lifecycle::stopping_error()),
        result = tokio::time::timeout(state.options.request_timeout, state.host.management(&caller, request)) =>
            result.map_err(|_| ServiceError::new(504, "management_timeout", "Management operation timed out"))?.map(Json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_requests_require_exact_fields_and_real_loopback() {
        assert!(parse_management(
            &Method::POST,
            "/v1/cockpit/diagnostics/config",
            br#"{"requestPayloadLogging":true}"#,
            false
        )
        .unwrap()
        .requires_internal_key());
        for body in [
            br#"{}"#.as_slice(),
            br#"{"requestPayloadLogging":true,"accountIds":["victim"]}"#,
        ] {
            assert!(
                parse_management(&Method::POST, "/v1/cockpit/diagnostics/config", body, false)
                    .is_err()
            );
        }
        assert_eq!(
            parse_management(
                &Method::POST,
                "/v1/cockpit/service-tier-default",
                br#"{"speed":"fast","revision":2}"#,
                false
            )
            .unwrap(),
            ManagementRequest::ServiceTier {
                speed: "fast".into(),
                revision: 2,
                loopback: false
            }
        );
        assert!(parse_management(
            &Method::POST,
            "/v1/cockpit/service-tier-default",
            br#"{"speed":"fast","revision":0}"#,
            true
        )
        .is_err());
    }

    #[test]
    fn reset_ids_are_bounded_nonempty_and_deduplicated() {
        assert_eq!(
            parse_management(
                &Method::POST,
                "/v1/cockpit/accounts/reset-scheduler",
                br#"{"accountIds":["a"," a "]}"#,
                false
            )
            .unwrap(),
            ManagementRequest::ResetScheduler {
                account_ids: vec!["a".into()]
            }
        );
        assert!(parse_management(
            &Method::POST,
            "/v1/cockpit/auth/reset",
            br#"{"accountIds":[]}"#,
            true
        )
        .is_err());
        assert!(parse_management(&Method::GET, "/v1/cockpit/auth/reset", b"", true).is_err());
        assert!(!ManagementRequest::Quota.requires_internal_key());
    }

    #[test]
    fn management_accepts_all_three_speed_choices_and_rejects_unknown_values() {
        for speed in ["standard", "fast", "ultrafast"] {
            let body = serde_json::json!({"speed":speed,"revision":1}).to_string();
            let request = parse_management(
                &Method::POST,
                "/v1/cockpit/service-tier-default",
                body.as_bytes(),
                true,
            )
            .unwrap();
            assert!(request.requires_internal_key());
            assert_eq!(
                request,
                ManagementRequest::ServiceTier {
                    speed: speed.into(),
                    revision: 1,
                    loopback: true
                }
            );
        }
        assert!(parse_management(
            &Method::POST,
            "/v1/cockpit/service-tier-default",
            br#"{"speed":"turbo","revision":1}"#,
            true
        )
        .is_err());
    }
}
