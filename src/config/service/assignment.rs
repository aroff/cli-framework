//! Assignment-rule evaluation (spec 022, "Assignment rule shape" /
//! "Profile resolution"): map the caller's validated claims onto exactly one
//! profile, or none.
//!
//! Pure functions over `&[AssignmentRule]` + `&serde_json::Value` — no
//! storage, no networking — so both `/v1/policy/{app}` and the
//! `/v1/resolve/{app}` diagnostic call the exact same code and can never
//! disagree about which profile an identity resolves to.

use super::types::{AssignmentRule, RuleOperator};
use serde_json::Value;

/// The outcome of [`resolve_profile`]: which rule matched (by shared
/// reference, so a caller can report its `ord`/`claim_path`/`operator` for
/// the `/v1/resolve/{app}` diagnostic) and which profile it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedAssignment<'a> {
    pub rule: &'a AssignmentRule,
}

impl<'a> ResolvedAssignment<'a> {
    pub fn profile(&self) -> &'a str {
        &self.rule.profile
    }
}

/// Evaluate `rules` (evaluated in ascending `ord` order regardless of the
/// order they're passed in — see the module docs on why the caller should
/// not need to pre-sort) against `claims`, returning the first match.
/// `None` means "no rule matched, and no default rule was present" — the
/// caller's cue to treat the application as unmanaged for this identity.
pub fn resolve_profile<'a>(
    rules: &'a [AssignmentRule],
    claims: &Value,
) -> Option<ResolvedAssignment<'a>> {
    let mut sorted: Vec<&AssignmentRule> = rules.iter().collect();
    sorted.sort_by_key(|r| r.ord);
    sorted
        .into_iter()
        .find(|rule| rule_matches(rule, claims))
        .map(|rule| ResolvedAssignment { rule })
}

/// Whether `rule` matches `claims`. A claim path that does not resolve on
/// the token means the rule does **not** match — never an error, since
/// tokens legitimately vary in which claims they carry (spec 022).
///
/// `pub(crate)` (spec 023): the administrative-role gate
/// ([`super::identity::require_admin_role`]) reuses this exact function to
/// evaluate the configured admin rule against a caller's claims — "the
/// identical `{claim_path, operator, value}` shape" spec 023 requires,
/// rather than a second, potentially-drifting copy of rule evaluation.
pub(crate) fn rule_matches(rule: &AssignmentRule, claims: &Value) -> bool {
    match rule.operator {
        RuleOperator::Default => true,
        RuleOperator::Exists => claim_at_path(claims, &rule.claim_path).is_some(),
        RuleOperator::Equals => match claim_at_path(claims, &rule.claim_path) {
            Some(actual) => rule
                .value
                .as_ref()
                .is_some_and(|expected| actual == expected),
            None => false,
        },
        RuleOperator::Contains => match claim_at_path(claims, &rule.claim_path) {
            Some(Value::Array(items)) => rule
                .value
                .as_ref()
                .is_some_and(|expected| items.contains(expected)),
            _ => false,
        },
    }
}

