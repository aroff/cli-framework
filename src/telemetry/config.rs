/// Configuration for the OpenTelemetry export pipeline.
///
/// Build one with [`TelemetryConfig::from_env`] (reads the standard `OTEL_*`
/// variables) or with struct literal syntax, then hand it to
/// `AppBuilder::with_telemetry` / `ApiServerBuilder::with_telemetry`. The SDK is
/// only initialised when [`is_active`](Self::is_active) returns `true`.
///
/// Note: several fields are reserved for signals not yet implemented — see the
/// per-field docs and the [module-level limitations](crate::telemetry).
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Master switch. When `false`, the SDK is never initialised regardless of
    /// the other fields.
    pub enabled: bool,
    /// OTLP collector base URL (e.g. `http://localhost:4318`). Export is a no-op
    /// until this is set; `/v1/traces` is appended automatically. Read from
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`.
    pub endpoint: Option<String>,
    /// Overrides the `service.name` resource attribute. When `None`, the app's
    /// own name is used. Read from `OTEL_SERVICE_NAME`.
    pub service_name: Option<String>,
    /// Overrides the `service.version` resource attribute. When `None`, the
    /// app's own version is used.
    pub service_version: Option<String>,
    /// OTLP protocol; `http/protobuf` (default) is the only value wired today.
    pub protocol: String,
    /// Head-sampling ratio in `[0.0, 1.0]`, applied via a parent-based
    /// `TraceIdRatioBased` sampler. `1.0` (default) keeps everything. Read from
    /// `OTEL_TRACES_SAMPLER_ARG`.
    pub sample_ratio: f64,
    /// Whether to export trace spans. Traces are the only signal exported today.
    pub traces_enabled: bool,
    /// Reserved: metrics export is not yet implemented (see module docs).
    pub metrics_enabled: bool,
    /// Reserved: log export is not yet implemented (see module docs).
    pub logs_enabled: bool,
    /// Reserved: argument-value capture is not yet implemented. Only argument
    /// *names* are recorded on command spans today.
    pub record_arg_values: bool,
    /// Reserved: allowlist for `record_arg_values`; unused until that lands.
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
    /// Build a config from the standard `OTEL_*` environment variables.
    ///
    /// Reads `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`,
    /// `OTEL_EXPORTER_OTLP_PROTOCOL`, and `OTEL_TRACES_SAMPLER_ARG`. Unset or
    /// empty variables leave the [`Default`] value in place.
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
