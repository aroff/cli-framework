use cli_framework::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use cli_framework::security::RiskEnforcer;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

// AC9: Safe tier always returns Ok regardless of assume_yes
#[test]
fn test_safe_tier_always_ok_false() {
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);
    let result = enforcer.enforce_preflight("list", None, false, false);
    assert!(result.is_ok());
}

#[test]
fn test_safe_tier_always_ok_true() {
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);
    let result = enforcer.enforce_preflight("list", None, true, false);
    assert!(result.is_ok());
}

// AC6: Destructive command blocked when ALLOW_DESTRUCTIVE_COMMANDS not set
#[test]
fn test_destructive_blocked_without_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), true, false);
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
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("rm-all", Some("destructive"), true, false);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

#[test]
fn test_destructive_blocked_assume_yes_false() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("rm-all", Some("destructive"), false, false);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

// AC8: Sensitive command blocked in non-interactive mode without assume_yes (no ailoop)
#[test]
fn test_sensitive_blocked_non_interactive_no_assume_yes() {
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("config-update", Some("config"), false, false);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION"),
        "Expected SENSITIVE_COMMAND_REQUIRES_CONFIRMATION, got: {}",
        msg
    );
}

// Sensitive command allowed when ailoop is available (non-interactive)
#[test]
fn test_sensitive_allowed_with_ailoop() {
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("config-update", Some("config"), false, true);
    assert!(
        result.is_ok(),
        "Sensitive command should be allowed when ailoop is available"
    );
}

// Destructive command allowed when ailoop + ALLOW_DESTRUCTIVE_COMMANDS=1
#[test]
fn test_destructive_allowed_with_ailoop_and_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), false, true);
    assert!(
        result.is_ok(),
        "Destructive command should be allowed when ailoop is available and env var set"
    );

    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
}

// Destructive command still blocked without ALLOW_DESTRUCTIVE_COMMANDS even with ailoop
#[test]
fn test_destructive_still_blocked_without_env_even_with_ailoop() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), false, true);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

// AC10: RiskEnforcer is callable without App/AppBuilder
#[test]
fn test_risk_enforcer_standalone() {
    let mut policy = CommandRiskPolicy::default();
    policy
        .tiers
        .insert("my-cmd".to_string(), CommandRiskTier::Safe);
    let enforcer = RiskEnforcer::new(policy);
    let result = enforcer.enforce_preflight("my-cmd", None, false, false);
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

    assert_eq!(
        policy.classify("safe-deploy", Some("deployment")),
        CommandRiskTier::Safe
    );
}

// AC7: Destructive-tier with ALLOW_DESTRUCTIVE_COMMANDS=1 returns Ok when interactive or ailoop,
// or DESTRUCTIVE_COMMAND_BLOCKED when non-interactive without ailoop (as in CI/test environments).
#[test]
fn test_ac7_destructive_allowed_with_env_and_interactive() {
    let _guard = env_lock().lock().unwrap();
    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let policy = CommandRiskPolicy::default();
    let enforcer = RiskEnforcer::new(policy);

    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), false, false);

    if cli_framework::cli_mode::is_interactive() {
        assert!(
            result.is_ok(),
            "Should return Ok when interactive and ALLOW_DESTRUCTIVE_COMMANDS=1"
        );
    } else {
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
