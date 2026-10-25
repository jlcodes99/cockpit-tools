use super::*;
use crate::modules::codex_native_api::{contract::*, start, Options};
use axum::http::HeaderMap;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn route(proxy: Option<&str>, name: &str) -> Route {
    let mut client = reqwest::Client::builder().no_proxy();
    if let Some(proxy) = proxy {
        client = client.proxy(reqwest::Proxy::all(proxy).unwrap());
    }
    Route {
        account_id: name.into(),
        continuity_account_id: None,
        owner_id: name.into(),
        installation_profile: None,
        transport_identity: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
        official_fallback_failure: None,
        base_url: "https://chatgpt.com/backend-api/codex".into(),
        model: "model".into(),
        headers: HeaderMap::new(),
        client: client.build().unwrap(),
        proxy_url: proxy.map(str::to_owned),
        proxy_route: Some(json!({"kind":"proxy","name":name})),
        chatgpt: false,
        chatgpt_plan: false,
        wire_api: WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit: None,
    }
}

#[test]
fn connection_messages_use_attempted_proxy_locale_and_conservative_evidence() {
    let selected = route(
        Some("socks5h://secret-user:secret-pass@127.0.0.1:9"),
        "静态美国ip",
    );
    let original = ServiceError::new(504, "upstream_connect_timeout", "raw transport error")
        .with_retry_after(Some(Duration::from_secs(2)));
    let error = explain(&selected, original, Cause::Timeout, "zh-CN");
    assert_eq!(
        (error.status, error.code),
        (504, "upstream_connect_timeout")
    );
    assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
    assert_eq!(
        error.message,
        "通过代理“静态美国ip”连接 ChatGPT 超时，可能与代理或网络有关，请检查代理或更换节点后重试。"
    );
    for locale in ["en-US", "en"] {
        let error = explain(&selected, error.clone(), Cause::Connection, locale);
        assert!(error.message.contains("through proxy “静态美国ip”"));
        assert!(error.message.contains("may be unavailable"));
        assert!(!error.message.contains("secret-user") && !error.message.contains("secret-pass"));
    }
    let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
    assert!(matches!(
        classify(&selected, &refused),
        Cause::LocalProxyUnavailable
    ));
    let remote = route(Some("http://proxy.example.test:80"), "remote");
    assert!(matches!(classify(&remote, &refused), Cause::Connection));
    let forwarded = std::io::Error::other("SOCKS error: connection refused");
    assert!(matches!(classify(&selected, &forwarded), Cause::Connection));
    let direct = route(None, "stale proxy name");
    assert!(matches!(classify(&direct, &refused), Cause::Connection));
    let direct_error = explain(&direct, error, Cause::Connection, "zh-CN");
    assert_eq!(
        direct_error.message,
        "连接 ChatGPT 失败，请检查网络后重试。"
    );
}

#[test]
fn proxy_labels_never_expose_credentials_urls_or_unbounded_names() {
    let proxy = "http://secret-user:secret-pass@127.0.0.1:1234/path?token=secret-query";
    let selected = route(Some("http://127.0.0.1:1234"), proxy);
    let message = explain(
        &selected,
        ServiceError::new(502, "upstream_unavailable", ""),
        Cause::Connection,
        "zh-CN",
    )
    .message;
    for secret in [
        "secret-user",
        "secret-pass",
        "secret-query",
        "/path",
        "http://",
    ] {
        assert!(!message.contains(secret), "{message}");
    }
    assert!(message.contains("127.0.0.1:1234"));
    let selected = route(
        Some("http://127.0.0.1:1234"),
        &format!("{}\npassword=private-password", "界".repeat(500)),
    );
    let name = proxy_name(&selected).unwrap();
    assert!(
        name.chars().count() <= 160 && !name.contains('\n') && !name.contains("private-password")
    );
}

struct ConnectionHost {
    routes: Vec<Route>,
    policy: ExecutionPolicy,
}
impl Host for ConnectionHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "fixture".into(),
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
            self.routes
                .iter()
                .find(|route| !request.excluded_account_ids.contains(&route.account_id))
                .cloned()
                .ok_or_else(|| ServiceError::new(503, "no_available_account", "Fixture exhausted"))
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

