//! The compatibility shim: `AppBuilder::with_telemetry(TelemetryConfig)` and
//! `TelemetryConfig::from_env()`, deprecated in v0.6.0 and removed in v0.8.0
//! (spec 025).
//!
//! A deprecation is a promise about *two* releases: the call keeps working
//! now, and the caller is told what to write instead. Both halves are
//! testable and both are tested here, because the failure mode of getting
//! either wrong is silent. An app that calls the shim today exports to the
//! endpoint it configured; if the framework quietly re-reads that app as an
//! end-user install, the end-user clamp pins its telemetry level to `off`
//! and the collector simply stops receiving data — no error, no warning, and
//! nothing in the app's own code changed to explain it.
//!
//! The whole file is `#![allow(deprecated)]`: a test for a deprecated API has
//! to call it, and the crate builds with `-D warnings`.
#![allow(deprecated)]

use cli_framework::app::AppBuilder;
use cli_framework::telemetry::TelemetryConfig;
use cli_framework::Deployment;

const COLLECTOR: &str = "http://collector:4318";

/// What every existing caller of the shim wrote: an endpoint, by hand.
fn shim_config() -> TelemetryConfig {
    TelemetryConfig {
        endpoint: Some(COLLECTOR.to_string()),
        ..Default::default()
    }
}

#[test]
fn the_old_api_still_produces_a_working_configuration() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry(shim_config())
        .build_for_test();

    assert_eq!(
        app.telemetry_policy().endpoint.as_deref(),
        Some(COLLECTOR),
        "a deprecation that breaks behaviour is a removal wearing a warning: the \
         endpoint the app configured has to survive into the policy that now \
         decides what is exported"
    );
    assert!(
        app.telemetry_policy().exports(),
        "carrying the endpoint but resolving to a level that exports nothing \
         would be the same silent outage with an extra step"
    );
}

#[test]
fn the_old_api_maps_onto_a_service_deployment_because_that_is_who_used_it() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry(shim_config())
        .build_for_test();

    assert!(
        matches!(app.deployment(), Deployment::Service),
        "every existing caller configured an endpoint explicitly, which is a \
         server; mapping them to EndUser would clamp them to off and silently \
         stop telemetry that works today"
    );
}

#[test]
fn an_explicit_deployment_wins_over_the_shims_inference() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::EndUser { privacy_url: None })
        .with_telemetry(shim_config())
        .build_for_test();

    assert!(
        matches!(app.deployment(), Deployment::EndUser { .. }),
        "the inference is a fallback for apps that never declared a shape, not \
         an override of one that did"
    );
}

#[test]
fn the_inference_does_not_depend_on_the_order_the_two_calls_are_written_in() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry(shim_config())
        .with_deployment(Deployment::EndUser { privacy_url: None })
        .build_for_test();

    assert!(
        matches!(app.deployment(), Deployment::EndUser { .. }),
        "a builder whose result depends on the order two independent setters \
         are called in is a builder that will surprise somebody"
    );
}

#[test]
fn the_new_api_wins_when_an_app_configures_both() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_defaults(cli_framework::TelemetryDefaults {
            endpoint: Some("http://new-api:4318".to_string()),
            ..Default::default()
        })
        .with_telemetry(shim_config())
        .build_for_test();

    assert_eq!(
        app.telemetry_policy().endpoint.as_deref(),
        Some("http://new-api:4318"),
        "the shim is the fallback during a migration: an app part-way through \
         one has both calls in its source, and the call it is migrating *to* \
         is the one it means"
    );
}

#[test]
fn the_deprecation_notes_name_the_replacement_and_the_removal() {
    // Read as source rather than asserted through the compiler because
    // `#[deprecated]` has no runtime footprint: nothing an app can call tells
    // it which release removes the shim. The check is that the two
    // attributes are attached to the two functions and say what to do next --
    // a bare `#[deprecated]` is a warning people turn off.
    for (label, source, signature, replacement) in [
        (
            "AppBuilder::with_telemetry",
            include_str!("../../src/app/builder.rs"),
            "pub fn with_telemetry(mut self, config: crate::telemetry::TelemetryConfig)",
            "with_telemetry_defaults",
        ),
        (
            "TelemetryConfig::from_env",
            include_str!("../../src/telemetry/config.rs"),
            "pub fn from_env() -> Self",
            "with_telemetry_defaults",
        ),
    ] {
        let at = source
            .find(signature)
            .unwrap_or_else(|| panic!("{label} is declared as `{signature}`"));
        let preceding = &source[at.saturating_sub(400)..at];
        assert!(
            preceding.contains("#[deprecated("),
            "{label} carries no deprecation attribute"
        );
        assert!(
            preceding.contains(r#"since = "0.6.0""#),
            "{label}'s deprecation does not say which release deprecated it"
        );
        assert!(
            preceding.contains(replacement),
            "{label}'s deprecation note must name `{replacement}` -- a note that \
             does not say what to write instead is a note people ignore"
        );
        assert!(
            preceding.contains("0.8.0"),
            "{label}'s deprecation note must name the release that removes it, so \
             a downstream app can plan the migration instead of discovering it"
        );
    }
}
