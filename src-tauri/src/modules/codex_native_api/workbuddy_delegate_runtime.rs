//! Native DelegateTool actors. Each conversation owns one isolated CLI connection.
//! A completed HTTP handoff does not complete the outstanding ACP prompt.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, watch, Mutex as AsyncMutex, Notify, OnceCell, Semaphore},
};

use super::{
    contract::ServiceError,
    lifecycle::StopSignal,
    workbuddy_delegate_protocol::{self as protocol, ToolCatalog},
};

const LAUNCHER: &str = include_str!("workbuddy_delegate_launcher.cjs");
const MAX_ACTORS: usize = 8;
const MAX_CALLER_ACTORS: usize = 2;
const MAX_ACCOUNT_ACTORS: usize = 2;
const MAX_INDEX_ENTRIES: usize = 4096;
const MAX_PENDING: usize = 8;
const MAX_CALLS: usize = 256;
const MAX_FRAME: usize = 1024 * 1024;
const MAX_HISTORY: usize = 8 * 1024 * 1024;
const MAX_RESULT: usize = 512 * 1024;
const MAX_RESPONSE_CACHE: usize = 32;
const START_TIMEOUT: Duration = Duration::from_secs(45);
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const TOOL_IDLE: Duration = Duration::from_secs(300);
const READY_IDLE: Duration = Duration::from_secs(180);
const ACTOR_TTL: Duration = Duration::from_secs(1800);
const PROCESS_GROUP_GRACE: Duration = Duration::from_millis(300);
static METADATA_SLOTS: Semaphore = Semaphore::const_new(2);

fn metadata_slot() -> Result<tokio::sync::SemaphorePermit<'static>, ServiceError> {
    METADATA_SLOTS.try_acquire().map_err(|_| {
        error(
            503,
            "workbuddy_catalog_busy",
            "Two WorkBuddy metadata processes are already active; retry after they finish",
        )
    })
}

/// Never derive Debug: the access token is sent only through anonymous stdin.
pub(super) struct LaunchConfig {
    pub account_id: String,
    pub source_generation: String,
    pub model: String,
    pub effort: String,
    pub access_token: String,
    pub domestic: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Owner {
    pub caller_id: String,
    pub thread_id: String,
}

#[derive(Clone)]
pub(super) struct Continuation {
    pub account_id: String,
    pub model: String,
    pub source_generation: String,
    pub thread_id: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Source {
    account_id: String,
    source_generation: String,
    model: String,
    effort: String,
    domestic: bool,
}

impl Source {
    fn from_config(config: &LaunchConfig) -> Self {
        Self {
            account_id: config.account_id.clone(),
            source_generation: config.source_generation.clone(),
            model: config.model.clone(),
            effort: config.effort.clone(),
            domestic: config.domestic,
        }
    }
}

#[derive(Default)]
pub(super) struct Registry {
    index: Mutex<Index>,
    #[cfg(test)]
    test_cli: Mutex<Option<(PathBuf, PathBuf)>>,
}

#[derive(Default)]
struct Index {
    entries: HashMap<String, Arc<Entry>>,
    threads: HashMap<(String, String), String>,
    responses: HashMap<String, String>,
    calls: HashMap<String, String>,
    messages: HashMap<String, String>,
    stopped: bool,
}

struct Entry {
    id: String,
    owner: Owner,
    source: Source,
    catalog: ToolCatalog,
    actor: OnceCell<Result<Arc<Actor>, ServiceError>>,
    gate: Arc<AsyncMutex<()>>,
    cancel: watch::Sender<bool>,
    created: Instant,
}

struct Pending {
    rpc_id: Value,
    item: Value,
    announced: bool,
    delivered: Option<Value>,
    sequence: usize,
}

struct Core {
    failed: Option<ServiceError>,
    rpc_waiting: HashSet<String>,
    rpc_results: HashMap<String, Value>,
    session: String,
    prompt_id: Option<String>,
    prompt_done: bool,
    text: String,
    usage: Option<Value>,
    pending: HashMap<String, Pending>,
    history: Vec<Value>,
    cache: HashMap<String, Value>,
    cache_order: Vec<String>,
    accounting: HashSet<String>,
    delivered: HashSet<String>,
    response_count: usize,
    latest_response: Option<String>,
    result_receipts: HashMap<String, Value>,
    active_result_key: Option<String>,
    ready: bool,
    tools_allowed: bool,
    parallel: bool,
    last_activity: Instant,
    turn_started: Instant,
    event_bytes: usize,
}

impl Default for Core {
    fn default() -> Self {
        Self {
            failed: None,
            rpc_waiting: HashSet::new(),
            rpc_results: HashMap::new(),
            session: String::new(),
            prompt_id: None,
            prompt_done: false,
            text: String::new(),
            usage: None,
            pending: HashMap::new(),
            history: Vec::new(),
            cache: HashMap::new(),
            cache_order: Vec::new(),
            accounting: HashSet::new(),
            delivered: HashSet::new(),
            response_count: 0,
            latest_response: None,
            result_receipts: HashMap::new(),
            active_result_key: None,
            ready: true,
            tools_allowed: true,
            parallel: true,
            last_activity: Instant::now(),
            turn_started: Instant::now(),
            event_bytes: 0,
        }
    }
}

struct Actor {
    core: Mutex<Core>,
    notify: Notify,
    outgoing: mpsc::Sender<Value>,
    cancel: watch::Sender<bool>,
    entry: Weak<Entry>,
    registry: Weak<Registry>,
    cleanup: Notify,
    workspace: PathBuf,
    expected_model: String,
    expected_effort: String,
    cleaned: std::sync::atomic::AtomicBool,
}

fn error(status: u16, code: &'static str, message: &'static str) -> ServiceError {
    ServiceError::new(status, code, message)
}

fn conflict() -> ServiceError {
    error(409, "workbuddy_continuation_conflict", "WorkBuddy conversation ownership, history, source or tool result does not match; start a new conversation")
}

fn expired() -> ServiceError {
    error(409, "workbuddy_conversation_expired", "The original WorkBuddy connection is unavailable; start a new conversation. Tool calls will not be replayed")
}

fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

impl Registry {
    #[cfg(test)]
    pub(super) fn set_test_cli(&self, executable: PathBuf, cli_path: PathBuf) {
        *self.test_cli.lock().unwrap_or_else(|e| e.into_inner()) = Some((executable, cli_path));
    }