async fn call(routes: Vec<Route>, open_timeout: Duration) -> (u16, Value) {
    let host = Arc::new(ConnectionHost {
        routes,
        policy: ExecutionPolicy {
            open_timeout,
            credential_retries: 3,
            ..Default::default()
        },
    });
    let temp = tempfile::tempdir().unwrap();
    let mut handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        temp.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("fixture")
        .json(&json!({"model":"model","input":[],"stream":false}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let value = response.json().await.unwrap();
    handle.shutdown(Duration::from_secs(1)).await.unwrap();
    (status, value)
}

async fn rejecting_proxy(socks: bool) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!(
        "{}://fixture-user:fixture-pass@{}",
        if socks { "socks5h" } else { "http" },
        listener.local_addr().unwrap()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        if socks {
            let mut header = [0; 2];
            stream.read_exact(&mut header).await.unwrap();
            let mut methods = vec![0; header[1] as usize];
            stream.read_exact(&mut methods).await.unwrap();
            stream.write_all(&[5, 2]).await.unwrap();
            stream.read_exact(&mut header).await.unwrap();
            let mut username = vec![0; header[1] as usize];
            stream.read_exact(&mut username).await.unwrap();
            let size = stream.read_u8().await.unwrap();
            let mut password = vec![0; size as usize];
            stream.read_exact(&mut password).await.unwrap();
            stream.write_all(&[1, 1]).await.unwrap();
        } else {
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).await.unwrap() > 0);
            stream
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
        }
    });
    (proxy, task)
}

#[tokio::test]
async fn exhausted_http_and_socks_proxy_auth_failures_keep_actionable_503_messages() {
    for socks in [false, true] {
        let (proxy, task) = rejecting_proxy(socks).await;
        let selected = route(Some(&proxy), "attempted-node");
        let (status, value) = call(vec![selected], Duration::from_secs(2)).await;
        assert_eq!(status, 503, "{value}");
        assert_eq!(value["error"]["code"], "no_available_account");
        let locale = config::get_user_config().language;
        assert_eq!(
            value["error"]["message"],
            i18n::translate(
                &locale,
                "common.networkErrors.proxyAuth",
                &[("proxy", "attempted-node")]
            )
        );
        assert!(
            !value.to_string().contains("fixture-user")
                && !value.to_string().contains("fixture-pass")
        );
        task.await.unwrap();
    }
}

#[tokio::test]
async fn exhausted_failover_names_the_last_attempted_proxy() {
    let (first_proxy, first_task) = rejecting_proxy(false).await;
    let (last_proxy, last_task) = rejecting_proxy(true).await;
    let (status, value) = call(
        vec![
            route(Some(&first_proxy), "first-node"),
            route(Some(&last_proxy), "last-node"),
        ],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(status, 503, "{value}");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("last-node"));
    assert!(!value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("first-node"));
    first_task.await.unwrap();
    last_task.await.unwrap();
}

#[tokio::test]
async fn refused_local_proxy_is_distinct_from_direct_connection_failure() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for proxied in [true, false] {
        let mut selected = route(proxied.then_some(endpoint.as_str()), "closed-local-proxy");
        if !proxied {
            selected.base_url = endpoint.clone();
        }
        let (status, value) = call(vec![selected], Duration::from_secs(2)).await;
        assert_eq!(status, 503, "{value}");
        let locale = config::get_user_config().language;
        let expected = if proxied {
            i18n::translate(
                &locale,
                "common.networkErrors.localProxyUnavailable",
                &[("proxy", "closed-local-proxy")],
            )
        } else {
            let target = i18n::translate(&locale, "common.networkErrors.upstream", &[]);
            i18n::translate(
                &locale,
                "common.networkErrors.directConnection",
                &[("target", &target)],
            )
        };
        assert_eq!(value["error"]["message"], expected);
    }
}

#[tokio::test]
async fn header_timeout_uses_proxy_snapshot_without_claiming_proxy_auth_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let _ = stream.read(&mut bytes).await;
        std::future::pending::<()>().await;
    });
    let (status, value) = call(
        vec![route(Some(&proxy), "slow-node")],
        Duration::from_millis(200),
    )
    .await;
    assert_eq!(status, 504, "{value}");
    assert_eq!(value["error"]["code"], "upstream_connect_timeout");
    let locale = config::get_user_config().language;
    assert_eq!(
        value["error"]["message"],
        i18n::translate(
            &locale,
            "common.networkErrors.proxyTimeout",
            &[("proxy", "slow-node"), ("target", "ChatGPT")]
        )
    );
    task.abort();
}

#[tokio::test]
async fn real_upstream_auth_rejection_survives_later_proxy_connection_failures() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut first = route(None, "real-upstream");
    first.base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route("/responses", axum::routing::post(|| async {
        (axum::http::StatusCode::UNAUTHORIZED, axum::Json(json!({"error":{"code":"invalid_grant","message":"Original account authorization failed"}})))
    }));
    let task = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let (proxy, proxy_task) = rejecting_proxy(false).await;
    let (status, value) = call(
        vec![first, route(Some(&proxy), "unavailable-fallback")],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(status, 401, "{value}");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Original account authorization failed"));
    assert!(!value.to_string().contains("unavailable-fallback"));
    proxy_task.await.unwrap();
    task.abort();
}
