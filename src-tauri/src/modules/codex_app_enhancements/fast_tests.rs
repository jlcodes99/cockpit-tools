use super::*;
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

#[test]
fn live_quota_condition_requires_valid_minified_locals() {
    assert_eq!(valid_quota_identifier(Some("Ct")), Some("Ct"));
    assert_eq!(valid_quota_identifier(Some("$gate")), Some("$gate"));
    for invalid in [
        None,
        Some(""),
        Some("1gate"),
        Some("a-b"),
        Some("a;alert(1)"),
    ] {
        assert_eq!(valid_quota_identifier(invalid), None);
    }
    assert_eq!(
        live_quota_condition("Ct", "vn"),
        "(window.__cockpitEnhancements?.apiService===true&&window.__cockpitEnhancements?.quotaCompat===true&&(Ct=false,vn=false),false)"
    );
}

struct Options {
    api_service: bool,
    native_fast: bool,
    api_key_auth_verified: bool,
    fast_only: bool,
    full_quota_only: bool,
    initially_active: bool,
    browser_preloading_reads: usize,
    quota_compat: bool,
    public_entry: bool,
    live_quota: bool,
    live_plan_invalid: bool,
    debugger_error: bool,
    preload_first: bool,
    profile_dir: PathBuf,
    draft_snapshot: Option<Value>,
    draft_approval_rejection: Option<&'static str>,
    startup: Option<Arc<StartupReload>>,
    prepare_failures: usize,
    prepare_reason: &'static str,
    inspect_failures: usize,
    inspect_reason: &'static str,
    inspect_sequence: Vec<&'static str>,
    registration_error: bool,
    delay_registration: bool,
    guard_ok: bool,
    reload_new_context: bool,
    reload_error: bool,
    reload_rejections: Vec<&'static str>,
    disconnect_on_reload: bool,
    active_new_context: bool,
    capability_only: bool,
    rejected_new_context: Option<&'static str>,
    metadata_length: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            api_service: false,
            native_fast: true,
            api_key_auth_verified: true,
            fast_only: false,
            full_quota_only: false,
            initially_active: false,
            browser_preloading_reads: 0,
            quota_compat: true,
            public_entry: false,
            live_quota: false,
            live_plan_invalid: false,
            debugger_error: false,
            preload_first: false,
            profile_dir: PathBuf::new(),
            draft_snapshot: None,
            draft_approval_rejection: None,
            startup: Some(Arc::new(StartupReload::new())),
            prepare_failures: 0,
            prepare_reason: "unsupported-gates",
            inspect_failures: 0,
            inspect_reason: "document-loading",
            inspect_sequence: Vec::new(),
            registration_error: false,
            delay_registration: false,
            guard_ok: true,
            reload_new_context: true,
            reload_error: false,
            reload_rejections: Vec::new(),
            disconnect_on_reload: false,
            active_new_context: true,
            capability_only: false,
            rejected_new_context: None,
            metadata_length: Some(32),
        }
    }
}

struct Running {
    stop: watch::Sender<bool>,
    runtime: tokio::task::JoinHandle<Result<(), String>>,
    server: tokio::task::JoinHandle<()>,
    events: mpsc::UnboundedSender<Value>,
    notices: mpsc::UnboundedReceiver<Value>,
    calls: Arc<Mutex<Vec<Value>>>,
    navigations: Arc<AtomicU64>,
}

fn context(id: i64, frame: &str) -> Value {
    json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":id,"origin":"app://-","auxData":{"isDefault":true,"frameId":frame}}}})
}

const FIXTURE_URL: &str = "app://-/assets/app-initial-mock.js";

fn argument(expression: &str, method: &str) -> Option<String> {
    expression
        .strip_prefix(&format!("window.__cockpitNativeFast.{method}("))
        .and_then(|value| value.strip_suffix(')'))
        .and_then(|value| serde_json::from_str(value).ok())
}

