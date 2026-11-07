use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::super::{contract::*, start, Options};
#[path = "request_fault_tests.rs"]
mod request_fault_tests;

#[tokio::test]
async fn parity22_runtime_compact_cooldown_matches_default_go_conductor() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/parity22_runtime_compact_go.json")).unwrap();
    for case in reference.as_array().unwrap() {
        let status = case["status"].as_u64().unwrap() as u16;
        let calls = Arc::new(AtomicUsize::new(0));
        let capture = calls.clone();
        let app = Router::new()
            .route(
                "/{account}/responses/compact",
                post(move || {
                    let capture = capture.clone();
                    async move {
                        capture.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::from_u16(status).unwrap(),
                            Json(json!({"error":{"message":"fixture compact failure"}})),
                        )
                    }
                }),
            )
            .route(
                "/{account}/responses",
                post(|| async {
                    Json(json!({"id":"ordinary-success","status":"completed","output":[]}))
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let host = Arc::new(CooldownHost {
            bases: [format!("http://{addr}/a"), format!("http://{addr}/b")],
            failures: Mutex::new(vec![]),
            selections: Mutex::new(vec![]),
            round_wait: Some(Duration::ZERO),
            credential_retries: 2,
            oauth: false,
            fail_refresh: false,
        });
        let root = tempfile::tempdir().unwrap();
        let mut service = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://{}/v1/responses/compact", service.addr))
            .bearer_auth("fixture-only")
            .json(&json!({"model":"model","input":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        response.bytes().await.unwrap();
        assert_eq!(
            !host.failures.lock().unwrap().is_empty(),
            case["cooled"].as_bool().unwrap(),
            "status {status}"
        );
        // Authentication/quota rounds remain independently enabled in Rust.
        if !case["cooled"].as_bool().unwrap() {
            assert_eq!(
                calls.load(Ordering::SeqCst),
                case["calls"].as_u64().unwrap() as usize
            );
            let response = client
                .post(format!("http://{}/v1/responses", service.addr))
                .bearer_auth("fixture-only")
                .json(&json!({"model":"model","input":[]}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            response.bytes().await.unwrap();
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

struct CooldownHost {
    bases: [String; 2],
    failures: Mutex<Vec<(String, u16)>>,
    selections: Mutex<Vec<RouteRequest>>,
    round_wait: Option<Duration>,
    credential_retries: u16,
    oauth: bool,
    fail_refresh: bool,
}

impl Host for CooldownHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: "caller".into(),
                label: "fixture".into(),
            })
        })
    }

    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            if request.force_refresh && self.fail_refresh {
                return Err(ServiceError::new(
                    401,
                    "reauthorization_required",
                    "Fixture refresh rejected",
                ));
            }
            let second = request.excluded_account_ids.iter().any(|id| id == "a");
            let account = if second { "b" } else { "a" };
            Ok(Route {
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
                owner_id: format!("owner-{account}"),
                base_url: self.bases[usize::from(second)].clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }

    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: self.credential_retries,
                total_timeout: Duration::from_secs(3),
                ..Default::default()
            })
        })
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

    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        _: &'a Caller,
        _: RouteRequest,
        _: Duration,
    ) -> BoxFuture<'a, Result<Option<Duration>, ServiceError>> {
        Box::pin(async move { Ok(self.round_wait) })
    }
}

