//! Connection-local steering acknowledgements and automatic successor ownership.
use super::{mapping::Prepared, ServiceError};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

pub(super) fn is_steer(text: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Kind<'a> {
        #[serde(rename = "type", borrow)]
        kind: &'a str,
    }
    serde_json::from_str::<Kind<'_>>(text).is_ok_and(|v| v.kind == "response.steer")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_lane_rejection_requires_an_unacknowledged_control() {
        let mut rejected =
            json!({"type":"error","error":{"code":"unsupported_native_inflight_message"}});
        let mut state = Steering::default();
        assert!(!state.control_event(&mut rejected));
        state.submitted(
            &json!({"previous_response_id":"r","event_id":"control"}),
            None,
        );
        assert!(!state.control_event(&mut json!({"type":"response.failed","response":{"error":{"code":"unsupported_native_inflight_message"}}})));
        assert!(!state.control_event(&mut json!({"type":"error","error":{"code":"server_error"}})));
        assert!(state.control_event(&mut rejected));
        assert_eq!(rejected["event_id"], "control");
        assert!(state.ready());
        assert!(!state.control_event(&mut rejected));
    }

    #[test]
    fn generic_control_rejection_follows_send_order_in_both_directions() {
        let mut state = Steering::default();
        let mut event =
            json!({"type":"error","error":{"code":"unsupported_native_inflight_message"}});
        state.submitted(
            &json!({"previous_response_id":"r","event_id":"steer"}),
            None,
        );
        let interrupt = state.next_control_order();
        assert!(!state.interrupt_rejected(&event, Some(&(interrupt, Some(json!("interrupt"))))));
        event["event_id"] = json!("interrupt");
        assert!(state.interrupt_rejected(&event, Some(&(interrupt, Some(json!("interrupt"))))));
        event.as_object_mut().unwrap().remove("event_id");
        assert!(state.control_event(&mut event));
        assert_eq!(event["event_id"], "steer");
        let mut state = Steering::default();
        let interrupt = state.next_control_order();
        state.submitted(
            &json!({"previous_response_id":"r","event_id":"steer"}),
            None,
        );
        assert!(!state.interrupt_rejected(&event, Some(&(interrupt, None))));
        event.as_object_mut().unwrap().remove("event_id");
        assert!(state.interrupt_rejected(&event, Some(&(interrupt, None))));
        assert!(!state.ready());
    }
}

