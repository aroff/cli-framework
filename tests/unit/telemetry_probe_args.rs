// tests/unit/telemetry_probe_args.rs
use cli_framework::telemetry::{arg_names, arg_value_attrs, usage_error_attrs};

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn keys(attrs: &[opentelemetry::KeyValue]) -> Vec<String> {
    attrs.iter().map(|a| a.key.to_string()).collect()
}

#[test]
fn argument_names_are_recorded_and_their_values_are_not() {
    let names = arg_names(&["output".to_string(), "verbose".to_string()]);
    assert_eq!(names, vec!["output".to_string(), "verbose".to_string()]);
}

#[test]
fn an_argument_value_is_recorded_only_when_the_author_allowlisted_that_argument() {
    let allowlist = vec!["format".to_string()];
    let attrs = arg_value_attrs(
        &allowlist,
        &pairs(&[("format", "json"), ("output", "/home/alice/tax.csv")]),
    );
    assert_eq!(
        keys(&attrs),
        vec!["cli.command.arg_values.format".to_string()]
    );
    assert_eq!(attrs[0].value.as_str(), "json");
}

#[test]
fn an_empty_allowlist_records_no_values_which_is_the_default() {
    let attrs = arg_value_attrs(&[], &pairs(&[("format", "json")]));
    assert!(
        attrs.is_empty(),
        "an author who never called with_telemetry_defaults gets no values, \
         because only they know which of their arguments are free text"
    );
}

#[test]
fn an_allowlisted_argument_whose_name_hits_the_never_list_is_still_dropped() {
    let allowlist = vec!["api_key".to_string(), "format".to_string()];
    let attrs = arg_value_attrs(
        &allowlist,
        &pairs(&[("api_key", "sk-live-1"), ("format", "json")]),
    );
    assert_eq!(
        keys(&attrs),
        vec!["cli.command.arg_values.format".to_string()],
        "the never-list is checked against the generated attribute key, so an \
         author cannot allowlist a credential-shaped argument by mistake"
    );
}

#[test]
fn a_usage_error_records_its_kind_without_the_token() {
    let attrs = usage_error_attrs("unknown_flag", None);
    let k = keys(&attrs);
    assert!(k.contains(&"cli.probe".to_string()));
    assert!(k.contains(&"cli.usage_error.kind".to_string()));
    assert!(!k.contains(&"cli.usage_error.token".to_string()));
}

#[test]
fn a_usage_error_token_is_emitted_under_its_own_probe_id() {
    let attrs = usage_error_attrs("unknown_flag", Some("--ouput"));
    let token = attrs
        .iter()
        .find(|a| a.key.as_str() == "cli.usage_error.token")
        .expect("token present when supplied");
    assert_eq!(token.value.as_str(), "--ouput");
    // The boundary drops it below debug; the callsite always emits it, which
    // is the whole approach-B contract.
}

#[test]
fn the_kind_of_a_usage_error_is_a_closed_vocabulary_not_the_error_text() {
    let attrs = usage_error_attrs("unknown_flag", None);
    let kind = attrs
        .iter()
        .find(|a| a.key.as_str() == "cli.usage_error.kind")
        .unwrap();
    assert_eq!(kind.value.as_str(), "unknown_flag");
    assert!(
        !kind.value.as_str().contains(' '),
        "a kind is a token, not a sentence: an error message would carry the \
         offending value inside it"
    );
}
