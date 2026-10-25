//! Native, in-process Responses API. The host owns accounts and policy; this module owns transport.
//!
//! Wire baseline: openai/codex f82e36592278a0a782fb0d0fc97e5fa10e832ac2.
//! See core/src/client.rs, core/src/responses_metadata.rs and
//! core/tests/suite/prompt_cache_key.rs in that revision. Session cache routing,
//! logical thread identities and server-issued continuation state are distinct.
mod agent_communication;
mod account_installation;
mod auxiliary;
mod blocking;
mod capabilities;
#[cfg(test)]
mod capability_fallback_http_tests;
mod bootstrap;
mod chat;
mod chat_upstream;
mod chatgpt_plan_request;
mod chatgpt_plan_wire;
pub(super) mod chatgpt_plan_tools;
mod client_tool_discovery;
mod codex_wire;
mod codex_cpa_wire;
mod compat;
mod compat_http;
mod compat_options;
mod connection_error;
pub mod contract;
pub mod continuity_bridge;
pub mod diagnostics;
mod deadline;
mod edge;
mod events;
pub(crate) mod execution;
pub(crate) mod gpt_thinking;
pub(crate) mod grok_thinking;
mod grok_errors;
mod grok_wire;
mod http;
pub mod identity;
mod images;
mod lifecycle;
mod mapping;
mod multi_agent_compat;
mod multi_agent_transport;
#[cfg(test)]
mod legacy_agent_wire_tests;
#[cfg(test)]
mod multi_agent_lifecycle_http_tests;
#[cfg(test)]
mod tls_recovery_http_tests;
mod native_chat;
mod official_fallback;
#[cfg(test)]
mod official_fallback_http_tests;
mod output_error;
mod output_items;
mod output_usage;
pub mod recovery;
pub mod rate_limits;
mod request_body;
mod responses_host_results;
mod responses_tool_history;
mod retry;
mod server;
mod token_count;
mod transport;
mod upstream;
pub(crate) mod upstream_errors;
mod websocket;

pub use server::{start, Handle, Options};

#[cfg(test)]
pub(crate) use compat::ollama_show as compat_catalog_show_for_test;

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod diagnostic_identity_http_tests;

#[cfg(test)]
mod catalog_deadline_tests;

#[cfg(test)]
mod agent_communication_http_tests;

#[cfg(test)]
mod context_rebuild_http_tests;

#[cfg(test)]
mod http_checkpoint_tests;

#[cfg(test)]
mod agent_communication_plaintext_tests;

#[cfg(test)]
mod grok_tool_search_http_tests;
#[cfg(test)]
mod grok_error_http_tests;

#[cfg(test)]
mod live_acceptance_tests;

#[cfg(test)]
mod chatgpt_plan_http_tests;

#[cfg(test)]
mod chatgpt_plan_continuation_tests;
