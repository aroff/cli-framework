use cli_framework::security::{CommandRiskPolicy, CommandRiskTier, RiskEnforcer};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn test_classify_matches_policy() {
    let mut policy = CommandRiskPolicy::default();
    policy
        .tiers
        .insert("cmd-override".to_string(), CommandRiskTier::Destructive);
    let enforcer = RiskEnforcer::new(policy.clone());

    assert_eq!(
        enforcer.classify("cmd-override", Some("config")),
        CommandRiskTier::Destructive
    );
    assert_eq!(
        enforcer.classify("x", Some("config")),
        policy.classify("x", Some("config"))
    );
    assert_eq!(enforcer.classify("x", None), policy.classify("x", None));
}

#[test]
fn test_preflight_safe_always_ok() {
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    assert!(enforcer.enforce_preflight("list", None, false, false).is_ok());
    assert!(enforcer.enforce_preflight("list", None, true, false).is_ok());
}

#[test]
fn test_preflight_sensitive_blocks_non_interactive_without_assume_yes_or_ailoop() {
    // Tests run in non-interactive mode (no TTY).
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let err = enforcer
        .enforce_preflight("config-update", Some("config"), false, false)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION"),
        "Expected SENSITIVE_COMMAND_REQUIRES_CONFIRMATION, got: {}",
        err
    );
}

#[test]
fn test_preflight_sensitive_allows_with_ailoop() {
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    assert!(
        enforcer
            .enforce_preflight("config-update", Some("config"), false, true)
            .is_ok()
    );
}

#[test]
fn test_preflight_destructive_blocks_without_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let err = enforcer
        .enforce_preflight("drop-db", Some("destructive"), true, true)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("DESTRUCTIVE_COMMAND_BLOCKED"),
        "Expected DESTRUCTIVE_COMMAND_BLOCKED, got: {}",
        err
    );
}

#[test]
fn test_preflight_destructive_blocks_non_interactive_without_ailoop_even_with_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let err = enforcer
        .enforce_preflight("drop-db", Some("destructive"), false, false)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("DESTRUCTIVE_COMMAND_BLOCKED"),
        "Expected DESTRUCTIVE_COMMAND_BLOCKED, got: {}",
        err
    );
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
}

