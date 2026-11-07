//! Exercise the host and updater HTTP dependency graphs against local TLS only.
//! No application state, system trust changes, credentials or external requests.
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::TlsAcceptor;

const DEADLINE: Duration = Duration::from_secs(5);

async fn tls_fixture(
    name: &str,
    version: &'static rustls::SupportedProtocolVersion,
) -> (u16, Vec<u8>, JoinHandle<bool>) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec![name.to_string()]).unwrap();
    let der = certificate.cert.der().to_vec();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_protocol_versions(&[version])
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der()).into(),
    )
    .unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        tokio::time::timeout(DEADLINE, async move {
            let (socket, _) = listener.accept().await.unwrap();
            let Ok(mut tls) = TlsAcceptor::from(Arc::new(config)).accept(socket).await else {
                return false;
            };
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let Ok(byte) = tls.read_u8().await else {
                    return false;
                };
                headers.push(byte);
                assert!(headers.len() < 16 * 1024);
            }
            assert!(headers.starts_with(b"GET /fixture HTTP/1.1\r\n"));
            tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            let _ = tls.shutdown().await;
            true
        })
        .await
        .expect("local TLS fixture timed out")
    });
    (port, der, task)
}

async fn proxy_fixture(socks: bool, target_port: u16) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        tokio::time::timeout(DEADLINE, async move {
            let (mut client, _) = listener.accept().await.unwrap();
            if socks {
                assert_eq!(client.read_u8().await.unwrap(), 5);
                let count = client.read_u8().await.unwrap();
                let mut methods = vec![0; count as usize];
                client.read_exact(&mut methods).await.unwrap();
                assert!(methods.contains(&0));
                client.write_all(&[5, 0]).await.unwrap();
                let mut header = [0; 4];
                client.read_exact(&mut header).await.unwrap();
                assert_eq!(header, [5, 1, 0, 1]);
                let mut address = [0; 4];
                client.read_exact(&mut address).await.unwrap();
                assert_eq!(address, [127, 0, 0, 1]);
                assert_eq!(client.read_u16().await.unwrap(), target_port);
                client
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
            } else {
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(client.read_u8().await.unwrap());
                    assert!(headers.len() < 16 * 1024);
                }
                assert!(headers.starts_with(
                    format!("CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\n").as_bytes()
                ));
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
            }
            let mut upstream = TcpStream::connect((Ipv4Addr::LOCALHOST, target_port))
                .await
                .unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        })
        .await
        .expect("local proxy fixture timed out");
    });
    (
        format!(
            "{}://127.0.0.1:{port}",
            if socks { "socks5" } else { "http" }
        ),
        task,
    )
}

async fn request(
    updater: bool,
    port: u16,
    trust: Option<&[u8]>,
    proxy: Option<&str>,
) -> Result<String, String> {
    let url = format!("https://127.0.0.1:{port}/fixture");
    if updater {
        // This alias is the exact reqwest 0.13 graph used by tauri-plugin-updater.
        let roots = trust
            .map(reqwest_updater_socks::Certificate::from_der)
            .transpose()
            .unwrap();
        let mut builder = reqwest_updater_socks::Client::builder()
            .no_proxy()
            .timeout(DEADLINE)
            .tls_certs_only(roots);
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest_updater_socks::Proxy::all(proxy).unwrap());
        }
        builder
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .map_err(|e| format!("{e:?}"))?
            .text()
            .await
            .map_err(|e| format!("{e:?}"))
    } else {
        // Keep the host client's configured default TLS backend, including native TLS.
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .timeout(DEADLINE)
            .tls_built_in_root_certs(false);
        if let Some(trust) = trust {
            builder = builder.add_root_certificate(reqwest::Certificate::from_der(trust).unwrap());
        }
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy).unwrap());
        }
        builder
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .map_err(|e| format!("{e:?}"))?
            .text()
            .await
            .map_err(|e| format!("{e:?}"))
    }
}

async fn trusted_routes(updater: bool) {
    // native-tls uses Secure Transport on Apple; that unchanged backend supports
    // TLS 1.2, while the updater's rustls backend also supports TLS 1.3.
    let versions: &[&rustls::SupportedProtocolVersion] =
        if !updater && cfg!(target_vendor = "apple") {
            &[&rustls::version::TLS12]
        } else {
            &[&rustls::version::TLS12, &rustls::version::TLS13]
        };
    for version in versions {
        for route in ["direct", "http", "socks5"] {
            let (port, certificate, server) = tls_fixture("127.0.0.1", version).await;
            let proxy = if route == "direct" {
                None
            } else {
                Some(proxy_fixture(route == "socks5", port).await)
            };
            let result = request(
                updater,
                port,
                Some(&certificate),
                proxy.as_ref().map(|p| p.0.as_str()),
            )
            .await;
            assert_eq!(
                result,
                Ok("ok".to_string()),
                "updater={updater}, route={route}, version={version:?}"
            );
            assert!(server.await.unwrap());
            if let Some((_, proxy)) = proxy {
                proxy.await.unwrap();
            }
        }
    }
}

async fn rejects_invalid_certificates(updater: bool) {
    for wrong_name in [false, true] {
        let name = if wrong_name {
            "wrong.invalid"
        } else {
            "127.0.0.1"
        };
        let (port, certificate, server) = tls_fixture(name, &rustls::version::TLS12).await;
        let trust = wrong_name.then_some(certificate.as_slice());
        assert!(request(updater, port, trust, None).await.is_err());
        assert!(
            !server.await.unwrap(),
            "invalid certificate reached an HTTP request"
        );
    }
}

#[tokio::test]
async fn host_supported_tls_direct_http_and_socks_proxy() {
    trusted_routes(false).await;
}

#[tokio::test]
async fn updater_tls12_tls13_direct_http_and_socks_proxy() {
    trusted_routes(true).await;
}

#[tokio::test]
async fn host_rejects_unknown_root_and_wrong_hostname() {
    rejects_invalid_certificates(false).await;
}

#[tokio::test]
async fn updater_rejects_unknown_root_and_wrong_hostname() {
    rejects_invalid_certificates(true).await;
}
