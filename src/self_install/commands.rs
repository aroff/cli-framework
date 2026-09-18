//! The `self` command group: thin CLI wrappers over [`super::ops`] and
//! [`super::update`].

use super::env::InstallEnv;
use super::ops::{
    self, InstallOutcome, InstallRequest, PathDecision, SelfInstallError, StatusReport,
};
use super::options::SelfInstallOptions;
use super::policy::SelfUpdatePolicy;
use super::update::{self, RollbackOutcome, UpdateAction, UpdateOutcome, UpdateRequest};
use crate::app::context::AppContext;
use crate::command::{Command, CommandRegistry, CommandResult};
use crate::parser::error_codes::{
    SI001, SI002, SI003, SI004, SI005, SI006, SI007, SI008, SI009, SI010, SI011,
};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandPath, CommandSpec, ExitCodeEntry, GroupMetadata};
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

type ExecuteFn = Arc<
    dyn for<'a> Fn(
            &'a mut dyn AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>>
        + Send
        + Sync,
>;

/// What every leaf needs to rebuild an [`InstallEnv`] at run time.
#[derive(Clone)]
struct Shared {
    app: &'static str,
    version: &'static str,
    options: Arc<SelfInstallOptions>,
    completion_command: Option<Vec<String>>,
    self_invocation: String,
}

impl Shared {
    fn env(&self) -> anyhow::Result<InstallEnv> {
        let mut env = InstallEnv::detect(self.app, self.version)
            .map_err(|e| fail(SI006, format!("cannot locate the running executable: {e}")))?;
        env.completion_command = self.completion_command.clone();
        env.self_invocation = self.self_invocation.clone();
        Ok(env)
    }
}

