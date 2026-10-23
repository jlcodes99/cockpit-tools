//! Chat/Completions compatibility at the edge of the native Responses engine.
//! Protocol definitions: OpenAI's Chat streaming reference and Responses migration guide.
use serde_json::Value;

use super::contract::ServiceError;

#[path = "chat_output.rs"]
mod output;
#[path = "chat_request.rs"]
mod request;
#[path = "chat_stream.rs"]
mod stream;
#[path = "chat_tools.rs"]
mod tools;

pub use output::response_to_chat;
pub(crate) use output::{chat_usage, finish_reason, image_mime};
pub use request::prepare_request;
pub use stream::ChatStreamAdapter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatFlavor {
    ChatCompletions,
    Completions,
}

#[derive(Clone, Debug)]
pub struct ChatContext {
    pub flavor: ChatFlavor,
    pub model: String,
    pub include_usage: bool,
    pub legacy_functions: bool,
    /// Request-local alias -> client name, also used by streaming tool deltas.
    pub tool_names: std::collections::BTreeMap<String, String>,
}

impl ChatContext {
    pub(crate) fn tool_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.tool_names
            .get(name)
            .map(String::as_str)
            .unwrap_or(name)
    }
}

pub struct ChatRequest {
    pub responses: Value,
    pub context: ChatContext,
    pub stream: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamOutput {
    Event(Value),
    Done,
}

pub(crate) fn invalid(field: &str, reason: &str) -> ServiceError {
    ServiceError::new(400, "invalid_chat_parameter", format!("{field}: {reason}"))
}

pub(crate) fn incompatible(field: &str, reason: &str) -> ServiceError {
    ServiceError::new(
        400,
        "chat_parameter_not_representable",
        format!("{field}: {reason}"),
    )
}

pub(crate) fn upstream_error(reason: impl Into<String>) -> ServiceError {
    ServiceError::new(502, "invalid_chat_upstream_response", reason)
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "chat_output_parity_tests.rs"]
mod output_parity_tests;
