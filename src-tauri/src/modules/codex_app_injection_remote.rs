//! Adapter only: all metrics and filtering are computed by the same authenticated
//! Key APIs used by the host's account details. No local pool/log fallback.
#[cfg(test)]
#[path = "codex_app_injection_remote_tests.rs"]
mod tests;
use super::{
    dialog_data::{self, DialogQuery},
    output_speed::{OutputSpeedSelection, SpeedRead, ThreadOutputSpeed},
};
use crate::modules::codex_remote_service::{self as remote, KeyQuery};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) struct Endpoint {
    pub base: String,
    pub key: String,
    pub binding: String,
    pub scope: super::recent_account::RequestScope,
    pub version: u32,
}
pub(super) fn endpoint(profile: &Path) -> Result<Option<Endpoint>, String> {
    let Some((base, key, scope)) = super::recent_account::profile_gateway(profile)? else {
        return Ok(None);
    };
    let Some(connection) = remote::cached_memory(&base, &key) else {
        return Ok(None);
    };
    let mut hash = Sha256::new();
    for value in [
        profile.to_string_lossy().as_ref(),
        &base,
        &key,
        &connection.service_id,
        scope.instance_id.as_deref().unwrap_or(""),
        scope.installation_id.as_deref().unwrap_or(""),
    ] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    Ok(Some(Endpoint {
        base,
        key,
        binding: format!("{:x}", hash.finalize()),
        scope,
        version: connection.key_views_version,
    }))
}
fn query(endpoint: &Endpoint, thread: Option<&str>) -> KeyQuery {
    KeyQuery {
        thread_id: thread.map(str::to_owned),
        instance_id: thread.and(endpoint.scope.instance_id.clone()),
        // Profiles without a sent installation header must not invent one.
        installation_id: thread.and(endpoint.scope.installation_id.clone()),
        ..Default::default()
    }
}
fn reason(error: &str) -> &'static str {
    if error.contains("UNAVAILABLE") || error.contains("HTTP_501") {
        "remote_unsupported"
    } else {
        "remote_unreachable"
    }
}
pub(super) fn dialog(
    profile: &Path,
    endpoint: &Endpoint,
    input: &DialogQuery,
) -> Result<Value, String> {
    let mut output = dialog_data::response(input, false);
    if input
        .binding
        .as_ref()
        .is_some_and(|b| b != &endpoint.binding)
    {
        output["unavailableReason"] = json!("binding_changed");
        return Ok(output);
    }
    if endpoint.version < 1 {
        output["unavailableReason"] = json!("remote_unsupported");
        return Ok(output);
    }
    let now = chrono::Utc::now().timestamp_millis();
    let day = 86_400_000;
    let mut request = query(
        endpoint,
        if input.scope == "chat" {
            input.thread_id.as_deref()
        } else {
            None
        },
    );
    request.start_at = Some(now / day * day - if input.period == "week" { 6 * day } else { 0 });
    request.end_at = Some((now / day + 1) * day);
    request.page_size = Some(10);
    if let Some(cursor) = &input.cursor {
        let cursor: Value = serde_json::from_str(cursor).map_err(|_| "invalid remote cursor")?;
        if cursor["binding"].as_str() != Some(&endpoint.binding[..16]) {
            return Err("remote cursor binding changed".into());
        }
        request.before_time = cursor["timestamp"].as_i64();
        request.before_id = cursor["id"].as_i64();
    }
    let result = tauri::async_runtime::block_on(async {
        if input.view == "logs" || input.view == "openLogs" {
            if input.scope == "chat" && input.thread_id.is_none() {
                return Ok(json!({"ready":false,"unlinked":true}));
            }
            remote::key_data(&endpoint.base, &endpoint.key, "requests", &request).await
        } else {
            let mut stats = request.clone();
            stats.thread_id = None;
            stats.instance_id = None;
            stats.installation_id = None;
            let (summary, session) = tokio::join!(
                remote::key_data(&endpoint.base, &endpoint.key, "stats", &stats),
                async {
                    if input.thread_id.is_some() {
                        let mut session = query(endpoint, input.thread_id.as_deref());
                        session.model = None;
                        remote::key_data(&endpoint.base, &endpoint.key, "session", &session).await
                    } else {
                        Ok(json!({"ready":true,"session":null}))
                    }
                }
            );
            let mut summary = summary?;
            let session = session?;
            summary["session"] = session["session"].clone();
            if session["ready"] == false {
                summary["sessionPending"] = json!(true);
            }
            Ok(summary)
        }
    });
    if super::output_speed::binding_snapshot(profile)?.as_deref() != Some(&endpoint.binding) {
        output["unavailableReason"] = json!("binding_changed");
        return Ok(output);
    }
    let value = match result {
        Ok(v) => v,
        Err(error) => {
            output["unavailableReason"] = json!(reason(&error));
            return Ok(output);
        }
    };
    if value["ready"] != true {
        output["unavailableReason"] = json!(if value["unlinked"] == true {
            "unlinked_session"
        } else {
            "statistics_preparing"
        });
        return Ok(output);
    }
    output["available"] = json!(true);
    output["remote"] = json!(true);
    if input.view == "overview" {
        let usage = &value["summary"];
        let count = usage["requestCount"].as_f64().unwrap_or(0.0);
        output["summary"] = json!({"requests":usage["requestCount"],"successRequests":usage["successCount"],
            "totalTokens":usage["totalTokens"],"estimatedCostUsd":usage["estimatedCostUsd"],
            "averageLatencyMs":usage["totalLatencyMs"].as_f64().filter(|_|count>0.0).map(|v|v/count),
            "sinceMs":value["startAt"],"untilMs":value["endAt"]});
        output["session"] = value["session"].clone();
        if output["session"].is_object() {
            output["session"]["threadId"] = json!(input.thread_id);
        }
    } else {
        let rows=value["items"].as_array().into_iter().flatten().map(|r|json!({
            "id":r["id"],"timestamp":r["timestamp"],"requestId":r["requestId"],"model":r["model"],
            "requestedModel":r["model"],"success":r["outcome"]=="success","httpStatus":r["httpStatus"],
            "requestKind":r["kind"],
            "inputTokens":r["inputTokens"],"outputTokens":r["outputTokens"],"cachedTokens":r["cachedTokens"],"totalTokens":r["totalTokens"],
            "latencyMs":r["latencyMs"],"firstResponseMs":r["firstResponseMs"],"outputGenerationMs":r["outputGenerationMs"],
            "errorCategory":if r["outcome"]=="canceled"{"client_canceled"}else if r["outcome"]=="incomplete"{"stream_incomplete"}else{""}
        })).collect::<Vec<_>>();
        let next = &value["nextCursor"];
        let cursor = if next.is_object() {
            Some(
                json!({"timestamp":next["beforeTime"],"id":next["beforeId"],
            "snapshotId":next["beforeId"],"binding":&endpoint.binding[..16]})
                .to_string(),
            )
        } else {
            None
        };
        output["requests"] = json!({"rows":rows,"nextCursor":cursor});
    }
    Ok(output)
}

