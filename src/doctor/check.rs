use crate::app::context::AppContext;
use std::future::Future;
use std::pin::Pin;

pub type DoctorFuture = Pin<Box<dyn Future<Output = DoctorFinding> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckSeverity {
    Ok,
    Warning,
    Error,
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorFinding {
    pub check_id: String,
    pub title: String,
    pub severity: CheckSeverity,
    pub message: String,
    pub detail: Option<String>,
    pub remediation: Option<String>,
}

pub trait DoctorCheck: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn description(&self) -> Option<&'static str> {
        None
    }
    fn run(&self, ctx: &dyn AppContext) -> DoctorFuture;
}
