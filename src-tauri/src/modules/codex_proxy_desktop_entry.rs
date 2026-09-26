//! Local listener observations, separate from proxy-engine liveness.
//! Only counts and fixed states live in memory: no destinations, payloads or credentials.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub port: Option<u16>,
    pub request_count: u64,
    pub last_request_state: &'static str,
    pub last_error: Option<&'static str>,
}

impl Status {
    fn new(state: &'static str, port: Option<u16>) -> Self {
        Self {
            state,
            port,
            request_count: 0,
            last_request_state: "none",
            last_error: None,
        }
    }
}

pub(super) struct Entry(Mutex<Status>);
// Status reads never wait for listener creation, disk writes or engine startup.
static ENTRIES: LazyLock<Mutex<HashMap<String, Arc<Entry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn snapshot(account_id: &str) -> Option<Status> {
    let entry = ENTRIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(account_id)
        .cloned()?;
    let status = entry.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Some(status)
}

pub(super) fn listening(account_id: &str, port: u16) -> Listener {
    let entry = Arc::new(Entry(Mutex::new(Status::new("listening", Some(port)))));
    ENTRIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(account_id.to_owned(), entry.clone());
    Listener(entry)
}

pub(super) fn failed_to_listen(account_id: &str) {
    let mut status = Status::new("failed", None);
    status.last_error = Some("PROXY_ENTRY_PORT_UNAVAILABLE");
    ENTRIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(account_id.to_owned(), Arc::new(Entry(Mutex::new(status))));
}

/// Constructed before spawning so even cancellation before the first poll is observed.
pub(super) struct Listener(Arc<Entry>);
impl Listener {
    pub(super) fn entry(&self) -> Arc<Entry> {
        self.0.clone()
    }
    pub(super) fn failed(&self) {
        let mut status = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        status.state = "failed";
        status.last_error = Some("PROXY_ENTRY_LISTENER_FAILED");
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let mut status = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        if status.state == "listening" {
            status.state = "stopped";
        }
    }
}

impl Entry {
    pub(super) fn request(self: &Arc<Self>) -> Request {
        let mut status = self.0.lock().unwrap_or_else(|e| e.into_inner());
        status.request_count = status.request_count.saturating_add(1);
        if status.state == "listening" {
            status.last_request_state = "connecting";
            status.last_error = None;
        }
        Request {
            entry: self.clone(),
            sequence: status.request_count,
            finished: false,
        }
    }
}

pub(super) struct Request {
    entry: Arc<Entry>,
    sequence: u64,
    finished: bool,
}
impl Request {
    fn finish(&mut self, result: &'static str, error: Option<&'static str>) {
        self.finished = true;
        let mut status = self.entry.0.lock().unwrap_or_else(|e| e.into_inner());
        // An older slow connection cannot overwrite the newest attempt or listener failure.
        if status.request_count == self.sequence && status.state == "listening" {
            status.last_request_state = result;
            status.last_error = error;
        }
    }
    pub(super) fn forwarded(mut self) {
        self.finish("forwarded", None);
    }
    pub(super) fn failed(mut self, error: &str) {
        self.finish("failed", Some(safe_error(error)));
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("failed", Some("PROXY_CONNECT_FAILED"));
        }
    }
}

fn safe_error(error: &str) -> &'static str {
    match error {
        "PROXY_ENGINE_MISSING" => "PROXY_ENGINE_MISSING",
        "PROXY_ENGINE_TIMEOUT" => "PROXY_ENGINE_TIMEOUT",
        "PROXY_ENGINE_START_FAILED" => "PROXY_ENGINE_START_FAILED",
        "PROXY_ENGINE_VERSION" => "PROXY_ENGINE_VERSION",
        "PROXY_RUNTIME_STARTING" => "PROXY_RUNTIME_STARTING",
        "PROXY_BINDING_CHANGED" => "PROXY_BINDING_CHANGED",
        "UNIFIED_PROXY_STORAGE" => "UNIFIED_PROXY_STORAGE",
        "UNIFIED_PROXY_LOADING" => "UNIFIED_PROXY_LOADING",
        "UNIFIED_PROXY_TIMEOUT" => "UNIFIED_PROXY_TIMEOUT",
        _ => "PROXY_CONNECT_FAILED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_lifecycle_is_independent_of_requests_and_engine_liveness() {
        let id = "entry-observation-lifecycle";
        assert!(snapshot(id).is_none());
        failed_to_listen(id);
        assert_eq!(snapshot(id).unwrap().state, "failed");
        let old = listening(id, 45001);
        let initial = snapshot(id).unwrap();
        assert_eq!(initial.port, Some(45001));
        assert_eq!(initial.request_count, 0);
        assert_eq!(initial.last_request_state, "none");
        let replacement = listening(id, 45002);
        drop(old);
        assert_eq!(snapshot(id).unwrap().state, "listening");
        drop(replacement);
        assert_eq!(snapshot(id).unwrap().state, "stopped");
    }

    #[test]
    fn newest_request_wins_failures_are_redacted_and_retry_clears_error() {
        let id = "entry-observation-requests";
        let listener = listening(id, 45003);
        let entry = listener.entry();
        let old = entry.request();
        let latest = entry.request();
        assert_eq!(snapshot(id).unwrap().last_request_state, "connecting");
        latest.forwarded();
        old.failed("password=secret token=secret");
        assert_eq!(snapshot(id).unwrap().last_request_state, "forwarded");
        entry.request().failed("password=secret token=secret");
        assert_eq!(
            snapshot(id).unwrap().last_error,
            Some("PROXY_CONNECT_FAILED")
        );
        let retry = entry.request();
        assert!(snapshot(id).unwrap().last_error.is_none());
        drop(retry);
        assert_eq!(snapshot(id).unwrap().last_request_state, "failed");
        assert_eq!(snapshot(id).unwrap().request_count, 4);
        let pending = entry.request();
        listener.failed();
        pending.forwarded();
        entry.request().forwarded();
        let result = snapshot(id).unwrap();
        assert_eq!(result.state, "failed");
        assert_eq!(result.last_error, Some("PROXY_ENTRY_LISTENER_FAILED"));
        assert!(!serde_json::to_string(&result).unwrap().contains("secret"));
    }
}
