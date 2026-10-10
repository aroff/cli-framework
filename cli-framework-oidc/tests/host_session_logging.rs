//! What building a `HostSessions` logs. Its own test binary: tracing caches a
//! callsite's interest the first time any thread reaches it, so a test in a
//! binary whose other tests reach the same `warn!` with no subscriber could
//! miss the line it is looking for.

use cli_framework_oidc::host_session::{HostSessionConfig, HostSessions, SessionKey};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The WARN lines logged while building a `HostSessions` from `cfg`.
fn warnings_building(cfg: HostSessionConfig) -> String {
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || HostSessions::new(cfg).unwrap());
    let out = buf.0.lock().unwrap().clone();
    String::from_utf8(out).unwrap()
}

fn config() -> HostSessionConfig {
    HostSessionConfig::new(
        "http://127.0.0.1:9/realms/meridis",
        "meridis-apps-host",
        "http://localhost:8080/_host/callback",
        SessionKey::from_bytes([7; 32]),
        "web-meridis.faseinfra.net",
    )
}

#[test]
fn the_unchecked_audience_warning_is_logged_only_without_the_azp_check() {
    let quiet = warnings_building(config());
    assert!(!quiet.contains("AudiencePolicy::Unchecked"), "{quiet}");

    let mut unbound = config();
    unbound.require_access_azp = false;
    let warned = warnings_building(unbound);
    assert!(warned.contains("AudiencePolicy::Unchecked"), "{warned}");
}
