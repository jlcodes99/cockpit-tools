// Opt-in #3165 timing comparison against a paced loopback upstream.
use axum::{
    body::{Body, Bytes},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, stream};
use std::convert::Infallible;

const POOL_PROBE_TOKENS: usize = 32;
const POOL_PROBE_MODEL: &str = "gpt-6.1-sol";

struct PoolTimingHost {
    bridge: NativeHostBridge,
    upstream: String,
    client: reqwest::Client,
    timings: Mutex<Vec<f64>>,
}

impl native_contract::Host for PoolTimingHost {
    fn authorize<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<native_contract::Caller, native_contract::ServiceError>> {
        native_contract::Host::authorize(&self.bridge, key)
    }

    fn route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
    ) -> BoxFuture<'a, Result<native_contract::Route, native_contract::ServiceError>> {
        Box::pin(async move {
            let start = Instant::now();
            let mut result = native_contract::Host::route(&self.bridge, caller, request).await?;
            self.timings
                .lock()
                .unwrap()
                .push(start.elapsed().as_secs_f64() * 1000.0);
            result.base_url = self.upstream.clone();
            result.client = self.client.clone();
            Ok(result)
        })
    }

    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        native_contract::Host::models(&self.bridge, caller)
    }

    fn record(&self, _: native_contract::RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

async fn pool_timing_mock(Json(body): Json<Value>) -> axum::response::Response {
    assert_eq!(body["model"], POOL_PROBE_MODEL);
    assert_eq!(body["stream"], true);
    let started = tokio::time::Instant::now();
    let response_id = format!("resp-{}", uuid::Uuid::new_v4());
    let body = Body::from_stream(stream::unfold(
        (0, response_id),
        move |(n, id)| async move {
            let event = match n {
                0 => json!({"type":"response.created","response":{"id":id,"status":"in_progress"}}),
                1..=POOL_PROBE_TOKENS => {
                    tokio::time::sleep_until(
                        started + Duration::from_millis(6 + 3 * (n as u64 - 1)),
                    )
                    .await;
                    json!({"type":"response.output_text.delta","item_id":"msg-pool","output_index":0,"content_index":0,"delta":" x"})
                }
                n if n == POOL_PROBE_TOKENS + 1 => json!({"type":"response.completed","response":{
                    "id":id,"status":"completed","output":[],"usage":{"input_tokens":8,"output_tokens":POOL_PROBE_TOKENS}
                }}),
                _ => return None,
            };
            Some((
                Ok::<_, Infallible>(Bytes::from(format!("data: {event}\n\n"))),
                (n + 1, id),
            ))
        },
    ));
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(body)
        .unwrap()
}

async fn measure_pool_stream(client: &reqwest::Client, url: &str) -> (f64, f64) {
    let payload = json!({"model":POOL_PROBE_MODEL,"input":"fixed fixture prompt","stream":true,"store":false});
    let start = Instant::now();
    let mut response = client
        .post(url)
        .bearer_auth("fixture-primary-key")
        .json(&payload)
        .send()
        .await
        .unwrap();
    if !response.status().is_success() {
        panic!("{} {}", response.status(), response.text().await.unwrap());
    }
    let mut buffer = Vec::new();
    let mut deltas = Vec::new();
    let mut completed = false;
    while let Some(chunk) = response.chunk().await.unwrap() {
        let now = start.elapsed().as_secs_f64() * 1000.0;
        buffer.extend_from_slice(&chunk);
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=end).collect::<Vec<_>>();
            let Some(raw) = line.strip_prefix(b"data: ") else {
                continue;
            };
            let event: Value = serde_json::from_slice(raw).unwrap();
            match event["type"].as_str() {
                Some("response.output_text.delta") => deltas.push(now),
                Some("response.completed") => {
                    assert_eq!(
                        event["response"]["usage"]["output_tokens"],
                        POOL_PROBE_TOKENS
                    );
                    completed = true;
                }
                Some("error" | "response.failed") => panic!("{event}"),
                _ => (),
            }
        }
    }
    assert!(completed);
    assert_eq!(deltas.len(), POOL_PROBE_TOKENS);
    (
        deltas[0],
        (POOL_PROBE_TOKENS - 1) as f64 * 1000.0 / (deltas[POOL_PROBE_TOKENS - 1] - deltas[0]),
    )
}

