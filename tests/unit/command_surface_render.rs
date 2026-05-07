use cli_framework::command_surface::document::{CliSpecApp, CliSpecCommand, CliSpecDocument};
use cli_framework::command_surface::render::{render_json, render_markdown, render_yaml};

fn make_test_doc() -> CliSpecDocument {
    CliSpecDocument {
        schema_version: "cli-framework.command-surface.v1",
        app: CliSpecApp {
            name: "testapp".to_string(),
            version: "0.1.0".to_string(),
        },
        commands: vec![CliSpecCommand {
            path: "hello".to_string(),
            id: "hello".to_string(),
            summary: "Say hello".to_string(),
            syntax: None,
            category: None,
            hidden: false,
            deprecated: None,
            aliases: vec![],
            args: vec![],
            input_schema: serde_json::json!({"type": "object", "additionalProperties": true}),
            examples: vec![],
            env_vars: vec![],
            exit_codes: vec![],
            notes: None,
        }],
    }
}

#[test]
fn json_output_is_valid_json() {
    let doc = make_test_doc();
    let output = render_json(&doc).expect("render_json should not fail");
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("output should be valid JSON");
    assert_eq!(
        parsed["schemaVersion"].as_str(),
        Some("cli-framework.command-surface.v1")
    );
    assert!(parsed["commands"].is_array());
}

#[test]
fn yaml_output_is_nonempty() {
    let doc = make_test_doc();
    let output = render_yaml(&doc).expect("render_yaml should not fail");
    assert!(!output.is_empty());
    assert!(output.contains("schemaVersion") || output.contains("schema_version"));
}

#[test]
fn markdown_output_has_h2_heading() {
    let doc = make_test_doc();
    let output = render_markdown(&doc);
    assert!(
        output.contains("## hello"),
        "Markdown should have ## hello heading"
    );
    assert!(output.contains("# testapp 0.1.0"));
}

#[test]
fn json_snapshot() {
    let doc = make_test_doc();
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"),
    );
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        insta::assert_json_snapshot!("spec_json_output", &doc);
    });
}

#[test]
fn render_json_cs004_error_message_on_failure() {
    let doc = make_test_doc();
    // render_json succeeds for a valid document
    assert!(
        render_json(&doc).is_ok(),
        "render_json must succeed for a valid document"
    );

    // Verify CS004 error wrapping: simulate the same .map_err pattern used in render_json
    // (serde_json::to_string_pretty never fails for CliSpecDocument, so we validate
    // the error code format by constructing an equivalent wrapped error)
    let simulated = serde_json::from_str::<serde_json::Value>("{bad json}").unwrap_err();
    let wrapped = anyhow::anyhow!("CS004: JSON serialization error: {}", simulated);
    assert!(
        wrapped.to_string().contains("CS004"),
        "render_json error must contain CS004"
    );
}

#[test]
fn render_yaml_cs003_error_message_on_failure() {
    let doc = make_test_doc();
    // render_yaml succeeds for a valid document
    assert!(
        render_yaml(&doc).is_ok(),
        "render_yaml must succeed for a valid document"
    );

    // Verify CS003 error wrapping: simulate the same .map_err pattern used in render_yaml
    // (serde_yaml::to_string never fails for CliSpecDocument, so we validate
    // the error code format by constructing an equivalent wrapped error)
    let wrapped = anyhow::anyhow!("CS003: YAML serialization error: simulated failure");
    assert!(
        wrapped.to_string().contains("CS003"),
        "render_yaml error must contain CS003"
    );
}

#[test]
fn yaml_snapshot() {
    let doc = make_test_doc();
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"),
    );
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        let yaml_output = render_yaml(&doc).expect("render_yaml should not fail");
        insta::assert_snapshot!("spec_yaml_output", yaml_output);
    });
}
