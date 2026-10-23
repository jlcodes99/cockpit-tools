//! Bounded, read-only dialog transport. The regular injection poll remains a
//! fallback, but clicks and completed reads need not wait for its next tick.
use super::*;
use std::sync::Arc;
use tokio::sync::mpsc;

const BINDING: &str = "__cockpitUsageRequest";
const TARGET_LIMIT: usize = 8;
const PAYLOAD_LIMIT: usize = 2048;

#[derive(Clone, Copy)]
struct ConnectionTiming {
    timeout: Duration,
    heartbeat: Duration,
    health: Duration,
}

impl Default for ConnectionTiming {
    fn default() -> Self {
        Self {
            timeout: CDP_CONNECT_TIMEOUT,
            heartbeat: Duration::from_secs(5),
            health: Duration::from_millis(250),
        }
    }
}

enum Event {
    Targets(Vec<String>),
    Fallback(Vec<dialog_data::DialogQuery>),
    Ready(String, u64),
    Query(String, u64, Option<dialog_data::DialogQuery>),
    Closed(String, u64),
}

pub(super) struct LiveDialogBridge {
    events: mpsc::Sender<Event>,
    ready: Arc<Mutex<HashSet<String>>>,
    latest: Arc<Mutex<Value>>,
    task: tokio::task::JoinHandle<()>,
}

impl LiveDialogBridge {
    pub(super) fn new(profile: &Path) -> Self {
        Self::with_reader(
            profile,
            Arc::new(dialog_data::read_dialog_data),
            ConnectionTiming::default(),
        )
    }

    fn with_reader(
        profile: &Path,
        reader: InjectionDialogReader,
        timing: ConnectionTiming,
    ) -> Self {
        let (events, receiver) = mpsc::channel(32);
        let ready = Arc::new(Mutex::new(HashSet::new()));
        let latest = Arc::new(Mutex::new(json!({})));
        let task = tokio::spawn(run(
            profile.to_path_buf(),
            events.clone(),
            receiver,
            ready.clone(),
            latest.clone(),
            reader,
            timing,
        ));
        Self {
            events,
            ready,
            latest,
            task,
        }
    }

    pub(super) fn collect(&self) -> Value {
        self.latest
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| json!({}))
    }

    pub(super) fn request(&self, queries: Vec<dialog_data::DialogQuery>) {
        let _ = self.events.try_send(Event::Fallback(queries));
    }

    pub(super) fn reconcile(&self, targets: &[CdpTarget], port: u16) {
        let urls = targets
            .iter()
            .filter(|target| is_codex_app_target(target))
            .filter_map(|target| target.websocket_url.as_ref())
            .filter(|url| is_safe_cdp_websocket_url(url, port))
            .take(TARGET_LIMIT)
            .cloned()
            .collect();
        let _ = self.events.try_send(Event::Targets(urls));
    }

    pub(super) fn ready(&self, target: &CdpTarget) -> bool {
        target
            .websocket_url
            .as_ref()
            .is_some_and(|url| self.ready.lock().is_ok_and(|ready| ready.contains(url)))
    }
}

impl Drop for LiveDialogBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Target {
    generation: u64,
    replies: mpsc::Sender<Value>,
    abort: tokio::task::AbortHandle,
    query: Option<dialog_data::DialogQuery>,
    delivered: Option<Value>,
}

