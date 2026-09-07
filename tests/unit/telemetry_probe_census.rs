// tests/unit/telemetry_probe_census.rs
//! Call-site census: every catalogued instrument and span name is either
//! emitted by production code in `src/`, or explicitly reserved.
//!
//! Why this file exists. `specs/028-telemetry-probe-emission-gap.md` found
//! that most of the probe catalog had no emission call site at all, and that
//! **no existing gate could have caught it**: the builders are `pub` and
//! re-exported, so rustc emits no dead-code warning and clippy is silent;
//! `probes.rs` measured 100% line coverage because the unit tests call every
//! builder directly. Coverage cannot detect an unused public API. The only
//! gate that catches a catalog-shaped gap is a census that asserts each
//! catalogued name has a producer.
//!
//! This census reads `src/` as text, because that is the only place the
//! question "does a call site exist?" can be answered. Two traps, both
//! documented in spec 028 after being hit by hand:
//!
//! 1. A name can be spelled as a string literal (`info_span!("cli.command")`)
//!    **or** through the const (`metrics::MCP_TOOL_CALLS`). Censusing one
//!    spelling misses sites written in the other.
//! 2. A name occurring in a *comment* is not a call site.
//!    `http.client.request.duration` appears in `src/http_retry/client.rs`
//!    only inside a comment explaining why it is *not* recorded — the exact
//!    false positive that makes a naive `grep` claim the gap is closed.
//!
//! So: strip comments first, then match on a call *expression*, never on the
//! name merely appearing in the file.
use cli_framework::telemetry::{
    metrics, spans, Emission, BUILDER_EMISSION, METRIC_EMISSION, RESERVED_PENDING_INSTRUMENTATION,
    SPAN_EMISSION,
};
use std::path::{Path, PathBuf};

