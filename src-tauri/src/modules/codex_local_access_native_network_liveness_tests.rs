// A dead peer keeps its TCP/TLS socket open but stops driving HTTP/2, just as
// a broken upstream path can behind a still-running local CONNECT proxy.
struct NativeNetworkPeer {
    address: std::net::SocketAddr,
    root: reqwest::Certificate,
    request_seen: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for NativeNetworkPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn native_network_peer(blackhole: bool) -> NativeNetworkPeer {
    use hyper::{server::conn::http2, service::service_fn};
    use hyper_util::rt::{TokioExecutor, TokioIo};

    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let root = reqwest::Certificate::from_der(certificate.cert.der()).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der()).into(),
    )
    .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_seen = Arc::new(tokio::sync::Notify::new());
    let server_seen = request_seen.clone();
    let task = tokio::spawn(async move {
        let (socket, peer) = listener.accept().await.unwrap();
        assert!(peer.ip().is_loopback());
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await
            .unwrap();
        if blackhole {
            let mut connection = h2::server::handshake(tls).await.unwrap();
            let (request, response) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.method(), axum::http::Method::POST);
            server_seen.notify_one();
            std::future::pending::<()>().await;
            // Hold the live socket and stream; dropping either would test EOF
            // instead of the otherwise-silent connection this test targets.
            drop((connection, request, response));
        } else {
            let service = service_fn(|_: hyper::Request<hyper::body::Incoming>| async {
                // No HTTP headers or model output for longer than the dead
                // peer's detection window, while HTTP/2 PING still gets ACKs.
                tokio::time::sleep(Duration::from_secs(50)).await;
                Ok::<_, std::convert::Infallible>(hyper::Response::new(axum::body::Body::from(
                    "healthy delayed response",
                )))
            });
            let _ = http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await;
        }
    });
    NativeNetworkPeer {
        address,
        root,
        request_seen,
        task,
    }
}

fn native_network_test_client(peer: &NativeNetworkPeer) -> Client {
    native_http_client_builder(true, false)
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(peer.root.clone())
        // The outer bound is deliberately later than heartbeat detection.
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

#[tokio::test]
async fn native_network_blackhole_is_detected_before_the_stream_open_deadline() {
    let peer = native_network_peer(true).await;
    let client = native_network_test_client(&peer);
    let started = Instant::now();
    let (result, observed) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(55),
            client
                .post(format!("https://{}/responses", peer.address))
                .body("synthetic network probe")
                .send(),
        ),
        tokio::time::timeout(Duration::from_secs(5), peer.request_seen.notified())
    );
    observed.expect("the peer must receive an HTTP/2 request before the path goes silent");
    let error = result
        .expect("a silent HTTP/2 path must fail before the long request-open timeout")
        .unwrap_err();
    assert!(started.elapsed() >= Duration::from_secs(30));
    assert!(started.elapsed() < Duration::from_secs(55));
    assert!(
        !error.is_builder(),
        "the established HTTP/2 transport must fail: {error}"
    );
}

#[tokio::test]
async fn native_network_healthy_silent_model_survives_the_heartbeat_window() {
    let peer = native_network_peer(false).await;
    let client = native_network_test_client(&peer);
    let response = client
        .post(format!("https://{}/responses", peer.address))
        .body("synthetic delayed model")
        .send()
        .await
        .unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    assert_eq!(response.text().await.unwrap(), "healthy delayed response");
}
