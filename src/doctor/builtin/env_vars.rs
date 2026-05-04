use crate::app::context::AppContext;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};

pub struct EnvRequiredCheck {
    vars: Vec<&'static str>,
}

impl EnvRequiredCheck {
    pub fn new(vars: &[&'static str]) -> Self {
        Self {
            vars: vars.to_vec(),
        }
    }
}

impl DoctorCheck for EnvRequiredCheck {
    fn id(&self) -> &'static str {
        "env-required"
    }

    fn title(&self) -> &'static str {
        "Required environment variables"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Checks that all required environment variables are set and non-empty")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let vars = self.vars.clone();
        Box::pin(async move {
            let mut missing = Vec::new();
            for var in &vars {
                match std::env::var(var) {
                    Ok(v) if !v.is_empty() => {}
                    _ => missing.push(*var),
                }
            }
            if missing.is_empty() {
                DoctorFinding {
                    check_id: "env-required".to_string(),
                    title: "Required environment variables".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "All required environment variables are set".to_string(),
                    detail: None,
                    remediation: None,
                }
            } else {
                DoctorFinding {
                    check_id: "env-required".to_string(),
                    title: "Required environment variables".to_string(),
                    severity: CheckSeverity::Error,
                    message: format!(
                        "Missing required environment variables: {}",
                        missing.join(", ")
                    ),
                    detail: None,
                    remediation: Some(format!(
                        "Set the following environment variables: {}",
                        missing.join(", ")
                    )),
                }
            }
        })
    }
}
