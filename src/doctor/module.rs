use crate::app::builder::AppBuilder;
use crate::app::module::Module;
use crate::doctor::check::DoctorCheck;
use anyhow::Result;
use std::sync::Arc;

pub struct DoctorModule {
    checks: Vec<Arc<dyn DoctorCheck>>,
}

impl DoctorModule {
    pub fn new(checks: Vec<Arc<dyn DoctorCheck>>) -> Self {
        Self { checks }
    }
}

impl Module for DoctorModule {
    fn id(&self) -> &'static str {
        "doctor"
    }

    fn register(&self, builder: &mut AppBuilder) -> Result<()> {
        builder.push_doctor_checks(self.checks.clone());
        Ok(())
    }
}
