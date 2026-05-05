use crate::app::context::AppContext;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};

pub struct TmpdirWritableCheck;

impl DoctorCheck for TmpdirWritableCheck {
    fn id(&self) -> &'static str {
        "tmpdir-writable"
    }

    fn title(&self) -> &'static str {
        "Temp directory writable"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Verifies that the system temp directory is accessible and writable")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        Box::pin(async move {
            let tmpdir = std::env::temp_dir();
            let probe_path = tmpdir.join(format!(".cli-framework-probe-{}", std::process::id()));
            match std::fs::write(&probe_path, b"probe") {
                Ok(_) => {
                    let _ = std::fs::remove_file(&probe_path);
                    DoctorFinding {
                        check_id: "tmpdir-writable".to_string(),
                        title: "Temp directory writable".to_string(),
                        severity: CheckSeverity::Ok,
                        message: format!(
                            "Temp directory '{}' is writable",
                            tmpdir.display()
                        ),
                        detail: None,
                        remediation: None,
                    }
                }
                Err(e) => DoctorFinding {
                    check_id: "tmpdir-writable".to_string(),
                    title: "Temp directory writable".to_string(),
                    severity: CheckSeverity::Warning,
                    message: format!(
                        "Temp directory '{}' is not writable: {}",
                        tmpdir.display(),
                        e
                    ),
                    detail: None,
                    remediation: Some(
                        "Check permissions on your temp directory or set TMPDIR to a writable path."
                            .to_string(),
                    ),
                },
            }
        })
    }
}
