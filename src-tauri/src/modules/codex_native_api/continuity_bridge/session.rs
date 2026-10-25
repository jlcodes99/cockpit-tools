//! One identity kernel for native and Go transports. All methods run on workers.
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::{
    contract::ServiceError,
    events::response_terminal_kind,
    identity::{rebuild::Plan, Binding, IdentityStore, RequestIdentity},
};
use super::{protocol::*, Config};

const MAX_TICKETS: usize = 1024;
const MAX_TOMBSTONES: usize = 4096;
const MAX_RESERVED_BYTES: usize = 256 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 4096;
const IDLE_SECONDS: u64 = 2 * 60 * 60;
const FINISHED_SECONDS: u64 = 60;

fn error(code: &'static str, message: &str) -> ServiceError {
    ServiceError::new(409, code, message)
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ServiceError {
    ServiceError::new(
        503,
        "continuity_state_unavailable",
        "Continuity state is unavailable",
    )
}

fn identifier(value: &str) -> Result<(), ServiceError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(ServiceError::new(
            400,
            "invalid_continuity_identifier",
            "Invalid continuity identifier",
        ));
    }
    Ok(())
}

fn fingerprint(value: &impl Serialize) -> Result<(String, usize), ServiceError> {
    let raw = serde_json::to_vec(value).map_err(|_| {
        ServiceError::new(
            400,
            "invalid_continuity_payload",
            "Invalid continuity payload",
        )
    })?;
    Ok((format!("{:x}", Sha256::digest(&raw)), raw.len()))
}

struct Registry {
    requests: HashMap<(String, String), Arc<Ticket>>,
    tickets: HashMap<String, Arc<Ticket>>,
    bytes: usize,
}

struct Ticket {
    id: String,
    request_hash: String,
    touched: AtomicU64,
    reserved: AtomicUsize,
    bound: AtomicBool,
    finished: AtomicBool,
    data: Mutex<Option<Result<Session, ServiceError>>>,
}

struct Session {
    caller: String,
    compact: bool,
    context: Option<RequestIdentity>,
    response: PrepareResponse,
    plan: Option<Plan>,
    attempts: HashSet<String>,
    last_bind: Option<(String, String, Result<BindResponse, ServiceError>)>,
    current: Option<Attempt>,
    finish: Option<(String, Result<FinishResponse, ServiceError>)>,
    finished_attempts: HashMap<String, (String, Result<FinishResponse, ServiceError>)>,
    allow_rebuild: bool,
    forwarding_account_id: Option<String>,
    forwarding_may_have_sent: bool,
}

struct Attempt {
    id: String,
    compact: bool,
    binding: Binding,
    next_sequence: u64,
    observation: Option<(u64, String, Result<ObserveResponse, ServiceError>)>,
    terminal: bool,
    succeeded: bool,
    output_observed: bool,
}

impl Session {
    fn drop_payload(&mut self) {
        self.context = None;
        self.plan = None;
        self.current = None;
        self.last_bind = None;
        self.attempts.clear();
        self.finished_attempts.clear();
        self.forwarding_account_id = None;
        self.forwarding_may_have_sent = false;
        self.response.body = serde_json::Value::Null;
        self.response.headers.clear();
    }
}

impl Attempt {
    fn fail(&self, _code: &str) {
        self.binding.abort_rebuild();
    }
}

pub(super) struct Core {
    identity: IdentityStore,
    config: Config,
    registry: Mutex<Registry>,
    epoch: Instant,
    pub stopped: AtomicBool,
}

impl Core {
    pub(super) fn open(config: Config) -> Result<Self, String> {
        if config.token.len() < 32 {
            return Err("Continuity bridge requires a private token of at least 32 bytes".into());
        }
        for (caller, accounts) in &config.caller_accounts {
            identifier(caller).map_err(|error| error.message)?;
            for account in accounts {
                identifier(account).map_err(|error| error.message)?;
                identifier(
                    config
                        .owners
                        .get(account)
                        .ok_or("Continuity account owner is missing")?,
                )
                .map_err(|error| error.message)?;
            }
        }
        let identity = IdentityStore::open(config.identity_path.clone())?;
        identity.set_session_identity_mapping_enabled(config.session_identity_mapping_enabled);
        Ok(Self {
            identity,
            config,
            registry: Mutex::new(Registry {
                requests: HashMap::new(),
                tickets: HashMap::new(),
                bytes: 0,
            }),
            epoch: Instant::now(),
            stopped: AtomicBool::new(false),
        })
    }

