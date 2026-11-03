use std::sync::Arc;

pub(crate) fn client_builder() -> Result<reqwest::ClientBuilder, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if roots.is_empty() {
        return Err("No system trust certificates available for account requests".to_string());
    }
    Ok(client_builder_with_roots(roots))
}

pub(crate) fn client_builder_with_roots(roots: rustls::RootCertStore) -> reqwest::ClientBuilder {
    // Secure Transport lacks TLS 1.3. Select AWS-LC per client so these flows
    // also support hybrid key exchange without changing the global provider.
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("AWS-LC supports the default TLS protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    reqwest::Client::builder().use_preconfigured_tls(tls)
}