fn pool_probe_percentile(values: impl Iterator<Item = f64>, fraction: f64) -> f64 {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "local timing benchmark; run separately with --ignored --nocapture"]
async fn native_gateway_mock_pool_scaling_3165() {
    use base64::Engine;
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _isolated = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let temp = tempfile::tempdir().unwrap();
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            upstream,
            Router::new().route("/v1/responses", post(pool_timing_mock)),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let pool = (0..128)
        .map(|n| {
            let mut account = native_test_account_with_quota(90, 80);
            account.id = format!("pool-probe-{n}");
            account.email = format!("{n}@example.invalid");
            account.plan_type = Some("plus".into());
            account.tokens.refresh_token = None;
            account.tokens.access_token = format!(
                "e30.{}.fixture",
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(json!({"exp":chrono::Utc::now().timestamp()+86400}).to_string())
            );
            account.egress_proxy_disabled = true;
            codex_account::save_account(&account).unwrap();
            account.id
        })
        .collect::<Vec<_>>();
    let mut endpoints = vec![(0, format!("{base}/responses"), None)];
    let mut handles = Vec::new();
    for size in [1, 16, 64, 128] {
        let mut collection = native_test_collection();
        collection.account_ids = pool[..size].to_vec();
        collection.session_affinity = false;
        let bridge = NativeHostBridge::for_collection(collection);
        let host = Arc::new(PoolTimingHost {
            bridge,
            upstream: base.clone(),
            client: client.clone(),
            timings: Mutex::new(Vec::new()),
        });
        let service = native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            temp.path().join(format!("identity-{size}.sqlite")),
            native_api::Options::default(),
        )
        .await
        .unwrap();
        endpoints.push((
            size,
            format!("http://{}/v1/responses", service.addr),
            Some(host),
        ));
        handles.push(service);
    }
    let mut timings = vec![Vec::new(); endpoints.len()];
    for (_, endpoint, _) in &endpoints {
        measure_pool_stream(&client, endpoint).await;
    }
    for round in 0..12 {
        for i in 0..endpoints.len() {
            let index = (i + round) % endpoints.len();
            timings[index].push(measure_pool_stream(&client, &endpoints[index].1).await);
        }
    }
    for (i, (size, _, host)) in endpoints.iter().enumerate() {
        let route = host
            .as_ref()
            .map(|host| {
                let samples = host.timings.lock().unwrap();
                assert_eq!(samples.len(), 13);
                pool_probe_percentile(samples[1..].iter().copied(), 0.5)
            })
            .unwrap_or_default();
        println!("POOL3165 serial size={size:3} first_ms p50={:7.2} p95={:7.2} tok_s p50={:7.2} route_ms p50={route:7.2}",
            pool_probe_percentile(timings[i].iter().map(|item| item.0), 0.5),
            pool_probe_percentile(timings[i].iter().map(|item| item.0), 0.95),
            pool_probe_percentile(timings[i].iter().map(|item| item.1), 0.5));
    }
    for (size, endpoint, host) in &endpoints {
        if let Some(host) = host {
            host.timings.lock().unwrap().clear();
        }
        let concurrency = 8;
        let mut samples = Vec::new();
        for _ in 0..4 {
            samples.extend(
                futures_util::future::join_all(
                    (0..concurrency).map(|_| measure_pool_stream(&client, endpoint)),
                )
                .await,
            );
        }
        let route = host
            .as_ref()
            .map(|host| {
                let samples = host.timings.lock().unwrap();
                assert_eq!(samples.len(), concurrency * 4);
                pool_probe_percentile(samples.iter().copied(), 0.5)
            })
            .unwrap_or_default();
        println!("POOL3165 concurrent={concurrency} size={size:3} first_ms p50={:7.2} p95={:7.2} tok_s p50={:7.2} route_ms p50={route:7.2}",
            pool_probe_percentile(samples.iter().map(|item| item.0), 0.5),
            pool_probe_percentile(samples.iter().map(|item| item.0), 0.95),
            pool_probe_percentile(samples.iter().map(|item| item.1), 0.5));
    }
    for mut service in handles {
        service.shutdown(Duration::from_secs(3)).await.unwrap();
    }
    server.abort();
}