/// Absolute path to the crate's `src/`, resolved from the manifest dir so the
/// census works from any working directory.
fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/`, recursively.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Remove `//` line comments and `/* */` block comments, keeping every string
/// literal intact — a span name *is* a string literal, so blanking strings
/// would blind the census.
///
/// Deliberately not a Rust parser, but it does have to track the four things
/// in real `src/` files that desynchronise a naive in-string flag. All four
/// are present in this crate, and `strip_comments_survives_the_syntax_that_is_actually_in_src`
/// below pins each one:
///
/// * escaped quotes inside a literal (`"\""`);
/// * raw strings, including hashed ones — `src/config/service/state.rs` holds
///   `r#"{"manifest_schema_version":1,...}"#`, whose inner quotes would close a
///   plain literal early and leave the rest of the file parsed as the wrong
///   thing;
/// * character literals holding a quote — `src/config/service/router.rs` calls
///   `trim_matches('"')`, which opens a string that never closes;
/// * lifetimes (`&'static str`), which share the `'` with char literals and
///   must not be mistaken for one.
///
/// A desync is silent and one-directional in the dangerous way: text that
/// should have been stripped survives, so a probe name in a comment can read
/// as a call site. So the machine asserts it ends in `Code` — an unterminated
/// string or block comment means the scan lost its place and the census's
/// answer for that file cannot be trusted.
fn strip_comments(src: &str, path: &str) -> String {
    #[derive(PartialEq, Debug)]
    enum S {
        Code,
        Str,
        RawStr(usize),
        Line,
        Block,
    }
    let mut state = S::Code;
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;

    /// Is the `r` of a possible raw-string prefix a standalone token rather
    /// than the tail of an identifier like `for` or `my_var`?
    fn is_token_start(chars: &[char], i: usize) -> bool {
        i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
    }

    /// `Some(hashes)` if a raw string starts at `i`: `r"`, `r#"`, `br##"`, ...
    fn raw_string_at(chars: &[char], i: usize) -> Option<usize> {
        let mut j = i;
        if chars.get(j) == Some(&'b') {
            j += 1;
        }
        if chars.get(j) != Some(&'r') {
            return None;
        }
        j += 1;
        let hashes = chars[j..].iter().take_while(|c| **c == '#').count();
        if chars.get(j + hashes) == Some(&'"') {
            Some(hashes)
        } else {
            None
        }
    }

    /// The index just past a char literal starting at `i`, or `None` when the
    /// `'` opens a lifetime instead.
    fn char_literal_end(chars: &[char], i: usize) -> Option<usize> {
        let mut j = i + 1;
        if chars.get(j) == Some(&'\\') {
            j += 1;
            // `\u{...}` and the one-character escapes both end at the next `'`.
            while j < chars.len() && chars[j] != '\'' {
                j += 1;
            }
            return (j < chars.len()).then_some(j + 1);
        }
        // `'a'` is a literal; `'a` followed by anything else is a lifetime.
        (chars.get(j + 1) == Some(&'\'')).then_some(j + 2)
    }

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            S::Code => {
                if c == '/' && next == Some('/') {
                    state = S::Line;
                    i += 2;
                    continue;
                }
                if c == '/' && next == Some('*') {
                    state = S::Block;
                    i += 2;
                    continue;
                }
                if (c == 'r' || c == 'b') && is_token_start(&chars, i) {
                    if let Some(hashes) = raw_string_at(&chars, i) {
                        let open_len = usize::from(c == 'b') + 1 + hashes + 1;
                        out.extend(&chars[i..i + open_len]);
                        state = S::RawStr(hashes);
                        i += open_len;
                        continue;
                    }
                }
                if c == '\'' {
                    if let Some(end) = char_literal_end(&chars, i) {
                        out.extend(&chars[i..end]);
                        i = end;
                        continue;
                    }
                }
                if c == '"' {
                    state = S::Str;
                }
                out.push(c);
            }
            S::Str => {
                // Skip escaped characters so `\"` does not close the literal.
                if c == '\\' {
                    out.push(c);
                    if let Some(n) = next {
                        out.push(n);
                        i += 2;
                        continue;
                    }
                } else {
                    if c == '"' {
                        state = S::Code;
                    }
                    out.push(c);
                }
            }
            // No escapes in a raw string: it ends at `"` plus exactly the
            // hash count it opened with.
            S::RawStr(hashes) => {
                out.push(c);
                if c == '"' && chars[i + 1..].iter().take_while(|x| **x == '#').count() >= hashes {
                    out.extend(&chars[i + 1..i + 1 + hashes]);
                    state = S::Code;
                    i += 1 + hashes;
                    continue;
                }
            }
            S::Line => {
                if c == '\n' {
                    state = S::Code;
                    out.push(c);
                }
            }
            S::Block => {
                if c == '*' && next == Some('/') {
                    state = S::Code;
                    i += 2;
                    continue;
                }
                if c == '\n' {
                    out.push(c);
                }
            }
        }
        i += 1;
    }

    assert!(
        matches!(state, S::Code | S::Line),
        "the comment scan ended inside {state:?} while reading {path}, so it lost \
         its place partway through the file; every census answer for {path} is \
         unreliable until the scanner handles whatever construct desynced it"
    );
    out
}

/// Every file under `src/` exactly as written, paired with its path relative
/// to the crate root.
fn raw_sources() -> Vec<(String, String)> {
    let root = src_root();
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(
        files.len() > 10,
        "census found only {} source files under {root:?}; the walk is broken, \
         and a census that reads nothing passes vacuously",
        files.len()
    );
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
            (rel, text)
        })
        .collect()
}

/// The same files with comments removed — what every census question below
/// actually reads.
fn production_sources() -> Vec<(String, String)> {
    raw_sources()
        .into_iter()
        .map(|(rel, text)| {
            let stripped = strip_comments(&text, &rel);
            (rel, stripped)
        })
        .collect()
}

/// The const identifier a catalogued name is declared under, if any —
/// `"cli.command.invocations"` -> `COMMAND_INVOCATIONS`. Used to census the
/// second spelling of a call site.
fn const_idents_for(name: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (n, ident) in METRIC_CONSTS.iter().chain(SPAN_CONSTS.iter()) {
        if *n == name {
            out.push(*ident);
        }
    }
    out
}

