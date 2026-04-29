use cli_framework::command::{enforce_risk_gate, CommandArgs};
use cli_framework::llm::CommandResolution;
use cli_framework::security::command_risk::{CommandRiskPolicy, CommandRiskTier};

fn make_resolution(command_id: &str) -> CommandResolution {
    CommandResolution {
        command_id: command_id.to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    }
}

// AC9: Safe tier always returns Ok regardless of assume_yes
#[test]
fn test_safe_tier_always_ok_false() {
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("list");
    let result = enforce_risk_gate(&policy, &resolution, None, false);
    assert!(result.is_ok());
}

#[test]
fn test_safe_tier_always_ok_true() {
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("list");
    let result = enforce_risk_gate(&policy, &resolution, None, true);
    assert!(result.is_ok());
}

// AC6: Destructive command blocked when ALLOW_DESTRUCTIVE_COMMANDS not set
#[test]
fn test_destructive_blocked_without_env() {
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("drop-db");

    let result = enforce_risk_gate(
        &policy,
        &resolution,
        Some("destructive"),
        true, // assume_yes is ignored for destructive
    );
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("DESTRUCTIVE_COMMAND_BLOCKED"),
        "Expected DESTRUCTIVE_COMMAND_BLOCKED, got: {}",
        msg
    );
}

// AC6: Destructive command blocked regardless of assume_yes
#[test]
fn test_destructive_blocked_assume_yes_true() {
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("rm-all");

    let result = enforce_risk_gate(&policy, &resolution, Some("destructive"), true);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

#[test]
fn test_destructive_blocked_assume_yes_false() {
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("rm-all");

    let result = enforce_risk_gate(&policy, &resolution, Some("destructive"), false);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

// AC8: Sensitive command blocked in non-interactive mode without assume_yes
#[test]
fn test_sensitive_blocked_non_interactive_no_assume_yes() {
    // Tests run in non-interactive mode (no TTY)
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("config-update");

    let result = enforce_risk_gate(&policy, &resolution, Some("config"), false);
    // In test env (non-interactive), should be blocked
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION"),
        "Expected SENSITIVE_COMMAND_REQUIRES_CONFIRMATION, got: {}",
        msg
    );
}

// AC10: enforce_risk_gate is callable without App/AppBuilder
#[test]
fn test_enforce_risk_gate_standalone() {
    let mut policy = CommandRiskPolicy::default();
    policy
        .tiers
        .insert("my-cmd".to_string(), CommandRiskTier::Safe);
    let resolution = make_resolution("my-cmd");
    let result = enforce_risk_gate(&policy, &resolution, None, false);
    assert!(result.is_ok());
}

// Test classify method directly
#[test]
fn test_classify_by_category_deployment() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(
        policy.classify("x", Some("deployment")),
        CommandRiskTier::Destructive
    );
}

#[test]
fn test_classify_by_category_admin() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(
        policy.classify("x", Some("admin")),
        CommandRiskTier::Destructive
    );
}

#[test]
fn test_classify_by_category_destructive() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(
        policy.classify("x", Some("destructive")),
        CommandRiskTier::Destructive
    );
}

#[test]
fn test_classify_by_category_data() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(
        policy.classify("x", Some("data")),
        CommandRiskTier::Sensitive
    );
}

#[test]
fn test_classify_by_category_config() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(
        policy.classify("x", Some("config")),
        CommandRiskTier::Sensitive
    );
}

#[test]
fn test_classify_no_category_defaults_safe() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(policy.classify("x", None), CommandRiskTier::Safe);
}

#[test]
fn test_classify_unknown_category_defaults_safe() {
    let policy = CommandRiskPolicy::default();
    assert_eq!(policy.classify("x", Some("ai")), CommandRiskTier::Safe);
}

// Test per-command override wins over category
#[test]
fn test_per_command_override_wins() {
    let mut policy = CommandRiskPolicy::default();
    policy
        .tiers
        .insert("safe-deploy".to_string(), CommandRiskTier::Safe);

    // Per-command override wins over category
    assert_eq!(
        policy.classify("safe-deploy", Some("deployment")),
        CommandRiskTier::Safe
    );
}

// AC7: Destructive-tier with ALLOW_DESTRUCTIVE_COMMANDS=1 returns Ok when interactive,
// or DESTRUCTIVE_COMMAND_BLOCKED when non-interactive (as in CI/test environments).
#[test]
fn test_ac7_destructive_allowed_with_env_and_interactive() {
    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let policy = CommandRiskPolicy::default();
    let resolution = make_resolution("drop-db");

    let result = enforce_risk_gate(&policy, &resolution, Some("destructive"), false);

    if cli_framework::cli_mode::is_interactive() {
        // Interactive terminal + ALLOW_DESTRUCTIVE_COMMANDS=1 → gate passes
        assert!(
            result.is_ok(),
            "Should return Ok when interactive and ALLOW_DESTRUCTIVE_COMMANDS=1"
        );
    } else {
        // Non-interactive (CI/test) → still blocked even with the env var set
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("DESTRUCTIVE_COMMAND_BLOCKED"),
            "Expected DESTRUCTIVE_COMMAND_BLOCKED in non-interactive mode, got: {}",
            msg
        );
    }

    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
}
