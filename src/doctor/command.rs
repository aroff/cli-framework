use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs};
use crate::doctor::check::DoctorCheck;
use crate::doctor::render::{render_json, render_terminal};
use crate::doctor::runner::DoctorRunner;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use std::sync::Arc;

pub fn create_doctor_command(checks: Vec<Arc<dyn DoctorCheck>>) -> Command {
    Command {
        id: "doctor",
        summary: "Run diagnostics and report environment health",
        syntax: Some("doctor [--json] [--check <id>]"),
        category: Some("ops"),
        spec: Some(doctor_spec()),
        validator: None,
        execute: Arc::new(move |_ctx, args| {
            let runner = DoctorRunner::from_checks(checks.clone());
            Box::pin(async move { execute_doctor(runner, args).await })
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

async fn execute_doctor(runner: DoctorRunner, args: CommandArgs) -> anyhow::Result<()> {
    let is_json = args.named.get("json").map(|v| v == "true").unwrap_or(false);
    let filter_id = args.named.get("check").cloned();

    let report = if let Some(ref id) = filter_id {
        runner.run_filtered(&NoopCtx, &[id.as_str()]).await
    } else {
        runner.run_all(&NoopCtx).await
    };

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
}

struct NoopCtx;
impl AppContext for NoopCtx {}
