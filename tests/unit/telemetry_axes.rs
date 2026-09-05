// tests/unit/telemetry_axes.rs
use cli_framework::telemetry::{Attribution, Deployment, TelemetryLevel};

#[test]
fn telemetry_levels_order_off_below_usage_below_diagnostic_below_debug() {
    assert!(TelemetryLevel::Off < TelemetryLevel::Usage);
    assert!(TelemetryLevel::Usage < TelemetryLevel::Diagnostic);
    assert!(TelemetryLevel::Diagnostic < TelemetryLevel::Debug);
    assert_eq!(TelemetryLevel::default(), TelemetryLevel::Off);
}

#[test]
fn telemetry_level_round_trips_through_its_wire_name() {
    for level in [
        TelemetryLevel::Off,
        TelemetryLevel::Usage,
        TelemetryLevel::Diagnostic,
        TelemetryLevel::Debug,
    ] {
        let text = level.as_str();
        assert_eq!(text.parse::<TelemetryLevel>().unwrap(), level);
        assert_eq!(level.to_string(), text);
    }
}

#[test]
fn an_unknown_telemetry_level_names_the_offending_value() {
    let err = "verbose".parse::<TelemetryLevel>().unwrap_err();
    assert!(err.to_string().contains("verbose"), "got: {err}");
    assert!(
        err.to_string().contains("debug"),
        "must list the valid values: {err}"
    );
}

#[test]
fn deployment_defaults_to_end_user_with_no_privacy_url() {
    let d = Deployment::default();
    assert!(d.is_end_user());
    assert_eq!(d.privacy_url(), None);
    assert_eq!(d.as_str(), "end_user");
}

#[test]
fn a_service_deployment_never_has_a_privacy_url() {
    assert_eq!(Deployment::Service.privacy_url(), None);
    assert!(!Deployment::Service.is_end_user());
    assert_eq!(Deployment::Service.as_str(), "service");
}

#[test]
fn attribution_defaults_to_pseudonymous_and_round_trips() {
    assert_eq!(Attribution::default(), Attribution::Pseudonymous);
    for a in [
        Attribution::Anonymous,
        Attribution::Pseudonymous,
        Attribution::Identified,
    ] {
        assert_eq!(a.as_str().parse::<Attribution>().unwrap(), a);
    }
    assert!("nobody".parse::<Attribution>().is_err());
}

#[test]
fn axes_serialize_as_their_lowercase_wire_names() {
    assert_eq!(
        serde_json::to_string(&TelemetryLevel::Diagnostic).unwrap(),
        "\"diagnostic\""
    );
    assert_eq!(
        serde_json::to_string(&Attribution::Pseudonymous).unwrap(),
        "\"pseudonymous\""
    );
}

#[test]
fn attribution_all_round_trips_through_as_str_and_from_str() {
    assert_eq!(
        Attribution::ALL,
        [
            Attribution::Anonymous,
            Attribution::Pseudonymous,
            Attribution::Identified
        ]
    );
    for a in Attribution::ALL {
        assert_eq!(a.as_str().parse::<Attribution>().unwrap(), a);
    }
}