/// Register `<namespace> self {install, update, rollback, uninstall, status}`.
///
/// `completion_command` is the argv after the binary name that reaches the
/// built-in completion command, or `None` when the app disabled it.
pub fn register_self_commands(
    registry: &mut CommandRegistry,
    namespace: &CommandPath,
    app: &'static str,
    version: &'static str,
    options: Arc<SelfInstallOptions>,
    completion_command: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let group = namespace
        .push("self")
        .expect("built-in command id is a valid path segment");
    let mut invocation: Vec<String> = vec![app.to_string()];
    invocation.extend(group.0.iter().cloned());
    let shared = Shared {
        app,
        version,
        options,
        completion_command,
        self_invocation: invocation.join(" "),
    };
    registry
        .register_group(
            &group,
            GroupMetadata {
                summary: "Install, inspect or remove this binary on this machine",
                hidden: false,
                category: Some("ops"),
                ..Default::default()
            },
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    for (leaf, command) in [
        ("install", install_command(shared.clone())),
        ("update", update_command(shared.clone())),
        ("rollback", rollback_command(shared.clone())),
        ("uninstall", uninstall_command(shared.clone())),
        ("status", status_command(shared)),
    ] {
        registry
            .register_at(&group.push(leaf).expect("valid segment"), command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    Ok(())
}

/// A runtime failure: the message carries the stable code and the command
/// exits 1 (usage errors, which exit 2, are caught by the parser first).
fn fail(code: &'static str, message: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{code}: {message}")
}

fn report(err: SelfInstallError) -> anyhow::Error {
    let code = match &err {
        SelfInstallError::RunningAsRoot { .. } | SelfInstallError::NeedsElevation { .. } => SI001,
        SelfInstallError::ForeignBinary { .. } => SI002,
        SelfInstallError::Receipt(_) => SI003,
        SelfInstallError::ManagedByPackageManager { .. }
        | SelfInstallError::NoReceipt { .. }
        | SelfInstallError::NotManaged { .. }
        | SelfInstallError::ReceiptMismatch { .. } => SI004,
        SelfInstallError::UnsafePurge { .. } => SI005,
        SelfInstallError::NoBinDir
        | SelfInstallError::NoStateDir
        | SelfInstallError::Place { .. }
        | SelfInstallError::Io { .. } => SI006,
        SelfInstallError::PolicyRefused { .. } => SI007,
        SelfInstallError::Release(_) => SI008,
        SelfInstallError::Verify(_) | SelfInstallError::BadRelease(_) => SI009,
        SelfInstallError::Downgrade { .. }
        | SelfInstallError::NothingToRollBack { .. }
        | SelfInstallError::BadRequest(_) => SI010,
        SelfInstallError::Locked { .. } => SI011,
    };
    fail(code, err.to_string())
}

fn flag(name: &'static str, long: &'static str, help: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        kind: ArgKind::Flag,
        long: Some(long),
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        default: Some(ArgValue::Bool(false)),
        help,
        ..Default::default()
    }
}

fn is_set(args: &HashMap<String, ArgValue>, name: &str) -> bool {
    matches!(args.get(name), Some(ArgValue::Bool(true)))
}

fn string_arg(args: &HashMap<String, ArgValue>, name: &str) -> Option<String> {
    match args.get(name) {
        Some(ArgValue::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn path_option(name: &'static str, long: &'static str, help: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        kind: ArgKind::Option,
        long: Some(long),
        value_type: ArgValueType::String,
        cardinality: Cardinality::Optional,
        help,
        ..Default::default()
    }
}

/// Malformed enforced keys are ignored (with their safe reading), and said.
fn policy_warnings(ctx: &mut dyn AppContext, policy: &SelfUpdatePolicy) {
    for key in &policy.malformed {
        ctx.framework_println(&format!("warning: ignoring the malformed policy key {key}"));
    }
}

fn exit_codes(failure: &'static str) -> Vec<ExitCodeEntry> {
    vec![
        ExitCodeEntry {
            code: 0,
            description: "Success",
        },
        ExitCodeEntry {
            code: 1,
            description: failure,
        },
    ]
}

fn leaf(
    id: &'static str,
    summary: &'static str,
    args: Vec<ArgSpec>,
    failure: &'static str,
    execute: ExecuteFn,
) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            category: Some("ops"),
            args,
            exit_codes: exit_codes(failure),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute,
    }
}

fn install_command(shared: Shared) -> Command {
    let args = vec![
        path_option(
            "bin_dir",
            "bin-dir",
            "Directory to install into (default: $XDG_BIN_HOME or ~/.local/bin)",
        ),
        flag(
            "system",
            "system",
            "Install for all users (/usr/local/bin, or %ProgramFiles%\\<app>\\bin); needs elevation",
        ),
        path_option(
            "from",
            "from",
            "Install from a local release archive, verified against the SHA256SUMS beside it",
        ),
        flag(
            "no_modify_path",
            "no-modify-path",
            "Do not edit shell startup files or the user Path",
        ),
        flag(
            "unmanaged",
            "unmanaged",
            "Place the binary only: no PATH edit and no install receipt",
        ),
        flag(
            "force",
            "force",
            "Replace a binary at the target that no receipt records",
        ),
        flag(
            "from_bootstrap",
            "from-bootstrap",
            "Move instead of copy; passed by the installer scripts",
        ),
    ];
    leaf(
        "install",
        "Install this binary for the current user and put it on PATH",
        args,
        "Refused (root, foreign binary, needs elevation), a bad archive, or a filesystem error",
        Arc::new(move |ctx, args| {
            let shared = shared.clone();
            Box::pin(async move {
                let env = shared.env()?;
                let req = InstallRequest {
                    bin_dir: string_arg(&args, "bin_dir").map(PathBuf::from),
                    no_modify_path: is_set(&args, "no_modify_path"),
                    unmanaged: is_set(&args, "unmanaged"),
                    force: is_set(&args, "force"),
                    from_bootstrap: is_set(&args, "from_bootstrap"),
                    system: is_set(&args, "system"),
                    from: string_arg(&args, "from").map(PathBuf::from),
                };
                let outcome = ops::install(&env, &shared.options, &req).map_err(report)?;
                for line in render_install(&env, &outcome) {
                    ctx.framework_println(&line);
                }
                Ok(())
            })
        }),
    )
}

/// The human summary of an install. Public to the crate for tests.
pub(crate) fn render_install(env: &InstallEnv, o: &InstallOutcome) -> Vec<String> {
    let mut lines = Vec::new();
    let verb = if o.already_in_place {
        "already installed at"
    } else if o.replaced_existing {
        "replaced"
    } else {
        "installed to"
    };
    lines.push(format!(
        "{} {} {verb} {}",
        env.app,
        o.version,
        o.binary_path.display()
    ));
    match &o.path {
        PathDecision::Edit if o.path_modifications.is_empty() => {
            lines.push(format!("PATH: {} is already set up", o.bin_dir.display()))
        }
        PathDecision::Edit => {
            lines.push(format!("PATH: added {} via:", o.bin_dir.display()));
            for m in &o.path_modifications {
                lines.push(format!("  {}", describe_modification(m)));
            }
        }
        PathDecision::AlreadyOnPath => {
            lines.push(format!("PATH: {} is already on PATH", o.bin_dir.display()))
        }
        PathDecision::Skip(reason) => {
            lines.push(format!("PATH: not modified ({reason})"));
        }
    }
    if let Some(hint) = &o.current_shell_hint {
        lines.push(format!("  to use it in this shell now: {hint}"));
    }
    match &o.receipt_path {
        Some(p) => lines.push(format!("receipt: {}", p.display())),
        None => lines.push("receipt: none (--unmanaged)".into()),
    }
    for c in &o.completions {
        lines.push(format!("completions: {}", c.display()));
    }
    for note in &o.notes {
        lines.push(format!("note: {note}"));
    }
    if let Some(src) = &o.source_left_behind {
        lines.push(format!(
            "the copy you ran is no longer needed: {}",
            src.display()
        ));
    }
    if o.completions.is_empty() {
        if let Some(argv) = &env.completion_command {
            lines.push(format!(
                "hint: `{} {} <shell>` prints shell completions",
                env.app,
                argv.join(" ")
            ));
        }
    }
    lines
}

fn describe_modification(m: &super::receipt::PathModification) -> String {
    use super::receipt::PathModification as M;
    match m {
        M::EnvFile { file } => format!("created {}", file.display()),
        M::RcLine { file, line } => format!("{}: {line}", file.display()),
        M::FishConf { file } => format!("created {}", file.display()),
        M::WindowsUserPath { key, entry } => format!("HKCU\\{key}\\Path += {entry}"),
    }
}

fn update_command(shared: Shared) -> Command {
    let args = vec![
        ArgSpec {
            name: "target",
            kind: ArgKind::Positional,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Optional,
            help: "stable, latest, or a version (default: the channel the receipt records)",
            ..Default::default()
        },
        flag(
            "check",
            "check",
            "Only report whether an update is available",
        ),
        path_option(
            "from",
            "from",
            "Update from a local release archive, verified against the SHA256SUMS beside it",
        ),
        flag("force", "force", "Reinstall even when already up to date"),
        flag("json", "json", "Print the result as JSON"),
    ];
    leaf(
        "update",
        "Update this binary to the newest release, or to a named version",
        args,
        "Not a self-managed install, refused by policy, unreachable source, or failed verification",
        Arc::new(move |ctx, args| {
            let shared = shared.clone();
            Box::pin(async move {
                let env = shared.env()?;
                let policy = SelfUpdatePolicy::from_context(ctx);
                policy_warnings(ctx, &policy);
                let req = UpdateRequest {
                    target: string_arg(&args, "target"),
                    check: is_set(&args, "check"),
                    from: string_arg(&args, "from").map(PathBuf::from),
                    force: is_set(&args, "force"),
                };
                let outcome = update::update(&env, &shared.options, &policy, &req)
                    .await
                    .map_err(report)?;
                if is_set(&args, "json") {
                    ctx.framework_println(&serde_json::to_string(&outcome)?);
                } else {
                    for line in render_update(&env, &outcome) {
                        ctx.framework_println(&line);
                    }
                }
                Ok(())
            })
        }),
    )
}

pub(crate) fn render_update(env: &InstallEnv, o: &UpdateOutcome) -> Vec<String> {
    let channel = o
        .channel
        .as_deref()
        .map(|c| format!(" ({c})"))
        .unwrap_or_default();
    let mut lines = match o.action {
        UpdateAction::UpToDate => vec![format!("{} {} is up to date{channel}", env.app, o.current)],
        UpdateAction::Available => vec![format!(
            "{} {} is available{channel}; running {}. Run `{} update` to install it",
            env.app, o.target, o.current, env.self_invocation
        )],
        UpdateAction::Updated => vec![format!(
            "{} updated {} -> {} at {}",
            env.app,
            o.current,
            o.target,
            o.binary_path.display()
        )],
    };
    if let Some(prev) = &o.previous {
        lines.push(format!(
            "{} kept at {}; `{} rollback` restores it",
            o.current,
            prev.display(),
            env.self_invocation
        ));
    }
    for note in &o.notes {
        lines.push(format!("note: {note}"));
    }
    lines
}

fn rollback_command(shared: Shared) -> Command {
    leaf(
        "rollback",
        "Swap back to the version the last update replaced",
        vec![flag("json", "json", "Print the result as JSON")],
        "Nothing to roll back to, not a self-managed install, or refused by policy",
        Arc::new(move |ctx, args| {
            let shared = shared.clone();
            Box::pin(async move {
                let env = shared.env()?;
                let policy = SelfUpdatePolicy::from_context(ctx);
                policy_warnings(ctx, &policy);
                let outcome = update::rollback(&env, &policy).map_err(report)?;
                if is_set(&args, "json") {
                    ctx.framework_println(&serde_json::to_string(&outcome)?);
                } else {
                    for line in render_rollback(&env, &outcome) {
                        ctx.framework_println(&line);
                    }
                }
                Ok(())
            })
        }),
    )
}

pub(crate) fn render_rollback(env: &InstallEnv, o: &RollbackOutcome) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{} rolled back {} -> {} at {}",
            env.app,
            o.from,
            o.to,
            o.binary_path.display()
        ),
        format!(
            "{} kept; `{} rollback` again returns to it",
            o.from, env.self_invocation
        ),
    ];
    for note in &o.notes {
        lines.push(format!("note: {note}"));
    }
    lines
}

