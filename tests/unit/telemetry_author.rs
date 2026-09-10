// tests/unit/telemetry_author.rs
//
//! What an author declares about telemetry, and the one promise the
//! declaration makes about secrets.
//!
//! `src/telemetry/author.rs` is all data -- three declarations with no
//! function bodies -- which is exactly why it needs its own tests. Nothing
//! else in the suite formats a `TelemetryDefaults`, compares two
//! `Identity` values or calls an `IdentityResolver`, so the derived impls
//! were never instantiated and the file produced no coverage records at all.
//! A derive that is never called is a derive nobody has checked.
//!
//! The header assertion is the one that matters. `TelemetryDefaults::headers`
//! is usually a bearer token, and the type's own documentation promises its
//! `Debug` renders as `Secret([REDACTED ..])`. That promise is load-bearing:
//! the builder holds a `TelemetryDefaults` for the life of the process and a
//! single `tracing::debug!(?defaults)` anywhere -- ours or an app author's --
//! would otherwise put a collector credential in a log file.

use cli_framework::app::builder::TestContext;
use cli_framework::telemetry::{Identity, IdentityResolver, TelemetryDefaults};
use secrecy::SecretString;
use std::sync::Arc;

const TOKEN: &str = "authorization=Bearer sk-live-do-not-print-me";

#[test]
fn debug_of_telemetry_defaults_prints_no_header_value() {
    let defaults = TelemetryDefaults {
        endpoint: Some("http://collector:4318".to_string()),
        headers: Some(SecretString::from(TOKEN.to_string())),
        arg_value_allowlist: vec!["format".to_string()],
        sample_ratio: Some(0.25),
    };

    let rendered = format!("{defaults:?}");

    assert!(
        !rendered.contains("sk-live-do-not-print-me"),
        "`TelemetryDefaults` printed its OTLP headers in full. The builder \
         holds one of these for the whole process, so any `?defaults` in a \
         log line now writes a collector credential to disk. Rendered: \
         {rendered}"
    );
    assert!(
        rendered.contains("REDACTED"),
        "the headers field vanished from the `Debug` output instead of being \
         redacted -- a reader cannot tell a configured token from an absent \
         one, which is the diagnostic the field exists for. Rendered: \
         {rendered}"
    );
    // The non-secret fields are diagnostics and must still be legible.
    assert!(
        rendered.contains("http://collector:4318"),
        "the endpoint is not a secret and must stay readable: {rendered}"
    );
    assert!(
        rendered.contains("0.25"),
        "the sample ratio is not a secret and must stay readable: {rendered}"
    );
}

#[test]
fn the_default_declaration_asks_for_nothing() {
    let defaults = TelemetryDefaults::default();

    assert!(defaults.endpoint.is_none(), "no endpoint by default");
    assert!(defaults.headers.is_none(), "no headers by default");
    assert!(
        defaults.arg_value_allowlist.is_empty(),
        "no argument value is allowlisted until an author names it -- an \
         empty allowlist is what keeps `cli.command.arg_values` silent"
    );
    assert!(
        defaults.sample_ratio.is_none(),
        "no head sampling by default; `resolve_policy` reads `None` as \
         sample everything"
    );

    // `TelemetryDefaults` is cloned into the resolver inputs on every build.
    let cloned = defaults.clone();
    assert_eq!(cloned.endpoint, defaults.endpoint);
    assert_eq!(cloned.arg_value_allowlist, defaults.arg_value_allowlist);
    assert_eq!(cloned.sample_ratio, defaults.sample_ratio);
}

#[test]
fn identity_compares_by_value_so_a_resolver_can_be_asserted_on() {
    let anonymous = Identity::default();
    assert_eq!(
        anonymous,
        Identity {
            enduser_id: None,
            tenant: None,
        },
        "the default identity knows nobody; neither field is ever inferred"
    );

    let person = Identity {
        enduser_id: Some("u-1".to_string()),
        tenant: Some("acme".to_string()),
    };
    assert_eq!(person, person.clone());
    assert_ne!(person, anonymous);
    assert_ne!(
        person,
        Identity {
            enduser_id: Some("u-1".to_string()),
            tenant: None,
        },
        "tenant is part of the identity, not decoration -- two callers with \
         the same person id in different tenants are different identities"
    );

    let rendered = format!("{person:?}");
    assert!(
        rendered.contains("u-1") && rendered.contains("acme"),
        "an `Identity` is not a secret -- it is what the app chose to attach \
         -- so `Debug` must show it: {rendered}"
    );
}

#[test]
fn an_identity_resolver_is_a_closure_the_framework_can_call_through_a_context() {
    // The alias is the whole contract: whatever an app writes has to be
    // callable by the framework holding only a `&dyn AppContext`.
    let resolver: IdentityResolver = Arc::new(|_ctx: &dyn cli_framework::app::AppContext| {
        Some(Identity {
            enduser_id: Some("resolved".to_string()),
            tenant: None,
        })
    });

    let ctx = TestContext;
    let resolved = resolver(&ctx);

    assert_eq!(
        resolved,
        Some(Identity {
            enduser_id: Some("resolved".to_string()),
            tenant: None,
        })
    );

    // An app that never authenticates answers `None` and pays nothing.
    let silent: IdentityResolver = Arc::new(|_ctx: &dyn cli_framework::app::AppContext| None);
    assert_eq!(silent(&ctx), None);
}
