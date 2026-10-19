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
mod bootstrap;
mod chat;
mod chat_upstream;
mod codex_app_outputs;
mod codex_wire;
mod compat;
mod compat_http;
mod compat_options;
pub mod contract;
pub mod continuity_bridge;
pub mod diagnostics;
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
mod native_chat;
mod output_error;
mod output_items;
mod output_usage;
pub mod recovery;
mod request_body;
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