    fn check_running(&self) -> Result<(), ServiceError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(ServiceError::new(
                503,
                "continuity_bridge_stopping",
                "Continuity bridge is stopping",
            ));
        }
        Ok(())
    }

    fn lookup(&self, id: &str) -> Result<Arc<Ticket>, ServiceError> {
        self.check_running()?;
        let registry = self.registry.lock().map_err(lock_error)?;
        let ticket = registry.tickets.get(id).cloned().ok_or_else(|| {
            error(
                "continuity_ticket_not_found",
                "Continuity ticket is unavailable",
            )
        })?;
        ticket
            .touched
            .store(self.epoch.elapsed().as_secs(), Ordering::Relaxed);
        Ok(ticket)
    }

    fn reserve(&self, ticket: &Ticket, bytes: usize) -> Result<(), ServiceError> {
        let mut registry = self.registry.lock().map_err(lock_error)?;
        let old = ticket.reserved.load(Ordering::Relaxed);
        let total = registry.bytes.saturating_sub(old).saturating_add(bytes);
        // Keep the shared budget for concurrent requests, but do not turn its
        // conservative copy accounting into a limit on one idle conversation.
        // Completed tombstones never compete with an exclusive large ticket;
        // shrinking a reservation must always be able to release capacity.
        if bytes > old
            && total > MAX_RESERVED_BYTES
            && registry
                .tickets
                .values()
                .any(|other| other.id != ticket.id && !other.finished.load(Ordering::Acquire))
        {
            return Err(ServiceError::new(
                503,
                "continuity_capacity",
                "Continuity request memory is full",
            ));
        }
        registry.bytes = total;
        ticket.reserved.store(bytes, Ordering::Relaxed);
        Ok(())
    }

    pub(super) fn prepare(&self, request: PrepareRequest) -> Result<PrepareResponse, ServiceError> {
        self.check_running()?;
        identifier(&request.request_id)?;
        identifier(&request.caller)?;
        if !self.config.caller_accounts.contains_key(&request.caller) {
            return Err(ServiceError::new(
                403,
                "continuity_caller_scope",
                "Caller is outside this continuity service",
            ));
        }
        if !request.body.is_object() {
            return Err(ServiceError::new(
                400,
                "invalid_continuity_payload",
                "Responses body must be an object",
            ));
        }
        let (request_hash, bytes) = fingerprint(&request)?;
        let key = (request.caller.clone(), request.request_id.clone());
        let ticket = {
            let mut registry = self.registry.lock().map_err(lock_error)?;
            if let Some(ticket) = registry.requests.get(&key) {
                ticket.clone()
            } else {
                let now = self.epoch.elapsed().as_secs();
                // Active streams are never evicted by a wall-clock TTL. Bind/observe
                // renew activity; only finished or never-bound tickets may age out.
                let expired: Vec<_> = registry
                    .requests
                    .iter()
                    .filter(|(_, ticket)| {
                        let idle = now.saturating_sub(ticket.touched.load(Ordering::Relaxed));
                        (ticket.finished.load(Ordering::Acquire) && idle > FINISHED_SECONDS)
                            || (!ticket.bound.load(Ordering::Acquire) && idle > IDLE_SECONDS)
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                let mut retired = Vec::new();
                for key in expired {
                    if let Some(ticket) = registry.requests.remove(&key) {
                        registry.bytes = registry
                            .bytes
                            .saturating_sub(ticket.reserved.load(Ordering::Relaxed));
                        registry.tickets.remove(&ticket.id);
                        retired.push(ticket);
                    }
                }
                if registry.tickets.len() >= MAX_TICKETS + MAX_TOMBSTONES {
                    let oldest = registry
                        .requests
                        .iter()
                        .filter(|(_, ticket)| ticket.finished.load(Ordering::Acquire))
                        .min_by_key(|(_, ticket)| ticket.touched.load(Ordering::Relaxed))
                        .map(|(key, _)| key.clone());
                    if let Some(key) = oldest {
                        if let Some(ticket) = registry.requests.remove(&key) {
                            registry.bytes = registry
                                .bytes
                                .saturating_sub(ticket.reserved.load(Ordering::Relaxed));
                            registry.tickets.remove(&ticket.id);
                            retired.push(ticket);
                        }
                    }
                }
                let reserved = bytes.saturating_mul(3);
                let active = registry
                    .tickets
                    .values()
                    .filter(|ticket| !ticket.finished.load(Ordering::Acquire))
                    .count();
                if active >= MAX_TICKETS
                    || (active > 0 && registry.bytes.saturating_add(reserved) > MAX_RESERVED_BYTES)
                {
                    drop(registry);
                    drop(retired);
                    return Err(ServiceError::new(
                        503,
                        "continuity_capacity",
                        "Continuity request capacity is full",
                    ));
                }
                let ticket = Arc::new(Ticket {
                    id: uuid::Uuid::new_v4().to_string(),
                    request_hash: request_hash.clone(),
                    touched: AtomicU64::new(now),
                    reserved: AtomicUsize::new(reserved),
                    bound: AtomicBool::new(false),
                    finished: AtomicBool::new(false),
                    data: Mutex::new(None),
                });
                registry.bytes = registry.bytes.saturating_add(reserved);
                registry.tickets.insert(ticket.id.clone(), ticket.clone());
                registry.requests.insert(key, ticket.clone());
                drop(registry);
                drop(retired);
                ticket
            }
        };
        ticket
            .touched
            .store(self.epoch.elapsed().as_secs(), Ordering::Relaxed);
        if ticket.request_hash != request_hash {
            return Err(error(
                "continuity_request_conflict",
                "Request identity was reused with different input",
            ));
        }
        let mut data = ticket.data.lock().map_err(lock_error)?;
        if data.is_none() {
            let result = self.prepare_inner(&ticket, request);
            if result.is_err() {
                ticket.finished.store(true, Ordering::Release);
                self.reserve(&ticket, ticket.reserved.load(Ordering::Relaxed).min(4096))?;
            }
            *data = Some(result);
        }
        match data.as_ref().unwrap() {
            Ok(session) if session.finish.is_some() => Err(error(
                "continuity_request_finished",
                "Continuity request is already complete",
            )),
            Ok(session) => Ok(session.response.clone()),
            Err(error) => Err(error.clone()),
        }
    }

    fn prepare_inner(
        &self,
        ticket: &Ticket,
        request: PrepareRequest,
    ) -> Result<Session, ServiceError> {
        self.check_running()?;
        let headers = decode_headers(&request.headers)?;
        let allow_rebuild = request.allow_rebuild && !request.compact;
        let mut context = self.identity.inspect_for_forwarding(
            &request.caller,
            &headers,
            &request.body,
            allow_rebuild || request.compact,
        )?;
        let mut plan = self.identity.configure_rebuild(
            &mut context,
            &headers,
            &request.body,
            allow_rebuild,
        )?;
        if allow_rebuild && plan.is_none() {
            plan = self.identity.preview_rebuild_with_checkpoint_parameters(
                &context,
                &headers,
                &request.body,
            )?;
        }
        if allow_rebuild && plan.is_none() {
            self.identity.protect_unrebuildable(&mut context)?;
        }
        if request.compact {
            context.disable_checkpoints();
        }
        let plan_bytes = plan.as_ref().map_or(0, |plan| plan.body.to_string().len());
        self.reserve(
            ticket,
            ticket
                .reserved
                .load(Ordering::Relaxed)
                .saturating_add(plan_bytes.saturating_mul(4)),
        )?;
        let response = PrepareResponse {
            ticket: ticket.id.clone(),
            preferred_account_id: context.state_account_id.clone(),
            session_id: context.session_id.clone(),
            thread_id: context.thread_id.clone(),
            parent_thread_id: context.parent_thread_id.clone(),
            self_contained: plan.is_some(),
            body: request.body,
            headers: request.headers,
        };
        Ok(Session {
            caller: request.caller,
            compact: request.compact,
            context: Some(context),
            response,
            plan,
            attempts: HashSet::new(),
            last_bind: None,
            current: None,
            finish: None,
            finished_attempts: HashMap::new(),
            allow_rebuild,
            forwarding_account_id: None,
            forwarding_may_have_sent: false,
        })
    }

    pub(super) fn bind(&self, request: BindRequest) -> Result<BindResponse, ServiceError> {
        identifier(&request.attempt_id)?;
        let (hash, bytes) = fingerprint(&request)?;
        let ticket = self.lookup(&request.ticket)?;
        let mut data = ticket.data.lock().map_err(lock_error)?;
        let session = data
            .as_mut()
            .ok_or_else(|| {
                error(
                    "continuity_not_prepared",
                    "Continuity request is not prepared",
                )
            })?
            .as_mut()
            .map_err(|error| error.clone())?;
        if let Some((id, previous, response)) = &session.last_bind {
            if id == &request.attempt_id {
                return if previous == &hash {
                    response.clone()
                } else {
                    Err(error(
                        "continuity_attempt_conflict",
                        "Attempt identity was reused with different input",
                    ))
                };
            }
        }
        if session.finish.is_some()
            || session
                .current
                .as_ref()
                .is_some_and(|current| current.succeeded)
        {
            return Err(error(
                "continuity_request_finished",
                "Continuity request is already complete",
            ));
        }
        if session.attempts.contains(&request.attempt_id) {
            return Err(error(
                "stale_continuity_attempt",
                "Continuity attempt was superseded",
            ));
        }
        if session
            .current
            .as_ref()
            .is_some_and(|current| current.output_observed)
        {
            return Err(error(
                "continuity_output_started",
                "An observed output cannot be replayed as another attempt",
            ));
        }
        let owner = self.config.owners.get(&request.account_id).ok_or_else(|| {
            ServiceError::new(
                403,
                "continuity_account_scope",
                "Account is outside this continuity service",
            )
        })?;
        let accounts = self.config.caller_accounts.get(&session.caller).unwrap();
        if !accounts.contains(&request.account_id)
            || request
                .owner_id
                .as_ref()
                .is_some_and(|value| value != owner)
        {
            return Err(ServiceError::new(
                403,
                "continuity_account_scope",
                "Account ownership or caller scope does not match",
            ));
        }
        if session.attempts.len() >= MAX_ATTEMPTS {
            return Err(ServiceError::new(
                503,
                "continuity_capacity",
                "Continuity attempt capacity is full",
            ));
        }
        validate_source(&session.response, &request)?;
        if session
            .forwarding_account_id
            .as_ref()
            .is_some_and(|account| account != &request.account_id)
        {
            return Err(error(
                "continuity_unverified_account_retry",
                "An unverified continuation cannot retry on a different account",
            ));
        }
        self.reserve(
            &ticket,
            ticket
                .reserved
                .load(Ordering::Relaxed)
                .saturating_sub(session.attempts.len().saturating_mul(512))
                .max(bytes.saturating_mul(4))
                .saturating_add(session.attempts.len().saturating_add(1).saturating_mul(512)),
        )?;
        if let Some(previous) = session.current.take() {
            session.forwarding_may_have_sent |= session.forwarding_account_id.is_some();
            previous.fail("continuity_attempt_superseded");
        }
        session.attempts.insert(request.attempt_id.clone());
        let result = self.bind_inner(session, &request, owner);
        session.last_bind = Some((request.attempt_id.clone(), hash, result.clone()));
        if result.is_ok() {
            if session.plan.is_none()
                && session
                    .context
                    .as_ref()
                    .is_some_and(RequestIdentity::requires_forwarding_account_pin)
            {
                session.forwarding_account_id = Some(request.account_id.clone());
            }
            ticket.bound.store(true, Ordering::Release);
        }
        result
    }

    fn bind_inner(
        &self,
        session: &mut Session,
        request: &BindRequest,
        owner: &str,
    ) -> Result<BindResponse, ServiceError> {
        self.check_running()?;
        let mut context = session
            .context
            .as_ref()
            .ok_or_else(|| {
                error(
                    "continuity_request_finished",
                    "Continuity request is already complete",
                )
            })?
            .clone();
        let mut headers = decode_headers(&request.headers)?;
        let original_headers = decode_headers(&session.response.headers)?;
        if !self.config.session_identity_mapping_enabled {
            // Go transports may synthesize a session fallback or normalize it
            // to Session_id before Bind. Restore the client's exact presence and
            // values when mapping is disabled, including an absent header.
            for name in ["session-id", "session_id", "thread-id"] {
                headers.remove(name);
                for value in original_headers.get_all(name) {
                    headers.append(name, value.clone());
                }
            }
        }
        // Executors apply their transport/authentication whitelist before Bind.
        // Restore the client's authoritative identities without losing the
        // selected account's fresh transport headers.
        for name in [
            "session-id",
            "thread-id",
            "x-client-request-id",
            "x-codex-parent-thread-id",
            "x-codex-turn-state",
            "x-codex-turn-metadata",
            "conversation_id",
            "x-codex-window-id",
        ] {
            if let Some(value) = original_headers.get(name) {
                headers.insert(name, value.clone());
            }
        }
        let mut body = request.body.clone();
        if request.expand_http_previous && !session.compact {
            self.identity
                .normalize_http_input(&mut context, &headers, &mut body)?;
        }
        let expand_http = request.expand_http_previous
            && !session.compact
            && body
                .get("previous_response_id")
                .is_some_and(serde_json::Value::is_string);
        let plan = if expand_http {
            Some(self.identity.expand_http_previous(
                &mut context,
                &headers,
                &body,
                &session.response.body,
            )?)
        } else {
            self.identity
                .configure_rebuild(&mut context, &headers, &body, session.allow_rebuild)?
        };
        if session.allow_rebuild && plan.is_none() {
            self.identity.protect_unrebuildable(&mut context)?;
        }
        let account_changed = context
            .state_account_id
            .as_ref()
            .is_some_and(|account| account != &request.account_id);
        let rebuild_applied = account_changed || expand_http;
        if rebuild_applied {
            if session.plan.is_none() && !expand_http {
                return Err(error(
                    "continuation_owner_mismatch",
                    "Continuation is pinned to its upstream account",
                ));
            }
            let plan = plan.ok_or_else(|| {
                error(
                    "continuation_owner_mismatch",
                    "A complete history is required to change accounts",
                )
            })?;
            let ticket = self.lookup(&request.ticket)?;
            self.reserve(
                &ticket,
                ticket.reserved.load(Ordering::Relaxed).max(
                    plan.body
                        .to_string()
                        .len()
                        .saturating_mul(4)
                        .saturating_add(session.attempts.len().saturating_mul(512)),
                ),
            )?;
            headers = plan.headers.clone();
            body = plan.body.clone();
            if account_changed {
                context = self
                    .identity
                    .inspect_for_rebuild(&session.caller, &headers, &body)?;
                self.identity
                    .configure_rebuild(&mut context, &headers, &body, true)?;
                context.start_rebuild_generation(&plan);
            }
        }
        let binding = self.identity.rewrite(
            &context,
            &request.account_id,
            owner,
            &mut headers,
            &mut body,
        )?;
        let response = BindResponse {
            ticket: request.ticket.clone(),
            attempt_id: request.attempt_id.clone(),
            headers: encode_headers(&headers)?,
            body,
            rebuild_applied,
        };
        session.current = Some(Attempt {
            id: request.attempt_id.clone(),
            compact: session.compact,
            binding,
            next_sequence: 1,
            observation: None,
            terminal: false,
            succeeded: false,
            output_observed: false,
        });
        Ok(response)
    }

    pub(super) fn observe(&self, request: ObserveRequest) -> Result<ObserveResponse, ServiceError> {
        let (hash, _) = fingerprint(&request)?;
        let ticket = self.lookup(&request.ticket)?;
        let mut data = ticket.data.lock().map_err(lock_error)?;
        let session = data
            .as_mut()
            .ok_or_else(|| {
                error(
                    "continuity_not_prepared",
                    "Continuity request is not prepared",
                )
            })?
            .as_mut()
            .map_err(|error| error.clone())?;
        if session.finish.is_some() {
            return Err(error(
                "continuity_attempt_finished",
                "Continuity attempt is already terminal",
            ));
        }
        let attempt = session
            .current
            .as_mut()
            .filter(|current| current.id == request.attempt_id)
            .ok_or_else(|| {
                error(
                    "stale_continuity_attempt",
                    "Continuity attempt was superseded",
                )
            })?;
        if let Some((sequence, previous, response)) = &attempt.observation {
            if *sequence == request.sequence {
                return if previous == &hash {
                    response.clone()
                } else {
                    Err(error(
                        "continuity_sequence_conflict",
                        "Observation sequence has different content",
                    ))
                };
            }
        }
        if session.finish.is_some() || attempt.terminal {
            return Err(error(
                "continuity_attempt_finished",
                "Continuity attempt is already terminal",
            ));
        }
        if request.sequence != attempt.next_sequence {
            return Err(error(
                "continuity_sequence_mismatch",
                "Continuity observation is out of order",
            ));
        }
        let result = self.observe_inner(attempt, &request);
        if let Err(error) = &result {
            attempt.fail(error.code);
            attempt.terminal = true;
        }
        // Only an ACK advances the caller's sequence. A rejected observation
        // must still allow Finish with the last acknowledged next_sequence.
        if result.is_ok() {
            attempt.next_sequence = attempt.next_sequence.saturating_add(1);
        }
        attempt.observation = Some((request.sequence, hash, result.clone()));
        result
    }

    fn observe_inner(
        &self,
        attempt: &mut Attempt,
        request: &ObserveRequest,
    ) -> Result<ObserveResponse, ServiceError> {
        self.check_running()?;
        if let Some(headers) = &request.headers {
            self.identity
                .observe_headers(&attempt.binding, &decode_headers(headers)?)?;
        }
        for event in &request.events {
            self.check_running()?;
            if !event.is_object() {
                return Err(ServiceError::new(
                    400,
                    "invalid_continuity_event",
                    "Upstream event must be an object",
                ));
            }
            // Feed the unmodified upstream terminal to the kernel. Public output
            // repair never supplies evidence for a complete history checkpoint.
            self.identity.observe_event(&attempt.binding, event)?;
            if event
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| {
                    kind.starts_with("response.output_")
                        || kind.starts_with("response.content_part.")
                        || kind.starts_with("response.function_call_arguments.")
                        || kind.starts_with("response.custom_tool_call_input.")
                })
            {
                attempt.output_observed = true;
            }
            if let Some(kind) = response_terminal_kind(event) {
                let response = event.get("response").unwrap_or(event);
                attempt.succeeded = matches!(kind, "response.completed" | "response.done")
                    && response["status"] == "completed"
                    && response["id"]
                        .as_str()
                        .is_some_and(|id| !id.trim().is_empty())
                    && event.get("error").is_none_or(serde_json::Value::is_null)
                    && response.get("error").is_none_or(serde_json::Value::is_null);
                attempt.terminal = true;
                if !attempt.succeeded {
                    attempt.fail("continuity_upstream_failed");
                }
                // A failure followed by a late success in one read remains a failure.
                break;
            }
            if attempt.compact
                && event
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty())
                && event.get("output").is_some_and(serde_json::Value::is_array)
                && event.get("error").is_none_or(serde_json::Value::is_null)
            {
                // /responses/compact returns a compaction object, not a
                // generated response.completed event. Keep that distinction.
                attempt.terminal = true;
                attempt.succeeded = true;
                break;
            }
        }
        Ok(ObserveResponse {
            next_sequence: request.sequence.saturating_add(1),
            terminal: attempt.terminal,
        })
    }

    pub(super) fn finish(&self, request: FinishRequest) -> Result<FinishResponse, ServiceError> {
        let (hash, _) = fingerprint(&request)?;
        let ticket = self.lookup(&request.ticket)?;
        let mut data = ticket.data.lock().map_err(lock_error)?;
        let session = data
            .as_mut()
            .ok_or_else(|| {
                error(
                    "continuity_not_prepared",
                    "Continuity request is not prepared",
                )
            })?
            .as_mut()
            .map_err(|error| error.clone())?;
        if let Some((previous, response)) = session.finished_attempts.get(&request.attempt_id) {
            return if previous == &hash {
                response.clone()
            } else {
                Err(error(
                    "continuity_finish_conflict",
                    "Attempt finish decision already exists",
                ))
            };
        }
        if let Some((previous, response)) = &session.finish {
            return if previous == &hash {
                response.clone()
            } else {
                Err(error(
                    "continuity_finish_conflict",
                    "Continuity finish decision already exists",
                ))
            };
        }
        let code = request
            .error_code
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(match request.status {
                FinishStatus::Cancelled => "continuity_cancelled",
                _ => "continuity_incomplete",
            });
        let mut close_ticket =
            request.attempt_id.is_empty() || request.status == FinishStatus::Succeeded;
        let result = if request.attempt_id.is_empty() && request.status != FinishStatus::Succeeded {
            if let Some(attempt) = &session.current {
                attempt.fail(code);
            }
            Ok(FinishResponse {})
        } else if let Some(attempt) = &mut session.current {
            if attempt.id != request.attempt_id {
                return Err(error(
                    "stale_continuity_attempt",
                    "Continuity attempt was superseded",
                ));
            }
            if request.sequence != attempt.next_sequence {
                return Err(error(
                    "continuity_sequence_mismatch",
                    "Continuity finish is out of order",
                ));
            }
            close_ticket |= attempt.output_observed;
            if request.status == FinishStatus::Succeeded && !attempt.succeeded {
                attempt.fail("continuity_missing_terminal");
                Err(error(
                    "continuity_missing_terminal",
                    "A genuine successful upstream terminal was not observed",
                ))
            } else {
                if request.status != FinishStatus::Succeeded {
                    attempt.fail(code);
                }
                // Only an explicit trusted preparation failure proves that this
                // binding never reached its upstream. A transport failure or a
                // rejected upstream response cannot release the account pin.
                let definitely_unsent = request.status == FinishStatus::Failed
                    && code == "continuity_upstream_not_sent"
                    && attempt.next_sequence == 1;
                if definitely_unsent && !session.forwarding_may_have_sent {
                    session.forwarding_account_id = None;
                } else if session.forwarding_account_id.is_some() {
                    session.forwarding_may_have_sent = true;
                }
                attempt.terminal = true;
                Ok(FinishResponse {})
            }
        } else if request.status != FinishStatus::Succeeded
            && (request.attempt_id.is_empty()
                || session
                    .last_bind
                    .as_ref()
                    .is_some_and(|(id, _, _)| id == &request.attempt_id))
        {
            Ok(FinishResponse {})
        } else {
            return Err(error(
                "stale_continuity_attempt",
                "No matching upstream attempt exists",
            ));
        };
        if !close_ticket {
            // Finish is per actual upstream execution. A 429, opening timeout
            // or WS fallback without output must retain Go's ordinary retry
            // path and the complete source history for a successor Bind.
            session
                .finished_attempts
                .insert(request.attempt_id, (hash, result.clone()));
            session.current = None;
            ticket.bound.store(false, Ordering::Release);
            return result;
        }
        session.finish = Some((hash, result.clone()));
        // A terminal tombstone retains idempotent Finish without retaining
        // complete transcripts and encrypted payloads between requests.
        session.drop_payload();
        self.reserve(&ticket, ticket.reserved.load(Ordering::Relaxed).min(4096))?;
        ticket.finished.store(true, Ordering::Release);
        result
    }

    pub(super) fn release(&self, request: ReleaseRequest) -> Result<ReleaseResponse, ServiceError> {
        identifier(&request.ticket)?;
        let ticket = match self.lookup(&request.ticket) {
            Ok(ticket) => ticket,
            Err(error) if error.code == "continuity_ticket_not_found" => {
                return Ok(ReleaseResponse {})
            }
            Err(error) => return Err(error),
        };
        let mut data = ticket.data.lock().map_err(lock_error)?;
        if let Some(Ok(session)) = data.as_mut() {
            if session.finish.is_none() {
                if let Some(attempt) = &session.current {
                    if !attempt.succeeded {
                        attempt.fail("continuity_released");
                    }
                }
                session.finish = Some(("released".into(), Ok(FinishResponse {})));
                session.drop_payload();
            }
        }
        self.reserve(&ticket, ticket.reserved.load(Ordering::Relaxed).min(4096))?;
        ticket.finished.store(true, Ordering::Release);
        Ok(ReleaseResponse {})
    }

    pub(super) fn close(&self) {
        self.stopped.store(true, Ordering::Release);
        let tickets: Vec<_> = self
            .registry
            .lock()
            .map(|registry| registry.tickets.values().cloned().collect())
            .unwrap_or_default();
        for ticket in tickets {
            if let Ok(mut data) = ticket.data.lock() {
                if let Some(Ok(session)) = data.as_mut() {
                    if let Some(attempt) = &session.current {
                        attempt.fail("continuity_bridge_stopped");
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn elapse_for_test(&mut self, seconds: u64) {
        self.epoch -= std::time::Duration::from_secs(seconds);
    }
}

fn validate_source(source: &PrepareResponse, request: &BindRequest) -> Result<(), ServiceError> {
    if !request.body.is_object()
        || [
            "input",
            "previous_response_id",
            "client_metadata",
            "conversation",
            "conversation_id",
        ]
        .iter()
        .any(|key| source.body.get(key) != request.body.get(key))
    {
        return Err(error(
            "continuity_source_changed",
            "Canonical continuation input changed after preparation",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "capacity_tests.rs"]
mod capacity_tests;