/// Resolve a dot-path (e.g. `realm_access.roles`) into `claims`, descending
/// through nested JSON objects only. Returns `None` as soon as any segment
/// is missing or the current value isn't an object to descend into — this
/// is what makes a claim path "resolves to nothing" rather than an error,
/// exactly as spec 022 requires.
fn claim_at_path<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = claims;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(
        ord: i64,
        op: RuleOperator,
        path: &str,
        value: Option<Value>,
        profile: &str,
    ) -> AssignmentRule {
        AssignmentRule {
            app: "myapp".to_string(),
            ord,
            claim_path: path.to_string(),
            operator: op,
            value,
            profile: profile.to_string(),
        }
    }

    #[test]
    fn claim_at_path_descends_nested_objects() {
        let claims = json!({"realm_access": {"roles": ["a", "b"]}});
        assert_eq!(
            claim_at_path(&claims, "realm_access.roles"),
            Some(&json!(["a", "b"]))
        );
    }

    #[test]
    fn claim_at_path_missing_segment_is_none_not_error() {
        let claims = json!({"realm_access": {}});
        assert_eq!(claim_at_path(&claims, "realm_access.roles"), None);
        assert_eq!(claim_at_path(&claims, "nonexistent.deep.path"), None);
    }

    #[test]
    fn claim_at_path_stops_descending_into_a_non_object() {
        let claims = json!({"sub": "u1"});
        assert_eq!(claim_at_path(&claims, "sub.nested"), None);
    }

    #[test]
    fn equals_matches_only_exact_scalar() {
        let claims = json!({"team": "platform"});
        let r = rule(
            0,
            RuleOperator::Equals,
            "team",
            Some(json!("platform")),
            "p",
        );
        assert!(rule_matches(&r, &claims));

        let r2 = rule(0, RuleOperator::Equals, "team", Some(json!("other")), "p");
        assert!(!rule_matches(&r2, &claims));
    }

    #[test]
    fn equals_does_not_match_an_array_claim_even_with_a_matching_element() {
        let claims = json!({"roles": ["platform"]});
        let r = rule(
            0,
            RuleOperator::Equals,
            "roles",
            Some(json!("platform")),
            "p",
        );
        assert!(!rule_matches(&r, &claims));
    }

    #[test]
    fn contains_matches_array_element_but_not_a_scalar_of_the_same_value() {
        let array_claims = json!({"roles": ["developers", "on-call"]});
        let r = rule(
            0,
            RuleOperator::Contains,
            "roles",
            Some(json!("developers")),
            "p",
        );
        assert!(rule_matches(&r, &array_claims));

        let scalar_claims = json!({"roles": "developers"});
        assert!(!rule_matches(&r, &scalar_claims));
    }

    #[test]
    fn exists_matches_regardless_of_value_and_ignores_configured_value() {
        let claims = json!({"department": "engineering"});
        let r = rule(0, RuleOperator::Exists, "department", None, "p");
        assert!(rule_matches(&r, &claims));

        let r_with_value = rule(
            0,
            RuleOperator::Exists,
            "department",
            Some(json!("ignored")),
            "p",
        );
        assert!(rule_matches(&r_with_value, &claims));
    }

    #[test]
    fn exists_does_not_match_an_absent_claim() {
        let claims = json!({});
        let r = rule(0, RuleOperator::Exists, "department", None, "p");
        assert!(!rule_matches(&r, &claims));
    }

    #[test]
    fn claim_path_absent_from_token_never_errors_just_does_not_match() {
        // A token deliberately missing the claim the rule inspects.
        let claims = json!({"sub": "u1"});
        for op in [
            RuleOperator::Equals,
            RuleOperator::Contains,
            RuleOperator::Exists,
        ] {
            let r = rule(0, op, "realm_access.roles", Some(json!("x")), "p");
            assert!(
                !rule_matches(&r, &claims),
                "{op:?} must not match, and must not panic"
            );
        }
    }

    #[test]
    fn default_operator_always_matches() {
        let r = rule(99, RuleOperator::Default, "", None, "fallback");
        assert!(rule_matches(&r, &json!({})));
    }

    #[test]
    fn first_matching_rule_by_ord_wins_reordering_changes_the_outcome() {
        let claims = json!({"realm_access": {"roles": ["developers", "kiosk"]}});
        let rules = vec![
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
        let resolved = resolve_profile(&rules, &claims).unwrap();
        assert_eq!(resolved.profile(), "developers");

        // Reordering (swapping `ord`) changes which profile is selected —
        // this is the anti-vacuity pin spec 022 requires: ordering is
        // load-bearing, not incidental.
        let reordered = vec![
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
        let resolved2 = resolve_profile(&reordered, &claims).unwrap();
        assert_eq!(resolved2.profile(), "kiosk");
    }

    #[test]
    fn resolve_profile_sorts_by_ord_regardless_of_input_order() {
        let claims = json!({"team": "b"});
        // Passed in reverse `ord` order on purpose.
        let rules = vec![
            rule(
                5,
                RuleOperator::Equals,
                "team",
                Some(json!("b")),
                "second-rule-wins-if-unsorted",
            ),
            rule(
                0,
                RuleOperator::Equals,
                "team",
                Some(json!("b")),
                "first-rule-by-ord",
            ),
        ];
        let resolved = resolve_profile(&rules, &claims).unwrap();
        assert_eq!(resolved.profile(), "first-rule-by-ord");
    }

    #[test]
    fn no_matching_rule_and_no_default_is_unmanaged() {
        let claims = json!({"team": "unmatched"});
        let rules = vec![rule(
            0,
            RuleOperator::Equals,
            "team",
            Some(json!("other")),
            "p",
        )];
        assert!(resolve_profile(&rules, &claims).is_none());
    }

    #[test]
    fn default_rule_governs_identities_matching_no_other_rule() {
        let claims = json!({"team": "unmatched"});
        let rules = vec![
            rule(
                0,
                RuleOperator::Equals,
                "team",
                Some(json!("other")),
                "specific",
            ),
            rule(1, RuleOperator::Default, "", None, "fallback"),
        ];
        let resolved = resolve_profile(&rules, &claims).unwrap();
        assert_eq!(resolved.profile(), "fallback");
    }

    #[test]
    fn empty_rules_is_unmanaged() {
        assert!(resolve_profile(&[], &json!({})).is_none());
    }

    /// Bug 4, demonstrated at the pure-function level (bypassing
    /// `validate_all` entirely, exactly as that bug's test plan calls for):
    /// a `Default` row that is **not** the last-ordered rule silently
    /// preempts a specific rule ordered after it, because
    /// `RuleOperator::Default` matches unconditionally and evaluation is
    /// first-match-wins by ascending `ord` (`resolve_profile`). This is
    /// precisely the gap `super::validate::validate_all`'s
    /// `DefaultRuleNotLast` check (bug 4's fix) now rejects at startup —
    /// this test proves what that validation prevents, by constructing the
    /// bad ordering directly and showing it resolves to the wrong profile.
    #[test]
    fn a_default_rule_ordered_before_a_specific_rule_wrongly_preempts_it() {
        let claims = json!({"team": "platform"});
        let rules = vec![
            // The default is ordered *first* -- a data error, or an admin
            // mistake once spec 023's write API exists.
            rule(0, RuleOperator::Default, "", None, "fallback"),
            rule(
                1,
                RuleOperator::Equals,
                "team",
                Some(json!("platform")),
                "platform-specific",
            ),
        ];
        let resolved = resolve_profile(&rules, &claims).unwrap();
        assert_eq!(
            resolved.profile(),
            "fallback",
            "an identity that should have matched the specific rule instead silently fell \
             through to the default, because the default was not ordered last"
        );

        // The negative check: the *same* rules, with the default correctly
        // last, resolve to the specific rule as intended.
        let corrected = vec![
            rule(
                0,
                RuleOperator::Equals,
                "team",
                Some(json!("platform")),
                "platform-specific",
            ),
            rule(1, RuleOperator::Default, "", None, "fallback"),
        ];
        let resolved2 = resolve_profile(&corrected, &claims).unwrap();
        assert_eq!(resolved2.profile(), "platform-specific");
    }
}
