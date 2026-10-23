use super::*;
use axum::{http::HeaderMap, routing::get, Router};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn grok_protocol_version_rejects_obsolete_malformed_and_injected_versions() {
    for version in [
        "0.2.93",
        "1.0.12",
        "1.0",
        "1.0.55-beta",
        "1.0.55\r\nx: y",
        "999999999999999999999999.0.0",
    ] {
        assert!(!acceptable(version), "{version}");
        assert_eq!(resolve(Some(version)), cached());
    }
    for version in ["1.0.13", "1.0.52", "1.1.0", "2.0.0"] {
        assert!(acceptable(version));
        assert_eq!(resolve(Some(version)), version);
    }
}

#[test]
fn grok_protocol_version_refresh_is_single_flight_and_retains_last_good_version() {
    let cache = Cache::default();
    let now = Instant::now();
    assert_eq!(cache.current(), FALLBACK);
    assert!(cache.begin(now));
    assert!(!cache.begin(now + REFRESH_INTERVAL));
    cache.finish(Some("1.0.55".into()));
    assert_eq!(cache.current(), "1.0.55");
    assert!(!cache.begin(now + Duration::from_secs(1)));
    assert!(cache.begin(now + REFRESH_INTERVAL));
    cache.finish(Some("0.2.93".into()));
    assert_eq!(cache.current(), "1.0.55");
    cache.finish(None);
    assert_eq!(cache.current(), "1.0.55");
}

#[tokio::test]
async fn grok_protocol_version_fetch_is_bounded_and_sends_no_account_headers() {
    let router = Router::new()
        .route(
            "/ok",
            get(|headers: HeaderMap| async move {
                for header in [
                    "authorization",
                    "cookie",
                    "x-xai-token-auth",
                    "x-grok-client-version",
                    "session_id",
                    "thread_id",
                ] {
                    assert!(!headers.contains_key(header));
                }
                "{\"version\":\"1.0.55\"}"
            }),
        )
        .route("/old", get(|| async { "{\"version\":\"0.2.93\"}" }))
        .route("/invalid", get(|| async { "not-json" }))
        .route("/large", get(|| async { "x".repeat(MAX_BODY + 1) }))
        .route(
            "/error",
            get(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    assert_eq!(
        fetch(&client, &format!("{base}/ok")).await.unwrap(),
        "1.0.55"
    );
    for path in ["old", "invalid", "large", "error"] {
        assert!(fetch(&client, &format!("{base}/{path}")).await.is_err());
    }
    task.abort();
}

#[tokio::test]
async fn grok_protocol_version_refresh_returns_immediately_and_deduplicates_requests() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let router = Router::new().route(
        "/",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                "{\"version\":\"1.0.55\"}"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let cache = Arc::new(Cache::default());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for _ in 0..10 {
        start_refresh(cache.clone(), client.clone(), url.clone());
    }
    assert_eq!(cache.current(), FALLBACK);
    tokio::time::timeout(Duration::from_secs(3), async {
        while cache.current() != "1.0.55" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    task.abort();
}
