//! Doctor checks for an installation: `install.on_path`, `install.shadowed`,
//! `install.receipt` and `install.stale_files`.
//!
//! Each check snapshots the process with [`InstallEnv::detect`], runs
//! [`ops::status`](super::ops::status) and maps the report to a finding. The
//! mapping functions are pure so tests feed them hand-built reports.

use super::env::InstallEnv;
use super::layout::same_path;
use super::ops::{self, StatusReport};
use crate::app::context::AppContext;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use std::sync::Arc;

type Mapper = fn(&StatusReport, &str, &InstallEnv) -> (CheckSeverity, String, Option<String>);

struct InstallCheck {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    app: &'static str,
    version: &'static str,
    self_invocation: String,
    map: Mapper,
}

impl DoctorCheck for InstallCheck {
    fn id(&self) -> &'static str {
        self.id
    }

    fn title(&self) -> &'static str {
        self.title
    }

    fn description(&self) -> Option<&'static str> {
        Some(self.description)
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let (id, title, app, version, map) =
            (self.id, self.title, self.app, self.version, self.map);
        let invocation = self.self_invocation.clone();
        Box::pin(async move {
            let (severity, message, remediation) = match InstallEnv::detect(app, version) {
                Ok(env) => map(&ops::status(&env), &invocation, &env),
                Err(e) => (
                    CheckSeverity::Skipped,
                    format!("cannot locate the running executable: {e}"),
                    None,
                ),
            };
            DoctorFinding {
                check_id: id.to_string(),
                title: title.to_string(),
                severity,
                message,
                detail: None,
                remediation,
            }
        })
    }
}

/// The installation checks, for the builder to add to `doctor`.
pub fn install_checks(
    app: &'static str,
    version: &'static str,
    self_invocation: String,
) -> Vec<Arc<dyn DoctorCheck>> {
    let make = |id, title, description, map: Mapper| -> Arc<dyn DoctorCheck> {
        Arc::new(InstallCheck {
            id,
            title,
            description,
            app,
            version,
            self_invocation: self_invocation.clone(),
            map,
        })
    };
    vec![
        make(
            "install.on_path",
            "Install directory on PATH",
            "The directory holding the installed binary is on PATH",
            on_path_finding,
        ),
        make(
            "install.shadowed",
            "Binary not shadowed",
            "The first copy on PATH is the installed one",
            shadowed_finding,
        ),
        make(
            "install.receipt",
            "Install receipt",
            "The install receipt is readable and matches the binary",
            receipt_finding,
        ),
        make(
            "install.stale_files",
            "No stale install files",
            "No leftover .old binaries, staging files or installer temp dirs",
            stale_files_finding,
        ),
    ]
}

type Finding = (CheckSeverity, String, Option<String>);

fn managed_elsewhere(r: &StatusReport) -> Option<Finding> {
    if r.receipt.is_none() && r.method.is_package_manager() {
        Some((
            CheckSeverity::Ok,
            format!("installed with {}", r.method.as_str()),
            None,
        ))
    } else {
        None
    }
}

pub(crate) fn on_path_finding(r: &StatusReport, invocation: &str, env: &InstallEnv) -> Finding {
    if let Some(f) = managed_elsewhere(r) {
        return f;
    }
    let Some(receipt) = &r.receipt else {
        return (
            CheckSeverity::Skipped,
            "not installed with self install".into(),
            None,
        );
    };
    if r.bin_dir_on_path {
        return (
            CheckSeverity::Ok,
            format!("{} is on PATH", receipt.bin_dir.display()),
            None,
        );
    }
    let remediation = if env.os.is_windows() {
        format!(
            "open a new terminal; if it is still missing, run `{invocation} install` to add {} to the user Path",
            receipt.bin_dir.display()
        )
    } else {
        format!(
            "open a new terminal or source the env file in {}; if it is still missing, run `{invocation} install`",
            receipt.bin_dir.display()
        )
    };
    (
        CheckSeverity::Warning,
        format!("{} is not on PATH", receipt.bin_dir.display()),
        Some(remediation),
    )
}

pub(crate) fn shadowed_finding(r: &StatusReport, _invocation: &str, env: &InstallEnv) -> Finding {
    let expected = r
        .receipt
        .as_ref()
        .map(|rc| rc.binary_path.clone())
        .unwrap_or_else(|| r.running_binary.clone());
    match r.copies_on_path.first() {
        None => (
            CheckSeverity::Skipped,
            format!("{} is not on PATH", r.app),
            None,
        ),
        Some(first) if same_path(first, &expected, env.os) => {
            if r.copies_on_path.len() > 1 {
                (
                    CheckSeverity::Ok,
                    format!(
                        "{} is used first; {} other copies later on PATH",
                        first.display(),
                        r.copies_on_path.len() - 1
                    ),
                    None,
                )
            } else {
                (
                    CheckSeverity::Ok,
                    format!("{} is used", first.display()),
                    None,
                )
            }
        }
        Some(first) => (
            CheckSeverity::Warning,
            format!(
                "`{}` resolves to {}, not {}",
                r.app,
                first.display(),
                expected.display()
            ),
            Some(format!(
                "remove the other copy at {} or move {} earlier on PATH",
                first.display(),
                expected
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            )),
        ),
    }
}

pub(crate) fn receipt_finding(r: &StatusReport, invocation: &str, _env: &InstallEnv) -> Finding {
    if let Some(err) = &r.receipt_error {
        return (
            CheckSeverity::Warning,
            format!("install receipt is unreadable: {err}"),
            Some(format!("run `{invocation} install --force` to rewrite it")),
        );
    }
    if let Some(f) = managed_elsewhere(r) {
        return f;
    }
    let Some(receipt) = &r.receipt else {
        return (
            CheckSeverity::Skipped,
            "no install receipt; not installed with self install".into(),
            None,
        );
    };
    if !receipt.binary_path.is_file() {
        return (
            CheckSeverity::Warning,
            format!(
                "the receipt records {}, which no longer exists",
                receipt.binary_path.display()
            ),
            Some(format!(
                "run `{invocation} install` to reinstall, or `{invocation} uninstall` to clean up"
            )),
        );
    }
    (
        CheckSeverity::Ok,
        format!(
            "{} {} installed at {}",
            receipt.app,
            receipt.version,
            receipt.binary_path.display()
        ),
        None,
    )
}

pub(crate) fn stale_files_finding(
    r: &StatusReport,
    _invocation: &str,
    _env: &InstallEnv,
) -> Finding {
    if r.stale_files.is_empty() {
        return (CheckSeverity::Ok, "no leftover files".into(), None);
    }
    let list = r
        .stale_files
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    (
        CheckSeverity::Warning,
        format!("leftover install files: {list}"),
        Some(
            "delete them; no running program needs them (on Windows, close running copies first)"
                .into(),
        ),
    )
}
