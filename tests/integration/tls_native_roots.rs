//! The framework's HTTP client trusts the operating system's certificate
//! store, not only the bundled Mozilla roots.
//!
//! Behind a TLS-intercepting corporate proxy every HTTPS response is signed
//! by a company CA that IT installed into the OS store. A client that only
//! trusts bundled roots fails there while `curl` and PowerShell succeed. The
//! test stands in for that CA with a throwaway one and installs it through
//! `SSL_CERT_FILE`, which `rustls-native-certs` reads in place of the
//! platform store on Linux, macOS and Windows alike, so the result does not
//! depend on the machine running it.
//!
//! One test function on purpose: `SSL_CERT_FILE` is process-global, and the
//! trusted and untrusted cases must not interleave.

use std::sync::Arc;

use cli_framework::http_retry::secure_reqwest_client;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_rustls::TlsAcceptor;

struct TestCa {
    cert: rcgen::Certificate,
    key: KeyPair,
}

fn new_ca(name: &str) -> TestCa {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, name);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let key = KeyPair::generate().expect("ca key");
    let cert = params.self_signed(&key).expect("self-sign ca");
    TestCa { cert, key }
}

/// Serves `200 ok` over HTTPS on 127.0.0.1 with a `localhost` certificate
/// signed by `ca`, and returns the port.
async fn serve_https(ca: &TestCa) -> u16 {
    let leaf_params = CertificateParams::new(vec!["localhost".to_string()]).expect("leaf params");
    let leaf_key = KeyPair::generate().expect("leaf key");
    let leaf = leaf_params
        .signed_by(&leaf_key, &ca.cert, &ca.key)
        .expect("sign leaf");

    let chain: Vec<CertificateDer<'static>> = vec![leaf.der().clone(), ca.cert.der().clone()];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
    // An explicit provider: under `--all-features` both `ring` and `aws-lc-rs`
    // are linked, and the implicit process default would be ambiguous.
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("server cert");
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A client that rejects the certificate aborts the handshake;
                // that is the expected outcome of the untrusted case.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = tls
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = tls.shutdown().await;
            });
        }
    });
    port
}

fn write_pem(dir: &std::path::Path, name: &str, ca: &TestCa) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, ca.cert.pem()).expect("write ca pem");
    path
}

#[tokio::test]
async fn trusts_a_ca_from_the_native_store_and_only_that_ca() {
    let dir = tempfile::tempdir().expect("tempdir");
    let company_ca = new_ca("cli-framework test company CA");
    let unrelated_ca = new_ca("cli-framework test unrelated CA");
    let port = serve_https(&company_ca).await;
    let url = format!("https://localhost:{port}/");

    // Trusted: the company CA is in the native store.
    std::env::set_var(
        "SSL_CERT_FILE",
        write_pem(dir.path(), "company.pem", &company_ca),
    );
    std::env::remove_var("SSL_CERT_DIR");
    let client = secure_reqwest_client().expect("client with native roots");
    let response = client
        .get(&url)
        .send()
        .await
        .expect("a CA installed in the OS trust store must be trusted");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.expect("body"), "ok");

    // Untrusted: the store holds a different CA, so the same server must be
    // rejected. Without this half the test would also pass against a client
    // that skipped verification.
    std::env::set_var(
        "SSL_CERT_FILE",
        write_pem(dir.path(), "unrelated.pem", &unrelated_ca),
    );
    let client = secure_reqwest_client().expect("client with native roots");
    let err = client
        .get(&url)
        .send()
        .await
        .expect_err("a server signed by a CA outside every trust store must be rejected");
    assert!(
        err.is_connect(),
        "expected a TLS connect error, got: {err:?}"
    );

    std::env::remove_var("SSL_CERT_FILE");
}
