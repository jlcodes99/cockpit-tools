//! Synthetic loopback regressions for browser observations injected by the client.
//! No real account, browser content or production identity store is used.
use super::codex_app_failover_tests::assert_oauth_rebuild;
use super::*;
use crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature;

fn browser_results() -> Vec<Value> {
    [
        "codex_apps_open_page",
        "codex_apps_selected_page_part_1_of_1",
    ]
    .into_iter()
    .map(|name| {
        json!({"type":"function_call_output","namespace":"agent","name":name,
            "output":"Synthetic browser observation\n  exact whitespace  "})
    })
    .collect()
}

fn root_output() -> Vec<Value> {
    vec![
        message("assistant", "Synthetic first answer"),
        json!({"type":"reasoning","id":"rs_agent-browser-root","summary":[],
            "encrypted_content":synthetic_gpt_signature(73),"status":"completed"}),
    ]
}

fn complete_history() -> Vec<Value> {
    let mut input = vec![message("user", "Synthetic initial question")];
    input.extend(root_output());
    input.push(
        json!({"type":"function_call","id":"fc_browser-real","namespace":"functions",
        "name":"read_file","call_id":"browser-real-call","arguments":"{}","status":"completed"}),
    );
    input.extend(browser_results());
    input.push(
        json!({"type":"function_call_output","call_id":"browser-real-call",
        "output":"Synthetic ordinary tool result"}),
    );
    input.push(message(
        "user",
        "Continue with the complete synthetic history",
    ));
    input
}

async fn establish_root(fixture: &Fixture) {
    let request = body(
        vec![message("user", "Synthetic initial question")],
        None,
        false,
    );
    let response = fixture.post(&request, None).await;
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("browser-a-root"));
    fixture.assert_owner("browser-a-root", "a");
}

fn assert_no_synthetic_host_ids(input: &Value) {
    for item in input
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["namespace"] == "agent")
    {
        assert!(
            item.get("call_id").is_none(),
            "Host observations must not borrow or invent a call ID"
        );
    }
}

#[tokio::test]
async fn agent_browser_host_history_switches_a_to_b_after_http_and_sse_quota_exhaustion() {
    for sse_failure in [false, true] {
        let failure = if sse_failure {
            Step::RawEvents(vec![json!({"type":"error","status_code":429,
                "error":{"type":"rate_limit_error","code":"usage_limit_reached",
                    "message":"Synthetic account A quota exhausted"}})])
        } else {
            Step::HttpRateLimit
        };
        let fixture = Fixture::new(
            true,
            vec![
                Step::CompleteItems("browser-a-root", root_output()),
                failure,
            ],
            vec![Step::Complete("browser-b-rebuilt", None)],
        )
        .await;
        fixture.host.oauth.store(true, Ordering::SeqCst);
        establish_root(&fixture).await;
        let input = complete_history();
        let request = body(input.clone(), None, sse_failure);
        let unchanged = request.clone();
        let response = fixture.post(&request, None).await;
        assert_eq!(response.status(), 200, "sse_failure={sse_failure}");
        assert!(response.text().await.unwrap().contains("browser-b-rebuilt"));
        let sent = fixture.sent();
        assert_eq!(
            sent.iter()
                .map(|sent| sent.account.as_str())
                .collect::<Vec<_>>(),
            ["a", "a", "b"]
        );
        assert!(fixture
            .host
            .failures
            .lock()
            .unwrap()
            .contains(&("a".into(), 429)));
        assert_eq!(sent[1].body["input"], json!(input.clone()));
        assert_oauth_rebuild(&sent[1], &sent[2], json!(input));
        assert_no_synthetic_host_ids(&sent[1].body["input"]);
        assert_no_synthetic_host_ids(&sent[2].body["input"]);
        assert_eq!(request, unchanged);
        assert_eq!(fixture.active().unwrap().0, "b");
        fixture.assert_owner("browser-a-root", "a");
        fixture.assert_owner("browser-b-rebuilt", "b");
        fixture.finish().await;
    }
}

struct CoolingHost {
    inner: Arc<TestHost>,
    cooling: AtomicBool,
}

impl Host for CoolingHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(bearer)
    }

    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        mut request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            if self.cooling.load(Ordering::SeqCst) {
                if request.preferred_account_id.as_deref() == Some("a") {
                    self.inner.selections.lock().unwrap().push(request);
                    return Err(ServiceError::new(
                        503,
                        "account_cooling",
                        "Synthetic owner A quota cooldown",
                    )
                    .with_retry_after(Some(Duration::from_secs(12567))));
                }
                if !request.excluded_account_ids.iter().any(|id| id == "a") {
                    request.excluded_account_ids.push("a".into());
                }
            }
            self.inner.route(caller, request).await
        })
    }

    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }

    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
}

