// tests/unit/telemetry_probe_catalog.rs
use cli_framework::telemetry::{probe::BUILTIN_PROBES, ProbeRegistry, TelemetryLevel};

#[test]
fn the_builtin_catalog_registers_without_error() {
    let r = ProbeRegistry::with_builtins();
    assert_eq!(r.len(), BUILTIN_PROBES.len());
}

#[test]
fn the_catalog_contains_exactly_the_probes_the_spec_lists() {
    let r = ProbeRegistry::with_builtins();
    let mut ids: Vec<&str> = r.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![
            "cli.auth",
            "cli.chat",
            "cli.command",
            "cli.command.arg_values",
            "cli.command.args",
            "cli.config",
            "cli.doctor",
            "cli.feature",
            "cli.help",
            "cli.panic",
            "cli.panic.message",
            "cli.plugin",
            "cli.process",
            "cli.secrets",
            "cli.usage_error",
            "cli.usage_error.token",
            "http.client",
            "http.client.server_address",
            "http.server",
            "mcp.session",
        ]
    );
}

#[test]
fn every_child_probe_sits_at_or_above_its_parents_minimum_telemetry_level() {
    let r = ProbeRegistry::with_builtins();
    for probe in r.iter() {
        if let Some((parent_id, _)) = probe.id.rsplit_once('.') {
            if let Some(parent) = r.get(parent_id) {
                assert!(
                    probe.min_level >= parent.min_level,
                    "{} is {:?} but its parent {} is {:?}: a child can never be \
                     reachable at a telemetry level its parent is not",
                    probe.id,
                    probe.min_level,
                    parent.id,
                    parent.min_level
                );
            }
        }
    }
}

#[test]
fn the_probes_carrying_free_text_are_debug_only() {
    let r = ProbeRegistry::with_builtins();
    for id in [
        "cli.command.arg_values",
        "cli.panic.message",
        "cli.usage_error.token",
    ] {
        assert_eq!(
            r.get(id).unwrap().min_level,
            TelemetryLevel::Debug,
            "{id} carries free text and must never be reachable below debug"
        );
    }
}

#[test]
fn every_builtin_probe_documents_what_it_sends() {
    for probe in BUILTIN_PROBES {
        assert!(!probe.summary.is_empty(), "{} has no summary", probe.id);
        assert!(
            !probe.sends.is_empty(),
            "{} does not say what it sends",
            probe.id
        );
    }
}
