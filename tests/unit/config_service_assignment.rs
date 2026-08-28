//! Assignment-rule evaluation, exercised through the public
//! `cli_framework::config::service` surface (the module-internal
//! `#[cfg(test)]` suite in `src/config/service/assignment.rs` covers the
//! pure-function rules exhaustively; this is the house-convention external
//! target pinning the same safety-critical properties from outside the
//! crate, the way every caller — including the router — actually reaches
//! this code).

use cli_framework::config::service::{resolve_profile, AssignmentRule, RuleOperator};
use serde_json::json;

fn rule(
    ord: i64,
    operator: RuleOperator,
    claim_path: &str,
    value: Option<serde_json::Value>,
    profile: &str,
) -> AssignmentRule {
    AssignmentRule {
        app: "myapp".to_string(),
        ord,
        claim_path: claim_path.to_string(),
        operator,
        value,
        profile: profile.to_string(),
    }
}

#[test]
fn ordering_is_load_bearing_reordering_rules_changes_which_profile_is_selected() {
    let claims = json!({"realm_access": {"roles": ["developers", "kiosk"]}});

    let rules_a = vec![
        rule(
            0,
            RuleOperator::Contains,
            "realm_access.roles",
            Some(json!("developers")),
            "developers",
        ),
        rule(
            1,
            RuleOperator::Contains,
            "realm_access.roles",
            Some(json!("kiosk")),
            "kiosk",
        ),
    ];
    assert_eq!(
        resolve_profile(&rules_a, &claims).unwrap().profile(),
        "developers"
    );

    let rules_b = vec![
        rule(
            1,
            RuleOperator::Contains,
            "realm_access.roles",
            Some(json!("developers")),
            "developers",
        ),
        rule(
            0,
            RuleOperator::Contains,
            "realm_access.roles",
            Some(json!("kiosk")),
            "kiosk",
        ),
    ];
    assert_eq!(
        resolve_profile(&rules_b, &claims).unwrap().profile(),
        "kiosk"
    );
}

#[test]
fn identity_in_several_groups_gets_a_deterministic_profile_regardless_of_claim_array_order() {
    let claims_a = json!({"groups": ["kiosk", "developers"]});
    let claims_b = json!({"groups": ["developers", "kiosk"]});
    let rules = vec![
        rule(
            0,
            RuleOperator::Contains,
            "groups",
            Some(json!("developers")),
            "developers",
        ),
        rule(
            1,
            RuleOperator::Contains,
            "groups",
            Some(json!("kiosk")),
            "kiosk",
        ),
    ];
    assert_eq!(
        resolve_profile(&rules, &claims_a).unwrap().profile(),
        "developers"
    );
    assert_eq!(
        resolve_profile(&rules, &claims_b).unwrap().profile(),
        "developers"
    );
}

#[test]
fn no_match_and_no_default_is_unmanaged() {
    let rules = vec![rule(
        0,
        RuleOperator::Equals,
        "team",
        Some(json!("ops")),
        "ops-profile",
    )];
    assert!(resolve_profile(&rules, &json!({"team": "eng"})).is_none());
}

#[test]
fn default_profile_governs_everyone_else() {
    let rules = vec![
        rule(
            0,
            RuleOperator::Equals,
            "team",
            Some(json!("ops")),
            "ops-profile",
        ),
        rule(1, RuleOperator::Default, "", None, "baseline"),
    ];
    assert_eq!(
        resolve_profile(&rules, &json!({"team": "eng"}))
            .unwrap()
            .profile(),
        "baseline"
    );
    assert_eq!(
        resolve_profile(&rules, &json!({"team": "ops"}))
            .unwrap()
            .profile(),
        "ops-profile"
    );
}

#[test]
fn equals_contains_exists_do_not_cross_match_each_others_shapes() {
    let scalar_claims = json!({"team": "developers"});
    let array_claims = json!({"team": ["developers"]});

    let equals_rule = rule(
        0,
        RuleOperator::Equals,
        "team",
        Some(json!("developers")),
        "p",
    );
    let contains_rule = rule(
        0,
        RuleOperator::Contains,
        "team",
        Some(json!("developers")),
        "p",
    );

    assert!(resolve_profile(&[equals_rule.clone()], &scalar_claims).is_some());
    assert!(resolve_profile(&[equals_rule], &array_claims).is_none());

    assert!(resolve_profile(&[contains_rule.clone()], &array_claims).is_some());
    assert!(resolve_profile(&[contains_rule], &scalar_claims).is_none());
}

#[test]
fn a_claim_missing_from_the_token_never_matches_and_never_panics() {
    let claims = json!({"sub": "u1"});
    for operator in [
        RuleOperator::Equals,
        RuleOperator::Contains,
        RuleOperator::Exists,
    ] {
        let r = rule(0, operator, "realm_access.roles", Some(json!("x")), "p");
        assert!(resolve_profile(&[r], &claims).is_none());
    }
}