#[tokio::test]
async fn agent_browser_host_history_keeps_a_rebuild_plan_and_bypasses_a_cooling_owner() {
    let mut fixture = Fixture::new(
        true,
        vec![Step::CompleteItems("browser-a-root", root_output())],
        vec![Step::Complete("browser-b-from-cooling", None)],
    )
    .await;
    fixture.host.oauth.store(true, Ordering::SeqCst);
    fixture
        .handle
        .shutdown(Duration::from_secs(3))
        .await
        .unwrap();
    let cooling_host = Arc::new(CoolingHost {
        inner: fixture.host.clone(),
        cooling: AtomicBool::new(false),
    });
    fixture.handle = start(
        "127.0.0.1:0".parse().unwrap(),
        cooling_host.clone(),
        fixture.path(),
        Options::default(),
    )
    .await
    .unwrap();
    establish_root(&fixture).await;

    let input = complete_history();
    let request = body(input.clone(), Some("browser-a-root"), false);
    let unchanged = request.clone();
    let store = IdentityStore::open(fixture.path()).unwrap();
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .or_else(|| {
            store
                .preview_rebuild_with_checkpoint_parameters(&context, &HeaderMap::new(), &request)
                .unwrap()
        })
        .expect("Complete browser host history must have a safe rebuild plan");
    assert_eq!(plan.body["input"], json!(input.clone()));
    assert!(plan.body.get("previous_response_id").is_none());
    drop(store);

    cooling_host.cooling.store(true, Ordering::SeqCst);
    let response = fixture.post(&request, None).await;
    assert_eq!(
        response.status(),
        200,
        "A's local 503 cooldown must permit a verified rebuild on B"
    );
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("browser-b-from-cooling"));
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|sent| sent.account.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_oauth_rebuild(&sent[0], &sent[1], json!(input));
    assert_no_synthetic_host_ids(&sent[1].body["input"]);
    let selections = fixture.host.selections.lock().unwrap();
    assert!(selections
        .iter()
        .any(|request| request.preferred_account_id.as_deref() == Some("a")));
    assert!(selections
        .iter()
        .any(|request| request.preferred_account_id.is_none()
            && request.excluded_account_ids.iter().any(|id| id == "a")));
    drop(selections);
    assert_eq!(request, unchanged);
    fixture.assert_owner("browser-a-root", "a");
    fixture.assert_owner("browser-b-from-cooling", "b");
    fixture.finish().await;
}

#[tokio::test]
async fn agent_browser_checkpoint_with_pending_custom_call_rebuilds_result_only_delta_on_b() {
    let pending_call = json!({"type":"custom_tool_call","id":"ctc_browser-pending",
        "name":"synthetic_custom_tool","call_id":"browser-pending-call",
        "input":"Synthetic pending operation","status":"completed"});
    let mut fixture = Fixture::new(
        true,
        vec![Step::CompleteItems(
            "browser-a-pending",
            vec![pending_call.clone()],
        )],
        vec![Step::Complete("browser-b-after-delta", None)],
    )
    .await;
    fixture.host.oauth.store(true, Ordering::SeqCst);
    fixture
        .handle
        .shutdown(Duration::from_secs(3))
        .await
        .unwrap();
    let cooling_host = Arc::new(CoolingHost {
        inner: fixture.host.clone(),
        cooling: AtomicBool::new(false),
    });
    fixture.handle = start(
        "127.0.0.1:0".parse().unwrap(),
        cooling_host.clone(),
        fixture.path(),
        Options::default(),
    )
    .await
    .unwrap();

    let mut initial_input = vec![
        message("user", "Synthetic browser-assisted task"),
        json!({"type":"reasoning","id":"rs_browser-earlier","summary":[],
            "encrypted_content":synthetic_gpt_signature(74),"status":"completed"}),
        json!({"type":"function_call","id":"fc_browser-earlier","namespace":"functions",
            "name":"read_file","call_id":"browser-earlier-call","arguments":"{}","status":"completed"}),
    ];
    initial_input.extend(browser_results());
    initial_input.push(
        json!({"type":"function_call_output","call_id":"browser-earlier-call",
        "output":"Synthetic earlier tool result\n  exact spacing  "}),
    );
    let first = body(initial_input.clone(), None, false);
    let response = fixture.post(&first, None).await;
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("browser-a-pending"));
    let mut checkpoint_input = initial_input;
    checkpoint_input.push(pending_call);
    let raw: String = Connection::open(fixture.path()).unwrap().query_row(
        "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='browser-a-pending'",
        [], |row| row.get(0)).expect("A completed output with a pending custom call must retain the full checkpoint");
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap(),
        json!(checkpoint_input.clone())
    );

    // The next client request contains only the pending tool's matching result.
    // All prior browser observations and opaque history must come from the checkpoint.
    let result = json!({"type":"custom_tool_call_output","id":"ctco_browser-pending-result",
        "call_id":"browser-pending-call","output":"Synthetic completed custom result\n  exact spacing  "});
    let request = body(vec![result.clone()], Some("browser-a-pending"), false);
    let unchanged = request.clone();
    let mut expected = checkpoint_input;
    expected.push(result);
    let store = IdentityStore::open(fixture.path()).unwrap();
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    let plan = store.configure_rebuild(&mut context, &HeaderMap::new(), &request, true).unwrap()
        .or_else(|| store.preview_rebuild_with_checkpoint_parameters(&context, &HeaderMap::new(), &request).unwrap())
        .expect("The custom result delta must merge with browser host checkpoint history before owner selection");
    assert_eq!(plan.body["input"], json!(expected.clone()));
    assert!(plan.body.get("previous_response_id").is_none());
    drop(store);

    cooling_host.cooling.store(true, Ordering::SeqCst);
    let response = fixture.post(&request, None).await;
    assert_eq!(
        response.status(),
        200,
        "A's cooldown must not strand a complete cached custom-tool continuation"
    );
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("browser-b-after-delta"));
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|sent| sent.account.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(sent[0].body["input"], first["input"]);
    assert_oauth_rebuild(&sent[0], &sent[1], json!(expected.clone()));
    assert_no_synthetic_host_ids(&sent[1].body["input"]);
    assert_eq!(request, unchanged);
    assert_eq!(fixture.active().unwrap().0, "b");
    fixture.assert_owner("browser-a-pending", "a");
    fixture.assert_owner("browser-b-after-delta", "b");
    let raw: String = Connection::open(fixture.path()).unwrap().query_row(
        "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='browser-b-after-delta'",
        [], |row| row.get(0)).unwrap();
    let replayed: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert!(
        replayed.starts_with(&expected),
        "Successful B continuation must preserve every original checkpoint item and the result"
    );
    fixture.finish().await;
}
