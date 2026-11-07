//! Catalog reads must remain bounded even when the account reader is stalled.
use super::{contract::*, start, Handle, Options};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct StalledCatalog {
    entered: tokio::sync::Notify,
    dropped: AtomicUsize,
}
struct CatalogRead<'a>(&'a AtomicUsize);
impl Drop for CatalogRead<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Host for StalledCatalog {
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
        _: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async { panic!("Catalog requests must not select a generation account") })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async move {
            let _read = CatalogRead(&self.dropped);
            self.entered.notify_one();
            futures_util::future::pending().await
        })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
async fn fixture(timeout: Duration) -> (Handle, Arc<StalledCatalog>, tempfile::TempDir) {
    let host = Arc::new(StalledCatalog::default());
    let directory = tempfile::tempdir().unwrap();
    let service = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        directory.path().join("identity.sqlite"),
        Options {
            request_timeout: timeout,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    (service, host, directory)
}

#[tokio::test]
async fn every_catalog_and_token_count_entry_bounds_stalled_model_reads() {
    let (mut service, host, _directory) = fixture(Duration::from_millis(200)).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (index, (path, body)) in [
        ("/v1/models", None),
        ("/models", None),
        ("/v1beta/models", None),
        ("/v1beta/models/m", None),
        ("/api/tags", None),
        ("/api/show", Some(json!({"model":"m"}))),
        (
            "/v1/messages/count_tokens",
            Some(json!({"model":"m","messages":[]})),
        ),
        ("/v1beta/models/m:countTokens", Some(json!({"contents":[]}))),
    ]
    .into_iter()
    .enumerate()
    {
        let url = format!("http://{}{}", service.addr, path);
        let request = match body {
            Some(body) => client.post(url).json(&body),
            None => client.get(url),
        };
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            request.bearer_auth("fixture").send(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), 504, "{path}");
        let value: Value = response.json().await.unwrap();
        assert!(
            value["error"].is_object() || value["error"].is_string(),
            "{path}: {value}"
        );
        assert_eq!(host.dropped.load(Ordering::SeqCst), index + 1, "{path}");
        tokio::time::timeout(Duration::from_secs(2), async {
            while service.active_requests() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn service_stop_cancels_pending_compat_catalog_read_and_releases_request() {
    let (mut service, host, _directory) = fixture(Duration::from_secs(5)).await;
    let url = format!("http://{}/api/show", service.addr);
    let request = tokio::spawn(async move {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(url)
            .bearer_auth("fixture")
            .json(&json!({"model":"m"}))
            .send()
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), host.entered.notified())
        .await
        .unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let response = request.await.unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(host.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(service.active_requests(), 0);
}
