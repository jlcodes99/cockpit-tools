use super::*;

fn reference() -> Value {
    serde_json::from_str(include_str!("fixtures/parity22_realtime_go.json")).unwrap()
}

fn route(base: &str, client: reqwest::Client) -> Route {
    Route {
        account_id: "account-a".into(),
        owner_id: "owner-a".into(),
        base_url: base.into(),
        model: "gpt-realtime".into(),
        headers: HeaderMap::new(),
        client,
        chatgpt: true,
        chatgpt_plan: false,
        wire_api: WireApi::Responses,
        continuity_account_id: None,
        installation_profile: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
                official_fallback_failure: None,
        transport_identity: None,
        proxy_url: None,
        proxy_route: None,
        compact_endpoint: None,
        websocket_endpoint: None,
        permit: None,
        default_service_tier: None,
    }
}

#[tokio::test]
async fn parity22_realtime_go_urls_match_actual_tls_proxy_wire() {
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
    async fn read_head(reader: &mut (impl AsyncRead + Unpin)) -> String {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(reader.read_u8().await.unwrap());
            assert!(bytes.len() < 16384);
        }
        String::from_utf8(bytes).unwrap()
    }
    let certificate =
        rcgen::generate_simple_self_signed(vec!["api.openai.com".into(), "chatgpt.com".into()])
            .unwrap();
    let root = reqwest::Certificate::from_der(certificate.cert.der()).unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
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
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let (sent, mut seen) = tokio::sync::mpsc::channel::<String>(16);
    let task = tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let sent = sent.clone();
            tokio::spawn(async move {
                let connect = read_head(&mut tcp).await;
                assert!(
                    connect.starts_with("CONNECT api.openai.com:443 ")
                        || connect.starts_with("CONNECT chatgpt.com:443 "),
                    "{connect}"
                );
                tcp.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
                let mut stream = acceptor.accept(tcp).await.unwrap();
                let head = read_head(&mut stream).await;
                sent.send(head.clone()).await.unwrap();
                if head.starts_with("GET ") {
                    let key = head
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-key"))
                                .map(|(_, value)| value.trim())
                        })
                        .unwrap();
                    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(
                        key.as_bytes(),
                    );
                    stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await.unwrap();
                    let _ = stream.read_u8().await;
                } else {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                }
            });
        }
    });
    let proxy_client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::https(format!("http://{proxy}")).unwrap())
        .add_root_certificate(root)
        .build()
        .unwrap();
    let selected = route("https://chatgpt.com/backend-api/codex", proxy_client);
    let (_stop, receiver) = tokio::sync::watch::channel(false);
    for case in reference()["urls"].as_array().unwrap() {
        let path = case["path"].as_str().unwrap();
        let expected = url::Url::parse(case["url"].as_str().unwrap()).unwrap();
        if path.ends_with("/hangup") {
            let target = realtime::hangup_url(&selected, "rtc_fixture");
            assert_eq!(target, expected.as_str());
            assert_eq!(
                selected.client.post(target).send().await.unwrap().status(),
                200
            );
        } else {
            let target = realtime_websocket_url(
                &selected,
                path,
                case["call_id"].as_str(),
                case["model"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(target, expected, "{case}");
            let (_socket, _) = super::super::super::transport::connect_websocket(
                &selected,
                target.as_str(),
                HeaderMap::new(),
                1024,
                Duration::from_secs(3),
                super::super::super::lifecycle::StopSignal(receiver.clone()),
            )
            .await
            .unwrap();
        }
        let head = tokio::time::timeout(Duration::from_secs(3), seen.recv())
            .await
            .unwrap()
            .unwrap();
        let line = head.lines().next().unwrap();
        assert_eq!(
            line.split_whitespace().nth(1).unwrap(),
            &expected[url::Position::BeforePath..],
            "{head}"
        );
        let host = head
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                    .map(|(_, value)| value.trim())
            })
            .unwrap();
        assert_eq!(host, expected.host_str().unwrap());
    }
    task.abort();
    let custom = route("http://127.0.0.1:9/custom", client());
    assert_eq!(
        realtime_websocket_url(&custom, "/v1/realtime", None, "gpt-realtime")
            .unwrap()
            .as_str(),
        "ws://127.0.0.1:9/custom/realtime?model=gpt-realtime"
    );
}

#[tokio::test]
async fn parity22_realtime_go_subprotocols_are_negotiated_without_local_credentials() {
    type Seen = Arc<Mutex<Vec<HeaderMap>>>;
    let observed: Seen = Arc::new(Mutex::new(vec![]));
    let app = Router::new()
        .route(
            "/v1/realtime",
            get(
                |ws: WebSocketUpgrade, headers: HeaderMap, State(seen): State<Seen>| async move {
                    assert_eq!(headers["authorization"], "Bearer upstream-secret");
                    seen.lock().unwrap().push(headers);
                    ws.protocols(["realtime"])
                        .on_upgrade(|mut socket| async move {
                            while let Some(Ok(message)) = socket.recv().await {
                                if let Message::Text(value) = message {
                                    let _ = socket.send(Message::Text(value)).await;
                                }
                            }
                        })
                },
            ),
        )
        .with_state(observed.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let mut request = format!("ws://{}/v1/realtime?model=gpt-realtime", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local-a"));
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static("realtime, openai-insecure-api-key.local-a, vendor.control"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    let go = reference();
    assert_eq!(
        response.headers()["sec-websocket-protocol"]
            .to_str()
            .unwrap(),
        go["selected_protocol"].as_str().unwrap()
    );
    let wire = observed.lock().unwrap().clone();
    let protocols = realtime::offered_protocols(&wire[0]);
    assert_eq!(json!(protocols), go["offered_protocols"]);
    assert!(!wire[0]["sec-websocket-protocol"]
        .to_str()
        .unwrap()
        .contains("local-a"));
    let body = json!({"type":"session.update","session":{"model":"gpt-realtime"}});
    socket
        .send(UpstreamMessage::Text(body.to_string().into()))
        .await
        .unwrap();
    let returned = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(returned.to_text().unwrap()).unwrap(),
        body
    );
    socket.close(None).await.unwrap();
    drop(socket);
    wait_free(&host).await;
    cleanup(handle, upstream, root).await;
}

#[test]
fn parity22_realtime_rejects_unoffered_upstream_subprotocols() {
    let headers = HeaderMap::from_iter([(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("openai-insecure-api-key.local-a"),
    )]);
    assert!(realtime::selected_protocol(&["realtime".into()], &headers).is_err());
}
