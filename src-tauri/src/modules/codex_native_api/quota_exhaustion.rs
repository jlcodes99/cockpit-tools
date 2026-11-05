//! Provider-confirmed quota exhaustion is independent of transient throttling.
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{
    contract::{OfficialFallbackFailure, Route, ServiceError},
    deadline::{self, Deadline},
    lifecycle::{stopping_error, RequestTracker},
    server::AppState,
};

pub(super) fn code(value: &Value) -> Option<&'static str> {
    let error = super::diagnostics::error_object(value);
    [error.get("code"), error.get("type")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|code| match code.trim() {
            "usage_limit_reached" => Some("usage_limit_reached"),
            "insufficient_quota" => Some("insufficient_quota"),
            "quota_exceeded" => Some("quota_exceeded"),
            "subscription_sharing_usage_limit_exceeded" => {
                Some("subscription_sharing_usage_limit_exceeded")
            }
            _ => None,
        })
}

pub(super) fn record<'a>(
    state: &'a AppState,
    tracker: &'a mut RequestTracker,
    route: &'a Route,
    code: &'static str,
    status: u16,
    retry_after: Option<Duration>,
    deadline: Deadline,
) -> BoxFuture<'a, Result<(), ServiceError>> {
    Box::pin(async move {
        if !tracker
            .quota_exhausted_accounts
            .insert(route.account_id.clone())
        {
            return Ok(());
        }
        let observation = async {
            if route.chatgpt_plan {
                let failure = OfficialFallbackFailure {
                    trigger_stage: "upstream",
                    status: 429,
                    code: code.into(),
                    message: None,
                    retry_after,
                };
                state
                    .host
                    .observe_official_failure(route, &route.model, &failure)
                    .await;
            } else {
                state
                    .host
                    .route_quota_exhausted(route, status, retry_after)
                    .await;
            }
        };
        let mut stop = state.stop.clone();
        tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = deadline::timeout_at(deadline.cap(tokio::time::Instant::now() + state.options.request_timeout),
                observation) =>
                    result.map_err(|_| super::retry::total_timeout()),
        }
    })
}