    /// Call before generic account selection/rebuild. IDs are references, not
    /// bearer capabilities: the authenticated caller and supplied thread match.
    pub(super) fn continuation(
        &self,
        caller_id: &str,
        body: &Value,
        thread_hint: Option<&str>,
    ) -> Result<Option<Continuation>, ServiceError> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let entry = lookup(&index, caller_id, body, thread_hint)?;
        entry
            .map(|entry| {
                if *entry.cancel.borrow() {
                    return Err(expired());
                }
                Ok(Continuation {
                    account_id: entry.source.account_id.clone(),
                    model: entry.source.model.clone(),
                    source_generation: entry.source.source_generation.clone(),
                    thread_id: entry.owner.thread_id.clone(),
                })
            })
            .transpose()
    }

    pub(super) async fn respond(
        self: &Arc<Self>,
        owner: Owner,
        config: LaunchConfig,
        body: &Value,
        stop: StopSignal,
    ) -> Result<Value, ServiceError> {
        if let Some(choice) = body.get("tool_choice") {
            if !matches!(choice.as_str(), Some("auto" | "none")) {
                return Err(error(
                    400,
                    "workbuddy_tool_choice_unsupported",
                    "Native WorkBuddy currently supports only explicit auto or none tool choice",
                ));
            }
        }
        if body
            .get("parallel_tool_calls")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(error(
                400,
                "workbuddy_parallel_invalid",
                "parallel_tool_calls must be a boolean",
            ));
        }
        if owner.caller_id.is_empty()
            || owner.thread_id.is_empty()
            || config.account_id.is_empty()
            || config.source_generation.is_empty()
            || config.model.is_empty()
            || config.effort.is_empty()
        {
            return Err(error(
                400,
                "workbuddy_config_required",
                "An explicit owner, source, catalog model and effort are required",
            ));
        }
        if body.to_string().len() > MAX_HISTORY {
            return Err(error(
                413,
                "workbuddy_request_too_large",
                "WorkBuddy input exceeds the conversation limit",
            ));
        }
        let items = protocol::input_items(body)?;
        let source = Source::from_config(&config);
        let entry = {
            let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            if index.stopped || stop.stopped() {
                return Err(expired());
            }
            if let Some(entry) = lookup(&index, &owner.caller_id, body, Some(&owner.thread_id))? {
                if entry.owner != owner || entry.source != source {
                    return Err(conflict());
                }
                if let Some(tools) = body.get("tools") {
                    if protocol::ToolCatalog::from_tools(Some(tools))?.fingerprint()
                        != entry.catalog.fingerprint()
                    {
                        return Err(conflict());
                    }
                }
                entry
            } else {
                // Never reconstruct an actor from tool history after a restart.
                if items.iter().any(|item| {
                    matches!(
                        item["type"].as_str(),
                        Some(
                            "function_call"
                                | "function_call_output"
                                | "custom_tool_call"
                                | "custom_tool_call_output"
                                | "reasoning"
                        )
                    ) || item["role"] == "assistant"
                }) {
                    return Err(expired());
                }
                let active: Vec<_> = index
                    .entries
                    .values()
                    .filter(|entry| !*entry.cancel.borrow())
                    .collect();
                if active.len() >= MAX_ACTORS
                    || active
                        .iter()
                        .filter(|entry| entry.owner.caller_id == owner.caller_id)
                        .count()
                        >= MAX_CALLER_ACTORS
                    || active
                        .iter()
                        .filter(|entry| entry.source.account_id == source.account_id)
                        .count()
                        >= MAX_ACCOUNT_ACTORS
                    || index.entries.len() >= MAX_INDEX_ENTRIES
                {
                    return Err(error(429, "workbuddy_actor_limit", "WorkBuddy conversation capacity reached; close or expire an existing conversation"));
                }
                let (cancel, _) = watch::channel(false);
                let entry = Arc::new(Entry {
                    id: uuid::Uuid::new_v4().to_string(),
                    owner: owner.clone(),
                    source,
                    catalog: ToolCatalog::from_tools(body.get("tools"))?,
                    actor: OnceCell::new(),
                    gate: Arc::new(AsyncMutex::new(())),
                    cancel,
                    created: Instant::now(),
                });
                index.threads.insert(
                    (owner.caller_id.clone(), owner.thread_id.clone()),
                    entry.id.clone(),
                );
                index.entries.insert(entry.id.clone(), entry.clone());
                entry
            }
        };
        let _gate = entry.gate.clone().try_lock_owned().map_err(|_| {
            error(
                409,
                "workbuddy_request_in_progress",
                "Another request is already continuing this WorkBuddy conversation",
            )
        })?;
        if *entry.cancel.borrow() {
            return Err(expired());
        }
        let mut cancellation = RequestCancellation {
            cancel: entry.cancel.clone(),
            armed: true,
        };
        let actor = entry
            .actor
            .get_or_init(|| {
                Actor::launch(
                    config,
                    Arc::downgrade(&entry),
                    Arc::downgrade(self),
                    stop.clone(),
                )
            })
            .await
            .clone()?;
        let result = actor.respond(body, items, stop).await;
        cancellation.armed = false;
        result
    }

    pub(super) fn delivery_guard(
        &self,
        caller_id: &str,
        response_id: &str,
    ) -> Result<DeliveryGuard, ServiceError> {
        let entry = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let id = index.responses.get(response_id).ok_or_else(expired)?;
            index.entries.get(id).cloned().ok_or_else(expired)?
        };
        if entry.owner.caller_id != caller_id {
            return Err(conflict());
        }
        let actor = entry
            .actor
            .get()
            .and_then(|actor| actor.as_ref().ok())
            .cloned()
            .ok_or_else(expired)?;
        if actor
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failed
            .is_some()
        {
            return Err(expired());
        }
        Ok(DeliveryGuard {
            actor,
            response_id: response_id.to_string(),
            committed: false,
        })
    }

    pub(super) fn claim_accounting(
        &self,
        caller_id: &str,
        response_id: &str,
    ) -> Result<bool, ServiceError> {
        let entry = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let id = index.responses.get(response_id).ok_or_else(expired)?;
            index.entries.get(id).cloned().ok_or_else(expired)?
        };
        if entry.owner.caller_id != caller_id {
            return Err(conflict());
        }
        let actor = entry
            .actor
            .get()
            .and_then(|actor| actor.as_ref().ok())
            .ok_or_else(expired)?;
        let mut core = actor.core.lock().unwrap_or_else(|e| e.into_inner());
        Ok(core.accounting.insert(response_id.to_string()))
    }

    pub(super) async fn shutdown(&self) {
        let entries: Vec<_> = {
            let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            index.stopped = true;
            index.entries.values().cloned().collect()
        };
        for entry in &entries {
            let _ = entry.cancel.send_replace(true);
        }
        for entry in entries {
            if let Some(Ok(actor)) = entry.actor.get() {
                actor.wait_cleanup().await;
            }
        }
    }

    fn index_call(&self, entry: &Entry, call_id: &str) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        index.calls.insert(call_id.to_string(), entry.id.clone());
    }

    fn index_response(&self, entry: &Entry, response_id: &str, document: &Value) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        index
            .responses
            .insert(response_id.to_string(), entry.id.clone());
        if let Some(items) = document["output"].as_array() {
            for item in items {
                if let Some(id) = item["id"].as_str().filter(|id| id.starts_with("msg_wb_")) {
                    index.messages.insert(id.to_string(), entry.id.clone());
                }
            }
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let index = self.index.get_mut().unwrap_or_else(|e| e.into_inner());
        for entry in index.entries.values() {
            let _ = entry.cancel.send_replace(true);
        }
    }
}

