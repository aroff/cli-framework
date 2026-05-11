use crate::doctor::check::CheckSeverity;
use crate::doctor::runner::DoctorReport;

pub fn render_terminal(ctx: &dyn crate::app::AppContext, report: &DoctorReport) {
    #[cfg(feature = "table-advanced")]
    {
        use comfy_table::{Cell, Color, Table};
        let mut table = Table::new();
        table.set_header(vec!["Severity", "ID", "Title", "Message", "Remediation"]);
        for finding in &report.findings {
            let (sev_str, color) = match finding.severity {
                CheckSeverity::Ok => ("[ok]", Color::Green),
                CheckSeverity::Warning => ("[warn]", Color::Yellow),
                CheckSeverity::Error => ("[error]", Color::Red),
                CheckSeverity::Skipped => ("[skip]", Color::DarkGrey),
            };
            table.add_row(vec![
                Cell::new(sev_str).fg(color),
                Cell::new(&finding.check_id),
                Cell::new(&finding.title),
                Cell::new(&finding.message),
                Cell::new(finding.remediation.as_deref().unwrap_or("")),
            ]);
        }
        ctx.framework_println(&format!("{table}"));
    }

    #[cfg(not(feature = "table-advanced"))]
    {
        for finding in &report.findings {
            let sev_str = match finding.severity {
                CheckSeverity::Ok => "[ok]   ",
                CheckSeverity::Warning => "[warn] ",
                CheckSeverity::Error => "[error]",
                CheckSeverity::Skipped => "[skip] ",
            };
            ctx.framework_println(&format!(
                "{} {:20} | {:30} | {}",
                sev_str, finding.check_id, finding.title, finding.message
            ));
            if let Some(ref rem) = finding.remediation {
                ctx.framework_println(&format!("         → {}", rem));
            }
        }
    }

    ctx.framework_println(&format!(
        "\n{} passed, {} warnings, {} errors, {} skipped.",
        report.ok, report.warnings, report.errors, report.skipped
    ));
}

pub fn render_json(ctx: &dyn crate::app::AppContext, report: &DoctorReport) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct JsonReport<'a> {
        findings: &'a Vec<crate::doctor::check::DoctorFinding>,
        summary: JsonSummary,
    }

    #[derive(serde::Serialize)]
    struct JsonSummary {
        ok: usize,
        warnings: usize,
        errors: usize,
        skipped: usize,
    }

    let json_report = JsonReport {
        findings: &report.findings,
        summary: JsonSummary {
            ok: report.ok,
            warnings: report.warnings,
            errors: report.errors,
            skipped: report.skipped,
        },
    };
    ctx.framework_println(&serde_json::to_string_pretty(&json_report)?);
    Ok(())
}
