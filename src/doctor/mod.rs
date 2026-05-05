pub mod builtin;
pub mod check;
pub mod command;
pub mod module;
pub mod render;
pub mod runner;

pub use check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
pub use module::DoctorModule;
pub use runner::{DoctorReport, DoctorRunner};

#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    #[error("DR001: duplicate check id '{0}'")]
    DuplicateCheckId(String),
    #[error("DR002: check '{0}' panicked: {1}")]
    CheckPanicked(String, String),
    #[error("DR003: unknown check id '{0}'")]
    UnknownCheckId(String),
}