async fn run(
    profile: PathBuf,
    events: mpsc::Sender<Event>,
    mut receiver: mpsc::Receiver<Event>,
    ready: Arc<Mutex<HashSet<String>>>,
    latest: Arc<Mutex<Value>>,
    reader: InjectionDialogReader,
    timing: ConnectionTiming,
) {
    let mut sockets = JoinSet::new();
    let mut targets: HashMap<String, Target> = HashMap::new();
    let mut poller = InjectionDialogPoller::with_reader(&profile, reader);
    let mut fallback = Vec::new();
    let mut generation = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(75));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            event = receiver.recv() => match event {
                None => break,
                Some(Event::Fallback(queries)) => fallback = queries,
                Some(Event::Targets(urls)) => {
                    targets.retain(|url, target| {
                        if urls.contains(url) { return true; }
                        target.abort.abort();
                        if let Ok(mut ready) = ready.lock() { ready.remove(url); }
                        false
                    });
                    for url in urls {
                        if targets.contains_key(&url) || targets.len() >= TARGET_LIMIT { continue; }
                        generation = generation.wrapping_add(1);
                        let current = generation;
                        let (replies, inbox) = mpsc::channel(2);
                        let target_url = url.clone();
                        let sender = events.clone();
                        let abort = sockets.spawn(async move {
                            connection(&target_url, current, sender.clone(), inbox, timing).await;
                            let _ = sender.send(Event::Closed(target_url, current)).await;
                        });
                        targets.insert(url, Target {generation:current,replies,abort,query:None,delivered:None});
                    }
                }
                Some(Event::Ready(url, generation)) => {
                    if targets.get(&url).is_some_and(|target| target.generation == generation) {
                        if let Ok(mut ready) = ready.lock() { ready.insert(url); }
                    }
                }
                Some(Event::Closed(url, generation)) => {
                    if targets.get(&url).is_some_and(|target| target.generation == generation) {
                        targets.remove(&url);
                        if let Ok(mut ready) = ready.lock() { ready.remove(&url); }
                    }
                }
                Some(Event::Query(url, generation, query)) => {
                    if let Some(target) = targets.get_mut(&url).filter(|target| target.generation == generation) {
                        if target.query != query { target.delivered = None; }
                        target.query = query;
                    }
                }
            },
            _ = tick.tick(), if !poller.tasks.is_empty() => {},
        }
        while sockets.try_join_next().is_some() {}
        // Both transports share one poller, including deduplication, deadlines
        // and the two physical blocking-worker slots during handovers.
        poller.request(
            targets
                .values()
                .filter_map(|target| target.query.clone())
                .chain(fallback.iter().cloned())
                .collect(),
        );
        let state = poller.collect();
        if let Ok(mut latest) = latest.lock() {
            if *latest != state {
                *latest = state.clone();
            }
        }
        for target in targets.values_mut() {
            let Some(query) = &target.query else {
                continue;
            };
            let Some(value) = state.get(&query.request_token) else {
                continue;
            };
            if value.get("loading").and_then(Value::as_bool) == Some(true)
                || target.delivered.as_ref() == Some(value)
            {
                continue;
            }
            if target.replies.try_send(value.clone()).is_ok() {
                target.delivered = Some(value.clone());
            }
        }
    }
}

fn binding_query(value: &Value) -> Option<Option<dialog_data::DialogQuery>> {
    if value.get("method")?.as_str()? != "Runtime.bindingCalled"
        || value.pointer("/params/name")?.as_str()? != BINDING
    {
        return None;
    }
    let payload = value.pointer("/params/payload")?.as_str()?;
    if payload.len() > PAYLOAD_LIMIT {
        return None;
    }
    let query: Value = serde_json::from_str(payload).ok()?;
    if query.is_null() {
        return Some(None);
    }
    dialog_data::parse_query(&query).map(Some)
}

fn delivery_script(value: &Value) -> String {
    // Serialize data as a JS value, never interpolate it into a quoted string.
    format!("(()=>{{const r=window.__cockpitCodexInjection;if(!r||!r.quotaDetailsOpen)return;const v={value};if(r.dialogState?.pending?.requestToken!==v.requestToken)return;r.usageState=r.usageState||{{}};r.usageState.__dialog={{[v.requestToken]:v}};r.render?.();}})()")
}

