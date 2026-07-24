//! Live-OpenBao trait-conformance test — opt-in only.
//!
//! This sandbox has no network path to Docker Hub (`docker pull
//! openbao/openbao` / `hashicorp/vault` both fail DNS resolution here), so
//! this test can't be exercised in this environment. It's gated behind
//! `CFW_TEST_OPENBAO_LIVE=1` and SKIPS (does not fail) when that's unset,
//! per PRD-005's fallback: "gate ONLY the live-OpenBao test behind an env
//! flag ... in-memory + env/file conformance are MANDATORY".
//!
//! Run it somewhere with Docker + registry egress to actually exercise the
//! OpenBao backend end to end:
//!
//! ```sh
//! CFW_TEST_OPENBAO_LIVE=1 cargo test --features secrets-openbao \
//!     --test unit_secrets_openbao_conformance
//! ```
//!
//! The mandatory conformance coverage (in-memory + env/file, always runs)
//! lives in `tests/unit/secrets_conformance.rs`.

use cli_framework::secrets::openbao::{OpenBaoAuth, OpenBaoConfig, OpenBaoSecretStore};
use cli_framework::secrets::{SecretError, SecretKey, SecretStore, SecretValue};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

#[tokio::test]
async fn openbao_backend_is_conformant() {
    if std::env::var("CFW_TEST_OPENBAO_LIVE").is_err() {
        eprintln!(
            "skipping openbao_backend_is_conformant: set CFW_TEST_OPENBAO_LIVE=1 \
             to run against a live OpenBao dev-mode container (requires Docker Hub egress)"
        );
        return;
    }

    let root_token = "cfw-test-root-token";
    let container = GenericImage::new("openbao/openbao", "latest")
        .with_exposed_port(8200.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Development mode"))
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", root_token)
        .start()
        .await
        .expect("start openbao dev-mode container");

    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("mapped host port for 8200/tcp");
    let address = format!("http://127.0.0.1:{port}");

    let store = OpenBaoSecretStore::new(OpenBaoConfig {
        address,
        auth: OpenBaoAuth::Token(root_token.to_string()),
        mount: "secret".to_string(),
        namespace: None,
    });

    let key = SecretKey::new(["conformance", "openbao-widget"]).unwrap();

    // get-missing → NotFound
    let err = store.get(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotFound), "got {err:?}");

    // put → get round-trip
    store
        .put(&key, SecretValue::from("first-value"))
        .await
        .expect("put");
    let got = store.get(&key).await.expect("get after put");
    assert_eq!(got.expose_str().unwrap(), "first-value");

    // overwrite via put
    store
        .put(&key, SecretValue::from("second-value"))
        .await
        .expect("overwrite put");
    let got = store.get(&key).await.expect("get after overwrite");
    assert_eq!(got.expose_str().unwrap(), "second-value");

    // delete → subsequent get NotFound
    store.delete(&key).await.expect("delete");
    let err = store.get(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotFound), "got {err:?}");

    // rotate is NotSupported in R1
    let err = store.rotate(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotSupported(_)), "got {err:?}");
}
