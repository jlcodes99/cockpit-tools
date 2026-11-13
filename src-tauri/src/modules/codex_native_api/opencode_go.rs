use axum::http::{HeaderMap, HeaderValue};
use sha2::{Digest, Sha256};

use super::identity::Binding;
pub(super) use crate::modules::codex_account::is_opencode_go_base_url;

pub(super) const SESSION_HEADER: &str = "x-opencode-session";

pub(super) fn apply_session_header(
    base_url: &str,
    caller_id: &str,
    binding: &Binding,
    generated_session: &mut Option<HeaderValue>,
    headers: &mut HeaderMap,
) {
    if !is_opencode_go_base_url(base_url)
        || headers
            .get(SESSION_HEADER)
            .is_some_and(|header| !header.is_empty())
    {
        return;
    }
    // Account retries can inspect a stateless request again and allocate a new
    // root. Reuse the first provider session for the lifetime of this request.
    let session = generated_session.get_or_insert_with(|| {
        let session_id = binding
            .session_id()
            .expect("inspected request has a session root");
        let mut digest = Sha256::new();
        digest.update((caller_id.len() as u64).to_be_bytes());
        digest.update(caller_id);
        digest.update(session_id);
        HeaderValue::from_str(&format!("{:x}", digest.finalize())).expect("valid hex header")
    });
    headers.insert(SESSION_HEADER, session.clone());
}