#[tokio::test]
async fn oauth_refresh_failure_preserves_original_status_and_safely_rotates() {
    let app = Router::new()
        .route("/a/responses", post(|| async {
            (StatusCode::UNAUTHORIZED, Json(json!({"error":{"code":"invalid_api_key","message":"expired fixture credential"}})))
        }))
        .route("/b/responses", post(|| async {
            Json(json!({"id":"resp-b","status":"completed","output":[]}))
        }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(CooldownHost {
        bases: [format!("http://{addr}/a"), format!("http://{addr}/b")],
        failures: Mutex::new(vec![]),
        selections: Mutex::new(vec![]),
        round_wait: None,
        credential_retries: 2,
        oauth: true,
        fail_refresh: true,
    });
    let root = tempfile::tempdir().unwrap();
    let mut service = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"model","input":[],"stream":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _: Value = response.json().await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let selections = host.selections.lock().unwrap();
    assert_eq!(selections.len(), 3);
    assert!(selections[1].force_refresh);
    assert_eq!(selections[2].excluded_account_ids, vec!["a"]);
    assert_eq!(*host.failures.lock().unwrap(), vec![("a".into(), 401)]);
    task.abort();
}

#[tokio::test]
async fn http_transport_failover_excludes_only_this_request_but_real_status_marks_account() {
    for transport_failure in [true, false] {
        let app = Router::new()
            .route(
                "/a/responses",
                post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable") }),
            )
            .route(
                "/b/responses",
                post(|| async { Json(json!({"id":"resp-b","status":"completed","output":[]})) }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let first = if transport_failure {
            let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/a", unused.local_addr().unwrap());
            drop(unused);
            url
        } else {
            format!("http://{addr}/a")
        };
        let host = Arc::new(CooldownHost {
            bases: [first, format!("http://{addr}/b")],
            failures: Mutex::new(Vec::new()),
            selections: Mutex::new(Vec::new()),
            round_wait: None,
            credential_retries: 2,
            oauth: false,
            fail_refresh: false,
        });
        let root =
            std::env::temp_dir().join(format!("native-cooldown-retry-{}", uuid::Uuid::now_v7()));
        let mut handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("fixture")
            .json(&json!({"model":"model","input":[],"stream":false}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _: Value = response.json().await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        let failures = host.failures.lock().unwrap().clone();
        assert_eq!(
            failures,
            if transport_failure {
                vec![]
            } else {
                vec![("a".into(), 503)]
            }
        );
        let selections = host.selections.lock().unwrap();
        assert_eq!(selections.len(), 2);
        assert_eq!(selections[1].excluded_account_ids, vec!["a"]);
        drop(selections);
        task.abort();
        let _ = task.await;
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn http_retry_wait_retries_one_round_with_same_opaque_owner_and_respects_maximum() {
    for wait_allowed in [true, false] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let app = Router::new().route(
            "/v1/responses",
            post({
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        use axum::response::IntoResponse;
                        let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if call == 1 {
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(
                                    json!({"error":{"code":"server_error","message":"temporary"}}),
                                ),
                            )
                                .into_response();
                        }
                        Json(json!({"id":"resp-a","status":"completed","output":[]}))
                            .into_response()
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base = format!("http://{addr}/v1");
        let host = Arc::new(CooldownHost {
            bases: [base.clone(), base],
            failures: Mutex::new(vec![]),
            selections: Mutex::new(vec![]),
            round_wait: Some(if wait_allowed {
                Duration::from_millis(40)
            } else {
                Duration::from_secs(5)
            }),
            credential_retries: 0,
            oauth: false,
            fail_refresh: false,
        });
        let temp = tempfile::tempdir().unwrap();
        let mut handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            temp.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{}/v1/responses", handle.addr);
        let first = client
            .post(&url)
            .bearer_auth("fixture")
            .header("thread-id", "original-thread")
            .json(&json!({"model":"model","input":[],"stream":false}))
            .send()
            .await
            .unwrap();
        assert_eq!(first.status(), 200);
        let _: Value = first.json().await.unwrap();
        // Without the complete checkpoint, this server-side continuation must
        // still wait for A rather than rebuilding on a different account.
        let evicted = rusqlite::Connection::open(temp.path().join("identity.sqlite"))
            .unwrap()
            .execute("DELETE FROM native_rebuild_checkpoints_v1", [])
            .unwrap();
        assert_eq!(evicted, 1);
        rusqlite::Connection::open(temp.path().join("identity.sqlite"))
            .unwrap()
            .execute("DELETE FROM native_rebuild_history_v1", [])
            .unwrap();
        let started = tokio::time::Instant::now();
        let response = client
            .post(&url)
            .bearer_auth("fixture")
            .header("thread-id", "original-thread")
            .json(
                &json!({"model":"model","input":[],"stream":false,"previous_response_id":"resp-a"}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), if wait_allowed { 200 } else { 503 });
        let _: Value = response.json().await.unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            if wait_allowed { 3 } else { 2 }
        );
        if wait_allowed {
            assert!(started.elapsed() >= Duration::from_millis(30));
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        let selections = host.selections.lock().unwrap();
        assert!(selections
            .iter()
            .skip(1)
            .all(|request| request.preferred_account_id.as_deref() == Some("a")));
        assert!(selections
            .iter()
            .all(|request| request.excluded_account_ids.is_empty()));
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test]
async fn model_availability_errors_rotate_but_client_faults_do_not() {
    for (status, error, rotates) in [
        (
            400,
            json!({"code":"model_not_supported","message":"not enabled"}),
            true,
        ),
        (
            404,
            json!({"code":"model_not_found","message":"missing"}),
            true,
        ),
        (422, json!({"message":"not available for your plan"}), true),
        (
            401,
            json!({"code":"invalid_api_key","type":"authentication_error"}),
            true,
        ),
        (
            402,
            json!({"code":"insufficient_balance","type":"invalid_request_error"}),
            true,
        ),
        (
            400,
            json!({"code":"invalid_value","message":"bad field"}),
            false,
        ),
        (
            400,
            json!({"code":"invalid_encrypted_content","message":"unsupported model"}),
            false,
        ),
    ] {
        use axum::response::IntoResponse;
        let error_response = (
            StatusCode::from_u16(status).unwrap(),
            Json(json!({"error":error})),
        );
        let app = Router::new()
            .route(
                "/a/responses",
                post(move || {
                    let body = error_response.clone();
                    async move { body.into_response() }
                }),
            )
            .route(
                "/b/responses",
                post(|| async { Json(json!({"id":"resp-b","status":"completed","output":[]})) }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(CooldownHost {
            bases: [format!("http://{addr}/a"), format!("http://{addr}/b")],
            failures: Mutex::new(vec![]),
            selections: Mutex::new(vec![]),
            round_wait: None,
            credential_retries: 0,
            oauth: false,
            fail_refresh: false,
        });
        let root = tempfile::tempdir().unwrap();
        let mut service = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("fixture")
            .json(&json!({"model":"model","input":[],"stream":false}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if rotates { 200 } else { status }
        );
        let _: Value = response.json().await.unwrap();
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        assert_eq!(
            host.selections.lock().unwrap().len(),
            if rotates { 2 } else { 1 }
        );
        assert_eq!(
            *host.failures.lock().unwrap(),
            if rotates {
                vec![(
                    "a".into(),
                    if matches!(status, 401 | 402) {
                        status
                    } else {
                        404
                    },
                )]
            } else {
                vec![]
            }
        );
        task.abort();
    }
}