async fn start(options: Options) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (events, mut events_rx) = mpsc::unbounded_channel::<Value>();
    let (notices_tx, notices) = mpsc::unbounded_channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server_calls = calls.clone();
    let navigations = Arc::new(AtomicU64::new(0));
    let server_navigations = navigations.clone();
    let startup = options.startup.clone();
    let profile_dir = options.profile_dir.clone();
    let api_service = options.api_service;
    let native_fast = options.native_fast;
    let api_key_auth_verified = options.api_key_auth_verified;
    let quota_compat = options.quota_compat;
    let public_entry = options.public_entry;
    let live_quota = options.live_quota;
    let preload_first = options.preload_first;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let mut registrations = 0;
        let mut preparations = 0;
        let mut inspections = 0;
        let mut browser_probes = 0;
        let mut reload_attempts = 0;
        let mut delayed_registration = None;
        let mut prepared_nonce: Option<String> = None;
        loop {
            let message = tokio::select! {
                event = events_rx.recv() => {
                    let Some(event) = event else { break; };
                    if socket.send(Message::Text(event.to_string().into())).await.is_err() { break; }
                    continue;
                }
                message = socket.next() => message,
            };
            let Some(Ok(Message::Text(text))) = message else {
                break;
            };
            let message: Value = serde_json::from_str(&text).unwrap();
            let method = message["method"].as_str().unwrap();
            server_calls.lock().unwrap().push(message.clone());
            let mut notice = None;
            let mut new_context = false;
            let mut error = None;
            let result = match method {
                "Page.enable" | "Page.removeScriptToEvaluateOnNewDocument" => {
                    json!({})
                }
                "Page.getFrameTree" => {
                    json!({"frameTree":{"frame":{"id":"main","url":"app://-/index.html"}}})
                }
                "Runtime.enable" => {
                    for event in [context(1, "main"), context(99, "child")] {
                        socket
                            .send(Message::Text(event.to_string().into()))
                            .await
                            .unwrap();
                    }
                    json!({})
                }
                "Runtime.evaluate" => {
                    let expression = message["params"]["expression"].as_str().unwrap();
                    let value = if expression == "location.href" {
                        json!("app://-/index.html")
                    } else if expression == "window.__cockpitQuotaCompat.prepareLiveQuotaGate()" {
                        json!({"ok":true,"plan":{
                            "url":if options.live_plan_invalid { "https://example.test/assets/app-primary-mock.js" } else { "app://-/assets/app-primary-mock.js" },
                            "lineNumber":12,"columnNumber":34,"hardVariable":"Ct","guardVariable":"vn"
                        }})
                    } else if expression
                        .starts_with("window.__cockpitQuotaCompat.installLiveQuotaGuard(")
                    {
                        json!({"ok":true})
                    } else if let Some(nonce) = argument(expression, "prepareHandoff") {
                        prepared_nonce = Some(nonce);
                        preparations += 1;
                        if preparations <= options.prepare_failures {
                            json!({"ok":false,"reason":options.prepare_reason})
                        } else {
                            json!({"ok":true,"url":FIXTURE_URL,"sourceLength":options.metadata_length})
                        }
                    } else if expression == "window.__cockpitNativeFast.inspectStartupReload()" {
                        inspections += 1;
                        if let Some(reason) = options.inspect_sequence.get(inspections - 1) {
                            json!({"ok":false,"reason":reason})
                        } else if inspections <= options.inspect_failures {
                            json!({"ok":false,"reason":options.inspect_reason,"draft":options.draft_snapshot})
                        } else {
                            json!({"ok":true,"reason":"clean-startup"})
                        }
                    } else if expression
                        .starts_with("window.__cockpitStartupGuard.approveSavedDraft(")
                    {
                        if let Some(reason) = options.draft_approval_rejection {
                            json!({"ok":false,"reason":reason})
                        } else {
                            json!({"ok":true,"reason":"saved-home-draft"})
                        }
                    } else if let Some(nonce) = argument(expression, "reloadPreparedDocument") {
                        assert_eq!(Some(&nonce), prepared_nonce.as_ref());
                        reload_attempts += 1;
                        if options.disconnect_on_reload {
                            let _ = notices_tx.send(json!({"phase":"reload-disconnected"}));
                            socket.close(None).await.unwrap();
                            break;
                        }
                        let rejection = options.reload_rejections.get(reload_attempts - 1);
                        let accepted = rejection.is_none() && options.guard_ok;
                        new_context = accepted && options.reload_new_context;
                        if options.reload_error {
                            error = Some(
                                json!({"code":-32000,"message":"Execution context was destroyed"}),
                            );
                        }
                        json!({"ok":accepted,"reason":rejection.copied().unwrap_or(if accepted { "reload-started" } else { "draft-present" })})
                    } else if expression.contains("state:s.nativePatchState") {
                        if message["params"]["contextId"] == 2 || options.initially_active {
                            if let Some(reason) = options.rejected_new_context {
                                json!({"state":"unsupported","reason":reason})
                            } else if options.active_new_context {
                                if options.capability_only {
                                    json!({"state":"active-capability","reason":"capability-only-quota-provider-not-unique"})
                                } else if options.fast_only {
                                    json!({"state":"active","reason":"fast-only-quota-unsupported","mode":"fast-only","features":"1010","quotaState":"unsupported","quotaReason":"quota-imports-missing"})
                                } else if options.full_quota_only {
                                    json!({"state":"active","reason":"module-evaluated","mode":"full","features":"0101"})
                                } else {
                                    json!({"state":"active"})
                                }
                            } else {
                                json!({"state":"pending"})
                            }
                        } else {
                            json!({"state":"pending"})
                        }
                    } else if expression.contains("typeof p?.prepareHandoff") {
                        browser_probes += 1;
                        if options.browser_preloading_reads > 0 {
                            json!({"installed":false,"preloading":browser_probes <= options.browser_preloading_reads})
                        } else {
                            json!(message["params"]["contextId"] == 2 || options.initially_active)
                        }
                    } else {
                        json!(true)
                    };
                    if let Some((_, rest)) = expression.split_once("s.nativePatchDiagnostic=") {
                        notice = Some(
                            serde_json::from_str::<Value>(rest.split(';').next().unwrap()).unwrap(),
                        );
                    }
                    json!({"result":{"type":"object","value":value}})
                }
                "Page.addScriptToEvaluateOnNewDocument" => {
                    registrations += 1;
                    if options.registration_error {
                        error = Some(json!({"code":-32000,"message":"registration unavailable"}));
                        json!({})
                    } else if options.delay_registration {
                        // Deliver this reply after the next evaluate response.
                        // Cleanup must not assume protocol responses are ordered.
                        delayed_registration = Some(
                            json!({"id":message["id"],"result":{"identifier":format!("registration-{registrations}")}}),
                        );
                        notices_tx
                            .send(json!({"phase":"registration-pending"}))
                            .unwrap();
                        continue;
                    } else {
                        json!({"identifier":format!("registration-{registrations}")})
                    }
                }
                "Debugger.enable" | "Debugger.disable" | "Debugger.removeBreakpoint"
                    if options.live_quota =>
                {
                    if options.debugger_error && method == "Debugger.enable" {
                        error = Some(json!({"code":-32601,"message":"Method not found"}));
                    }
                    json!({})
                }
                "Debugger.setBreakpointByUrl" if options.live_quota => {
                    json!({"breakpointId":"live-quota"})
                }
                _ => {
                    // This simulated modern client deliberately has no
                    // Debugger or live-edit protocol implementation.
                    error = Some(json!({"code":-32601,"message":"Method not found"}));
                    json!({})
                }
            };
            let reply = if let Some(error) = error {
                json!({"id":message["id"],"error":error})
            } else {
                json!({"id":message["id"],"result":result})
            };
            if socket
                .send(Message::Text(reply.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
            if method == "Runtime.evaluate" {
                if let Some(reply) = delayed_registration.take() {
                    if socket
                        .send(Message::Text(reply.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            if let Some(notice) = notice {
                let _ = notices_tx.send(notice);
            }
            if new_context {
                server_navigations.fetch_add(1, Ordering::AcqRel);
                for event in [
                    json!({"method":"Runtime.executionContextsCleared"}),
                    context(2, "main"),
                ] {
                    if socket
                        .send(Message::Text(event.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    let (stop, stop_rx) = watch::channel(false);
    let runtime = tokio::spawn(async move {
        let url = format!("ws://{address}/devtools/page/mock");
        if public_entry {
            run(
                url,
                profile_dir,
                native_fast,
                quota_compat,
                api_key_auth_verified,
                api_service,
                preload_first,
                startup,
                stop_rx,
            )
            .await
        } else {
            run_with_live_quota(
                url,
                profile_dir,
                native_fast,
                quota_compat,
                api_key_auth_verified,
                api_service,
                preload_first,
                startup,
                stop_rx,
                live_quota,
            )
            .await
        }
    });
    Running {
        stop,
        runtime,
        server,
        events,
        notices,
        calls,
        navigations,
    }
}

impl Running {
    async fn notice(&mut self, phase: &str, reason: Option<&str>) -> Value {
        timeout(Duration::from_secs(10), async {
            loop {
                let notice = self.notices.recv().await.expect("runtime diagnostic");
                if notice["phase"] == phase && reason.is_none_or(|value| notice["reason"] == value)
                {
                    return notice;
                }
            }
        })
        .await
        .expect("expected diagnostic in time")
    }

    async fn finish(self, abort: bool) -> Vec<Value> {
        if abort {
            self.runtime.abort();
            assert!(self.runtime.await.unwrap_err().is_cancelled());
        } else {
            let _ = self.stop.send(true);
            timeout(Duration::from_secs(4), self.runtime)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        timeout(Duration::from_secs(4), self.server)
            .await
            .unwrap()
            .unwrap();
        Arc::try_unwrap(self.calls).unwrap().into_inner().unwrap()
    }
}

fn expressions(calls: &[Value], expression: &str) -> usize {
    calls
        .iter()
        .filter(|call| {
            if call["method"] != "Runtime.evaluate" {
                return false;
            }
            let actual = call["params"]["expression"].as_str().unwrap_or_default();
            actual == expression
                || expression
                    .strip_suffix("()")
                    .is_some_and(|name| actual.starts_with(&format!("{name}(")))
        })
        .count()
}

fn methods(calls: &[Value], method: &str) -> usize {
    calls.iter().filter(|call| call["method"] == method).count()
}

fn assert_safe_protocol(calls: &[Value]) {
    assert!(calls
        .iter()
        .all(|call| !call["method"].as_str().unwrap().starts_with("Debugger.")));
    assert!(calls
        .iter()
        .all(|call| !call["method"].as_str().unwrap().starts_with("Fetch.")));
    assert_eq!(methods(calls, "Page.reload"), 0);
    assert_eq!(methods(calls, "Page.setBypassCSP"), 0);
    assert!(calls
        .iter()
        .filter(|call| call["method"] == "Runtime.evaluate")
        .all(|call| call["params"]["contextId"] != 99));
}

#[tokio::test]
async fn api_service_fallback_waits_without_automatic_reload() {
    let mut runtime = start(Options {
        api_service: true,
        native_fast: false,
        api_key_auth_verified: false,
        quota_compat: false,
        public_entry: true,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        0
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[tokio::test]
async fn api_service_public_entry_uses_current_page_on_supported_platforms() {
    let mut runtime = start(Options {
        api_service: true,
        live_quota: true,
        public_entry: true,
        startup: None,
        ..Options::default()
    })
    .await;
    runtime
        .notice("activation", Some("quota-breakpoint-armed"))
        .await;
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Debugger.setBreakpointByUrl"), 1);
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
}

#[tokio::test]
async fn live_quota_activates_current_document_without_reload_and_cleans_up() {
    let mut runtime = start(Options {
        api_service: true,
        live_quota: true,
        ..Options::default()
    })
    .await;
    assert_eq!(
        runtime
            .notice("activation", Some("quota-breakpoint-armed"))
            .await["state"],
        "active"
    );
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Debugger.enable"), 1);
    let breakpoint = calls
        .iter()
        .find(|call| call["method"] == "Debugger.setBreakpointByUrl")
        .unwrap();
    assert_eq!(breakpoint["params"]["lineNumber"], 12);
    assert_eq!(breakpoint["params"]["columnNumber"], 34);
    assert_eq!(
        breakpoint["params"]["condition"],
        live_quota_condition("Ct", "vn")
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitQuotaCompat.installLiveQuotaGuard()"
        ),
        1
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.prepareHandoff()"),
        0
    );
    assert_eq!(methods(&calls, "Page.reload"), 0);
    assert_eq!(methods(&calls, "Debugger.removeBreakpoint"), 1);
    assert_eq!(methods(&calls, "Debugger.disable"), 1);
    assert_eq!(
        methods(&calls, "Page.removeScriptToEvaluateOnNewDocument"),
        1
    );
    assert!(calls.iter().any(|call| call["params"]["expression"]
        .as_str()
        .is_some_and(|value| value.contains("disposeLiveQuotaGuard?.()"))));
    assert!(calls
        .iter()
        .filter(|call| call["method"] == "Runtime.evaluate")
        .all(|call| call["params"]["contextId"] == 1));
}

#[tokio::test]
async fn live_quota_failure_never_falls_back_to_reload_or_reports_active() {
    for invalid_plan in [false, true] {
        let mut runtime = start(Options {
            api_service: true,
            live_quota: true,
            live_plan_invalid: invalid_plan,
            debugger_error: !invalid_plan,
            ..Options::default()
        })
        .await;
        let reason = if invalid_plan {
            "quota-location-invalid"
        } else {
            "debugger-enable-failed"
        };
        assert_ne!(
            runtime.notice("activation", Some(reason)).await["state"],
            "active"
        );
        let calls = runtime.finish(false).await;
        assert_eq!(methods(&calls, "Debugger.setBreakpointByUrl"), 0);
        assert_eq!(
            expressions(
                &calls,
                "window.__cockpitQuotaCompat.installLiveQuotaGuard()"
            ),
            0
        );
        assert_eq!(
            expressions(
                &calls,
                "window.__cockpitNativeFast.reloadPreparedDocument()"
            ),
            0
        );
        assert_eq!(methods(&calls, "Page.reload"), 0);
    }
}

#[tokio::test]
async fn fresh_launch_registers_then_reloads_once_and_requires_module_evaluation() {
    let mut runtime = start(Options::default()).await;
    assert_eq!(
        runtime.notice("document", Some("registered")).await["state"],
        "next-document"
    );
    assert_eq!(
        runtime.notice("activation", Some("module-evaluated")).await["state"],
        "active"
    );
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_eq!(
        methods(&calls, "Page.removeScriptToEvaluateOnNewDocument"),
        1
    );
    let registration = calls
        .iter()
        .position(|call| call["method"] == "Page.addScriptToEvaluateOnNewDocument")
        .unwrap();
    let reload = calls
        .iter()
        .position(|call| {
            call["params"]["expression"]
                .as_str()
                .is_some_and(|value| argument(value, "reloadPreparedDocument").is_some())
        })
        .unwrap();
    assert!(registration < reload);
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn capability_only_activation_reports_partial_state_and_never_claims_full_activation() {
    let mut runtime = start(Options {
        api_service: true,
        capability_only: true,
        ..Options::default()
    })
    .await;
    let notice = runtime
        .notice(
            "activation",
            Some("capability-only-quota-provider-not-unique"),
        )
        .await;
    assert_eq!(notice["state"], "active-capability");
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert_eq!(
        methods(&calls, "Page.removeScriptToEvaluateOnNewDocument"),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn restored_running_page_registers_but_never_reloads() {
    let mut runtime = start(Options {
        startup: None,
        ..Options::default()
    })
    .await;
    let notice = runtime.notice("document", Some("restart-required")).await;
    assert_eq!(notice["state"], "next-document");
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn saved_native_home_draft_activates_once_but_stale_disk_or_editor_proof_never_navigates() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = json!({"placement":"home","document":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"saved draft"}]}]}});
    let state = json!({"electron-persisted-atom-state":{
        "draft-thread-identities-v1":{"new-conversation":"client-new-thread:fixture"},
        "composer-prompt-drafts-v2":{"client-new-thread:fixture":{"prompt":{"document":snapshot["document"],"plainTextMode":false}}}
    }});
    let path = dir.path().join(".codex-global-state.json");
    tokio::fs::write(&path, serde_json::to_vec(&state).unwrap())
        .await
        .unwrap();
    let mut runtime = start(Options {
        profile_dir: dir.path().to_path_buf(),
        draft_snapshot: Some(snapshot.clone()),
        inspect_failures: 1,
        inspect_reason: "draft-present",
        ..Options::default()
    })
    .await;
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitStartupGuard.approveSavedDraft()"),
        1
    );
    assert_safe_protocol(&calls);

    for stale_disk in [false, true] {
        if stale_disk {
            tokio::fs::write(&path, b"{}").await.unwrap();
        }
        let mut runtime = start(Options {
            profile_dir: dir.path().to_path_buf(),
            draft_snapshot: Some(snapshot.clone()),
            inspect_failures: usize::MAX,
            inspect_reason: "draft-present",
            draft_approval_rejection: Some("draft-present"),
            ..Options::default()
        })
        .await;
        runtime.notice("document", Some("draft-present")).await;
        assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
        let calls = runtime.finish(false).await;
        assert_eq!(
            expressions(&calls, "window.__cockpitStartupGuard.approveSavedDraft()"),
            usize::from(!stale_disk)
        );
        assert_eq!(
            expressions(
                &calls,
                "window.__cockpitNativeFast.reloadPreparedDocument()"
            ),
            0
        );
    }
}

#[tokio::test]
async fn dirty_startup_page_is_not_reloaded_and_does_not_claim_active() {
    let mut runtime = start(Options {
        guard_ok: false,
        ..Options::default()
    })
    .await;
    assert_eq!(
        runtime.notice("document", Some("draft-present")).await["state"],
        "next-document"
    );
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert!(calls.iter().all(|call| !call["params"]["expression"]
        .as_str()
        .unwrap_or_default()
        .contains("\"state\":\"active\"")));
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn loading_page_waits_without_spending_startup_permission() {
    let startup = Arc::new(StartupReload::new());
    let mut runtime = start(Options {
        startup: Some(startup.clone()),
        inspect_failures: 1,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("document-loading")).await;
    assert!(!startup.claimed.load(Ordering::Acquire));
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        2
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn repeated_interaction_settles_before_one_real_navigation() {
    let mut runtime = start(Options {
        inspect_failures: 3,
        inspect_reason: "interaction-settling",
        ..Options::default()
    })
    .await;
    runtime
        .notice("document", Some("interaction-settling"))
        .await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        4
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn open_native_ui_waits_until_closed_before_navigation() {
    let mut runtime = start(Options {
        inspect_failures: 2,
        inspect_reason: "ui-busy",
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("ui-busy")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        3
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn atomic_transient_rejections_return_claim_then_retry_one_real_navigation() {
    let startup = Arc::new(StartupReload::new());
    let mut runtime = start(Options {
        startup: Some(startup.clone()),
        reload_rejections: vec!["interaction-settling", "ui-busy"],
        ..Options::default()
    })
    .await;
    runtime
        .notice("document", Some("interaction-settling"))
        .await;
    assert!(!startup.claimed.load(Ordering::Acquire));
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    runtime.notice("document", Some("ui-busy")).await;
    assert!(!startup.claimed.load(Ordering::Acquire));
    runtime.notice("activation", Some("module-evaluated")).await;
    assert!(startup.claimed.load(Ordering::Acquire));
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        3
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn terminal_atomic_rejections_keep_claim_spent_and_never_retry() {
    for reason in [
        "unsupported-document",
        "interaction-history-unavailable",
        "reload-failed",
        "unrecognized-reason",
    ] {
        let startup = Arc::new(StartupReload::new());
        let mut runtime = start(Options {
            startup: Some(startup.clone()),
            reload_rejections: vec![reason],
            ..Options::default()
        })
        .await;
        runtime.notice("document", Some(reason)).await;
        tokio::time::sleep(Duration::from_millis(650)).await;
        assert!(
            startup.claimed.load(Ordering::Acquire),
            "claim must remain spent for {reason}"
        );
        assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
        let calls = runtime.finish(false).await;
        assert_eq!(
            expressions(
                &calls,
                "window.__cockpitNativeFast.reloadPreparedDocument()"
            ),
            1
        );
        assert_safe_protocol(&calls);
    }
}

#[tokio::test]
async fn disconnected_atomic_request_keeps_claim_spent_across_reconnection() {
    let startup = Arc::new(StartupReload::new());
    let mut first = start(Options {
        startup: Some(startup.clone()),
        disconnect_on_reload: true,
        ..Options::default()
    })
    .await;
    first.notice("reload-disconnected", None).await;
    let calls = first.finish(false).await;
    assert!(startup.claimed.load(Ordering::Acquire));
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    let mut second = start(Options {
        startup: Some(startup),
        ..Options::default()
    })
    .await;
    second
        .notice("document", Some("startup-permission-expired"))
        .await;
    assert_eq!(second.navigations.load(Ordering::Acquire), 0);
    let calls = second.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn persistent_draft_waits_until_deadline_without_claim_or_navigation() {
    let startup = Arc::new(StartupReload {
        deadline: StdInstant::now() + Duration::from_millis(1200),
        claimed: AtomicBool::new(false),
    });
    let mut runtime = start(Options {
        startup: Some(startup.clone()),
        inspect_failures: usize::MAX,
        inspect_reason: "draft-present",
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("draft-present")).await;
    assert!(!startup.claimed.load(Ordering::Acquire));
    runtime
        .notice("document", Some("startup-permission-expired"))
        .await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    assert!(!startup.claimed.load(Ordering::Acquire));
    let calls = runtime.finish(false).await;
    assert!(expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()") >= 2);
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn startup_progress_and_temporary_draft_wait_until_blank_editor_then_activate() {
    let sequence = vec![
        "document-loading",
        "composer-unavailable",
        "conversation-busy",
        "draft-present",
        "ui-busy",
        "interaction-settling",
    ];
    let mut runtime = start(Options {
        inspect_sequence: sequence.clone(),
        ..Options::default()
    })
    .await;
    for reason in &sequence {
        runtime.notice("document", Some(reason)).await;
        assert_eq!(
            runtime.navigations.load(Ordering::Acquire),
            0,
            "page must remain intact while {reason}"
        );
    }
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        sequence.len() + 1
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn atomic_busy_and_draft_races_wait_for_clean_page_before_single_navigation() {
    let mut runtime = start(Options {
        reload_rejections: vec!["conversation-busy", "draft-present"],
        ..Options::default()
    })
    .await;
    for reason in ["conversation-busy", "draft-present"] {
        runtime.notice("document", Some(reason)).await;
        assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    }
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        3
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn persistent_atomic_draft_expires_and_expired_ticket_cannot_reload_on_reconnect() {
    let startup = Arc::new(StartupReload {
        deadline: StdInstant::now() + Duration::from_millis(1200),
        claimed: AtomicBool::new(false),
    });
    let mut first = start(Options {
        startup: Some(startup.clone()),
        guard_ok: false,
        ..Options::default()
    })
    .await;
    first.notice("document", Some("draft-present")).await;
    first
        .notice("document", Some("startup-permission-expired"))
        .await;
    assert_eq!(first.navigations.load(Ordering::Acquire), 0);
    let calls = first.finish(false).await;
    assert!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ) >= 2
    );
    let mut second = start(Options {
        startup: Some(startup),
        ..Options::default()
    })
    .await;
    second
        .notice("document", Some("startup-permission-expired"))
        .await;
    assert_eq!(second.navigations.load(Ordering::Acquire), 0);
    let calls = second.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn missing_interaction_history_is_terminal_and_never_claims_or_navigates() {
    let startup = Arc::new(StartupReload::new());
    let mut runtime = start(Options {
        startup: Some(startup.clone()),
        inspect_failures: usize::MAX,
        inspect_reason: "interaction-history-unavailable",
        ..Options::default()
    })
    .await;
    runtime
        .notice("document", Some("interaction-history-unavailable"))
        .await;
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(!startup.claimed.load(Ordering::Acquire));
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        1
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn successful_registration_without_new_module_is_not_activation() {
    let mut runtime = start(Options {
        reload_new_context: false,
        ..Options::default()
    })
    .await;
    assert_eq!(
        runtime.notice("document", Some("reload-requested")).await["state"],
        "next-document"
    );
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn failed_new_document_reports_loader_reason_without_becoming_active_or_timeout() {
    let mut runtime = start(Options {
        rejected_new_context: Some("previous-preload-failed"),
        ..Options::default()
    })
    .await;
    let diagnostic = runtime
        .notice("activation", Some("previous-preload-failed"))
        .await;
    assert_eq!(diagnostic["state"], "next-document");
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    tokio::time::sleep(Duration::from_millis(650)).await;
    while let Ok(notice) = runtime.notices.try_recv() {
        assert_ne!(notice["reason"], "module-load-timeout");
        assert_ne!(notice["state"], "active");
    }
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert!(calls.iter().all(|call| !call["params"]["expression"]
        .as_str()
        .unwrap_or_default()
        .contains("module-load-timeout")));
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn navigation_losing_reload_reply_does_not_repeat_reload() {
    let mut runtime = start(Options {
        reload_error: true,
        ..Options::default()
    })
    .await;
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn registration_failure_does_not_reload_or_report_activation() {
    let mut runtime = start(Options {
        registration_error: true,
        ..Options::default()
    })
    .await;
    assert_eq!(
        runtime
            .notice("document", Some("registration-failed"))
            .await["state"],
        "pending"
    );
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn unsupported_module_stops_preparation_without_touching_page() {
    let mut runtime = start(Options {
        prepare_failures: usize::MAX,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("unsupported-gates")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.prepareHandoff()"),
        1
    );
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 0);
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn initially_missing_assets_are_retried_then_activate() {
    let mut runtime = start(Options {
        prepare_failures: 1,
        prepare_reason: "unsupported-entry",
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("unsupported-entry")).await;
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.prepareHandoff()"),
        2
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn manual_context_rebuild_uses_existing_registration_without_another_reload() {
    let mut runtime = start(Options {
        startup: None,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    runtime
        .events
        .send(json!({"method":"Runtime.executionContextsCleared"}))
        .unwrap();
    runtime.events.send(context(2, "main")).unwrap();
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn startup_permission_is_shared_and_cannot_reload_again_after_reconnect() {
    let startup = Arc::new(StartupReload::new());
    let mut first = start(Options {
        startup: Some(startup.clone()),
        ..Options::default()
    })
    .await;
    first.notice("activation", Some("module-evaluated")).await;
    let calls = first.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        1
    );
    let mut second = start(Options {
        startup: Some(startup),
        ..Options::default()
    })
    .await;
    second
        .notice("document", Some("startup-permission-expired"))
        .await;
    let calls = second.finish(false).await;
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn cancellation_cleans_registration_even_when_add_reply_arrives_out_of_order() {
    let mut runtime = start(Options {
        delay_registration: true,
        ..Options::default()
    })
    .await;
    runtime.notice("registration-pending", None).await;
    let calls = runtime.finish(true).await;
    assert_eq!(
        methods(&calls, "Page.removeScriptToEvaluateOnNewDocument"),
        1
    );
    assert!(calls.iter().any(
        |call| call["method"] == "Page.removeScriptToEvaluateOnNewDocument"
            && call["params"]["identifier"] == "registration-1"
    ));
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn handoff_uses_one_nonce_and_keeps_new_document_script_small() {
    let mut runtime = start(Options::default()).await;
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    let prepared: Vec<_> = calls
        .iter()
        .filter_map(|call| {
            call["params"]["expression"]
                .as_str()
                .and_then(|value| argument(value, "prepareHandoff"))
        })
        .collect();
    assert_eq!(prepared.len(), 1);
    let nonce = &prepared[0];
    assert!(uuid::Uuid::parse_str(nonce).is_ok());
    let script = calls
        .iter()
        .find(|call| call["method"] == "Page.addScriptToEvaluateOnNewDocument")
        .unwrap()["params"]["source"]
        .as_str()
        .unwrap();
    assert!(script.contains(&format!("consumeHandoff({})", json!(nonce))));
    assert!(script.contains("\"apiKeyAuthVerified\":true"));
    assert!(script.contains("\"apiService\":false"));
    assert!(script.len() < 200_000);
    for expression in calls
        .iter()
        .filter_map(|call| call["params"]["expression"].as_str())
    {
        if let Some(actual) = argument(expression, "reloadPreparedDocument") {
            assert_eq!(&actual, nonce);
        }
    }
    assert!(calls.iter().any(|call| call["params"]["expression"]
        .as_str()
        .is_some_and(|value| value.contains(&format!("cancelHandoff?.({})", json!(nonce))))));
    // The new-document runtime already owns the consumed source and navigation
    // listener. Re-evaluating it would destroy that private cache.
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["params"]["expression"].as_str().is_some_and(
                |value| value.contains("const previous = window.__cockpitEnhancements")
            ))
            .count(),
        1
    );
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn api_service_binding_is_carried_into_the_new_document_and_revoked_on_cleanup() {
    let mut runtime = start(Options {
        api_service: true,
        ..Options::default()
    })
    .await;
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    let script = calls
        .iter()
        .find(|call| call["method"] == "Page.addScriptToEvaluateOnNewDocument")
        .unwrap()["params"]["source"]
        .as_str()
        .unwrap();
    assert!(script.contains("\"apiService\":true"));
    // The CDP startup script must retain native dictation eligibility and
    // transcription rather than installing either retired audio adapter.
    assert!(!script.contains("function capability("));
    assert!(!script.contains("function policyOverride("));
    assert!(!script.contains("function installCockpitDictation("));
    assert!(!script.contains("/audio/transcriptions"));
    assert!(!script.contains("__cockpitDictationBridge_"));
    assert!(script.contains("previous.apiDictation = false"));
    assert!(script.contains("capability = nativeValue => nativeValue"));
    assert!(calls
        .iter()
        .filter_map(|call| call["params"]["expression"].as_str())
        .any(|value| value.contains("s.apiService=false;")));
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn restored_page_prepares_handoff_without_staging_or_navigating() {
    let mut runtime = start(Options {
        startup: None,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.prepareHandoff()"),
        1
    );
    assert_eq!(
        expressions(
            &calls,
            "window.__cockpitNativeFast.reloadPreparedDocument()"
        ),
        0
    );
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert_safe_protocol(&calls);
}

#[tokio::test]
async fn invalid_handoff_metadata_cancels_cache_before_any_registration() {
    for length in [None, Some(0), Some(SOURCE_LIMIT as u64 + 1)] {
        let mut runtime = start(Options {
            metadata_length: length,
            ..Options::default()
        })
        .await;
        runtime
            .notice("document", Some("invalid-module-source-length"))
            .await;
        let calls = runtime.finish(false).await;
        assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 0);
        assert_eq!(
            expressions(
                &calls,
                "window.__cockpitNativeFast.reloadPreparedDocument()"
            ),
            0
        );
        assert!(calls
            .iter()
            .any(|call| call["params"]["expression"].as_str().is_some_and(
                |value| value.starts_with("window.__cockpitNativeFast?.cancelHandoff?.(")
            )));
        assert_safe_protocol(&calls);
    }
}

#[test]
fn startup_claim_expires_and_is_single_use() {
    let expired = StartupReload {
        deadline: StdInstant::now() - Duration::from_secs(1),
        claimed: AtomicBool::new(false),
    };
    assert!(!expired.claim());
    let startup = StartupReload::new();
    assert!(startup.claim());
    assert!(!startup.claim());
}

#[test]
fn diagnostic_reasons_never_echo_browser_content() {
    assert_eq!(
        safe_reason(
            &json!({"reason":"secret token https://example.com"}),
            "unsupported"
        ),
        "unsupported"
    );
    assert_eq!(
        safe_reason(
            &json!({"diagnostics":{"reason":"unsupported-gates"}}),
            "unsupported"
        ),
        "unsupported-gates"
    );
}

#[test]
fn transport_and_asset_boundaries_reject_remote_or_ambiguous_targets() {
    assert!(local_websocket("ws://127.0.0.1:9333/devtools/page/abc"));
    assert!(local_websocket("ws://[::1]:9333/devtools/page/abc"));
    for value in [
        "wss://127.0.0.1:9333/devtools/page/abc",
        "ws://example.com:9333/devtools/page/abc",
        "ws://localhost:9333/devtools/page/abc",
        "ws://127.0.0.1:9333/devtools/browser/abc",
        "ws://x:y@127.0.0.1:9333/devtools/page/abc",
    ] {
        assert!(!local_websocket(value));
    }
    assert!(local_document("app://-/index.html"));
    assert!(local_document(
        "file:///Applications/ChatGPT.app/index.html"
    ));
    assert!(!local_document("https://chatgpt.com"));
    assert!(!local_document("app://external/index.html"));
    assert!(local_initial("app://-/assets/app-initial-valid.js"));
    assert!(!local_initial("app://-/assets/app-initial-valid.js?other"));
    assert!(local_primary("app://-/assets/app-primary-98d23d806dbd.js"));
    for value in [
        "https://example.com/assets/app-primary-valid.js",
        "app://-/assets/app-initial-valid.js",
        "app://-/assets/app-primary-valid.js?other",
        "app://-/assets/app-primary-valid.js#fragment",
    ] {
        assert!(!local_primary(value), "{value}");
    }
}

#[tokio::test]
async fn disabled_and_stopped_sessions_do_not_connect() {
    let (_, receiver) = watch::channel(false);
    assert!(run(
        "invalid".to_string(),
        PathBuf::new(),
        false,
        false,
        false,
        false,
        false,
        None,
        receiver
    )
    .await
    .is_ok());
    let (_, receiver) = watch::channel(true);
    assert!(run(
        "invalid".to_string(),
        PathBuf::new(),
        true,
        true,
        true,
        false,
        false,
        None,
        receiver
    )
    .await
    .is_ok());
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn verified_api_fast_reuses_fresh_startup_and_reports_partial_activation() {
    let mut runtime = start(Options {
        api_service: true,
        quota_compat: false,
        public_entry: true,
        fast_only: true,
        ..Options::default()
    })
    .await;
    let notice = runtime
        .notice("activation", Some("fast-only-quota-unsupported"))
        .await;
    assert_eq!(notice["quotaState"], "unsupported");
    assert_eq!(notice["quotaReason"], "quota-imports-missing");
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        1
    );
    assert_eq!(methods(&calls, "Debugger.enable"), 0);
}

#[tokio::test]
async fn verified_api_fast_running_document_never_reloads() {
    let mut runtime = start(Options {
        api_service: true,
        quota_compat: false,
        public_entry: true,
        startup: None,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        0
    );
}

#[tokio::test]
async fn verified_api_fast_preload_late_attach_waits_without_second_reload() {
    let mut runtime = start(Options {
        api_service: true,
        quota_compat: false,
        public_entry: true,
        preload_first: true,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        0
    );
    assert_eq!(methods(&calls, "Page.addScriptToEvaluateOnNewDocument"), 1);
    assert!(calls.iter().any(|call| call["params"]["source"]
        .as_str()
        .is_some_and(|source| source.contains("__cockpitStartupPreloadStarted"))));
}

#[tokio::test]
async fn live_quota_worker_preserves_an_already_active_fast_module() {
    let mut runtime = start(Options {
        api_service: true,
        live_quota: true,
        fast_only: true,
        initially_active: true,
        ..Options::default()
    })
    .await;
    runtime
        .notice("activation", Some("fast-only-quota-unsupported"))
        .await;
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Debugger.enable"), 0);
    assert_eq!(
        expressions(&calls, "window.__cockpitQuotaCompat.prepareLiveQuotaGate()"),
        0
    );
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn windows_verified_api_fresh_startup_enforces_preload_without_caller_flag() {
    let mut runtime = start(Options {
        api_service: true,
        quota_compat: false,
        public_entry: true,
        preload_first: false,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("restart-required")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        0
    );
    assert!(calls.iter().any(|call| call["params"]["source"]
        .as_str()
        .is_some_and(|source| source.contains("__cockpitStartupPreloadStarted"))));
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn verified_quota_route_fresh_launch_installs_full_modules_with_single_guarded_reload() {
    let mut runtime = start(Options {
        api_service: true,
        native_fast: false,
        api_key_auth_verified: false,
        quota_compat: true,
        public_entry: true,
        full_quota_only: true,
        ..Options::default()
    })
    .await;
    runtime.notice("activation", Some("module-evaluated")).await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 1);
    let calls = runtime.finish(false).await;
    assert_eq!(
        expressions(&calls, "window.__cockpitNativeFast.inspectStartupReload()"),
        1
    );
    assert_eq!(methods(&calls, "Debugger.enable"), 0);
}

#[tokio::test]
async fn full_quota_only_module_is_not_replaced_by_a_live_guard_on_reconnect() {
    let mut runtime = start(Options {
        api_service: true,
        native_fast: false,
        api_key_auth_verified: false,
        live_quota: true,
        full_quota_only: true,
        initially_active: true,
        ..Options::default()
    })
    .await;
    runtime.notice("activation", Some("module-evaluated")).await;
    let calls = runtime.finish(false).await;
    assert_eq!(methods(&calls, "Debugger.enable"), 0);
    assert_eq!(
        expressions(&calls, "window.__cockpitQuotaCompat.prepareLiveQuotaGate()"),
        0
    );
}

#[tokio::test]
async fn page_worker_waits_for_browser_completion_before_replacing_its_owner() {
    let mut runtime = start(Options {
        browser_preloading_reads: 2,
        initially_active: true,
        fast_only: true,
        ..Options::default()
    })
    .await;
    runtime.notice("document", Some("browser-preloading")).await;
    runtime
        .notice("activation", Some("fast-only-quota-unsupported"))
        .await;
    assert_eq!(runtime.navigations.load(Ordering::Acquire), 0);
    let calls = runtime.finish(false).await;
    let expressions: Vec<_> = calls
        .iter()
        .filter_map(|call| call["params"]["expression"].as_str())
        .collect();
    let bootstrap = expressions
        .iter()
        .position(|expression| expression.contains("const previous = window.__cockpitEnhancements"))
        .unwrap();
    assert_eq!(
        expressions[..bootstrap]
            .iter()
            .filter(|expression| expression.contains("typeof p?.prepareHandoff"))
            .count(),
        3
    );
}