pub(super) fn speed(
    profile: &Path,
    endpoint: &Endpoint,
    thread: &str,
    selection: &OutputSpeedSelection,
) -> SpeedRead {
    let selection = selection.normalized();
    let mut request = query(endpoint, Some(thread));
    request.model = selection.model.clone();
    request.service_tier = selection.service_tier.clone();
    let result = tauri::async_runtime::block_on(remote::key_data(
        &endpoint.base,
        &endpoint.key,
        "session",
        &request,
    ));
    let result = result.and_then(|value| {
        if super::output_speed::binding_snapshot(profile)?.as_deref() != Some(&endpoint.binding) {
            return Ok(None);
        }
        if value["ready"] != true {
            return Err("remote statistics preparing".into());
        }
        let metrics = &value["metrics"];
        let selected = &metrics["selected"];
        if metrics.is_null() {
            return Ok(None);
        }
        let Some(rate) = metrics["tokensPerSecond"].as_f64() else {
            return Ok(None);
        };
        Ok(Some(ThreadOutputSpeed {
            thread_id: thread.into(),
            binding: Some(endpoint.binding.clone()),
            tokens_per_second: rate,
            samples: metrics["samples"].as_u64().unwrap_or(0) as u32,
            observed_at_ms: metrics["observedAt"].as_i64().unwrap_or(0),
            latest_tokens_per_second: selected["tokensPerSecond"].as_f64(),
            latest_input_tokens: selected["inputTokens"].as_i64(),
            latest_cached_tokens: selected["cachedTokens"].as_i64(),
            latest_cache_hit_rate: selected["inputTokens"]
                .as_f64()
                .filter(|n| *n > 0.0)
                .zip(selected["cachedTokens"].as_f64())
                .map(|(i, c)| c / i),
            avg_cache_hit_rate: metrics["averageCacheHitRate"].as_f64(),
            total_input_tokens: metrics["totalInputTokens"].as_i64(),
            total_cached_tokens: metrics["totalCachedTokens"].as_i64(),
            latest_observed_at_ms: selected["timestamp"].as_i64(),
            request_id: selected["requestId"].as_str().map(str::to_owned),
            model: selected["model"].as_str().map(str::to_owned),
            requested_service_tier: selected["requestedServiceTier"].as_str().map(str::to_owned),
            response_service_tier: selected["responseServiceTier"].as_str().map(str::to_owned),
            selection: selection.clone(),
        }))
    });
    let transient_failure = result.is_err();
    SpeedRead {
        binding: Some(endpoint.binding.clone()),
        selection,
        result,
        transient_failure,
    }
}
