//! Local account-pool metadata, independent of request logs, conversations and quotas.
//! Single-flight reads run on a blocking worker and never delay the CDP heartbeat.

use super::{recent_account, ProfileGatewayConfig};
use crate::models::codex_local_access::CodexLocalAccessCollection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

const REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const READ_BUDGET: Duration = Duration::from_secs(5);

struct Snapshot {
    binding: String,
    count: Option<i64>,
}

#[derive(Default)]
pub(super) struct PoolPoller {
    tasks: JoinSet<Snapshot>,
    snapshot: Option<Snapshot>,
    requested: Option<(String, Instant)>,
    refresh_due: bool,
}

fn binding(gateway: &ProfileGatewayConfig) -> String {
    let mut hash = Sha256::new();
    hash.update(gateway.base_url.as_bytes());
    hash.update([0]);
    hash.update(gateway.api_key.as_bytes());
    format!("{:x}", hash.finalize())
}

impl PoolPoller {
    pub(super) fn refresh(&mut self) {
        self.refresh_due = true;
    }

    pub(super) fn collect(&mut self, gateway: Option<&ProfileGatewayConfig>) -> Value {
        while let Some(result) = self.tasks.try_join_next() {
            if let Ok(snapshot) = result {
                if self.requested.as_ref().is_some_and(|(binding, at)| {
                    binding == &snapshot.binding && at.elapsed() <= READ_BUDGET
                }) {
                    self.snapshot = Some(snapshot);
                } else {
                    // A timed-out blocking read still holds its single worker
                    // slot until completion. Discard its late result, then retry.
                    self.requested = None;
                }
            } else {
                self.requested = None;
            }
        }
        let Some(gateway) = gateway else {
            self.snapshot = None;
            self.requested = None;
            return json!({"binding":null,"gptApiKeyCount":null,"pending":false});
        };
        let current_binding = binding(gateway);
        // Immediately discard an old service/key snapshot, including while its
        // worker is still running. Never borrow a count from another key scope.
        if self.snapshot.as_ref().is_some_and(|value| value.binding != current_binding) {
            self.snapshot = None;
        }
        let due = self.refresh_due || self.requested.as_ref().map_or(true, |(last, at)| {
            last != &current_binding || at.elapsed() >= REFRESH_INTERVAL
        });
        if due && self.tasks.is_empty() {
            let gateway = gateway.clone();
            let worker_binding = current_binding.clone();
            self.tasks.spawn_blocking(move || Snapshot {
                binding: worker_binding,
                count: recent_account::read_only_data_dir().ok().and_then(|dir| {
                    read_count(&dir, &gateway)
                }),
            });
            self.requested = Some((current_binding.clone(), Instant::now()));
            self.refresh_due = false;
        }
        let count = self.snapshot.as_ref()
            .filter(|value| value.binding == current_binding)
            .and_then(|value| value.count);
        let pending = self.snapshot.is_none() && self.requested.as_ref().is_some_and(|(last, at)| {
            last == &current_binding && at.elapsed() <= READ_BUDGET
        });
        // Only a one-way scope fingerprint and count reach the renderer.
        json!({"binding":current_binding,"gptApiKeyCount":count,"pending":pending})
    }
}

fn read_count(data_dir: &Path, gateway: &ProfileGatewayConfig) -> Option<i64> {
    let url = reqwest::Url::parse(&gateway.base_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_end_matches('/') != "/v1"
        || !url.host_str().is_some_and(|host| {
            host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        })
    {
        return None;
    }
    let raw = recent_account::read_small_file(&data_dir.join("codex_local_access.json")).ok()??;
    let collection: CodexLocalAccessCollection = serde_json::from_str(&raw).ok()?;
    if url.port_or_known_default() != Some(collection.port) {
        return None;
    }
    crate::modules::codex_local_access::injection_gpt_api_key_account_count_for_bearer(
        &collection,
        &gateway.api_key,
        data_dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gateway() -> ProfileGatewayConfig {
        ProfileGatewayConfig {
            base_url: "http://localhost:58554/v1".into(),
            api_key: "fixture-bearer".into(),
            provider_name: "Fixture".into(),
            managed_voice: false,
        }
    }

    #[test]
    fn local_count_does_not_require_any_log_database_or_conversation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("codex_local_access.json"), json!({
            "enabled":true,"port":58554,"apiKey":"unused",
            "apiKeys":[{"id":"client","label":"Client","key":"fixture-bearer","enabled":true,
                "inheritAccountPool":false,"accountIds":[]}],
            "accountIds":["not-in-this-key-scope"],"createdAt":1,"updatedAt":1
        }).to_string()).unwrap();
        assert_eq!(read_count(dir.path(), &gateway()), Some(0));
        assert!(!dir.path().join("codex_local_access_logs.sqlite").exists());
        let mut other = gateway();
        other.api_key = "another-key".into();
        assert_eq!(read_count(dir.path(), &other), None);
        other = gateway();
        other.base_url = "http://localhost:58555/v1".into();
        assert_eq!(read_count(dir.path(), &other), None);
        other.base_url = "https://upstream.example/v1".into();
        assert_eq!(read_count(dir.path(), &other), None);
    }

    #[tokio::test]
    async fn binding_changes_drop_old_counts_without_waiting_for_metadata() {
        let mut poller = PoolPoller::default();
        let current = gateway();
        poller.snapshot = Some(Snapshot {binding:binding(&current),count:Some(2)});
        poller.requested = Some((binding(&current),Instant::now()));
        assert_eq!(poller.collect(Some(&current))["gptApiKeyCount"],2);
        let mut other = current;
        other.api_key = "another-key".into();
        assert!(poller.collect(Some(&other))["gptApiKeyCount"].is_null());
        assert!(poller.collect(None)["gptApiKeyCount"].is_null());
    }
}