struct Submission {
    order: u64,
    parent: String,
    event_id: Option<Value>,
    accepted: Option<String>,
    successor: Option<String>,
    permit: Option<std::sync::Arc<dyn Send + Sync>>,
}
#[derive(Default)]
pub(super) struct Steering {
    next_order: u64,
    submissions: VecDeque<Submission>,
    parents: HashMap<String, Prepared>,
    order: VecDeque<String>,
    waiting: Option<String>,
}
impl Steering {
    pub fn parent(&self, body: &Value) -> Result<Prepared, ServiceError> {
        if self.submissions.len() >= 16 {
            return Err(ServiceError::new(
                429,
                "websocket_queue_full",
                "Too many outstanding steering requests",
            ));
        }
        let parent = body["previous_response_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ServiceError::new(
                    400,
                    "steer_parent_required",
                    "response.steer requires previous_response_id",
                )
            })?;
        self.snapshot(parent).ok_or_else(|| {
            ServiceError::new(
                409,
                "unknown_steer_response",
                "Steering must refer to a response on this upstream connection",
            )
        })
    }
    pub fn remember(&mut self, id: &str, prepared: &Prepared) {
        if self.parents.contains_key(id) {
            return;
        }
        let mut snapshot = prepared.clone();
        snapshot.route.permit = None;
        snapshot.binding = snapshot.binding.steering_scope();
        for body in [&mut snapshot.body, &mut snapshot.source_body] {
            if let Some(object) = body.as_object_mut() {
                for key in ["input", "event_id", "previous_response_id"] {
                    object.remove(key);
                }
            }
        }
        snapshot.bridge_body = None;
        snapshot.rebuild_plan = None;
        snapshot.rebuild_applied = false;
        snapshot.preflight_attempts.clear();
        self.parents.insert(id.to_owned(), snapshot);
        self.order.push_back(id.to_owned());
        // Outstanding submissions retain their parents independently of the LRU.
        while self.order.len() > 32 {
            let Some(index) = self
                .order
                .iter()
                .position(|id| !self.submissions.iter().any(|s| &s.parent == id))
            else {
                break;
            };
            if let Some(id) = self.order.remove(index) {
                self.parents.remove(&id);
            }
        }
    }
    pub fn submitted(&mut self, body: &Value, permit: Option<std::sync::Arc<dyn Send + Sync>>) {
        let order = self.next_control_order();
        self.submissions.push_back(Submission {
            order,
            parent: body["previous_response_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            event_id: body.get("event_id").cloned(),
            accepted: None,
            successor: None,
            permit,
        });
    }
    pub fn next_control_order(&mut self) -> u64 {
        let order = self.next_order;
        self.next_order += 1;
        order
    }
    pub fn interrupt_rejected(
        &self,
        event: &Value,
        pending: Option<&(u64, Option<Value>)>,
    ) -> bool {
        let Some((order, event_id)) = pending else {
            return false;
        };
        if !super::protocol::is_interrupt_rejection(event) {
            return false;
        }
        // Native-lane errors omit the event kind/ID. Attribute them in the
        // actual write order, rather than consuming an unrelated later steer.
        if event.pointer("/error/code").and_then(Value::as_str)
            != Some("unsupported_native_inflight_message")
        {
            return true;
        }
        if event
            .get("event_id")
            .is_some_and(|id| Some(id) == event_id.as_ref())
        {
            return true;
        }
        if event.pointer("/error/param").and_then(Value::as_str) == Some("response.steer")
            || event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("response.steer"))
            || event.get("event_id").is_some_and(|id| {
                self.submissions
                    .iter()
                    .any(|s| s.event_id.as_ref() == Some(id))
            })
        {
            return false;
        }
        !self
            .submissions
            .iter()
            .any(|s| s.accepted.is_none() && s.order < *order)
    }
    pub fn ready(&self) -> bool {
        self.submissions.iter().all(|s| {
            s.accepted.is_some()
                && s.successor.is_none()
                && self.waiting.as_deref() == Some(&s.parent)
        })
    }
    pub fn waiting(&self) -> Option<&str> {
        self.waiting.as_deref()
    }
    fn snapshot(&self, id: &str) -> Option<Prepared> {
        let mut snapshot = self.parents.get(id)?.clone();
        snapshot.route.permit = self
            .submissions
            .iter()
            .find(|s| s.parent == id)
            .and_then(|s| s.permit.clone());
        Some(snapshot)
    }
    pub fn waiting_parent(&self) -> Option<Prepared> {
        self.waiting.as_ref().and_then(|id| self.snapshot(id))
    }
    pub fn control_event(&mut self, event: &mut Value) -> bool {
        let kind = event["type"].as_str().unwrap_or_default();
        let steer_event = kind.starts_with("response.steer.");
        // The Codex native lane rejects in-flight controls using only this
        // code, without param/event_id. Correlate it only while a submitted
        // steer is awaiting acknowledgement; it is not a generation terminal.
        let native_control_rejection = event.pointer("/error/code").and_then(Value::as_str)
            == Some("unsupported_native_inflight_message")
            && self
                .submissions
                .iter()
                .any(|submission| submission.accepted.is_none());
        let explicit_error = kind == "error"
            && (native_control_rejection
                || event.pointer("/error/param").and_then(Value::as_str) == Some("response.steer")
                || event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v.contains("response.steer"))
                || event.get("event_id").is_some_and(|id| {
                    self.submissions
                        .iter()
                        .any(|s| s.event_id.as_ref() == Some(id))
                }));
        if !steer_event && !explicit_error {
            return false;
        }
        let id = event.pointer("/steer/id").and_then(Value::as_str);
        let parent = event
            .pointer("/steer/previous_response_id")
            .and_then(Value::as_str);
        let index = self
            .submissions
            .iter()
            .position(|s| id.is_some() && s.accepted.as_deref() == id)
            .or_else(|| {
                self.submissions.iter().position(|s| {
                    s.accepted.is_none()
                        && parent.is_none_or(|p| p == s.parent)
                        && event
                            .get("event_id")
                            .is_none_or(|id| s.event_id.as_ref() == Some(id))
                })
            });
        if let Some(index) = index {
            let event_id = self.submissions[index].event_id.clone();
            match kind {
                "response.steer.accepted" => {
                    self.submissions[index].accepted = id.map(str::to_owned)
                }
                "response.steer.pending" => {
                    self.waiting = Some(self.submissions[index].parent.clone());
                    if self.submissions[index].accepted.is_none() {
                        self.submissions[index].accepted = id.map(str::to_owned);
                    }
                }
                "response.steer.failed" | "error" => {
                    self.submissions.remove(index);
                }
                _ => {}
            }
            if event.get("event_id").is_none() {
                if let Some(event_id) = event_id {
                    event["event_id"] = event_id;
                }
            }
        }
        if self.submissions.is_empty() {
            self.waiting = None;
        }
        true
    }
    pub fn successor(&mut self, event: &Value) -> Option<Prepared> {
        let id = event.pointer("/response/id").and_then(Value::as_str)?;
        let parent = event
            .pointer("/response/previous_response_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let parent = self
            .submissions
            .iter()
            .find(|s| {
                s.accepted.is_some()
                    && s.successor.is_none()
                    && parent.is_none_or(|p| p == s.parent)
            })?
            .parent
            .clone();
        let snapshot = self.snapshot(&parent)?;
        for s in &mut self.submissions {
            if s.parent == parent && s.accepted.is_some() && s.successor.is_none() {
                s.successor = Some(id.to_owned());
            }
        }
        self.waiting = None;
        Some(snapshot)
    }
    pub fn completed(&mut self, id: Option<&str>) {
        if let Some(id) = id {
            self.submissions
                .retain(|s| s.successor.as_deref() != Some(id));
        }
        if self.submissions.is_empty() {
            self.waiting = None;
        }
    }
    pub fn lost(&mut self) -> Vec<Value> {
        let ids = self
            .submissions
            .drain(..)
            .filter_map(|s| s.event_id)
            .collect();
        self.parents.clear();
        self.order.clear();
        self.waiting = None;
        ids
    }
}