/// Name -> const identifier, so the census can match `metrics::FOO` as well as
/// the literal. Asserted complete against `metrics::ALL` below, so a new
/// instrument cannot quietly escape the const spelling.
const METRIC_CONSTS: &[(&str, &str)] = &[
    (metrics::COMMAND_INVOCATIONS, "COMMAND_INVOCATIONS"),
    (metrics::COMMAND_DURATION_MS, "COMMAND_DURATION_MS"),
    (metrics::PROCESS_DURATION_MS, "PROCESS_DURATION_MS"),
    (metrics::USAGE_ERRORS, "USAGE_ERRORS"),
    (metrics::PANICS, "PANICS"),
    (metrics::HELP_SHOWN, "HELP_SHOWN"),
    (metrics::FEATURE_USES, "FEATURE_USES"),
    (metrics::AUTH_EVENTS, "AUTH_EVENTS"),
    (metrics::DOCTOR_FINDINGS, "DOCTOR_FINDINGS"),
    (metrics::PLUGIN_LOADS, "PLUGIN_LOADS"),
    (metrics::CHAT_TURNS, "CHAT_TURNS"),
    (metrics::CHAT_SESSIONS, "CHAT_SESSIONS"),
    (
        metrics::HTTP_CLIENT_REQUEST_DURATION,
        "HTTP_CLIENT_REQUEST_DURATION",
    ),
    (
        metrics::HTTP_SERVER_REQUEST_DURATION,
        "HTTP_SERVER_REQUEST_DURATION",
    ),
    (metrics::MCP_TOOL_CALLS, "MCP_TOOL_CALLS"),
];

const SPAN_CONSTS: &[(&str, &str)] = &[
    (spans::ROOT_COMMAND, "ROOT_COMMAND"),
    (spans::CONFIG_LOAD, "CONFIG_LOAD"),
    (spans::CONFIG_MIGRATE, "CONFIG_MIGRATE"),
    (spans::CONFIG_POLICY_REFRESH, "CONFIG_POLICY_REFRESH"),
    (spans::SECRETS_OP, "SECRETS_OP"),
    (spans::PLUGIN_LOAD, "PLUGIN_LOAD"),
    (spans::HTTP_CLIENT_REQUEST, "HTTP_CLIENT_REQUEST"),
    (spans::HTTP_SERVER_REQUEST, "HTTP_SERVER_REQUEST"),
];

/// The text of a call's first argument, for every occurrence of `call` in
/// `text`.
///
/// Scans to the matching close paren so a nested call in the argument does not
/// truncate it, then cuts at the first top-level comma. Returns the trimmed
/// argument source, which the callers below compare against the two legal
/// spellings of a catalogued name.
fn first_args_of(text: &str, call: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(idx) = text[from..].find(call) {
        let start = from + idx + call.len();
        let mut depth = 1usize;
        let mut arg = String::new();
        for c in text[start..].chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ',' if depth == 1 => break,
                _ => {}
            }
            arg.push(c);
        }
        out.push(arg.trim().to_string());
        from = start;
    }
    out
}

/// Does `arg_text` name `name`, in either legal spelling?
///
/// The literal spelling must match exactly. The const spelling is matched by
/// *suffix*, because a call site may reach the const by any path —
/// `src/mcp/mod.rs` writes `crate::telemetry::metrics::MCP_TOOL_CALLS` while a
/// site inside the telemetry module would write `metrics::MCP_TOOL_CALLS`.
/// Anchoring on the full path would miss the only const-spelled call site the
/// crate currently has, which is precisely the trap spec 028 recorded.
fn arg_names(arg_text: &str, name: &str, kind_path: &str) -> bool {
    if arg_text == format!("\"{name}\"") {
        return true;
    }
    const_idents_for(name)
        .iter()
        .any(|ident| arg_text.ends_with(&format!("{kind_path}::{ident}")))
}

/// Does `name` appear as a *metric call expression* in `text`?
///
/// Matches `counter(<name>)` / `histogram(<name>)` in either spelling. The
/// name merely occurring in the file is not enough — that is what makes a
/// comment mentioning an unrecorded instrument a false positive.
fn records_metric(text: &str, name: &str) -> bool {
    ["counter(", "histogram("].iter().any(|call| {
        first_args_of(text, call)
            .iter()
            .any(|arg| arg_names(arg, name, "metrics"))
    })
}

