//! Behavioural tests for the generated bash completion script.
//!
//! String-matching the generated text does not prove the script works, so these
//! tests source the emitted script in a real `bash`, set the completion
//! variables the shell would set (`COMP_LINE` / `COMP_POINT` / `COMP_WORDS` /
//! `COMP_CWORD`), invoke the generated function, and read back `COMPREPLY`.
//!
//! Skipped (not failed) when no `bash` is on `PATH` — every other supported
//! platform still compiles and runs the rest of the suite.

use cli_framework::app::{App, AppBuilder, AppContext, Shell};
use cli_framework::command::Command;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use std::io::Write;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

fn noop_cmd(id: &'static str, summary: &'static str, args: Vec<ArgSpec>) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            args,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    }
}

fn opt(name: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        kind: ArgKind::Option,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Optional,
        ..Default::default()
    }
}

fn flag(name: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        kind: ArgKind::Flag,
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        ..Default::default()
    }
}

/// An app shaped like the reported case: a `repos` group with three
/// subcommands, plus a flag-carrying top-level command.
fn build_app() -> App<DummyCtx> {
    AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_group(
            &CommandPath::root_for("repos"),
            GroupMetadata {
                summary: "Repository management",
                hidden: false,
            },
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["repos", "add"]).unwrap(),
            noop_cmd("add", "Add a repository", vec![opt("url")]),
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["repos", "list"]).unwrap(),
            noop_cmd("list", "List repositories", vec![]),
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["repos", "remove"]).unwrap(),
            noop_cmd("remove", "Remove a repository", vec![]),
        )
        .unwrap()
        .register_command(noop_cmd(
            "search",
            "Search skills",
            vec![opt("limit"), flag("json")],
        ))
        .unwrap()
        .build(DummyCtx)
        .unwrap()
}

