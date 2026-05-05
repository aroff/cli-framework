use cli_framework::doctor::builtin::{EnvRequiredCheck, TmpdirWritableCheck};
use cli_framework::doctor::{
    CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture, DoctorModule,
};
use cli_framework::prelude::*;
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

struct AlwaysOkCheck;

impl DoctorCheck for AlwaysOkCheck {
    fn id(&self) -> &'static str {
        "always-ok"
    }
    fn title(&self) -> &'static str {
        "Framework sanity check"
    }
    fn description(&self) -> Option<&'static str> {
        Some("Verifies the doctor framework is operational")
    }
    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        Box::pin(async {
            DoctorFinding {
                check_id: "always-ok".to_string(),
                title: "Framework sanity check".to_string(),
                severity: CheckSeverity::Ok,
                message: "Doctor framework is operational".to_string(),
                detail: None,
                remediation: None,
            }
        })
    }
}

struct MyTokenCheck;

impl DoctorCheck for MyTokenCheck {
    fn id(&self) -> &'static str {
        "my-token"
    }
    fn title(&self) -> &'static str {
        "Application token"
    }
    fn description(&self) -> Option<&'static str> {
        Some("Checks that MY_APP_TOKEN is configured")
    }
    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        Box::pin(async {
            if std::env::var("MY_APP_TOKEN")
                .map(|v| !v.is_empty())
                .unwrap_or(false)
            {
                DoctorFinding {
                    check_id: "my-token".to_string(),
                    title: "Application token".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "MY_APP_TOKEN is configured".to_string(),
                    detail: None,
                    remediation: None,
                }
            } else {
                DoctorFinding {
                    check_id: "my-token".to_string(),
                    title: "Application token".to_string(),
                    severity: CheckSeverity::Warning,
                    message: "MY_APP_TOKEN is not set".to_string(),
                    detail: Some("Some features may be unavailable without a token.".to_string()),
                    remediation: Some("Set MY_APP_TOKEN to your API token.".to_string()),
                }
            }
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let checks: Vec<Arc<dyn DoctorCheck>> = vec![
        Arc::new(AlwaysOkCheck),
        Arc::new(MyTokenCheck),
        Arc::new(TmpdirWritableCheck),
        Arc::new(EnvRequiredCheck::new(&["HOME", "PATH"])),
    ];

    let mut app = AppBuilder::new()
        .with_version("with-doctor", "0.1.0")
        .register_module(DoctorModule::new(checks))?
        .build(AppCtx)?;

    app.run().await
}
