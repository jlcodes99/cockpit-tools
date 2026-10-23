//! Exercise the production subscription client against TLS on loopback only.
//! A generated certificate is trusted by this client alone, never by the system.
use super::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn https_subscription_download_parses_modern_nodes_without_client_credentials() {
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (socket, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            let mut tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(tls.read_u8().await.unwrap());
                assert!(headers.len() < 8192);
            }
            let headers = String::from_utf8(headers).unwrap().to_lowercase();
            assert!(headers.starts_with("get /subscription http/1.1\r\n"));
            assert!(headers.contains("user-agent: clash.meta/cockpittools"));
            for name in ["authorization:", "cookie:", "chatgpt-account-id:", "proxy-authorization:"] {
                assert!(!headers.contains(name), "subscription requests must not carry {name}");
            }
            let body = "proxies:\n  - {name: Synthetic, type: vless, server: 127.0.0.1, port: 9, uuid: 11111111-2222-3333-4444-555555555555, tls: true}\nproxy-groups:\n  - {name: Choose, type: select, proxies: [Synthetic]}\n";
            tls.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).as_bytes()).await.unwrap();
            let _ = tls.shutdown().await;
        }).await.expect("loopback TLS fixture deadline");
    });
    let client = subscription_client_builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(certificate.cert.der()).unwrap())
        .build()
        .unwrap();
    let url = subscription_url(&format!("https://{address}/subscription")).unwrap();
    let body = fetch_with_client(client, url.clone(), url.origin())
        .await
        .unwrap();
    let catalog = parser::parse(&body.body).unwrap();
    assert_eq!(catalog.nodes.len(), 1);
    assert_eq!(catalog.groups.len(), 1);
    assert!(catalog.nodes[0].outbound.is_some() && catalog.nodes[0].error.is_none());
    assert!(catalog.groups[0].error.is_none());
    fixture.await.unwrap();
}

#[tokio::test]
async fn https_subscription_rejects_untrusted_certificate_without_system_trust_changes() {
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (socket, _) = listener.accept().await.unwrap();
            assert!(tokio_rustls::TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await
                .is_err());
        })
        .await
        .expect("untrusted certificate fixture deadline");
    });
    let client = subscription_client_builder().no_proxy().build().unwrap();
    let url = subscription_url(&format!("https://{address}/synthetic-secret-token")).unwrap();
    let error = fetch_with_client(client, url.clone(), url.origin())
        .await
        .err()
        .unwrap();
    assert_eq!(error, "CATALOG_DOWNLOAD_TLS");
    assert!(!error.contains("synthetic-secret-token") && !error.contains(&address.to_string()));
    fixture.await.unwrap();
}