fn uninstall_command(shared: Shared) -> Command {
    leaf(
        "uninstall",
        "Remove what `self install` recorded in the install receipt",
        vec![flag(
            "purge",
            "purge",
            "Also delete this app's config and data directories (never keychain entries)",
        )],
        "No receipt, a package-manager install, a refused purge, or a filesystem error",
        Arc::new(move |ctx, args| {
            let shared = shared.clone();
            Box::pin(async move {
                let env = shared.env()?;
                let purge = is_set(&args, "purge");
                if purge {
                    for dir in ops::purge_roots(&env)
                        .map_err(report)?
                        .into_iter()
                        .filter(|d| d.exists())
                    {
                        ctx.framework_println(&format!("purging {}", dir.display()));
                    }
                }
                let out = ops::uninstall(&env, purge).map_err(report)?;
                for path in &out.removed {
                    ctx.framework_println(&format!("removed {}", path.display()));
                }
                for dir in &out.purged {
                    ctx.framework_println(&format!("purged {}", dir.display()));
                }
                for kept in &out.kept {
                    ctx.framework_println(&format!("kept {kept}"));
                }
                if out.deletion_deferred {
                    ctx.framework_println("the binary is deleted as soon as this process exits");
                }
                ctx.framework_println(&format!("{} uninstalled", env.app));
                Ok(())
            })
        }),
    )
}