pub(super) struct DeliveryGuard {
    actor: Arc<Actor>,
    response_id: String,
    committed: bool,
}

impl DeliveryGuard {
    pub(super) fn commit(mut self) {
        self.actor
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .delivered
            .insert(self.response_id.clone());
        self.committed = true;
    }
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let delivered = self
            .actor
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .delivered
            .contains(&self.response_id);
        if !delivered {
            self.actor.fail(expired());
        }
    }
}

struct SecretBootstrap(Vec<u8>);

impl Drop for SecretBootstrap {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

struct RequestCancellation {
    cancel: watch::Sender<bool>,
    armed: bool,
}
impl Drop for RequestCancellation {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cancel.send_replace(true);
        }
    }
}

fn lookup(
    index: &Index,
    caller: &str,
    body: &Value,
    thread: Option<&str>,
) -> Result<Option<Arc<Entry>>, ServiceError> {
    let mut selected: Option<String> = None;
    let mut add = |actor_id: Option<&String>, required: bool| -> Result<(), ServiceError> {
        let Some(actor_id) = actor_id else {
            return if required { Err(expired()) } else { Ok(()) };
        };
        if selected.as_ref().is_some_and(|old| old != actor_id) {
            return Err(conflict());
        }
        selected = Some(actor_id.clone());
        Ok(())
    };
    if let Some(thread) = thread {
        add(
            index.threads.get(&(caller.to_string(), thread.to_string())),
            false,
        )?;
    }
    if let Some(previous) = body.get("previous_response_id").filter(|v| !v.is_null()) {
        let previous = previous
            .as_str()
            .filter(|id| id.starts_with("resp_wb_"))
            .ok_or_else(expired)?;
        add(index.responses.get(previous), true)?;
    }
    for item in protocol::input_items(body)? {
        if let Some(id) = item["id"].as_str().filter(|id| id.starts_with("msg_wb_")) {
            add(index.messages.get(id), true)?;
        }
        if matches!(
            item["type"].as_str(),
            Some(
                "function_call"
                    | "function_call_output"
                    | "custom_tool_call"
                    | "custom_tool_call_output"
            )
        ) {
            let id = item["call_id"].as_str().ok_or_else(conflict)?;
            add(index.calls.get(id), true)?;
        }
    }
    let Some(id) = selected else {
        return Ok(None);
    };
    let entry = index.entries.get(&id).cloned().ok_or_else(expired)?;
    if entry.owner.caller_id != caller
        || thread.is_some_and(|thread| thread != entry.owner.thread_id)
    {
        return Err(conflict());
    }
    Ok(Some(entry))
}

impl Actor {
    async fn launch(
        config: LaunchConfig,
        entry: Weak<Entry>,
        registry: Weak<Registry>,
        stop: StopSignal,
    ) -> Result<Arc<Self>, ServiceError> {
        let parent = entry.upgrade().ok_or_else(expired)?;
        #[cfg(test)]
        let launch = match registry
            .upgrade()
            .and_then(|registry| registry.test_cli.lock().ok().and_then(|cli| cli.clone()))
        {
            Some(cli) => prepare_process_with_cli(cli)?,
            None => prepare_process(&config)?,
        };
        #[cfg(not(test))]
        let launch = prepare_process(&config)?;
        Self::launch_prepared(config, entry, registry, parent.cancel.clone(), stop, launch).await
    }

