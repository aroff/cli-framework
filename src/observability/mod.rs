// Optional observability module
#[cfg(feature = "observability")]
pub mod opentelemetry;

pub use opentelemetry::{init_observability, ObservabilityConfig};
