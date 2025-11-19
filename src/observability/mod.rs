// Optional observability module
#[cfg(feature = "observability")]
pub mod opentelemetry;

pub use opentelemetry::{ObservabilityConfig, init_observability};