fn bash_available() -> bool {
    std::process::Command::new("bash")
        .arg("-c")
        .arg("exit 0")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Source the generated script in a real bash, drive the completion function
/// for `words` with the cursor on index `cword`, and return `COMPREPLY`.
fn complete(script: &str, words: &[&str], cword: usize) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("completion.bash");
    std::fs::write(&script_path, script).expect("write completion script");

    // COMP_LINE / COMP_POINT are what the real shell exports alongside the
    // word array; set them the same way so a generator that reads them
    // instead of COMP_WORDS is exercised faithfully too.
    let comp_line = words.join(" ");
    let quoted: Vec<String> = words
        .iter()
        .map(|w| format!("'{}'", w.replace('\'', r"'\''")))
        .collect();

    let driver = format!(
        r#"set -u
source {script}
COMP_LINE={line}
COMP_POINT=${{#COMP_LINE}}
COMP_WORDS=({words})
COMP_CWORD={cword}
export COMP_LINE COMP_POINT
COMPREPLY=()
_myapp
printf '%s\n' "${{COMPREPLY[@]}}"
"#,
        script = shell_quote(script_path.to_str().unwrap()),
        line = shell_quote(&comp_line),
        words = quoted.join(" "),
        cword = cword,
    );

    let driver_path = dir.path().join("driver.bash");
    let mut f = std::fs::File::create(&driver_path).expect("create driver");
    f.write_all(driver.as_bytes()).expect("write driver");
    drop(f);

    let out = std::process::Command::new("bash")
        .arg(&driver_path)
        .output()
        .expect("run bash driver");

    assert!(
        out.status.success(),
        "bash driver failed ({:?})\n--- stderr ---\n{}\n--- script ---\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        script
    );

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn bash_script(app: &App<DummyCtx>) -> String {
    let mut out = Vec::<u8>::new();
    app.emit_completion(Shell::Bash, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// The generated function must read the word the cursor is on
/// (`COMP_WORDS[COMP_CWORD]`), never a hardcoded first argument.
#[test]
fn generated_script_uses_comp_cword_not_index_one() {
    let script = bash_script(&build_app());

    assert!(
        script.contains("COMP_WORDS[COMP_CWORD]"),
        "generated script does not read the word under the cursor:\n{}",
        script
    );
    assert!(
        !script.contains("COMP_WORDS[1]"),
        "generated script still hardcodes COMP_WORDS[1]:\n{}",
        script
    );
}

/// `myapp repos <TAB>` must offer the `repos` subcommands.
#[test]
fn completes_subcommands_of_a_group() {
    if !bash_available() {
        eprintln!("skipping: no bash on PATH");
        return;
    }

    let script = bash_script(&build_app());
    let reply = complete(&script, &["myapp", "repos", ""], 2);

    for expected in ["add", "list", "remove"] {
        assert!(
            reply.iter().any(|w| w == expected),
            "COMPREPLY for `myapp repos <TAB>` missing {:?}: {:?}\n--- script ---\n{}",
            expected,
            reply,
            script
        );
    }
    assert!(
        !reply.iter().any(|w| w == "repos"),
        "COMPREPLY for `myapp repos <TAB>` echoed the group itself: {:?}",
        reply
    );
}

/// A partial word at the second level must filter the subcommand list, not the
/// top-level verb list.
#[test]
fn completes_partial_subcommand_word() {
    if !bash_available() {
        eprintln!("skipping: no bash on PATH");
        return;
    }

    let script = bash_script(&build_app());
    let reply = complete(&script, &["myapp", "repos", "l"], 2);

    assert_eq!(
        reply,
        vec!["list".to_string()],
        "COMPREPLY for `myapp repos l<TAB>`: {:?}\n--- script ---\n{}",
        reply,
        script
    );
}

/// Flags of the command under the cursor must be offered.
#[test]
fn completes_flags_of_a_leaf_command() {
    if !bash_available() {
        eprintln!("skipping: no bash on PATH");
        return;
    }

    let script = bash_script(&build_app());
    let reply = complete(&script, &["myapp", "search", "--"], 2);

    for expected in ["--limit", "--json"] {
        assert!(
            reply.iter().any(|w| w == expected),
            "COMPREPLY for `myapp search --<TAB>` missing {:?}: {:?}\n--- script ---\n{}",
            expected,
            reply,
            script
        );
    }
}

/// Flags of a nested leaf command must be offered at their own level.
#[test]
fn completes_flags_of_a_nested_leaf_command() {
    if !bash_available() {
        eprintln!("skipping: no bash on PATH");
        return;
    }

    let script = bash_script(&build_app());
    let reply = complete(&script, &["myapp", "repos", "add", "--"], 3);

    assert!(
        reply.iter().any(|w| w == "--url"),
        "COMPREPLY for `myapp repos add --<TAB>` missing \"--url\": {:?}\n--- script ---\n{}",
        reply,
        script
    );
}

/// The first word still completes to the top-level verbs.
#[test]
fn completes_top_level_verbs_at_the_first_word() {
    if !bash_available() {
        eprintln!("skipping: no bash on PATH");
        return;
    }

    let script = bash_script(&build_app());
    let reply = complete(&script, &["myapp", ""], 1);

    for expected in ["repos", "search", "completion", "spec"] {
        assert!(
            reply.iter().any(|w| w == expected),
            "COMPREPLY for `myapp <TAB>` missing {:?}: {:?}\n--- script ---\n{}",
            expected,
            reply,
            script
        );
    }
    assert!(
        !reply.iter().any(|w| w == "add"),
        "COMPREPLY for `myapp <TAB>` leaked a nested subcommand: {:?}",
        reply
    );
}

/// Hidden commands stay out of the completion surface at every level.
#[test]
fn hidden_commands_are_not_completed() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_group(
            &CommandPath::root_for("repos"),
            GroupMetadata {
                summary: "Repository management",
                hidden: false,
            },
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["repos", "add"]).unwrap(),
            noop_cmd("add", "Add a repository", vec![]),
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["repos", "secret"]).unwrap(),
            Command {
                spec: Arc::new(CommandSpec {
                    summary: "Hidden",
                    hidden: true,
                    ..Default::default()
                }),
                ..noop_cmd("secret", "Hidden", vec![])
            },
        )
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let script = bash_script(&app);
    assert!(
        !script.contains("secret"),
        "hidden subcommand leaked into the completion script:\n{}",
        script
    );

    if !bash_available() {
        return;
    }
    let reply = complete(&script, &["myapp", "repos", ""], 2);
    assert!(
        reply.iter().any(|w| w == "add"),
        "COMPREPLY missing visible sibling: {:?}",
        reply
    );
    assert!(
        !reply.iter().any(|w| w == "secret"),
        "hidden subcommand offered by COMPREPLY: {:?}",
        reply
    );
}