fn status_command(shared: Shared) -> Command {
    leaf(
        "status",
        "Show how this binary is installed, where, and what shadows it",
        vec![flag("json", "json", "Print the status as JSON")],
        "The running executable could not be located",
        Arc::new(move |ctx, args| {
            let shared = shared.clone();
            Box::pin(async move {
                let env = shared.env()?;
                let report = ops::status(&env);
                if is_set(&args, "json") {
                    ctx.framework_println(&serde_json::to_string(&report)?);
                } else {
                    for line in render_status(&report) {
                        ctx.framework_println(&line);
                    }
                }
                Ok(())
            })
        }),
    )
}

pub(crate) fn render_status(r: &StatusReport) -> Vec<String> {
    let mut lines = vec![
        format!("{} {} ({})", r.app, r.version, r.target),
        format!("running binary: {}", r.running_binary.display()),
        format!("install method: {}", r.method.as_str()),
    ];
    match (&r.receipt, &r.receipt_error, &r.receipt_path) {
        (Some(receipt), _, Some(p)) => {
            lines.push(format!(
                "receipt: {} (version {}, channel {}, installed {})",
                p.display(),
                receipt.version,
                receipt.channel,
                receipt.installed_at
            ));
            if let Some(prev) = &receipt.previous_version {
                lines.push(format!("rollback: {prev} is kept beside the binary"));
            }
            if receipt.system {
                lines.push("scope: system (all users)".into());
            }
            if let Some(key) = &receipt.apps_and_features {
                lines.push(format!("Apps & Features: HKCU\\{key}"));
            }
        }
        (None, Some(err), _) => lines.push(format!("receipt: unreadable: {err}")),
        (None, None, Some(p)) => lines.push(format!("receipt: none (expected at {})", p.display())),
        _ => lines.push("receipt: no local data directory".into()),
    }
    if let Some(dir) = &r.bin_dir {
        lines.push(format!(
            "bin dir: {} ({})",
            dir.display(),
            if r.bin_dir_on_path {
                "on PATH"
            } else {
                "not on PATH"
            }
        ));
    }
    match r.copies_on_path.as_slice() {
        [] => lines.push("on PATH: not found".into()),
        [only] => lines.push(format!("on PATH: {}", only.display())),
        [first, rest @ ..] => {
            lines.push(format!("on PATH: {} (used)", first.display()));
            for other in rest {
                lines.push(format!("  shadowed: {}", other.display()));
            }
        }
    }
    for stale in &r.stale_files {
        lines.push(format!("stale: {}", stale.display()));
    }
    if let Some(hint) = &r.upgrade_hint {
        lines.push(format!("managed by a package manager: {hint}"));
    }
    lines
}