/// Does `name` appear as a *span-creation* expression in `text`?
///
/// `tracing`'s span macros fix their metadata at the callsite, so in practice
/// the name must be a literal there — a const cannot be substituted. The const
/// spelling is censused anyway, so a future site that opens a span through
/// some other constructor is not silently missed.
fn opens_span(text: &str, name: &str) -> bool {
    ["info_span!(", "debug_span!(", "trace_span!(", "span!("]
        .iter()
        .any(|call| {
            first_args_of(text, call)
                .iter()
                .any(|arg| arg_names(arg, name, "spans"))
        })
}

/// Which production sources call `init_from_policy` -- the only constructor of
/// either half of the export boundary (the redacting span exporter, and the
/// meter provider's per-instrument Views)?
///
/// Definitions and re-exports are not calls, so `fn init_from_policy(` and
/// `pub use init::{init_from_policy, ...}` are both excluded.
fn redaction_boundary_call_sites(sources: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, text) in sources {
        for (i, _) in text.match_indices("init_from_policy(") {
            let before = &text[..i];
            if before.ends_with("fn ") {
                continue;
            }
            out.push(path.clone());
        }
    }
    out
}

/// The emission table's claim for `name`, or `None` if it makes none.
fn declared(table: &[(&str, Emission)], name: &str) -> Option<Emission> {
    table.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
}

/// Every catalogued span name: both roots and every child.
fn all_span_names() -> Vec<&'static str> {
    spans::ROOTS
        .iter()
        .chain(spans::CHILDREN.iter())
        .copied()
        .collect()
}