async fn connection(
    url: &str,
    generation: u64,
    events: mpsc::Sender<Event>,
    mut replies: mpsc::Receiver<Value>,
    timing: ConnectionTiming,
) {
    let Ok(Ok((mut socket, _))) = timeout(timing.timeout, connect_async(url)).await else {
        return;
    };
    for command in [
        json!({"id":1,"method":"Runtime.enable"}),
        json!({"id":2,"method":"Runtime.addBinding","params":{"name":BINDING}}),
    ] {
        if !matches!(
            timeout(
                timing.timeout,
                socket.send(Message::Text(command.to_string().into()))
            )
            .await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
    let mut connected = false;
    let mut setup_timeout = Box::pin(tokio::time::sleep(timing.timeout));
    let mut command_id = 3u64;
    let mut acknowledgements: HashMap<u64, Instant> = HashMap::new();
    let mut heartbeat = tokio::time::interval(timing.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut health = tokio::time::interval(timing.health);
    loop {
        tokio::select! {
            _ = &mut setup_timeout, if !connected => return,
            _ = health.tick() => {
                if acknowledgements.values().any(|deadline|Instant::now()>=*deadline) { return; }
            }
            _ = heartbeat.tick(), if connected => {
                let command = json!({"id":command_id,"method":"Runtime.evaluate","params":{"expression":"true","returnByValue":true}});
                acknowledgements.insert(command_id,Instant::now()+timing.timeout);
                command_id=command_id.wrapping_add(1);
                if !matches!(timeout(timing.timeout,socket.send(Message::Text(command.to_string().into()))).await,Ok(Ok(()))) { return; }
            }
            response = replies.recv() => {
                let Some(response) = response else { return; };
                let command = json!({"id":command_id,"method":"Runtime.evaluate","params":{"expression":delivery_script(&response),"returnByValue":true}});
                acknowledgements.insert(command_id,Instant::now()+timing.timeout);
                command_id = command_id.wrapping_add(1);
                if !matches!(timeout(timing.timeout, socket.send(Message::Text(command.to_string().into()))).await, Ok(Ok(()))) { return; }
            }
            message = socket.next() => {
                let Some(Ok(message)) = message else { return; };
                match message {
                    Message::Ping(payload) => { if !matches!(timeout(timing.timeout,socket.send(Message::Pong(payload))).await,Ok(Ok(()))) { return; } }
                    Message::Close(_) => return,
                    Message::Text(text) if text.len() <= 65536 => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };
                        if let Some(id)=value.get("id").and_then(Value::as_u64) {
                            if acknowledgements.remove(&id).is_some()
                                && (value.get("error").is_some() || value.pointer("/result/exceptionDetails").is_some()) { return; }
                        }
                        if value.get("id").and_then(Value::as_u64) == Some(2) {
                            if value.get("error").is_some() { return; }
                            connected = true;
                            let _ = events.send(Event::Ready(url.into(), generation)).await;
                            // Pick up an already-open dialog when the connection starts/reconnects.
                            let script = format!("(()=>{{const r=window.__cockpitCodexInjection;const q=r?.dialogState?.pending;if(r?.quotaDetailsOpen&&q)window.{BINDING}(JSON.stringify(q));}})()");
                            acknowledgements.insert(0,Instant::now()+timing.timeout);
                            if !matches!(timeout(timing.timeout,socket.send(Message::Text(json!({"id":0,"method":"Runtime.evaluate","params":{"expression":script}}).to_string().into()))).await,Ok(Ok(()))) { return; }
                        } else if let Some(query) = binding_query(&value) {
                            if events.try_send(Event::Query(url.into(), generation, query)).is_err() { return; }
                        }
                    }
                    _ => {},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Condvar,
    };

    enum Control {
        Event(Value),
        Silent,
        Error,
        Exception,
        Close,
    }

    struct MockCdp {
        port: u16,
        url: String,
        peers: Arc<Mutex<Vec<mpsc::Sender<Control>>>>,
        commands: mpsc::UnboundedReceiver<(usize, Value)>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockCdp {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl MockCdp {
        async fn new() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let peers = Arc::new(Mutex::new(Vec::new()));
            let server_peers = peers.clone();
            let (commands, receiver) = mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        accepted=listener.accept()=> {
                            let Ok((stream,_))=accepted else {return;};
                            let peers=server_peers.clone();
                            let commands=commands.clone();
                            connections.spawn(async move {
                                let mut socket=tokio_tungstenite::accept_async(stream).await.unwrap();
                                let (sender,mut controls)=mpsc::channel(16);
                                let index={let mut peers=peers.lock().unwrap();let index=peers.len();peers.push(sender);index};
                                let mut silent=false;
                                let mut fail_next=None;
                                loop {
                                    tokio::select! {
                                        control=controls.recv()=>match control {
                                            Some(Control::Event(value))=>if socket.send(Message::Text(value.to_string().into())).await.is_err(){return;},
                                            Some(Control::Silent)=>silent=true,
                                            Some(Control::Error)=>fail_next=Some(false),
                                            Some(Control::Exception)=>fail_next=Some(true),
                                            Some(Control::Close)|None=>{let _=socket.close(None).await;return;},
                                        },
                                        message=socket.next()=>{
                                            let Some(Ok(Message::Text(text)))=message else {return;};
                                            let value:Value=serde_json::from_str(&text).unwrap();
                                            let _=commands.send((index,value.clone()));
                                            if silent {continue;}
                                            let Some(id)=value["id"].as_u64() else {continue;};
                                            let failure=if value["method"]=="Runtime.evaluate" {fail_next.take()} else {None};
                                            let reply=match failure {
                                                Some(false)=>json!({"id":id,"error":{"message":"fixture protocol error"}}),
                                                Some(true)=>json!({"id":id,"result":{"exceptionDetails":{"text":"fixture exception"}}}),
                                                None=>json!({"id":id,"result":{"result":{"type":"boolean","value":true}}}),
                                            };
                                            if socket.send(Message::Text(reply.to_string().into())).await.is_err(){return;}
                                        }
                                    }
                                }
                            });
                        },
                        _=connections.join_next(), if !connections.is_empty()=>{},
                    }
                }
            });
            Self {
                port,
                url: format!("ws://127.0.0.1:{port}/devtools/page/fixture"),
                peers,
                commands: receiver,
                task,
            }
        }

        fn target(&self) -> CdpTarget {
            CdpTarget {
                target_id: "fixture".into(),
                target_type: "page".into(),
                url: "app://-/fixture".into(),
                websocket_url: Some(self.url.clone()),
            }
        }

        async fn send(&self, peer: usize, control: Control) {
            let sender = self
                .peers
                .lock()
                .unwrap()
                .get(peer)
                .cloned()
                .expect("mock connection exists");
            timeout(Duration::from_secs(1), sender.send(control))
                .await
                .unwrap()
                .unwrap();
        }

        async fn query(&self, peer: usize, query: &dialog_data::DialogQuery) {
            self.send(
                peer,
                Control::Event(json!({"method":"Runtime.bindingCalled","params":{
                    "name":BINDING,"payload":serde_json::to_string(query).unwrap()
                }})),
            )
            .await;
        }

        async fn delivery(&mut self, token: &str) -> Value {
            timeout(Duration::from_secs(2), async {
                loop {
                    let (_, command) = self.commands.recv().await.expect("mock server stays alive");
                    if command
                        .pointer("/params/expression")
                        .and_then(Value::as_str)
                        .is_some_and(|script| script.contains("const v=") && script.contains(token))
                    {
                        return command;
                    }
                }
            })
            .await
            .expect("live result must arrive without fallback polling")
        }
    }

    fn timing() -> ConnectionTiming {
        ConnectionTiming {
            timeout: Duration::from_millis(300),
            heartbeat: Duration::from_millis(100),
            health: Duration::from_millis(10),
        }
    }

    fn query(token: &str) -> dialog_data::DialogQuery {
        dialog_data::parse_query(&json!({"requestToken":token,"threadId":null,
            "scope":"service","period":"today","cursor":null,"view":"logs","binding":null}))
        .unwrap()
    }

    fn result(query: &dialog_data::DialogQuery) -> Value {
        json!({"requestToken":query.request_token,"threadId":query.thread_id,"binding":query.binding,
            "scope":query.scope,"period":query.period,"cursor":query.cursor,"view":query.view,
            "available":true,"requests":{"rows":[{"id":"1","model":"fixture-model"}],"nextCursor":null}})
    }

    async fn ready(bridge: &LiveDialogBridge, target: &CdpTarget, expected: bool) {
        timeout(Duration::from_secs(2), async {
            while bridge.ready(target) != expected {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("live transport readiness must settle promptly");
    }

    #[tokio::test]
    async fn live_mock_cdp_delivers_click_results_without_waiting_for_poll_tick() {
        let mut server = MockCdp::new().await;
        let bridge = LiveDialogBridge::with_reader(
            Path::new("unused-fixture"),
            Arc::new(|_, query| Ok(result(query))),
            timing(),
        );
        let target = server.target();
        bridge.reconcile(&[target], server.port);
        ready(&bridge, &server.target(), true).await;
        let query = query("direct-click");
        server.query(0, &query).await;
        let delivery = server.delivery(&query.request_token).await;
        assert_eq!(delivery["method"], "Runtime.evaluate");
        assert!(delivery["params"]["expression"]
            .as_str()
            .unwrap()
            .contains("fixture-model"));
        assert_eq!(bridge.collect()["direct-click"], result(&query));
    }

    #[tokio::test]
    async fn live_and_fallback_deduplicate_tokens_and_share_two_physical_workers() {
        let mut server = MockCdp::new().await;
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let (reader_gate, reader_running, reader_peak, reader_calls) =
            (gate.clone(), running.clone(), peak.clone(), calls.clone());
        let reader: InjectionDialogReader = Arc::new(move |_, query| {
            let current = reader_running.fetch_add(1, Ordering::SeqCst) + 1;
            reader_peak.fetch_max(current, Ordering::SeqCst);
            *reader_calls
                .lock()
                .unwrap()
                .entry(query.request_token.clone())
                .or_default() += 1;
            let (lock, condition) = &*reader_gate;
            let released = lock.lock().unwrap();
            let (released, _) = condition
                .wait_timeout_while(released, Duration::from_secs(2), |released| !*released)
                .unwrap();
            reader_running.fetch_sub(1, Ordering::SeqCst);
            if !*released {
                return Err("fixture gate timed out".into());
            }
            Ok(result(query))
        });
        let bridge = LiveDialogBridge::with_reader(Path::new("unused-fixture"), reader, timing());
        bridge.reconcile(&[server.target()], server.port);
        ready(&bridge, &server.target(), true).await;
        server.query(0, &query("shared-token")).await;
        timeout(Duration::from_secs(1), async {
            while running.load(Ordering::SeqCst) < 1 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        bridge.request(vec![
            query("shared-token"),
            query("fallback-2"),
            query("queued-3"),
        ]);
        timeout(Duration::from_secs(1), async {
            while running.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.lock().unwrap()["shared-token"], 1);
        bridge.request(vec![query("replacement-4")]);
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert!(!calls.lock().unwrap().contains_key("replacement-4"));
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        server.delivery("shared-token").await;
        timeout(Duration::from_secs(1), async {
            loop {
                if bridge
                    .collect()
                    .get("replacement-4")
                    .is_some_and(|value| value["available"] == true)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(calls.lock().unwrap()["shared-token"], 1);
        assert!(!calls.lock().unwrap().contains_key("queued-3"));
    }

    #[tokio::test]
    async fn live_silent_connection_times_out_and_fallback_remains_usable() {
        let mut server = MockCdp::new().await;
        let bridge = LiveDialogBridge::with_reader(
            Path::new("unused-fixture"),
            Arc::new(|_, query| Ok(result(query))),
            timing(),
        );
        bridge.reconcile(&[server.target()], server.port);
        ready(&bridge, &server.target(), true).await;
        server.send(0, Control::Silent).await;
        server.query(0, &query("unacknowledged")).await;
        server.delivery("unacknowledged").await;
        ready(&bridge, &server.target(), false).await;
        bridge.request(vec![query("fallback-after-timeout")]);
        timeout(Duration::from_secs(1), async {
            loop {
                if bridge
                    .collect()
                    .get("fallback-after-timeout")
                    .is_some_and(|value| value["available"] == true)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn live_protocol_errors_and_exceptions_clear_ready_before_reconnect() {
        for control in [Control::Error, Control::Exception] {
            let server = MockCdp::new().await;
            let bridge = LiveDialogBridge::with_reader(
                Path::new("unused-fixture"),
                Arc::new(|_, query| Ok(result(query))),
                timing(),
            );
            bridge.reconcile(&[server.target()], server.port);
            ready(&bridge, &server.target(), true).await;
            server.send(0, control).await;
            server.query(0, &query("failed-delivery")).await;
            ready(&bridge, &server.target(), false).await;
        }
    }

    #[tokio::test]
    async fn live_reconnect_rejects_old_generation_queries_and_close_events() {
        let mut server = MockCdp::new().await;
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let reader_calls = calls.clone();
        let bridge = LiveDialogBridge::with_reader(
            Path::new("unused-fixture"),
            Arc::new(move |_, query| {
                reader_calls
                    .lock()
                    .unwrap()
                    .push(query.request_token.clone());
                Ok(result(query))
            }),
            timing(),
        );
        bridge.reconcile(&[server.target()], server.port);
        ready(&bridge, &server.target(), true).await;
        server.send(0, Control::Close).await;
        ready(&bridge, &server.target(), false).await;
        bridge.reconcile(&[server.target()], server.port);
        ready(&bridge, &server.target(), true).await;
        for event in [
            Event::Query(server.url.clone(), 1, Some(query("stale-generation"))),
            Event::Ready(server.url.clone(), 1),
            Event::Closed(server.url.clone(), 1),
        ] {
            bridge.events.send(event).await.unwrap();
        }
        server.query(1, &query("new-generation")).await;
        server.delivery("new-generation").await;
        assert!(bridge.ready(&server.target()));
        assert!(!calls
            .lock()
            .unwrap()
            .iter()
            .any(|token| token == "stale-generation"));
        assert!(bridge.collect().get("stale-generation").is_none());
    }

    #[test]
    fn live_queries_are_bounded_validated_and_cannot_request_paths() {
        let event = |payload: Value| json!({"method":"Runtime.bindingCalled","params":{"name":BINDING,"payload":payload.to_string()}});
        assert!(matches!(binding_query(&event(Value::Null)), Some(None)));
        let valid = json!({"requestToken":"live-1","threadId":null,"scope":"service","period":"today","cursor":null,"view":"logs","binding":null});
        assert!(matches!(
            binding_query(&event(valid.clone())),
            Some(Some(_))
        ));
        let mut invalid = valid;
        invalid["threadId"] = json!("../../auth.json");
        assert!(binding_query(&event(invalid)).is_none());
        assert!(binding_query(&event(json!("x".repeat(PAYLOAD_LIMIT)))).is_none());
        assert!(binding_query(&json!({"method":"Other","params":{}})).is_none());
    }

    #[test]
    fn live_results_require_the_current_open_dialog_and_request_token() {
        let script = delivery_script(&json!({"requestToken":"token","requests":{"rows":[]}}));
        assert!(script.contains("!r.quotaDetailsOpen"));
        assert!(script.contains("pending?.requestToken!==v.requestToken"));
        assert!(script.contains("r.render?.()"));
        assert!(!script.contains("fetch("));
    }
}
