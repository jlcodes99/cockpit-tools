//! Shared versioned account identity metadata. Only digests are retained.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryIdentity {
    pub version: u32,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub credential_digest: Option<String>,
    pub platform: String,
}

pub(crate) fn fingerprint(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| format!("{:x}", Sha256::digest(v.as_bytes())))
}
