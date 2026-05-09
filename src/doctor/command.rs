use crate::command::Command;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use crate::doctor::render::{render_json, render_terminal};
use crate::doctor::runner::DoctorReport;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use std::sync::Arc;
use tokio::task::JoinSet;

pub fn create_doctor_command(checks: Vec<Arc<dyn DoctorCheck>>) -> Command {
    Command {
        id: "doctor",
        summary: "Run diagnostics and report environment health",
        syntax: Some("doctor [--json] [--check <id>]"),
        category: Some("ops"),
        spec: Some(doctor_spec()),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |ctx, args| {
            let is_json = args.named.get("json").map(|v| v == "true").unwrap_or(false);
            let filter_id = args.named.get("check").cloned();

            // Sync phase: call check.run(ctx) to produce 'static DoctorFutures.
            // ctx is only borrowed here — DoctorFuture is 'static so it does not
            // capture ctx, meaning ctx is released before any .await below.
            let mut pending: Vec<(usize, DoctorFuture)> = Vec::new();
            let mut pre_findings: Vec<DoctorFinding> = Vec::new();
            let total: usize;

            if let Some(ref id) = filter_id {
                total = 1;
                match checks.iter().find(|c| c.id() == id.as_str()) {
                    Some(check) => pending.push((0, check.run(ctx))),
                    None => pre_findings.push(DoctorFinding {
                        check_id: id.clone(),
                        title: format!("Unknown check '{}'", id),
                        severity: CheckSeverity::Error,
                        message: format!("DR003: unknown check id '{}'", id),
                        detail: None,
                        remediation: Some(
                            "Run 'doctor' without --check to see all available checks.".to_string(),
                        ),
                    }),
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

                let mut findings: Vec<DoctorFinding> =
                    slots.into_iter().filter_map(|s| s).collect();
                findings.extend(pre_findings);

                let report = DoctorReport::from_findings(findings);
                if is_json {
                    render_json(&report)?;
                } else {
                    render_terminal(&report);
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

fn doctor_spec() -> Arc<CommandSpec> {
    Arc::new(CommandSpec {
        summary: "Run diagnostics and report environment health",
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
            },
        ],
        ..Default::default()
    })
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
