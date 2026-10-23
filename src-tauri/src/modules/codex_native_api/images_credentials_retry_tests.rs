//! Loopback-only credential failures against the public Images HTTP routes.
use super::*;
use crate::modules::codex_native_api::identity::IdentityStore;
use axum::http::StatusCode;

struct CredentialHost {
    base: String,
    oauth: bool,
    alternate: bool,
    refresh_failure: bool,
    change_refresh_owner: bool,
    selections: Mutex<Vec<ImageRouteRequest>>,
    reports: Mutex<Vec<RequestReport>>,
    failures: Mutex<Vec<(String, u16)>>,
    capacity: Arc<tokio::sync::Semaphore>,
}
impl Host for CredentialHost {
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "images-caller".into(),
                label: "fixture".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            if !self.oauth || request.preferred_account_id.as_deref() != Some("a") {
                return Err(ServiceError::new(
                    500,
                    "unexpected_text_route",
                    "Fixture requires pinned OAuth routing",
                ));
            }
            self.image_route(
                caller,
                ImageRouteRequest {
                    model: None,
                    responses_model: Some(request.model),
                    edit: false,
                    target_account_id: request.target_account_id,
                    preferred_account_id: request.preferred_account_id,
                    force_refresh: request.force_refresh,
                    excluded_account_ids: request.excluded_account_ids,
                },
            )
            .await
            .map(|selected| selected.route)
        })
    }
    fn image_route<'a>(
        &'a self,
        _: &'a Caller,
        request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            if request.force_refresh && self.refresh_failure {
                return Err(ServiceError::new(
                    403,
                    "reauthorization_required",
                    "private refresh failure detail",
                ));
            }
            let account = if request.excluded_account_ids.iter().any(|id| id == "a") {
                if !self.alternate || request.excluded_account_ids.iter().any(|id| id == "b") {
                    return Err(ServiceError::new(
                        503,
                        "image_source_unavailable",
                        "Fixture source exhausted",
                    ));
                }
                "b"
            } else {
                "a"
            };
            if request
                .preferred_account_id
                .as_deref()
                .is_some_and(|preferred| preferred != account)
            {
                return Err(ServiceError::new(
                    409,
                    "continuation_owner_mismatch",
                    "Fixture source is pinned",
                ));
            }
            let permit = self.capacity.clone().try_acquire_owned().map_err(|_| {
                ServiceError::new(429, "account_busy", "Fixture image slot is busy")
            })?;
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                HeaderValue::from_str(&format!("Bearer fixture-{account}")).unwrap(),
            );
            Ok(ImageRoute {
                route: Route {
                    continuity_account_id: None,
                    installation_profile: None,
                    official_authorization_revision: None,
                    official_registration_id: None,
                    official_fallback_registration_id: None,
                    official_fallback_authorization_revision: None,
                    authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                    transport_identity: None,
                    account_id: account.into(),
                    owner_id: if request.force_refresh && self.change_refresh_owner {
                        "different-owner".into()
                    } else {
                        format!("owner-{account}")
                    },
                    base_url: self.base.clone(),
                    model: if self.oauth {
                        "text-main"
                    } else {
                        "gpt-image-2.5"
                    }
                    .into(),
                    headers,
                    client: client(),
                    proxy_url: None,
                    proxy_route: None,
                    chatgpt: self.oauth,
                    chatgpt_plan: false,
                    wire_api: WireApi::Responses,
                    compact_endpoint: None,
                    websocket_endpoint: None,
                    default_service_tier: None,
                    permit: Some(Arc::new(permit)),
                },
                image_model: "gpt-image-2.5".into(),
                main_model: self.oauth.then(|| "text-main".into()),
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 2,
                total_timeout: Duration::from_secs(3),
                retry_max_delay: Duration::from_millis(1),
                ..Default::default()
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        _: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures.lock().unwrap().push((account.into(), status));
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Fixture {
    handle: Handle,
    host: Arc<CredentialHost>,
    requests: Arc<Mutex<Vec<String>>>,
    payloads: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
    root: tempfile::TempDir,
    original: String,
}
impl Fixture {
    async fn start(
        oauth: bool,
        status: u16,
        alternate: bool,
        refresh_failure: bool,
        first_success: bool,
        change_refresh_owner: bool,
    ) -> Self {
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let capture = requests.clone();
        let payloads = Arc::new(Mutex::new(Vec::<Value>::new()));
        let payload_capture = payloads.clone();
        let original = json!({"error":{"code":if status == 401 {"invalid_api_key"} else {"insufficient_quota"},"message":"original image failure","extension":"preserved"},"unknown":[1,2]}).to_string();
        let raw = original.clone();
        let app = Router::new().route(if oauth { "/v1/responses" } else { "/v1/images/generations" }, post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone(); let raw = raw.clone();
            let payload_capture = payload_capture.clone();
            async move {
                payload_capture.lock().unwrap().push(body);
                let account = if headers["authorization"] == "Bearer fixture-b" { "b" } else { "a" };
                let index = { let mut requests = capture.lock().unwrap(); let index = requests.len(); requests.push(account.into()); index };
                if account == "a" && !(first_success && index == 0) {
                    return (StatusCode::from_u16(status).unwrap(), [("content-type", "application/json"), ("retry-after", "17")], raw).into_response();
                }
                if oauth {
                    Json(json!({"id":format!("image-{account}-{index}"),"status":"completed","output":[{"type":"image_generation_call","result":"AA=="}],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}})).into_response()
                } else { Json(json!({"data":[{"b64_json":"AA=="}],"usage":{"total_tokens":3}})).into_response() }
            }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(CredentialHost {
            base,
            oauth,
            alternate,
            refresh_failure,
            change_refresh_owner,
            selections: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
            failures: Mutex::new(vec![]),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        let root = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            requests,
            payloads,
            task,
            root,
            original,
        }
    }
    async fn request(&self, body: Value, protected: bool) -> reqwest::Response {
        let request = client()
            .post(format!("http://{}/v1/images/generations", self.handle.addr))
            .bearer_auth("fixture")
            .json(&body);
        let request = if protected {
            request
                .header("session-id", "root")
                .header("thread-id", "root")
                .header("x-codex-turn-state", "known-image-state")
        } else {
            request
        };
        request.send().await.unwrap()
    }
    async fn finish(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.task.abort();
        let _ = self.task.await;
        assert_eq!(self.host.capacity.available_permits(), 1);
    }
}

#[tokio::test]
async fn api_key_image_401_never_refreshes_or_resends_the_same_key() {
    let fixture = Fixture::start(false, 401, true, false, false, false).await;
    let response = fixture
        .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), false)
        .await;
    assert_eq!(response.status(), 401);
    assert_eq!(response.text().await.unwrap(), fixture.original);
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
    assert_eq!(fixture.host.selections.lock().unwrap().len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_image_refresh_rejection_can_select_another_safe_source() {
    let fixture = Fixture::start(true, 401, true, true, false, false).await;
    let response = fixture
        .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), false)
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a", "b"]);
    let selections = fixture.host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 3);
    assert!(selections[1].force_refresh);
    assert_eq!(selections[1].preferred_account_id.as_deref(), Some("a"));
    assert!(!selections[2].force_refresh);
    assert_eq!(selections[2].excluded_account_ids, vec!["a"]);
    assert_eq!(
        *fixture.host.failures.lock().unwrap(),
        vec![("a".into(), 401)]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_image_payment_required_can_select_another_safe_source() {
    let fixture = Fixture::start(true, 402, true, false, false, false).await;
    let response = fixture
        .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), false)
        .await;
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a", "b"]);
    assert!(fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .all(|request| !request.force_refresh));
    assert_eq!(
        *fixture.host.failures.lock().unwrap(),
        vec![("a".into(), 402)]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn exhausted_oauth_image_pool_preserves_original_status_body_and_retry_after() {
    for status in [401, 402] {
        let fixture = Fixture::start(true, status, false, status == 401, false, false).await;
        let response = fixture
            .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), false)
            .await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()["retry-after"], "17");
        assert_eq!(response.text().await.unwrap(), fixture.original);
        assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn image_turn_state_owner_and_committed_batch_never_broaden_to_another_account() {
    let fixture = Fixture::start(true, 401, true, true, false, false).await;
    let store = IdentityStore::open(fixture.root.path().join("identity.sqlite")).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("session-id", HeaderValue::from_static("root"));
    headers.insert("thread-id", HeaderValue::from_static("root"));
    let mut body = json!({"model":"text-main","input":[]});
    let context = store.inspect("images-caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    let mut upstream = HeaderMap::new();
    upstream.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("known-image-state"),
    );
    store.observe_headers(&binding, &upstream).unwrap();
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"seed-image","status":"completed","output":[]}})).unwrap();
    drop(store);
    let response = fixture
        .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), true)
        .await;
    assert_eq!(response.status(), 401);
    assert_eq!(response.text().await.unwrap(), fixture.original);
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
    assert!(fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.excluded_account_ids.is_empty()));
    fixture.finish().await;

    let fixture = Fixture::start(true, 402, true, false, true, false).await;
    let response = fixture
        .request(
            json!({"prompt":"draw","model":"gpt-image-2.5","n":2}),
            false,
        )
        .await;
    assert_eq!(response.status(), 402);
    let _ = response.bytes().await.unwrap();
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a", "a"]);
    assert_eq!(fixture.host.selections.lock().unwrap().len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn image_refresh_owner_change_remains_a_conflict_without_source_fallback() {
    let fixture = Fixture::start(true, 401, true, false, false, true).await;
    let response = fixture
        .request(json!({"prompt":"draw","model":"gpt-image-2.5"}), false)
        .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "refresh_owner_changed"
    );
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
    fixture.finish().await;
}