    async fn launch_prepared(
        config: LaunchConfig,
        entry: Weak<Entry>,
        registry: Weak<Registry>,
        cancel: watch::Sender<bool>,
        stop: StopSignal,
        launch: PreparedProcess,
    ) -> Result<Arc<Self>, ServiceError> {
        let PreparedProcess {
            mut command,
            profile,
            cli_path,
            workspace,
            metadata_slot,
        } = launch;
        let mut child = command.spawn().map_err(|_| {
            error(
                503,
                "workbuddy_cli_unavailable",
                "Unable to launch the isolated WorkBuddy CLI",
            )
        })?;
        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or_else(expired)?;
        let stdout = child.stdout.take().ok_or_else(expired)?;
        // The secret is absent from argv, initial environment and the profile.
        let mut bootstrap = serde_json::to_vec(&json!({"accessToken":config.access_token,"domestic":config.domestic,"cliPath":cli_path,"model":config.model,"effort":config.effort})).map_err(|_| expired())?;
        bootstrap.push(b'\n');
        let mut bootstrap = SecretBootstrap(bootstrap);
        let (outgoing, mut receive) = mpsc::channel::<Value>(32);
        let actor = Arc::new(Self {
            core: Mutex::new(Core::default()),
            notify: Notify::new(),
            outgoing,
            cancel,
            entry,
            registry,
            cleanup: Notify::new(),
            workspace,
            expected_model: config.model.clone(),
            expected_effort: config.effort.clone(),
            cleaned: std::sync::atomic::AtomicBool::new(false),
        });
        let mut initialization_cancel = RequestCancellation {
            cancel: actor.cancel.clone(),
            armed: true,
        };
        let writer_actor = Arc::downgrade(&actor);
        let writer = tokio::spawn(async move {
            // Register writer, reader and monitor synchronously before the
            // launch future's first await. A full or non-reading bootstrap pipe
            // is owned by the same cancellation/group-cleanup path as ACP.
            let written = tokio::time::timeout(START_TIMEOUT, stdin.write_all(&bootstrap.0)).await;
            bootstrap.0.fill(0);
            if !matches!(written, Ok(Ok(()))) {
                if let Some(actor) = writer_actor.upgrade() {
                    actor.fail(error(
                        504,
                        "workbuddy_start_timeout",
                        "WorkBuddy private bootstrap write timed out or failed",
                    ));
                }
                return;
            }
            drop(bootstrap);
            while let Some(frame) = receive.recv().await {
                let mut bytes = match serde_json::to_vec(&frame) {
                    Ok(bytes) => bytes,
                    Err(_) => break,
                };
                bytes.push(b'\n');
                if stdin.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            if let Some(actor) = writer_actor.upgrade() {
                actor.fail(expired());
            }
        });
        let reader_actor = Arc::downgrade(&actor);
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut discarded = 0;
            loop {
                let mut line = Vec::new();
                let read = (&mut reader)
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut line)
                    .await;
                let Some(actor) = reader_actor.upgrade() else {
                    break;
                };
                match read {
                    Ok(0) | Err(_) => {
                        actor.fail(expired());
                        break;
                    }
                    Ok(_) if line.len() > MAX_FRAME || !line.ends_with(b"\n") => {
                        actor.fail(error(
                            413,
                            "workbuddy_event_limit",
                            "WorkBuddy CLI event exceeds the bounded transport limit",
                        ));
                        break;
                    }
                    _ => {}
                }
                match serde_json::from_slice::<Value>(&line) {
                    Ok(frame) if frame["jsonrpc"] == "2.0" => actor.handle_frame(frame),
                    _ => {
                        discarded += line.len();
                        if discarded > MAX_FRAME {
                            actor.fail(expired());
                            break;
                        }
                    }
                }
            }
        });
        let monitor_actor = actor.clone();
        let mut cancelled = actor.cancel.subscribe();
        let mut service_stop = stop.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                if *cancelled.borrow() || service_stop.stopped() {
                    break;
                }
                tokio::select! {
                    _ = cancelled.changed() => { if *cancelled.borrow() { break; } }
                    _ = service_stop.wait() => break,
                    _ = interval.tick() => {
                        let now = Instant::now();
                        let expired_now = {
                            let core = monitor_actor.core.lock().unwrap_or_else(|e| e.into_inner());
                            let absolute = monitor_actor.entry.upgrade().is_some_and(|entry| now.duration_since(entry.created) >= ACTOR_TTL);
                            let pending = core.pending.values().any(|call| call.delivered.is_none());
                            absolute || now.duration_since(core.last_activity) >= if pending { TOOL_IDLE } else if core.ready { READY_IDLE } else { TURN_TIMEOUT }
                        };
                        if expired_now { break; }
                    }
                    _ = observe_owned_leader_exit(&mut child, pid) => break,
                }
            }
            monitor_actor.fail(expired());
            // Best-effort errors unblock pending delegate awaits, followed by a
            // bounded cancellation and termination of only this owned child tree.
            let session = monitor_actor
                .core
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session
                .clone();
            let _ = monitor_actor.outgoing.try_send(
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}),
            );
            terminate_owned_child(&mut child, pid).await;
            writer.abort();
            reader.abort();
            let _ = writer.await;
            let _ = reader.await;
            cleanup_profile(profile);
            // A cancelled metadata HTTP must not release its slot while the
            // owned process group is still being terminated.
            drop(metadata_slot);
            {
                let mut core = monitor_actor.core.lock().unwrap_or_else(|e| e.into_inner());
                core.history.clear();
                core.text.clear();
                core.pending.clear();
                core.cache.clear();
                core.cache_order.clear();
                core.result_receipts.clear();
                core.rpc_results.clear();
                core.rpc_waiting.clear();
                core.usage = None;
            }
            monitor_actor
                .cleaned
                .store(true, std::sync::atomic::Ordering::Release);
            monitor_actor.cleanup.notify_waiters();
        });
        let init = actor.rpc("initialize", json!({"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"cockpit-codex-native-delegate","version":"1"}})).await?;
        if init["agentCapabilities"]["delegateToolsSupport"] != true {
            actor.fail(error(
                503,
                "workbuddy_delegate_unsupported",
                "Installed WorkBuddy CLI does not advertise native delegated tools",
            ));
            return Err(error(
                503,
                "workbuddy_delegate_unsupported",
                "Installed WorkBuddy CLI does not advertise native delegated tools",
            ));
        }
        // CLI working directory is the private, empty process directory, never
        // the caller's project. ACP requires the canonical absolute cwd.
        let new = actor
            .rpc(
                "session/new",
                json!({"cwd":actor.workspace,"mcpServers":[]}),
            )
            .await?;
        let session = new["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(expired)?
            .to_string();
        actor.core.lock().unwrap_or_else(|e| e.into_inner()).session = session;
        if !config.model.is_empty() {
            validate_catalog(&new, &config.model, &config.effort)?;
        }
        actor
            .core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rpc_results
            .insert(
                "catalog".into(),
                json!({"models":new["models"],"configOptions":new["configOptions"]}),
            );
        if let Some(entry) = actor.entry.upgrade() {
            if !entry.catalog.is_empty() {
                actor.rpc("_codebuddy.ai/delegateToolsChanged", json!({"changeType":"added","providerId":"cockpit-codex-native","tools":entry.catalog.delegate_definitions()})).await?;
            }
        }
        initialization_cancel.armed = false;
        Ok(actor)
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, ServiceError> {
        let id = format!("wb_rpc_{}", uuid::Uuid::new_v4());
        self.core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rpc_waiting
            .insert(id.clone());
        self.outgoing
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await
            .map_err(|_| expired())?;
        tokio::time::timeout(START_TIMEOUT, async {
            loop {
                let notified = self.notify.notified();
                {
                    let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(error) = &core.failed {
                        return Err(error.clone());
                    }
                    if let Some(frame) = core.rpc_results.remove(&id) {
                        return if frame.get("error").is_some() {
                            Err(error(
                                502,
                                "workbuddy_cli_rpc_failed",
                                "WorkBuddy CLI rejected a required protocol operation",
                            ))
                        } else {
                            Ok(frame["result"].clone())
                        };
                    }
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            error(
                504,
                "workbuddy_start_timeout",
                "WorkBuddy CLI protocol initialization timed out",
            )
        })?
    }

    fn fail(&self, error: ServiceError) {
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if core.failed.is_none() {
            core.failed = Some(error);
        }
        drop(core);
        let _ = self.cancel.send_replace(true);
        self.notify.notify_waiters();
    }

    fn handle_frame(&self, frame: Value) {
        let result = self.handle_frame_inner(frame);
        if let Err(error) = result {
            self.fail(error);
        }
        self.notify.notify_waiters();
    }

    fn handle_frame_inner(&self, frame: Value) -> Result<(), ServiceError> {
        let method = frame["method"].as_str();
        let params = &frame["params"];
        if let Some(method) = method {
            if let Some(rpc_id) = frame.get("id") {
                let entry = self.entry.upgrade().ok_or_else(expired)?;
                let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
                if core.failed.is_some() {
                    return Ok(());
                }
                match method {
                    "session/request_permission" => {
                        let valid = core.tools_allowed
                            && params["sessionId"].as_str() == Some(core.session.as_str())
                            && entry.catalog.validate_permission(params);
                        let allow = params["options"]
                            .as_array()
                            .and_then(|options| {
                                options.iter().find(|option| option["kind"] == "allow_once")
                            })
                            .and_then(|option| option.get("optionId"))
                            .filter(|id| id.is_string());
                        let outcome = if valid && allow.is_some() {
                            json!({"outcome":"selected","optionId":allow.unwrap()})
                        } else {
                            json!({"outcome":"cancelled"})
                        };
                        self.outgoing
                            .try_send(
                                json!({"jsonrpc":"2.0","id":rpc_id,"result":{"outcome":outcome}}),
                            )
                            .map_err(|_| expired())?;
                        if !valid {
                            return Err(error(
                                403,
                                "workbuddy_permission_denied",
                                "WorkBuddy requested an undeclared or non-delegated capability",
                            ));
                        }
                    }
                    "_codebuddy.ai/delegateTool" => {
                        if !core.tools_allowed
                            || core.prompt_id.is_none()
                            || core.pending.len() >= MAX_CALLS
                            || core
                                .pending
                                .values()
                                .filter(|call| call.delivered.is_none())
                                .count()
                                >= MAX_PENDING
                        {
                            return Err(error(
                                429,
                                "workbuddy_pending_limit",
                                "WorkBuddy delegated tool capacity exceeded or tools are disabled",
                            ));
                        }
                        if params["sessionId"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty() && id != core.session)
                        {
                            return Err(conflict());
                        }
                        if core
                            .pending
                            .values()
                            .any(|pending| pending.rpc_id == *rpc_id)
                        {
                            return Err(conflict());
                        }
                        let call_id = format!("call_wb_{}", uuid::Uuid::new_v4().simple());
                        let item = entry.catalog.call_item(params, &call_id)?;
                        let sequence = core.pending.len();
                        core.event_bytes += item.to_string().len();
                        if core.event_bytes > MAX_FRAME {
                            return Err(error(
                                413,
                                "workbuddy_event_limit",
                                "WorkBuddy tool events exceed the bounded turn limit",
                            ));
                        }
                        core.pending.insert(
                            call_id.clone(),
                            Pending {
                                rpc_id: rpc_id.clone(),
                                item,
                                announced: false,
                                delivered: None,
                                sequence,
                            },
                        );
                        drop(core);
                        if let Some(registry) = self.registry.upgrade() {
                            registry.index_call(&entry, &call_id);
                        }
                    }
                    _ => {
                        self.outgoing.try_send(json!({"jsonrpc":"2.0","id":rpc_id,"error":{"code":-32601,"message":"Client capability not permitted by the native delegate bridge"}})).map_err(|_| expired())?;
                        return Err(error(403, "workbuddy_permission_denied", "WorkBuddy requested a forbidden file, terminal or unknown client capability"));
                    }
                }
                return Ok(());
            }
            if method == "session/update" {
                let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|id| !core.session.is_empty() && id != core.session)
                {
                    return Err(conflict());
                }
                let update = &params["update"];
                if update["sessionUpdate"] == "agent_message_chunk" {
                    if let Some(text) = update["content"]["text"].as_str() {
                        core.event_bytes += text.len();
                        if core.event_bytes > MAX_FRAME {
                            return Err(error(
                                413,
                                "workbuddy_event_limit",
                                "WorkBuddy text exceeds the bounded turn limit",
                            ));
                        }
                        core.text.push_str(text);
                    }
                }
                if update["sessionUpdate"] == "model_update" {
                    if update["currentModelId"].as_str().is_some_and(|id| {
                        !self.expected_model.is_empty() && id != self.expected_model
                    }) {
                        return Err(error(
                            409,
                            "workbuddy_model_changed",
                            "WorkBuddy changed the explicitly selected model",
                        ));
                    }
                }
                if let Some(catalog) = core.rpc_results.get_mut("catalog") {
                    if update["sessionUpdate"] == "model_update" {
                        if let Some(models) = update.get("availableModels") {
                            catalog["models"]["availableModels"] = models.clone();
                        }
                        if let Some(current) = update.get("currentModelId") {
                            catalog["models"]["currentModelId"] = current.clone();
                        }
                    } else if update["sessionUpdate"] == "config_option_update" {
                        catalog["configOptions"] = update["configOptions"].clone();
                    }
                    if !self.expected_model.is_empty()
                        && matches!(
                            update["sessionUpdate"].as_str(),
                            Some("model_update" | "config_option_update")
                        )
                    {
                        validate_catalog(catalog, &self.expected_model, &self.expected_effort)?;
                    }
                }
            }
            return Ok(());
        }
        let id = frame["id"].as_str().ok_or_else(conflict)?.to_string();
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if !core.rpc_waiting.remove(&id) {
            return Err(conflict());
        }
        if core.prompt_id.as_deref() == Some(&id) {
            if frame.get("error").is_some() || frame["result"]["stopReason"] != "end_turn" {
                return Err(error(502, "workbuddy_prompt_failed", "WorkBuddy prompt did not finish successfully; no automatic replay is permitted"));
            }
            if core.pending.values().any(|call| call.delivered.is_none()) {
                return Err(conflict());
            }
            core.prompt_done = true;
            core.usage = actual_usage(&frame["result"]);
        } else {
            core.rpc_results.insert(id, frame);
        }
        Ok(())
    }

    async fn respond(
        &self,
        body: &Value,
        items: Vec<Value>,
        mut stop: StopSignal,
    ) -> Result<Value, ServiceError> {
        let canonical: Vec<_> = items.iter().map(semantic).collect();
        let request_key = digest(
            &json!({"input":canonical,"previous_response_id":body.get("previous_response_id"),"instructions":body.get("instructions"),"tools":body.get("tools"),"tool_choice":body.get("tool_choice"),"parallel_tool_calls":body.get("parallel_tool_calls"),"model":body.get("model")}),
        );
        let mut outgoing = Vec::new();
        let mut receipt_key = None;
        {
            let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(error) = &core.failed {
                return Err(error.clone());
            }
            if let Some(document) = core.cache.get(&request_key) {
                return Ok(document.clone());
            }
            if body["previous_response_id"]
                .as_str()
                .is_some_and(|previous| core.latest_response.as_deref() != Some(previous))
            {
                return Err(conflict());
            }
            let delta = history_delta(
                &core.history,
                &canonical,
                body.get("previous_response_id")
                    .is_some_and(Value::is_string),
            )?;
            let results = protocol::tool_results(&delta)?;
            let mut seen = HashSet::new();
            for (call_id, output) in &results {
                if !seen.insert(call_id) {
                    return Err(conflict());
                }
                if output.to_string().len() > MAX_RESULT {
                    return Err(error(
                        413,
                        "workbuddy_result_limit",
                        "Delegated tool result exceeds the bounded result limit",
                    ));
                }
                let pending = core.pending.get(call_id).ok_or_else(conflict)?;
                if !pending.announced || pending.delivered.as_ref().is_some_and(|old| old != output)
                {
                    return Err(conflict());
                }
            }
            let prompt = protocol::turn_prompt(&delta)?;
            if !results.is_empty() && prompt.is_empty() {
                let key = digest(&json!(results));
                if let Some(document) = core.result_receipts.get(&key) {
                    return Ok(document.clone());
                }
                receipt_key = Some(key);
            }
            if !core.ready && !prompt.trim().is_empty() {
                return Err(error(
                    409,
                    "workbuddy_tools_pending",
                    "Finish the outstanding delegated tool results before sending a new user turn",
                ));
            }
            if core.ready
                && results.iter().any(|(id, _)| {
                    core.pending
                        .get(id)
                        .is_none_or(|call| call.delivered.is_none())
                })
            {
                return Err(conflict());
            }
            if core.ready && prompt.trim().is_empty() {
                return Err(error(
                    400,
                    "workbuddy_input_required",
                    "A new WorkBuddy user turn requires text input",
                ));
            }
            let history_size = serde_json::to_vec(&core.history)
                .map_or(MAX_HISTORY + 1, |bytes| bytes.len())
                + serde_json::to_vec(&delta).map_or(MAX_HISTORY + 1, |bytes| bytes.len());
            if history_size > MAX_HISTORY {
                return Err(error(
                    413,
                    "workbuddy_history_limit",
                    "WorkBuddy conversation history exceeds the bounded limit",
                ));
            }
            core.history.extend(delta);
            core.active_result_key = receipt_key;
            for (call_id, output) in results {
                let pending = core.pending.get_mut(&call_id).ok_or_else(conflict)?;
                if pending.delivered.is_none() {
                    outgoing.push(json!({"jsonrpc":"2.0","id":pending.rpc_id,"result":{"status":"success","output":output}}));
                    pending.delivered = Some(output);
                }
            }
            core.tools_allowed = body.get("tool_choice").and_then(Value::as_str) != Some("none");
            core.parallel = body["parallel_tool_calls"] != false;
            if core.ready {
                core.ready = false;
                core.prompt_done = false;
                core.text.clear();
                core.usage = None;
                core.event_bytes = 0;
                core.turn_started = Instant::now();
                let id = format!("wb_prompt_{}", uuid::Uuid::new_v4());
                core.prompt_id = Some(id.clone());
                core.rpc_waiting.insert(id.clone());
                let content = if let Some(instructions) =
                    body["instructions"].as_str().filter(|s| !s.is_empty())
                {
                    format!("{instructions}\n\n{prompt}")
                } else {
                    prompt
                };
                let text = format!("Codex client user turn:\n\n{content}");
                outgoing.push(json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{"sessionId":core.session,"prompt":[{"type":"text","text":text}]}}));
            }
            core.last_activity = Instant::now();
        }
        for frame in outgoing {
            self.outgoing.send(frame).await.map_err(|_| expired())?;
        }
        tokio::time::timeout(TOOL_IDLE, async {
            loop {
                let notified = self.notify.notified();
                match self.next_document(body, &request_key) {
                    Ok(Some(document)) => return Ok(document),
                    Ok(None) => {},
                    Err(error) => { self.fail(error.clone()); return Err(error); }
                }
                tokio::select! { _ = notified => {}, _ = stop.wait() => { self.fail(expired()); return Err(expired()); } }
            }
        }).await.map_err(|_| { self.fail(expired()); error(504, "workbuddy_turn_timeout", "WorkBuddy turn timed out; its original connection was cancelled") })?
    }

    fn next_document(&self, body: &Value, key: &str) -> Result<Option<Value>, ServiceError> {
        let entry = self.entry.upgrade().ok_or_else(expired)?;
        let mut core = self.core.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &core.failed {
            return Err(error.clone());
        }
        let mut calls: Vec<_> = core
            .pending
            .iter()
            .filter(|(_, call)| !call.announced && call.delivered.is_none())
            .map(|(id, call)| (id.clone(), call.sequence))
            .collect();
        calls.sort_by_key(|(_, sequence)| *sequence);
        if !core.parallel {
            if core
                .pending
                .values()
                .any(|call| call.announced && call.delivered.is_none())
            {
                calls.clear();
            } else {
                calls.truncate(1);
            }
        }
        if calls.is_empty() && !core.prompt_done {
            return Ok(None);
        }
        let mut output = Vec::new();
        if !core.text.is_empty() {
            output.push(json!({"id":format!("msg_wb_{}", uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":std::mem::take(&mut core.text),"annotations":[]}]}));
        }
        for (id, _) in calls {
            let call = core.pending.get_mut(&id).ok_or_else(conflict)?;
            call.announced = true;
            output.push(call.item.clone());
        }
        if core.prompt_done {
            core.ready = true;
            core.prompt_id = None;
        }
        core.history.extend(output.iter().map(semantic));
        if core.response_count >= MAX_CALLS {
            return Err(error(
                429,
                "workbuddy_history_limit",
                "WorkBuddy conversation reached its bounded response count",
            ));
        }
        core.response_count += 1;
        let response_id = format!("resp_wb_{}", uuid::Uuid::new_v4().simple());
        core.latest_response = Some(response_id.clone());
        let model = body["model"].as_str().unwrap_or(&entry.source.model);
        let mut document = protocol::response_document(&response_id, model, output);
        if core.prompt_done {
            if let Some(usage) = core.usage.take() {
                document["usage"] = usage;
            }
        }
        if let Some(receipt) = core.active_result_key.take() {
            if core.result_receipts.len() >= MAX_RESPONSE_CACHE {
                core.result_receipts.clear();
            }
            core.result_receipts.insert(receipt, document.clone());
        }
        core.cache.insert(key.to_string(), document.clone());
        core.cache_order.push(key.to_string());
        if core.cache_order.len() > MAX_RESPONSE_CACHE {
            let oldest = core.cache_order.remove(0);
            core.cache.remove(&oldest);
        }
        core.last_activity = Instant::now();
        drop(core);
        if let Some(registry) = self.registry.upgrade() {
            registry.index_response(&entry, &response_id, &document);
        }
        Ok(Some(document))
    }

    async fn wait_cleanup(&self) {
        let notified = self.cleanup.notified();
        if self.cleaned.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), notified).await;
    }
}

/// Initialize/new only; never submit a model prompt. Empty model selects no CLI
/// override. A selected model is still checked against actual returned metadata.
pub(super) async fn catalog(config: LaunchConfig, stop: StopSignal) -> Result<Value, ServiceError> {
    let slot = metadata_slot()?;
    let (cancel, _) = watch::channel(false);
    let mut launch = prepare_process(&config)?;
    launch.metadata_slot = Some(slot);
    let actor = Actor::launch_prepared(
        config,
        Weak::new(),
        Weak::new(),
        cancel.clone(),
        stop,
        launch,
    )
    .await?;
    let value = actor
        .core
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .rpc_results
        .remove("catalog")
        .ok_or_else(expired)?;
    let _ = cancel.send_replace(true);
    actor.wait_cleanup().await;
    Ok(value)
}

fn history_delta(
    history: &[Value],
    input: &[Value],
    previous: bool,
) -> Result<Vec<Value>, ServiceError> {
    if history.is_empty() {
        return Ok(input.to_vec());
    }
    if input.len() >= history.len() && input[..history.len()] == *history {
        return Ok(input[history.len()..].to_vec());
    }
    let only_results = input.iter().all(|item| {
        matches!(
            item["type"].as_str(),
            Some("function_call_output" | "custom_tool_call_output")
        )
    });
    let delta_messages = input.iter().all(|item| {
        matches!(
            item["type"].as_str(),
            Some("function_call_output" | "custom_tool_call_output")
        ) || item["type"] == "message" && item["role"] != "assistant"
    });
    if only_results || previous && delta_messages {
        return Ok(input.to_vec());
    }
    Err(conflict())
}

fn semantic(item: &Value) -> Value {
    match item["type"].as_str().unwrap_or("message") {
        "message" => {
            let content = if let Some(text) = item["content"].as_str() {
                vec![json!({"type":"text","text":text})]
            } else {
                item["content"]
                    .as_array()
                    .map(|content| {
                        content
                            .iter()
                            .map(|part| json!({"type":"text","text":part["text"]}))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            json!({"type":"message","role":item["role"],"content":content})
        }
        "function_call" => {
            let arguments = item["arguments"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .map(|value| value.to_string())
                .unwrap_or_else(|| item["arguments"].as_str().unwrap_or("").to_string());
            let mut value = json!({"type":"function_call","call_id":item["call_id"],"name":item["name"],"arguments":arguments});
            if let Some(namespace) = item.get("namespace") {
                value["namespace"] = namespace.clone();
            }
            value
        }
        "custom_tool_call" => {
            let mut value = json!({"type":"custom_tool_call","call_id":item["call_id"],"name":item["name"],"input":item["input"]});
            if let Some(namespace) = item.get("namespace") {
                value["namespace"] = namespace.clone();
            }
            value
        }
        "function_call_output" | "custom_tool_call_output" => {
            json!({"type":item["type"],"call_id":item["call_id"],"output":item["output"]})
        }
        "reasoning" => {
            let mut item = item.clone();
            if let Some(object) = item.as_object_mut() {
                object.remove("id");
                object.remove("status");
            }
            item
        }
        _ => item.clone(),
    }
}

fn validate_catalog(new: &Value, model: &str, effort: &str) -> Result<(), ServiceError> {
    // A catalog alias is not a pinned upstream model, even when the CLI marks
    // that alias as current and advertises supported efforts for it.
    if ["auto", "default", "enabled"]
        .iter()
        .any(|alias| model.eq_ignore_ascii_case(alias))
    {
        return Err(error(400, "workbuddy_model_unavailable", "Select an explicit WorkBuddy model; automatic or default model aliases are not permitted"));
    }
    let selected_model = new["models"]["availableModels"]
        .as_array()
        .and_then(|models| models.iter().find(|item| item["modelId"] == model));
    if selected_model.is_none() || new["models"]["currentModelId"] != model {
        return Err(error(400, "workbuddy_model_unavailable", "The explicitly selected WorkBuddy model is absent or not active in the current CLI catalog"));
    }
    if effort.is_empty() {
        return Ok(());
    }
    if matches!(effort, "enabled" | "default" | "auto")
        || !selected_model.unwrap()["_meta"]["effortOptions"]
            .as_array()
            .is_some_and(|options| {
                options
                    .iter()
                    .any(|value| value.as_str() == Some(effort) || value["value"] == effort)
            })
    {
        return Err(error(
            400,
            "workbuddy_effort_unavailable",
            "The explicitly selected effort is not supported by this actual model",
        ));
    }
    let choice = new["configOptions"].as_array().and_then(|options| {
        options.iter().find(|item| {
            item["id"].as_str().is_some_and(|id| {
                matches!(
                    id,
                    "thinking" | "thought_level" | "reasoning_effort" | "effort" | "thinkingLevel"
                )
            }) || item["category"].as_str().is_some_and(|category| {
                matches!(
                    category,
                    "thought_level" | "thinking" | "reasoning" | "effort"
                )
            })
        })
    });
    let allowed = choice.is_some_and(|choice| {
        choice["options"]
            .as_array()
            .is_some_and(|options| options.iter().any(|option| option["value"] == effort))
            && choice["currentValue"] == effort
    });
    if !allowed {
        return Err(error(
            400,
            "workbuddy_effort_unavailable",
            "The explicitly selected effort is absent or not active in the current model catalog",
        ));
    }
    Ok(())
}

fn actual_usage(result: &Value) -> Option<Value> {
    let usage = result.get("usage")?;
    let mut out = serde_json::Map::new();
    for (public, alternatives) in [
        ("input_tokens", ["input_tokens", "inputTokens"]),
        ("output_tokens", ["output_tokens", "outputTokens"]),
        ("total_tokens", ["total_tokens", "totalTokens"]),
    ] {
        if let Some(value) = alternatives
            .iter()
            .find_map(|key| usage.get(*key).filter(|v| v.as_u64().is_some()))
        {
            out.insert(public.into(), value.clone());
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

struct PreparedProcess {
    command: Command,
    profile: TempDir,
    cli_path: PathBuf,
    workspace: PathBuf,
    metadata_slot: Option<tokio::sync::SemaphorePermit<'static>>,
}

fn prepare_process(_config: &LaunchConfig) -> Result<PreparedProcess, ServiceError> {
    prepare_process_with_cli(discover_cli()?)
}

fn prepare_process_with_cli(
    (executable, cli_path): (PathBuf, PathBuf),
) -> Result<PreparedProcess, ServiceError> {
    if !executable.is_absolute()
        || !executable.is_file()
        || !cli_path.is_absolute()
        || !cli_path.is_file()
    {
        return Err(expired());
    }
    let profile = tempfile::Builder::new()
        .prefix("cockpit-workbuddy-delegate-")
        .tempdir()
        .map_err(|_| {
            error(
                503,
                "workbuddy_profile_unavailable",
                "Unable to create a private WorkBuddy profile",
            )
        })?;
    let root = profile.path().canonicalize().map_err(|_| expired())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| expired())?;
    }
    for name in ["home", "profile", "workspace", "tmp"] {
        std::fs::create_dir(root.join(name)).map_err(|_| expired())?;
    }
    let launcher = root.join("launcher.cjs");
    std::fs::write(&launcher, LAUNCHER).map_err(|_| expired())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| expired())?;
    }
    let mut command = Command::new(executable);
    command
        .arg(&launcher)
        .env_clear()
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("TMPDIR", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .current_dir(root.join("workspace"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        command.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(0x00000200 | 0x08000000);
        for key in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    Ok(PreparedProcess {
        command,
        profile,
        cli_path,
        workspace: root.join("workspace"),
        metadata_slot: None,
    })
}

fn discover_cli() -> Result<(PathBuf, PathBuf), ServiceError> {
    let configured = crate::modules::config::get_user_config().workbuddy_app_path;
    let mut candidates = Vec::new();
    if !configured.trim().is_empty() {
        let configured = PathBuf::from(configured.trim());
        if configured.extension().is_some_and(|ext| ext == "app") {
            candidates.push(configured.join("Contents/MacOS/Electron"));
            candidates.push(configured.join("Contents/MacOS/WorkBuddy"));
        } else if configured.is_dir() {
            candidates.push(configured.join("WorkBuddy.exe"));
        } else {
            candidates.push(configured);
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
        ));
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/WorkBuddy.app/Contents/MacOS/Electron"));
        }
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local).join("Programs/WorkBuddy/WorkBuddy.exe"));
        }
        if let Some(programs) = std::env::var_os("PROGRAMFILES") {
            candidates.push(PathBuf::from(programs).join("WorkBuddy/WorkBuddy.exe"));
        }
    }
    for executable in candidates {
        if !executable.is_file() {
            continue;
        }
        let Some(parent) = executable.parent() else {
            continue;
        };
        for relative in [
            "../Resources/app.asar.unpacked/cli/dist/codebuddy-lite-wb.mjs",
            "resources/app.asar.unpacked/cli/dist/codebuddy-lite-wb.mjs",
        ] {
            let cli = parent.join(relative);
            if cli.is_file() {
                return Ok((
                    executable.canonicalize().map_err(|_| expired())?,
                    cli.canonicalize().map_err(|_| expired())?,
                ));
            }
        }
    }
    Err(error(503, "workbuddy_cli_unavailable", "Install a compatible WorkBuddy desktop CLI or configure its application path; no fallback is available"))
}

#[cfg(unix)]
async fn observe_owned_leader_exit(_child: &mut tokio::process::Child, pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    loop {
        // WNOWAIT is essential: retain the leader (including a zombie) until
        // group cleanup completes, so its PID/PGID cannot be recycled.
        let exited = {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let observed = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            observed != 0 || info.si_signo != 0
        };
        if exited {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(not(unix))]
async fn observe_owned_leader_exit(child: &mut tokio::process::Child, _pid: Option<u32>) {
    let _ = child.wait().await;
}

async fn terminate_owned_child(child: &mut tokio::process::Child, pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.filter(|pid| *pid > 1 && child.id() == Some(*pid)) {
        let pid = pid as libc::pid_t;
        // Never signal a stale number. The owned leader is deliberately unreaped
        // by the monitor, anchoring its ID through TERM, grace and group KILL.
        // A peer cannot move this dedicated child into the application's group.
        // process_group(0) succeeded atomically with spawn. Query the group:
        // getpgid(leader) may fail after a quick exit even before it is reaped.
        let owned_group = unsafe { libc::kill(-pid, 0) == 0 };
        if owned_group {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            // Leader exit must not skip the descendant grace period. Do not
            // poll Child::wait/try_wait here: either would release the anchor.
            tokio::time::sleep(PROCESS_GROUP_GRACE).await;
            // getpgid(leader) may return ESRCH for an unreaped zombie on
            // macOS. Its unreaped PID still anchors the group identity; query
            // the group itself, not the dead leader, before the final signal.
            let group_exists = unsafe { libc::kill(-pid, 0) == 0 };
            if group_exists {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let mut kill = Command::new(PathBuf::from(root).join("System32/taskkill.exe"));
            kill.args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            kill.creation_flags(0x08000000);
            let _ = tokio::time::timeout(Duration::from_secs(2), kill.status()).await;
        }
    }
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
}

fn cleanup_profile(profile: TempDir) {
    let target = profile.path().to_path_buf();
    // TempDir only ever owns this random path. Verify absolute/canonical identity
    // immediately before its delete; never delete a caller-supplied directory.
    let expected = target
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .zip(target.file_name())
        .map(|(parent, name)| parent.join(name));
    let verified = target.is_absolute()
        && target.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with("cockpit-workbuddy-delegate-")
        })
        && target
            .canonicalize()
            .is_ok_and(|actual| expected.as_ref() == Some(&actual));
    if verified {
        let _ = profile.close();
    } else {
        let _ = profile.keep();
    }
}

#[cfg(test)]
#[path = "workbuddy_delegate_runtime_tests.rs"]
mod tests;
