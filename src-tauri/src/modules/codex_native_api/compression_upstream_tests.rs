use super::*;
use http_body_util::BodyExt;

struct CompressionHost {
    host: Arc<TestHost>,
    client: reqwest::Client,
    url: String,
    stalled_rejection: bool,
}
impl Host for CompressionHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.host.authorize(key)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.host.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.host.record(report)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            let mut policy = self.host.execution_policy().await?;
            if self.stalled_rejection {
                policy.open_timeout = Duration::from_millis(250);
                policy.total_timeout = Duration::from_secs(3);
                policy.send_retries = 2;
            }
            Ok(policy)
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut route = self.host.route(caller, request).await?;
            route.base_url = self.url.clone();
            route.client = self.client.clone();
            route.chatgpt = true;
            Ok(route)
        })
    }
}

#[tokio::test]
async fn full_compat_official_zstd_request_rejection_retries_exact_plaintext_once() {
    for (rejected_status, executed, stalled) in [
        (400, false, false),
        (415, false, false),
        (400, true, false),
        (415, true, false),
        (400, false, true),
    ] {
        let cert = rcgen::generate_simple_self_signed(vec!["chatgpt.com".into()]).unwrap();
        let root = reqwest::Certificate::from_der(cert.cert.der()).unwrap();
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let captured = Arc::new(Mutex::new(Vec::<(HeaderMap, Vec<u8>)>::new()));
        let capture = captured.clone();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let tls = acceptor.clone();
                let capture = capture.clone();
                tokio::spawn(async move {
                    let socket = tls.accept(socket).await.unwrap();
                    let service = hyper::service::service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let capture = capture.clone();
                            async move {
                                assert!(request.uri().path().starts_with("/backend-api/codex/"));
                                let (parts, body) = request.into_parts();
                                let bytes = body.collect().await.unwrap().to_bytes();
                                let compressed = parts
                                    .headers
                                    .get("content-encoding")
                                    .is_some_and(|v| v == "zstd");
                                let plain = if compressed {
                                    zstd::stream::decode_all(bytes.as_ref()).unwrap()
                                } else {
                                    bytes.to_vec()
                                };
                                capture.lock().unwrap().push((parts.headers, plain));
                                let response = if compressed && stalled {
                                    Response::builder()
                                        .status(rejected_status)
                                        .header("content-type", "application/json")
                                        .body(Body::from_stream(futures_util::stream::pending::<
                                            Result<axum::body::Bytes, std::io::Error>,
                                        >(
                                        )))
                                        .unwrap()
                                } else if compressed {
                                    let rejected = if executed {
                                        json!({"error":{"code":"unsupported_content_encoding","message":"Use identity"},"response":{"output":[{"type":"function_call","call_id":"already-run","name":"write_file","arguments":"{}"}]}}).to_string()
                                    } else {
                                        r#"{"error":{"code":"unsupported_content_encoding","message":"Use identity"}}"#.into()
                                    };
                                    Response::builder()
                                        .status(rejected_status)
                                        .header("content-type", "application/json")
                                        .body(Body::from(rejected))
                                        .unwrap()
                                } else {
                                    let bytes=zstd::stream::encode_all(br#"{"id":"compressed-upstream","status":"completed","output":[]}"#.as_slice(),3).unwrap();
                                    Response::builder()
                                        .header("content-type", "application/json")
                                        .header("content-encoding", "zstd")
                                        .body(Body::from(bytes))
                                        .unwrap()
                                };
                                Ok::<_, std::convert::Infallible>(response)
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        let (mut unused, host, path) = native(addr, true, false, Options::default()).await;
        unused.shutdown(Duration::from_secs(2)).await.unwrap();
        let host = Arc::new(CompressionHost {
            host,
            client: reqwest::Client::builder()
                .no_proxy()
                .add_root_certificate(root)
                .resolve("chatgpt.com", addr)
                .build()
                .unwrap(),
            url: format!("https://chatgpt.com:{}/backend-api/codex", addr.port()),
            stalled_rejection: stalled,
        });
        let mut handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.join("compressed.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let request = json!({"model":"m","input":[{"role":"user","content":"compressible conversation ".repeat(4096)}]});
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("local-test-key")
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let value: Value = response.json().await.unwrap();
        assert_eq!(
            status.as_u16(),
            if stalled {
                504
            } else if executed {
                rejected_status
            } else {
                200
            },
            "{value}"
        );
        if !executed && !stalled {
            assert_eq!(value["id"], "compressed-upstream");
        }
        if stalled {
            assert_eq!(value["error"]["code"], "upstream_idle_timeout");
        }
        let capture = captured.lock().unwrap();
        assert_eq!(capture.len(), if executed || stalled { 1 } else { 2 });
        assert_eq!(capture[0].0["content-encoding"], "zstd");
        if !executed && !stalled {
            assert!(!capture[1].0.contains_key("content-encoding"));
            assert_eq!(capture[0].1, capture[1].1);
            assert_eq!(capture[0].0["authorization"], capture[1].0["authorization"]);
        }
        drop(capture);
        assert_eq!(host.host.selections.lock().unwrap().len(), 1);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tokio::test]
async fn full_compat_compound_upstream_compression_decodes_stream_and_error() {
    use std::io::Write;
    for failed in [false, true] {
        let original = if failed {
            r#"{"error":{"code":"invalid_parameter","message":"rejected"}}"#.to_owned()
        } else {
            format!("{}{}", created(), completed())
        };
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(original.as_bytes()).unwrap();
        let gzip = gzip.finish().unwrap();
        let bytes = zstd::stream::encode_all(gzip.as_slice(), 3).unwrap();
        let (addr, task) = mock(Router::new().route(
            "/v1/responses",
            post(move || {
                let bytes = bytes.clone();
                async move {
                    Response::builder()
                        .status(if failed { 400 } else { 200 })
                        .header(
                            "content-type",
                            if failed {
                                "application/json"
                            } else {
                                "text/event-stream"
                            },
                        )
                        .header("content-encoding", "gzip, zstd")
                        .body(Body::from(bytes))
                        .unwrap()
                }
            }),
        ))
        .await;
        let (mut handle, _, path) = native(addr, false, false, Options::default()).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("local-test-key")
            .json(&body(!failed))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), if failed { 400 } else { 200 });
        let text = response.text().await.unwrap();
        assert!(
            text.contains(if failed {
                "rejected"
            } else {
                "response.completed"
            }),
            "{text}"
        );
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}