#[tokio::test]
async fn forced_oauth_images_reject_missing_server_history_and_have_no_manual_mapping_reservation()
{
    let fixture = Fixture::start(true, 402, false, false, false, false).await;
    let path = fixture.root.path().join("identity.sqlite");
    let body = json!({"model":"text-main","previous_response_id":"missing-image-response","input":[{"role":"user","content":"Draw"}],"tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}});
    let url = format!("http://{}/v1/responses", fixture.handle.addr);
    let response = client()
        .post(&url)
        .bearer_auth("fixture")
        .header("session-id", "root")
        .header("thread-id", "root")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "previous_response_not_found"
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
    let mut clean = body;
    clean
        .as_object_mut()
        .unwrap()
        .remove("previous_response_id");
    let response = client()
        .post(&url)
        .bearer_auth("fixture")
        .header("session-id", "root")
        .header("thread-id", "root")
        .json(&clean)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 402);
    assert_eq!(response.text().await.unwrap(), fixture.original);
    assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
    assert!(!path.with_extension("recovery").exists());
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
    drop(connection);
    fixture.finish().await;
}

#[tokio::test]
async fn forced_oauth_images_accept_complete_unknown_history_and_only_success_adopts_input() {
    use sha2::{Digest, Sha256};
    for successful in [true, false] {
        let fixture = Fixture::start(true, 402, false, false, successful, false).await;
        let cipher = crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature(13);
        let original = json!({"model":"text-main","client_metadata":{"session_id":"image-root","thread_id":"image-thread"},"input":[
            {"role":"user","content":"prior request"},
            {"type":"reasoning","id":"rs_original-reasoning-id","encrypted_content":cipher},
            {"type":"compaction","encrypted_content":"external-image-summary-cipher"},
            {"role":"user","content":"Draw a bird"}
        ],"tools":[{"type":"image_generation","model":"gpt-image-2.5"}],"tool_choice":{"type":"image_generation"}});
        let response = client()
            .post(format!("http://{}/v1/responses", fixture.handle.addr))
            .bearer_auth("fixture")
            .json(&original)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if successful { 200 } else { 402 }
        );
        let received = response.text().await.unwrap();
        if successful {
            assert!(received.contains("image_generation_call"), "{received}");
        } else {
            assert_eq!(received, fixture.original);
        }
        assert_eq!(*fixture.requests.lock().unwrap(), vec!["a"]);
        let payloads = fixture.payloads.lock().unwrap();
        assert_eq!(payloads[0]["input"], original["input"]);
        drop(payloads);
        let path = fixture.root.path().join("identity.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        for cipher in [
            cipher.as_str(),
            "external-image-summary-cipher",
        ] {
            let digest = format!("{:x}", Sha256::digest(cipher.as_bytes()));
            let rows:i64=connection.query_row("SELECT COUNT(*) FROM native_opaque_v1 WHERE caller='images-caller' AND kind='encrypted' AND digest=?1",[digest],|row|row.get(0)).unwrap();
            assert_eq!(rows, i64::from(successful), "{cipher}");
        }
        for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
            let rows: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        drop(connection);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn forced_api_key_images_reject_opaque_history_without_dropping_it_or_holding_a_permit() {
    let fixture = Fixture::start(false, 402, false, false, true, false).await;
    for history in [
        json!({"type":"reasoning","encrypted_content":"cannot-flatten-this-history"}),
        json!({"type":"function_call","name":"lookup","call_id":"tool-call","arguments":"{}","encrypted_function_args":["private-tool-cipher"]}),
        json!({"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"agent-call","arguments":"{\"message\":\"private-agent-cipher\"}"}),
    ] {
        let original = json!({"model":"text-main","input":[history,{"role":"user","content":"Draw a bird"}],"tools":[{"type":"image_generation","model":"gpt-image-2.5"}],"tool_choice":{"type":"image_generation"}});
        let response = client()
            .post(format!("http://{}/v1/responses", fixture.handle.addr))
            .bearer_auth("fixture")
            .json(&original)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "continuation_not_supported"
        );
        assert!(fixture.requests.lock().unwrap().is_empty());
        assert!(fixture.payloads.lock().unwrap().is_empty());
    }
    fixture.finish().await;
}