/// The probe-data builders `probes.rs` declares, read out of the source
/// rather than listed here by hand.
///
/// The rule is the return type: a `pub fn` in `probes.rs` that returns a
/// `Vec` produces probe data — attributes (`Vec<KeyValue>`), argument names
/// (`Vec<String>`) or the registry adapter's name list (`Vec<&str>`).
/// Discovering them is what makes `BUILDER_EMISSION` *total by construction* —
/// a builder added tomorrow without a table entry fails this census instead of
/// joining the catalog uncensused, which is the defect this whole file exists
/// to close. A hand-written list would have to be remembered; this one cannot
/// be forgotten.
fn builders_declared_in_probes_rs(sources: &[(String, String)]) -> Vec<String> {
    let (_, text) = sources
        .iter()
        .find(|(path, _)| path == "src/telemetry/probes.rs")
        .expect("src/telemetry/probes.rs is missing from the source walk");

    const DECL: &str = "pub fn ";
    let mut out: Vec<String> = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find(DECL) {
        let after = &rest[at + DECL.len()..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // The signature ends at the body's opening brace; no signature in this
        // file contains one.
        let sig = &after[..after.find('{').unwrap_or(after.len())];
        if sig.contains("-> Vec<") {
            out.push(name.clone());
        }
        rest = &after[name.len()..];
    }

    assert!(
        out.len() > 10,
        "only {} probe-data builders found in probes.rs; the signature scan is \
         broken, and a census that discovers nothing passes vacuously",
        out.len()
    );
    out.sort();
    let before = out.len();
    out.dedup();
    assert_eq!(before, out.len(), "probes.rs declares a builder name twice");
    out
}

/// Is the byte offset `i` the start of a token, rather than the middle of a
/// longer identifier?
fn is_token_start(text: &str, i: usize) -> bool {
    !text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Is `name` *reached* by `text`?
///
/// Two forms count, because both are real:
///
/// 1. A call expression, `name(`.
/// 2. A path reference, `::name` — which covers a builder passed as a
///    function *value*. `src/app/context.rs` writes
///    `.map(crate::telemetry::registered_feature_names)`, with no parentheses
///    of its own. A census that matched only form 1 would have called that
///    builder unreached, and would have said the same of any future
///    `.map(process_attrs)`. That is the false negative this census cannot
///    afford: it would report a wired builder as an open gap, or worse, let a
///    `Reserved` one be wired without failing.
///
/// Excluded are the declaration itself (`pub fn process_attrs(`) and an
/// identifier that is merely part of a longer one.
///
/// Both needles carry punctuation, and that is load-bearing rather than
/// incidental. `src/telemetry/mod.rs` re-exports every builder — `pub use
/// probes::{arg_names, process_attrs, ...}` — and a re-export names a builder
/// without reaching it. Inside those braces a name is followed by `,` or `}`
/// and preceded by `{` or a space, so it matches neither `name(` nor
/// `::name`, and no `use`-item skipping is needed to exclude it. Relaxing the
/// second needle to a bare word was tried: it makes
/// `no_reserved_builder_is_reached_anywhere_in_src` fail with all eleven
/// reserved builders "reached from src/telemetry/mod.rs". That is the shape a
/// future re-export written one name per line would take too — a loud,
/// file-naming failure rather than a silent pass, which is why this scan
/// carries no special case for it. For the same reason the *string*
/// `"process_attrs"` — how `BUILDER_EMISSION` spells its own keys — matches
/// neither form, so the table cannot census itself into looking wired.
///
/// `probes.rs` is deliberately *not* excluded from the search. A builder
/// reached only from inside the catalog file is still reached: a `Reserved`
/// builder that some `Wired` one calls would be emitted in fact, and counting
/// the internal call is what makes this census say so.
fn references_builder(text: &str, name: &str) -> bool {
    let called = text
        .match_indices(&format!("{name}("))
        .any(|(i, _)| !text[..i].ends_with("fn ") && is_token_start(text, i));

    let referenced = text.match_indices(&format!("::{name}")).any(|(i, _)| {
        let after = i + 2 + name.len();
        !text[after..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    });

    called || referenced
}

/// The scanner is the census's only defence against a probe name in a comment
/// reading as a call site, so it gets its own test rather than being trusted
/// implicitly.
///
/// Every construct below is taken from real `src/` code, not invented: a
/// hashed raw string holding quotes (`src/config/service/state.rs`), a char
/// literal holding a quote (`src/config/service/router.rs`), a lifetime, and
/// an escaped quote. Each one desyncs a naive in-string flag, and a desync
/// makes the *dangerous* mistake — unstripped comment text scored as code.
#[test]
fn strip_comments_survives_the_syntax_that_is_actually_in_src() {
    let fixture = r####"
fn f() -> &'static str {
    let _ = r#"{"a":1,"b":"// not a comment"}"#;
    let _ = "he said \"hi\" // still a string";
    let _ = x.trim_matches('"');
    let _ = '\'';
    // .counter(metrics::PANICS)
    /* .histogram(metrics::PANICS) */
    "kept"
}
"####;
    let stripped = strip_comments(fixture, "fixture.rs");

    // Comments go, on both sides of every hazard above.
    assert!(
        !stripped.contains("metrics::PANICS"),
        "a commented-out call survived the scan, which is the exact false \
         positive this census exists to avoid:\n{stripped}"
    );
    // String contents stay — span names are string literals.
    assert!(stripped.contains("kept"), "{stripped}");
    assert!(
        stripped.contains("// not a comment"),
        "the scanner stripped the inside of a raw string:\n{stripped}"
    );
    assert!(
        stripped.contains("// still a string"),
        "the scanner stripped the inside of a normal literal:\n{stripped}"
    );
    // And it did not lose its place: the trailing code is still there.
    assert!(stripped.trim_end().ends_with('}'), "{stripped}");
}

/// The syntax the fixture above exercises is really in `src/`, so the fixture
/// is guarding this tree and not a hypothetical one.
///
/// The two hazards asserted here — a *hashed* raw string and a `'"'` char
/// literal — are the two that made the first version of `strip_comments`
/// wrong. Each one, mishandled, flips the scanner into the opposite state and
/// silently mangles everything after it, which would make every census
/// question below answer about text that is not the source.
///
/// # What this test does not prove
///
/// Running `production_sources()` does re-run `strip_comments`'s own terminal
/// "ended neutral" assertion over the whole tree. That assertion is much
/// weaker than it looks and this test does not lean on it: when the
/// raw-string and char-literal handling was deliberately removed to check
/// this, the fixture test failed as intended but a whole-tree end-state check
/// still *passed*. A naive scanner desyncs on the first hazard and then
/// re-syncs on some later quote, so it lands back in a neutral state having
/// corrupted only the span in between. End-of-file state is therefore not an
/// oracle for mid-file correctness; the fixture is, and this test is what
/// keeps the fixture honest.
#[test]
fn the_hazards_the_scanner_handles_are_really_in_src() {
    let raw = raw_sources();

    let has_hashed_raw_string = raw.iter().any(|(_, text)| text.contains("r#\""));
    assert!(
        has_hashed_raw_string,
        "no file under src/ contains a hashed raw string (r#\"...\"#) any more. \
         The fixture in strip_comments_survives_the_syntax_that_is_actually_in_src \
         now guards syntax this tree does not use — either find the construct \
         that replaced it and add it to the fixture, or drop the raw-string \
         state from strip_comments so the scanner and its test stay honest \
         about what they cover."
    );

    let has_quote_char_literal = raw.iter().any(|(_, text)| text.contains("'\"'"));
    assert!(
        has_quote_char_literal,
        "no file under src/ contains a '\"' char literal any more. See the \
         message above: the fixture is now guarding syntax this tree does not \
         use, so either extend the fixture or simplify the scanner."
    );

    // Stripping every file also runs strip_comments' internal end-state
    // assertion across the whole tree. See "What this test does not prove".
    for (path, text) in production_sources() {
        assert!(
            !text.is_empty() || path.ends_with("mod.rs"),
            "{path} stripped to nothing"
        );
    }
}

#[test]
fn const_tables_cover_the_whole_catalog() {
    for name in metrics::ALL {
        assert!(
            METRIC_CONSTS.iter().any(|(n, _)| n == name),
            "{name} is in metrics::ALL but has no entry in this file's \
             METRIC_CONSTS, so the census cannot match its const spelling"
        );
    }
    assert_eq!(
        METRIC_CONSTS.len(),
        metrics::ALL.len(),
        "METRIC_CONSTS and metrics::ALL have drifted"
    );

    for name in all_span_names() {
        assert!(
            SPAN_CONSTS.iter().any(|(n, _)| *n == name),
            "{name} is a catalogued span but has no entry in this file's \
             SPAN_CONSTS, so the census cannot match its const spelling"
        );
    }
    assert_eq!(
        SPAN_CONSTS.len(),
        all_span_names().len(),
        "SPAN_CONSTS and spans::ROOTS + spans::CHILDREN have drifted"
    );
}

/// The emission table is only a gate if it is *total*. A catalogued name
/// missing from it would be censused by nothing at all — the original defect,
/// one level up.
#[test]
fn the_emission_table_covers_every_catalogued_name() {
    for name in metrics::ALL {
        assert!(
            declared(METRIC_EMISSION, name).is_some(),
            "{name} is in metrics::ALL with no METRIC_EMISSION entry: it would be \
             censused by neither direction of this file"
        );
    }
    assert_eq!(
        METRIC_EMISSION.len(),
        metrics::ALL.len(),
        "METRIC_EMISSION has an entry for a name that is not in metrics::ALL, \
         or lists one twice"
    );

    for name in all_span_names() {
        assert!(
            declared(SPAN_EMISSION, name).is_some(),
            "{name} is a catalogued span with no SPAN_EMISSION entry: it would be \
             censused by neither direction of this file"
        );
    }
    assert_eq!(
        SPAN_EMISSION.len(),
        all_span_names().len(),
        "SPAN_EMISSION has an entry for a name that is not a catalogued span, \
         or lists one twice"
    );

    // A span cannot be both a root and a child.
    for name in spans::ROOTS {
        assert!(
            !spans::CHILDREN.contains(name),
            "{name} is listed in both spans::ROOTS and spans::CHILDREN"
        );
    }
}

/// The third category the original gap report missed.
///
/// Spec 025 §4 promises attributes as well as spans and metrics — the row for
/// `cli.process`, for instance, promises a root-span attribute
/// `process.exit.code`. That attribute is produced by exactly one function,
/// `process_attrs`, and a function with no caller emits nothing however
/// faithfully it is written. Attribute builders are where most of the catalog
/// actually lives, so a census that covered only instrument and span *names*
/// would have answered the narrow question and left the larger one open.
#[test]
fn the_builder_table_covers_every_builder_in_probes_rs() {
    let sources = production_sources();
    let declared_builders = builders_declared_in_probes_rs(&sources);

    for name in &declared_builders {
        assert!(
            BUILDER_EMISSION.iter().any(|(n, _)| n == name),
            "probes.rs declares the probe-data builder `{name}` but \
             BUILDER_EMISSION has no entry for it, so nothing censuses whether \
             it has a caller — the exact shape of the original gap"
        );
    }
    for (name, _) in BUILDER_EMISSION {
        assert!(
            declared_builders.iter().any(|d| d == name),
            "BUILDER_EMISSION lists `{name}`, which probes.rs no longer \
             declares as a probe-data builder"
        );
    }
    assert_eq!(
        BUILDER_EMISSION.len(),
        declared_builders.len(),
        "BUILDER_EMISSION lists a builder twice"
    );
}

// ---------------------------------------------------------------------------
// Direction 1: every `Wired` claim is true.
// ---------------------------------------------------------------------------

#[test]
fn every_wired_metric_is_created_in_the_file_the_table_names() {
    let sources = production_sources();
    let mut wrong = Vec::new();
    for (name, emission) in METRIC_EMISSION {
        let Emission::Wired(file) = emission else {
            continue;
        };
        let found_in: Vec<&str> = sources
            .iter()
            .filter(|(_, text)| records_metric(text, name))
            .map(|(path, _)| path.as_str())
            .collect();
        if !found_in.contains(file) {
            wrong.push(format!(
                "{name}: METRIC_EMISSION says Wired({file}), but counter()/histogram() \
                 for it appears in {found_in:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the emission table claims instruments are wired that are not: {wrong:#?}"
    );
}

#[test]
fn every_wired_span_is_opened_in_the_file_the_table_names() {
    let sources = production_sources();
    let mut wrong = Vec::new();
    for (name, emission) in SPAN_EMISSION {
        let Emission::Wired(file) = emission else {
            continue;
        };
        let found_in: Vec<&str> = sources
            .iter()
            .filter(|(_, text)| opens_span(text, name))
            .map(|(path, _)| path.as_str())
            .collect();
        if !found_in.contains(file) {
            wrong.push(format!(
                "{name}: SPAN_EMISSION says Wired({file}), but a span macro for it \
                 appears in {found_in:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the emission table claims spans are wired that are not: {wrong:#?}"
    );
}

// ---------------------------------------------------------------------------
// Direction 2: every `Reserved` claim is true.
//
// This is the half that keeps the table honest going the other way. Someone
// wiring a probe without moving its entry to `Wired` fails here, which is what
// makes the reservation a tracked decision rather than a stale comment — and,
// because every reservation is gated on the export boundary, what stops a
// probe reaching the wire while the bare exporter is still on the production
// path.
// ---------------------------------------------------------------------------

#[test]
fn no_reserved_metric_is_created_anywhere_in_src() {
    let sources = production_sources();
    let mut leaked = Vec::new();
    for (name, emission) in METRIC_EMISSION {
        let Emission::Reserved(_) = emission else {
            continue;
        };
        for (path, text) in &sources {
            if records_metric(text, name) {
                leaked.push(format!("{name} is created in {path}"));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "these instruments are recorded but METRIC_EMISSION still calls them \
         Reserved — if the wiring is intended, move the entry to \
         Emission::Wired and confirm the export boundary is on the production \
         path first: {leaked:#?}"
    );
}

#[test]
fn no_reserved_span_is_opened_anywhere_in_src() {
    let sources = production_sources();
    let mut leaked = Vec::new();
    for (name, emission) in SPAN_EMISSION {
        let Emission::Reserved(_) = emission else {
            continue;
        };
        for (path, text) in &sources {
            if opens_span(text, name) {
                leaked.push(format!("{name} is opened in {path}"));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "these spans are opened but SPAN_EMISSION still calls them Reserved — \
         if the wiring is intended, move the entry to Emission::Wired and \
         confirm the export boundary is on the production path first: {leaked:#?}"
    );
}

#[test]
fn every_wired_builder_is_reached_in_the_file_the_table_names() {
    let sources = production_sources();
    let mut wrong = Vec::new();
    for (name, emission) in BUILDER_EMISSION {
        let Emission::Wired(file) = emission else {
            continue;
        };
        let found_in: Vec<&str> = sources
            .iter()
            .filter(|(_, text)| references_builder(text, name))
            .map(|(path, _)| path.as_str())
            .collect();
        if !found_in.contains(file) {
            wrong.push(format!(
                "{name}: BUILDER_EMISSION says Wired({file}), but it is \
                 reached only from {found_in:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the emission table claims builders are reached that are not: {wrong:#?}"
    );
}

#[test]
fn no_reserved_builder_is_reached_anywhere_in_src() {
    let sources = production_sources();
    let mut leaked = Vec::new();
    for (name, emission) in BUILDER_EMISSION {
        let Emission::Reserved(_) = emission else {
            continue;
        };
        for (path, text) in &sources {
            if references_builder(text, name) {
                leaked.push(format!("{name} is reached from {path}"));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "these builders are now reached from production but BUILDER_EMISSION \
         still calls them Reserved — if the wiring is intended, move the entry to \
         Emission::Wired and confirm the export boundary is on the production \
         path first: {leaked:#?}"
    );
}

#[test]
fn every_reserved_entry_gives_a_reason() {
    for (name, emission) in METRIC_EMISSION
        .iter()
        .chain(SPAN_EMISSION.iter())
        .chain(BUILDER_EMISSION.iter())
    {
        if let Emission::Reserved(reason) = emission {
            assert!(
                reason.len() > 30 && reason.contains(' '),
                "{name} is Reserved with no usable reason: {reason:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The tripwire on the reservation's premise.
// ---------------------------------------------------------------------------

/// The export boundary is on the production startup path, and must stay there.
///
/// This test replaces `the_stated_blocker_for_every_reservation_still_holds`,
/// which asserted the opposite: that `init_from_policy` had *no* production
/// caller, and that this was what made "catalogued but not emitted" a dormant
/// gap rather than a live leak. That premise was true when the census was
/// written and is now false -- `run_startup` calls `init_from_policy`, so the
/// redacting span exporter and the per-instrument Views are built on every
/// real start.
///
/// Inverting the assertion rather than deleting it keeps the load-bearing
/// invariant guarded, because which direction is dangerous has flipped with
/// it. Redaction is enforced at the export boundary and nowhere else -- never
/// at the instrumentation call site -- so while the boundary was off the path
/// the risk was *adding* an emitter, and now that it is on the path the risk
/// is *removing* the boundary: every probe that gets wired from here on is
/// one more attribute set that would reach a collector unstripped if this
/// call disappeared, and nothing else in the repository would say so.
///
/// It is falsifiable in the way that matters: delete the `init_from_policy`
/// call from `run_startup`, or route startup back through `init_batch`, and
/// this fails.
#[test]
fn the_redacting_export_boundary_is_on_the_production_startup_path() {
    let sources = production_sources();
    let callers = redaction_boundary_call_sites(&sources);

    assert!(
        callers.iter().any(|p| p == "src/telemetry/startup.rs"),
        "run_startup no longer calls init_from_policy, so nothing builds the \
         redacting span exporter or the metric-label Views on the production \
         path. Callers found: {callers:?}. Every wired probe now exports its \
         attributes unstripped."
    );

    assert!(
        RESERVED_PENDING_INSTRUMENTATION.contains("run_startup")
            && RESERVED_PENDING_INSTRUMENTATION.contains("init_from_policy"),
        "the shared reason no longer names the two symbols this test checks \
         for; one of the two has drifted"
    );
}
