//! Edge protocol translations. Network, authentication and account routing remain in Responses.
//! Contracts: platform.claude.com/docs/en/api/messages, ai.google.dev/api/generate-content,
//! and docs.ollama.com/api/chat. Token-count endpoints label local estimates explicitly.
use super::contract::ServiceError;
use serde_json::Value;
#[path = "compat_native_chat.rs"]
mod native_chat;
#[path = "compat_output.rs"]
mod output;
#[path = "compat_request.rs"]
mod request;
#[path = "compat_search.rs"]
mod search;
#[path = "compat_signature.rs"]
mod signature;
#[path = "compat_stream.rs"]
mod stream;
#[path = "compat_tools.rs"]
mod tools;
pub use native_chat::{
    prepare_native_chat, response_from_chat, NativeChatRequest, NativeChatStreamAdapter,
};
#[cfg(test)]
pub use output::count_tokens_response;
pub use output::{models, ollama_show, response_from_responses};
pub use request::{count_tokens_request, prepare_request};
pub use stream::CompatStreamAdapter;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatFlavor {
    Claude,
    Gemini,
    Ollama,
}
#[derive(Clone, Debug)]
pub struct CompatContext {
    pub flavor: CompatFlavor,
    pub model: String,
    pub tool_names: std::collections::BTreeMap<String, String>,
}
impl CompatContext {
    pub(crate) fn tool_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.tool_names
            .get(name)
            .map(String::as_str)
            .unwrap_or(name)
    }
}
pub struct CompatRequest {
    pub responses: Value,
    pub context: CompatContext,
    pub stream: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CompatFrame {
    pub event: Option<String>,
    pub data: Value,
}
pub(crate) fn invalid(field: &str, reason: &str) -> ServiceError {
    ServiceError::new(
        400,
        "invalid_compat_parameter",
        format!("{field}: {reason}"),
    )
}
pub(crate) fn unsupported(field: &str) -> ServiceError {
    ServiceError::new(
        400,
        "compat_parameter_not_representable",
        format!("{field} has no equivalent setting in the selected Responses engine"),
    )
}
pub(crate) fn upstream(message: &str) -> ServiceError {
    ServiceError::new(502, "invalid_compat_upstream_response", message)
}
pub fn error(error: &ServiceError, flavor: CompatFlavor) -> Value {
    use serde_json::json;
    let public = super::output_error::json_error(error);
    let details = &public["error"];
    let message = details["message"].clone();
    match flavor {
        CompatFlavor::Claude => {
            let parsed = serde_json::from_str::<Value>(&error.message).ok();
            let original = error.response_error.as_ref().or_else(|| {
                parsed.as_ref().and_then(|value| {
                    value
                        .pointer("/response/error")
                        .or_else(|| value.get("error"))
                })
            });
            let upstream_type = original
                .and_then(|value| value["type"].as_str())
                .filter(|kind| !kind.trim().is_empty())
                .and_then(|_| details["type"].as_str())
                .map(str::trim);
            let kind =
                if details["code"] == "cyber_policy" || upstream_type == Some("invalid_request") {
                    "invalid_request_error"
                } else {
                    upstream_type.unwrap_or(match error.status {
                        401 => "authentication_error",
                        402 => "billing_error",
                        403 => "permission_error",
                        404 => "not_found_error",
                        413 => "request_too_large",
                        429 => "rate_limit_error",
                        504 => "timeout_error",
                        529 => "overloaded_error",
                        500..=599 => "api_error",
                        _ => "invalid_request_error",
                    })
                };
            let mut details = details.clone();
            details["type"] = json!(kind);
            json!({"type":"error","error":details})
        }
        CompatFlavor::Gemini => {
            json!({"error":{"code":error.status,"message":message,"status":match error.status{401=>"UNAUTHENTICATED",403=>"PERMISSION_DENIED",404=>"NOT_FOUND",429=>"RESOURCE_EXHAUSTED",400..=499=>"INVALID_ARGUMENT",_=>"INTERNAL"}}})
        }
        CompatFlavor::Ollama => json!({"error":message}),
    }
}

#[cfg(test)]
#[path = "compat_hosted_tests.rs"]
mod hosted_tests;
#[cfg(test)]
#[path = "compat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "compat_output_parity_tests.rs"]
mod output_parity_tests;
