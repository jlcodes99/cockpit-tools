//! Read-only compatibility for completed records in older continuity journals.
//! New requests use automatic complete-history continuation; this module exposes no controls.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Completion {
    pub id: String,
    pub caller: String,
    pub thread: String,
    pub account: String,
    pub owner: String,
    pub completed_at: i64,
}

#[cfg(test)]
#[path = "recovery_http_tests.rs"]
mod automatic_http_tests;

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod compatibility_tests;
