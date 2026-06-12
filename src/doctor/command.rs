use crate::app::diagnostic_reporter::DiagnosticReporter;
use crate::app::UsageError;
use crate::command::Command;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use crate::doctor::render::{render_json, render_terminal};
use crate::doctor::runner::DoctorReport;
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::E_UNKNOWN_DOCTOR_CHECK;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::sync::Arc;
use tokio::task::JoinSet;

pub fn create_doctor_command(checks: Vec<Arc<dyn DoctorCheck>>) -> Command {
    Command {
        id: Arc::from("doctor"),
        spec: Arc::new(CommandSpec {
            summary: "Run diagnostics and report environment health",
            syntax: Some("doctor [--json] [--check <id>]"),
            category: Some("ops"),
            ..doctor_spec()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, args| {
            let is_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
            let filter_id: Option<String> = args.get("check").and_then(|v| {
                if let ArgValue::Str(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });

            // Sync phase: call check.run(ctx) to produce 'static DoctorFutures.
            // ctx is only borrowed here — DoctorFuture is 'static so it does not
            // capture ctx, meaning ctx is released before any .await below.
            let mut pending: Vec<(usize, DoctorFuture)> = Vec::new();
            let total: usize;

            if let Some(ref id) = filter_id {
                match checks.iter().find(|c| c.id() == id.as_str()) {
                    Some(check) => {
                        total = 1;
                        pending.push((0, check.run(ctx)));
                    }
                    None => {
                        // Usage error: unknown check id — report and return exit 2.
                        let err_msg = format!("unknown check id '{}'", id);
                        DiagnosticReporter::report(&Diagnostic {
                            code: E_UNKNOWN_DOCTOR_CHECK,
                            category: DiagnosticCategory::Validation,
                            message: err_msg.clone(),
                            suggestion: Some(
                                "Run 'doctor' without --check to see available checks.".to_string(),
                            ),
                            span: Some(id.clone()),
                        });
                        return Box::pin(
                            async move { Err(anyhow::Error::new(UsageError(err_msg))) },
                        );
                    }
                }
            } else {
                total = checks.len();
                for (idx, check) in checks.iter().enumerate() {
                    pending.push((idx, check.run(ctx)));
                }
            }

            // Async phase: await the pre-collected 'static futures — ctx not needed.
            Box::pin(async move {
                let mut slots: Vec<Option<DoctorFinding>> = vec![None; total];

                let mut join_set: JoinSet<(usize, DoctorFinding)> = JoinSet::new();
                for (idx, future) in pending {
                    join_set.spawn(async move { (idx, future.await) });
                }

                while let Some(result) = join_set.join_next().await {
                    match result {
                        Ok((idx, finding)) => {
                            if idx < slots.len() {
                                slots[idx] = Some(finding);
                            }
                        }
                        Err(join_err) => {
                            let panic_str = extract_panic_message(join_err);
                            for slot in slots.iter_mut() {
                                if slot.is_none() {
                                    *slot = Some(DoctorFinding {
                                        check_id: "unknown".to_string(),
                                        title: "Panicked check".to_string(),
                                        severity: CheckSeverity::Error,
                                        message: format!("DR002: check panicked: {}", panic_str),
                                        detail: None,
                                        remediation: None,
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }

                let findings: Vec<DoctorFinding> = slots.into_iter().flatten().collect();

                let report = DoctorReport::from_findings(findings);
                if is_json {
                    render_json(ctx, &report)?;
                } else {
                    render_terminal(ctx, &report);
                }

                if report.errors > 0 {
                    Err(anyhow::anyhow!("doctor: {} error(s) found", report.errors))
                } else {
                    Ok(())
                }
            })
        }),
    }
}

fn doctor_spec() -> CommandSpec {
    CommandSpec {
        args: vec![
            ArgSpec {
                name: "json",
                kind: ArgKind::Flag,
                short: Some('j'),
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Emit JSON output to stdout",
                ..Default::default()
            },
            ArgSpec {
                name: "check",
                kind: ArgKind::Option,
                short: Some('c'),
                long: None,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Run only the check with this ID",
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn extract_panic_message(e: tokio::task::JoinError) -> String {
    if e.is_panic() {
        let p = e.into_panic();
        if let Some(s) = p.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = p.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        }
    } else {
        "task was cancelled".to_string()
    }
}
