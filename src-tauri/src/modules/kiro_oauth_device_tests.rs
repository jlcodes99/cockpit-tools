use super::*;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn enterprise_login() -> PendingOAuthState {
    PendingOAuthState {
        login_id: "enterprise-session".into(),
        expires_at: now_timestamp() + 25,
        verification_uri: "https://example.test/verify".into(),
        verification_uri_complete: "https://example.test/verify".into(),
        callback_url: String::new(),
        callback_port: 0,
        state_token: String::new(),
        code_verifier: String::new(),
        callback_result: None,
        flow: "idc".into(),
        idc: Some(PendingKiroIdcLogin {
            provider: "Enterprise".into(),
            region: "us-east-1".into(),
            start_url: "https://example.test/start".into(),
            client_id: "fixture-client".into(),
            client_secret: "fixture-secret".into(),
            client_id_hash: "fixture-hash".into(),
            client_registration: json!({}),
            device_code: "fixture-device-code".into(),
            interval_seconds: 1,
        }),
    }
}

#[tokio::test]
async fn kiro_enterprise_device_polling_waits_for_browser_and_ignores_social_cleanup() {
    let mut pending = Some(enterprise_login());
    assert!(take_matching_kiro_login(&mut pending, Some("old-social-session")).is_err());
    let state = pending.as_ref().unwrap();
    assert!(state.callback_url.is_empty());
    assert_eq!(state.callback_port, 0);

    let polls = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/token",
        post(|State(polls): State<Arc<AtomicUsize>>, Json(body): Json<Value>| async move {
            assert_eq!(body["clientId"], "fixture-client");
            assert_eq!(body["clientSecret"], "fixture-secret");
            assert_eq!(body["deviceCode"], "fixture-device-code");
            assert_eq!(body["grantType"], "urn:ietf:params:oauth:grant-type:device_code");
            if polls.fetch_add(1, Ordering::Relaxed) == 0 {
                (StatusCode::BAD_REQUEST, Json(json!({"error":"authorization_pending"})))
            } else {
                (StatusCode::OK, Json(json!({
                    "accessToken":"fixture-access","refreshToken":"fixture-refresh","expiresIn":3600
                })))
            }
        }),
    ).with_state(polls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let token = tokio::time::timeout(
        tokio::time::Duration::from_secs(8),
        poll_idc_device_token(
            &client,
            &endpoint,
            &state.login_id,
            state.expires_at,
            state.idc.as_ref().unwrap(),
            || true,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(polls.load(Ordering::Relaxed), 2);
    assert_eq!(token.refresh_token, "fixture-refresh");
    assert!(take_matching_kiro_login(&mut pending, Some("enterprise-session")).unwrap());
    server.abort();
}

#[tokio::test]
async fn kiro_enterprise_discards_a_success_response_after_the_session_is_replaced() {
    let state = enterprise_login();
    let active = Arc::new(AtomicBool::new(true));
    let app = Router::new()
        .route(
            "/token",
            post(|State(active): State<Arc<AtomicBool>>| async move {
                active.store(false, Ordering::Relaxed);
                Json(json!({"accessToken":"late","refreshToken":"late","expiresIn":3600}))
            }),
        )
        .with_state(active.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(8),
        poll_idc_device_token(
            &client,
            &endpoint,
            &state.login_id,
            state.expires_at,
            state.idc.as_ref().unwrap(),
            || active.load(Ordering::Relaxed),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        result.err().as_deref(),
        Some("IAM Identity Center 登录已取消")
    );
    server.abort();
}
