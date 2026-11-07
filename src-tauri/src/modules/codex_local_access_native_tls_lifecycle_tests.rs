// A real TLS/H2 stream through CONNECT, paused across account-pool eviction.
// Uses loopback certificates only; never reads user accounts or network settings.
#[tokio::test]
async fn native_tls_proxy_stream_survives_account_removal_and_another_identity() {
    use hyper::{server::conn::http2, service::service_fn};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
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
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let server_release = release.clone();
    let server_observed = observed.clone();
    let server = tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let mut connections = Vec::new();
        for _ in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let tls = acceptor.accept(socket).await.unwrap();
            let release = server_release.clone();
            let observed = server_observed.clone();
            connections.push(tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let release = release.clone();
                    let identity = request.headers()["x-codex-installation-id"].to_str().unwrap().to_owned();
                    observed.lock().unwrap().push(identity.clone());
                    async move {
                        use futures_util::{stream, StreamExt};
                        let first = format!("data: {{\"type\":\"response.created\",\"identity\":\"{identity}\"}}\n\n");
                        let chunks = stream::once(async move { Ok::<_, std::io::Error>(axum::body::Bytes::from(first)) })
                            .chain(stream::once(async move {
                                if identity == "installation-a" { release.notified().await; }
                                Ok(axum::body::Bytes::from_static(b"data: {\"type\":\"response.completed\"}\n\n"))
                            }));
                        Ok::<_, std::convert::Infallible>(hyper::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(axum::body::Body::from_stream(chunks)).unwrap())
                    }
                });
                // Dropping the client after completion can reset an idle H2 session.
                let _ = http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls), service).await;
            }));
        }
        for connection in connections {
            connection.await.unwrap();
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let proxy_task = tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(client.read_u8().await.unwrap());
                    assert!(header.len() < 8192);
                }
                assert!(header.starts_with(b"CONNECT localhost:"));
                let mut upstream = tokio::net::TcpStream::connect(address).await.unwrap();
                client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    let client = || {
        native_http_client_builder(true, false)
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
            .tls_built_in_root_certs(false)
            .add_root_certificate(root.clone())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    };
    let mut cache = NativeHttpClientCache::default();
    let first = client();
    cache.insert("a".into(), "account-a".into(), first.clone());
    let mut response = first
        .get(format!("https://localhost:{}/responses", address.port()))
        .header("x-codex-installation-id", "installation-a")
        .send()
        .await
        .unwrap();
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    assert!(
        String::from_utf8_lossy(&response.chunk().await.unwrap().unwrap())
            .contains("response.created")
    );
    cache.retain_accounts(|account| account != "account-a");
    drop(first);
    let second = client();
    cache.insert("b".into(), "account-b".into(), second.clone());
    let other = second
        .get(format!("https://localhost:{}/responses", address.port()))
        .header("x-codex-installation-id", "installation-b")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(other.contains("installation-b") && other.contains("response.completed"));
    release.notify_one();
    assert_eq!(
        response.text().await.unwrap(),
        "data: {\"type\":\"response.completed\"}\n\n"
    );
    assert_eq!(
        *observed.lock().unwrap(),
        ["installation-a", "installation-b"]
    );
    drop(second);
    drop(cache);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    proxy_task.abort();
}
