//! Real loopback transports, without GUI, credentials or external upstreams.
use super::*;

async fn read_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(stream.read_u8().await.unwrap());
        assert!(bytes.len() <= 16 * 1024);
    }
    bytes
}

async fn ingress(entry: &str, http: bool, host: &str, port: u16) -> TcpStream {
    let mut stream = TcpStream::connect((
        Ipv4Addr::LOCALHOST,
        url::Url::parse(entry).unwrap().port().unwrap(),
    ))
    .await
    .unwrap();
    if http {
        // A client may send the TLS preface in the same write as CONNECT.
        let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: do-not-forward\r\n\r\nping");
        stream.write_all(request.as_bytes()).await.unwrap();
        assert_eq!(
            read_headers(&mut stream).await,
            b"HTTP/1.1 200 Connection Established\r\n\r\n"
        );
    } else {
        stream.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 0]);
        let mut request = vec![5, 1, 0, 3, host.len() as u8];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await.unwrap();
        let mut reply = [0; 10];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        stream.write_all(b"ping").await.unwrap();
    }
    stream
}

#[tokio::test]
async fn both_ingress_protocols_share_binding_changes_and_preserve_tunnel_bytes() {
    let fixture = LaunchFixture::new("mixed-transport");
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut account = eligible_account("mixed-transport");
        let mut first_entry = None;
        for upstream_socks in [false, true] {
            let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let scheme = if upstream_socks { "socks5" } else { "http" };
            account.egress_proxy_url = Some(format!(
                "{scheme}://127.0.0.1:{}",
                upstream.local_addr().unwrap().port()
            ));
            fixture.save(&account);
            let entry = ensure(&account.id).await.unwrap().unwrap();
            if let Some(first) = &first_entry {
                assert_eq!(&entry, first);
            }
            first_entry = Some(entry.clone());
            let upstream_task = tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut socket, _) = upstream.accept().await.unwrap();
                    if upstream_socks {
                        let target = read_socks_request(&mut socket).await.unwrap();
                        assert_eq!(
                            (target.host.as_str(), target.port),
                            ("fixture.invalid", 443)
                        );
                        socket
                            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                            .await
                            .unwrap();
                    } else {
                        let headers = read_headers(&mut socket).await;
                        assert!(headers.starts_with(b"CONNECT fixture.invalid:443 HTTP/1.1\r\n"));
                        assert!(!String::from_utf8(headers)
                            .unwrap()
                            .contains("do-not-forward"));
                        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                    }
                    let mut payload = [0; 4];
                    socket.read_exact(&mut payload).await.unwrap();
                    assert_eq!(&payload, b"ping");
                    socket.write_all(b"pong").await.unwrap();
                }
            });
            for http in [true, false] {
                let mut client = ingress(&entry, http, "fixture.invalid", 443).await;
                let mut reply = [0; 4];
                client.read_exact(&mut reply).await.unwrap();
                assert_eq!(&reply, b"pong");
            }
            upstream_task.await.unwrap();
        }
        assert_eq!(entry_status(&account.id).unwrap().request_count, 4);
    })
    .await
    .expect("mixed ingress routing deadline");
}

#[tokio::test]
async fn http_connect_returns_bad_gateway_and_can_recover_on_the_same_entry() {
    let fixture = LaunchFixture::new("http-recovery");
    tokio::time::timeout(Duration::from_secs(5), async {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut account = eligible_account("http-recovery");
        account.egress_proxy_url = Some(format!(
            "http://127.0.0.1:{}",
            upstream.local_addr().unwrap().port()
        ));
        fixture.save(&account);
        let entry = ensure(&account.id).await.unwrap().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 502 Failed\r\n\r\n")
                .await
                .unwrap();
        });
        let mut client = TcpStream::connect((
            Ipv4Addr::LOCALHOST,
            url::Url::parse(&entry).unwrap().port().unwrap(),
        ))
        .await
        .unwrap();
        client
            .write_all(b"CONNECT fixture.invalid:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        assert!(read_headers(&mut client).await.starts_with(b"HTTP/1.1 502"));
        assert_eq!(
            entry_status(&account.id).unwrap().last_request_state,
            "failed"
        );
        task.await.unwrap();

        // Unbinding affects the next HTTP connection, while the entry remains stable.
        account.egress_proxy_url = None;
        fixture.save(&account);
        let origin = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let echo = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut payload = [0; 4];
            socket.read_exact(&mut payload).await.unwrap();
            socket.write_all(&payload).await.unwrap();
        });
        let mut retry = ingress(&entry, true, "127.0.0.1", port).await;
        let mut reply = [0; 4];
        retry.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"ping");
        echo.await.unwrap();
        assert_eq!(
            entry_status(&account.id).unwrap().last_request_state,
            "forwarded"
        );
    })
    .await
    .expect("HTTP recovery deadline");
}

#[tokio::test]
async fn malformed_http_handshake_is_rejected_without_resolving_an_account() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let observed = entry::listening("malformed-http", port);
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, "missing-account", observed.entry()).await
        });
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        client.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        assert!(read_headers(&mut client).await.starts_with(b"HTTP/1.1 400"));
        assert_eq!(task.await.unwrap().unwrap_err(), "PROXY_CONNECT_FAILED");
        assert_eq!(entry_status("malformed-http").unwrap().request_count, 0);
    })
    .await
    .expect("malformed handshake deadline");
}

#[tokio::test]
async fn stalled_http_header_is_bounded_by_the_handshake_deadline() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let observed = entry::listening("stalled-http-header", port);
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        handle(socket, "missing-account", observed.entry()).await
    });
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    client
        .write_all(b"CONNECT fixture.invalid:443 HTTP/1.1\r\n")
        .await
        .unwrap();
    let result = tokio::time::timeout(CONNECT_TIMEOUT + Duration::from_secs(2), task)
        .await
        .expect("stalled header must time out")
        .unwrap();
    assert_eq!(result.unwrap_err(), "PROXY_ENGINE_TIMEOUT");
    assert!(read_headers(&mut client).await.starts_with(b"HTTP/1.1 400"));
    assert_eq!(
        entry_status("stalled-http-header").unwrap().request_count,
        0
    );
}
