#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub service_name: Option<String>,
    pub service_version: Option<String>,
    pub protocol: String,
    pub sample_ratio: f64,
    pub traces_enabled: bool,
    pub metrics_enabled: bool,
    pub logs_enabled: bool,
    pub record_arg_values: bool,
    pub arg_value_allowlist: Vec<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: None,
            service_name: None,
            service_version: None,
            protocol: "http/protobuf".to_string(),
            sample_ratio: 1.0,
            traces_enabled: true,
            metrics_enabled: true,
            logs_enabled: true,
            record_arg_values: false,
            arg_value_allowlist: Vec::new(),
        }
    }
}

impl TelemetryConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            if !v.is_empty() {
                cfg.endpoint = Some(v);
            }
        }
        if let Ok(v) = std::env::var("OTEL_SERVICE_NAME") {
            if !v.is_empty() {
                cfg.service_name = Some(v);
            }
        }
        if let Ok(v) = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL") {
            if !v.is_empty() {
                cfg.protocol = v;
            }
        }
        if let Ok(v) = std::env::var("OTEL_TRACES_SAMPLER_ARG") {
            if let Ok(r) = v.parse::<f64>() {
                cfg.sample_ratio = r;
            }
        }
        cfg
    }

    /// Returns true if the SDK should be initialised (enabled + endpoint present + not disabled).
    pub fn is_active(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.endpoint.is_none() {
            return false;
        }
        if std::env::var("OTEL_SDK_DISABLED").as_deref() == Ok("true") {
            return false;
        }
        true
    }
}
