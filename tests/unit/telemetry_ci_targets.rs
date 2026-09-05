//! Guard: every telemetry `[[test]]` target declared in `Cargo.toml` must
//! actually execute in the `Test (telemetry)` step of
//! `.github/workflows/ci.yml`.
//!
//! Why this exists. `cargo test --test <name>` takes *exact* target names and
//! silently runs nothing for a target it was not given; a target whose
//! `required-features` are not all enabled is skipped just as silently. Either
//! way the target is compiled, never executed, and the job still reports a
//! green build — so the required check that is supposed to gate the merge
//! passes without ever having run the test.
//!
//! That is not hypothetical. When this guard was written the step named two
//! targets while `Cargo.toml` declared twenty, so eighteen telemetry suites had
//! never executed in CI even once — including every target added by the
//! telemetry work itself, whose pull requests had merged green.
//!
//! The fix is to remove the hand-maintained list rather than police it: the
//! step enables the feature union and lets cargo select the targets, which it
//! does from `required-features` and therefore cannot get wrong. The two tests
//! below pin the two ways that could regress — someone re-scoping the step
//! with `--test`, and someone adding a target that needs a feature the step
//! does not enable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `[[test]]` target whose `required-features` include `telemetry`,
/// mapped to the full feature set it needs.
fn telemetry_targets() -> BTreeMap<String, BTreeSet<String>> {
    let manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("Cargo.toml parses");
    let targets = manifest
        .get("test")
        .and_then(toml::Value::as_array)
        .expect("Cargo.toml declares [[test]] targets");

    let mut out = BTreeMap::new();
    for target in targets {
        let name = target
            .get("name")
            .and_then(toml::Value::as_str)
            .expect("every [[test]] entry has a name");
        let features: BTreeSet<String> = target
            .get("required-features")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if features.contains("telemetry") {
            out.insert(name.to_owned(), features);
        }
    }
    out
}

/// The `Test (telemetry)` step's command, as (`--test` names, `--features`).
///
/// Parsed from the workflow text rather than a YAML value: the step body is a
/// folded scalar, so the shape this cares about — which flags the shell
/// actually receives — is the same either way, and text keeps the test free of
/// a YAML dependency it would otherwise need only here.
fn telemetry_ci_step() -> (BTreeSet<String>, BTreeSet<String>) {
    let ci = read(".github/workflows/ci.yml");
    let start = ci
        .find("- name: Test (telemetry)")
        .expect("ci.yml has a `Test (telemetry)` step");
    // The step ends at the next step at the same indentation.
    let rest = &ci[start + 1..];
    let end = rest
        .find("\n      - name: ")
        .map_or(ci.len(), |i| start + 1 + i);
    let step = &ci[start..end];

    let mut tests = BTreeSet::new();
    let mut features = BTreeSet::new();
    let tokens: Vec<&str> = step.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            "--test" => {
                tests.insert(tokens[i + 1].to_owned());
            }
            "--features" => {
                for f in tokens[i + 1].trim_matches('"').split(',') {
                    features.insert(f.trim().to_owned());
                }
            }
            _ => {}
        }
    }
    (tests, features)
}

/// Anti-vacuity. Both halves of this guard are "no unexpected difference"
/// assertions, which a parser that silently returns nothing would satisfy
/// forever. This is the test that fails when the parsing breaks.
#[test]
fn the_manifest_and_the_workflow_both_parse() {
    let targets = telemetry_targets();
    assert!(
        targets.len() >= 10,
        "expected the telemetry suites to be found in Cargo.toml, got {}: the \
         parser is broken, and the guards below are passing vacuously",
        targets.len()
    );

    let (_, features) = telemetry_ci_step();
    assert!(
        features.contains("telemetry"),
        "the `Test (telemetry)` step was found but its --features did not parse: {features:?}"
    );
}

#[test]
fn the_telemetry_step_lets_cargo_select_the_targets() {
    let (tests, _) = telemetry_ci_step();
    assert!(
        tests.is_empty(),
        "the `Test (telemetry)` step must not name targets with --test: cargo \
         already selects them from required-features, and a hand-maintained \
         list silently stops running whatever it forgets. It named: {tests:?}"
    );
}

#[test]
fn the_telemetry_step_enables_every_feature_its_targets_require() {
    let targets = telemetry_targets();
    let (_, enabled) = telemetry_ci_step();

    let mut missing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, required) in &targets {
        let gap: BTreeSet<String> = required.difference(&enabled).cloned().collect();
        if !gap.is_empty() {
            missing.insert(name.clone(), gap);
        }
    }

    assert!(
        missing.is_empty(),
        "these telemetry targets need features the `Test (telemetry)` step does \
         not enable, so cargo skips them without a word and the job still goes \
         green: {missing:?}"
    );
}
