use std::collections::HashMap;

/// The only OTLP protocol this crate can actually export with.
///
/// The exporters are built with `.with_http()` and the `http-proto` feature;
/// gRPC would need `grpc-tonic` and a different builder.
pub const SUPPORTED_PROTOCOL: &str = "http/protobuf";

/// Configuration for the OpenTelemetry export pipeline.
///
/// Build one with [`TelemetryConfig::from_env`] (reads the standard `OTEL_*`
/// variables) or with struct literal syntax, then hand it to
/// `AppBuilder::with_telemetry` / `ApiServerBuilder::with_telemetry`. The SDK is
/// only initialised when [`is_active`](Self::is_active) returns `true`.
///
/// Note: several fields are reserved for signals not yet implemented — see the
/// per-field docs and the [module-level limitations](crate::telemetry).
#[derive(Clone)]
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
    /// Headers sent with every OTLP request, for collectors behind
    /// authentication. Read from `OTEL_EXPORTER_OTLP_HEADERS` in the standard
    /// `key=value,key2=value2` form, with percent-decoded values.
    ///
    /// These are credentials. [`TelemetryConfig`]'s `Debug` impl prints the
    /// header **names** and redacts every value, so logging a config cannot leak
    /// a bearer token.
    pub headers: HashMap<String, String>,
    /// OTLP protocol. [`SUPPORTED_PROTOCOL`] is the only accepted value; an
    /// unsupported one is **rejected loudly** at init rather than silently
    /// exported over HTTP anyway. Read from `OTEL_EXPORTER_OTLP_PROTOCOL`.
    pub protocol: String,
    /// Head-sampling ratio in `[0.0, 1.0]`, applied via a parent-based
    /// `TraceIdRatioBased` sampler. `1.0` (default) keeps everything. Read from
    /// `OTEL_TRACES_SAMPLER_ARG`.
    pub sample_ratio: f64,
    /// Whether to export trace spans. When `false` the tracer provider is built
    /// without an exporter, so spans are still created (and still carry context
    /// across services) but nothing is sent. Metrics are unaffected.
    pub traces_enabled: bool,
    /// Whether to export metrics. Honoured: `false` skips building the meter
    /// provider entirely, and the handle's counters/histograms become no-ops.
    pub metrics_enabled: bool,
    /// Reserved: there is no OTLP logs pipeline yet, so this is reader-visible
    /// intent only and changing it has no effect. Implementing it means an
    /// `SdkLoggerProvider` plus the `opentelemetry-appender-tracing` bridge
    /// (spec 020 item 5).
    pub logs_enabled: bool,
    /// Reserved: argument-value capture is not yet implemented. Only argument
    /// *names* are recorded on command spans today.
    pub record_arg_values: bool,
    /// Reserved: allowlist for `record_arg_values`; unused until that lands.
    pub arg_value_allowlist: Vec<String>,
}

/// Hand-written so header values never reach a log.
///
/// `TelemetryConfig` is `Debug` and gets printed in diagnostics; with
/// `OTEL_EXPORTER_OTLP_HEADERS` carrying an `Authorization: Bearer …`, a derived
/// impl would put a live credential wherever that diagnostic goes. Names are
/// kept because "which headers are set" is the useful half for debugging and is
/// not itself sensitive.
impl std::fmt::Debug for TelemetryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("TelemetryConfig")
            .field("enabled", &self.enabled)
            .field("endpoint", &self.endpoint)
            .field("service_name", &self.service_name)
            .field("service_version", &self.service_version)
            .field("headers", &format_args!("{names:?} (values redacted)"))
            .field("protocol", &self.protocol)
            .field("sample_ratio", &self.sample_ratio)
            .field("traces_enabled", &self.traces_enabled)
            .field("metrics_enabled", &self.metrics_enabled)
            .field("logs_enabled", &self.logs_enabled)
            .field("record_arg_values", &self.record_arg_values)
            .field("arg_value_allowlist", &self.arg_value_allowlist)
            .finish()
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: None,
            service_name: None,
            service_version: None,
            headers: HashMap::new(),
            protocol: SUPPORTED_PROTOCOL.to_string(),
            sample_ratio: 1.0,
            traces_enabled: true,
            metrics_enabled: true,
            logs_enabled: true,
            record_arg_values: false,
            arg_value_allowlist: Vec::new(),
        }
    }
}

/// Decode `%XX` escapes in an OTLP header value.
///
/// The OTLP exporter spec allows values to be percent-encoded, which is how a
/// value containing `,` or `=` survives the list format. Without decoding, an
/// operator following the spec gets a literal `%20` inside their credential and
/// a 401 that points nowhere near the cause.
///
/// A malformed escape is left verbatim rather than dropped — mangling a
/// credential silently is worse than passing through something the collector
/// will reject with a clear error.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &value[i + 1..i + 3];
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

/// Parse the `key=value,key2=value2` form of `OTEL_EXPORTER_OTLP_HEADERS`.
///
/// Splits each pair on the FIRST `=` only, because base64 credentials routinely
/// end in `=` padding and splitting on all of them would truncate the value.
/// Entries without a `=`, or with an empty key, are skipped.
fn parse_headers(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), percent_decode(v.trim())))
        })
        .collect()
}

impl TelemetryConfig {
    /// Build a config from the standard `OTEL_*` environment variables.
    ///
    /// Reads `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`,
    /// `OTEL_EXPORTER_OTLP_PROTOCOL`, `OTEL_EXPORTER_OTLP_HEADERS`, and
    /// `OTEL_TRACES_SAMPLER_ARG`. Unset or empty variables leave the [`Default`]
    /// value in place.
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
        if let Ok(v) = std::env::var("OTEL_EXPORTER_OTLP_HEADERS") {
            if !v.is_empty() {
                cfg.headers = parse_headers(&v);
            }
        }
        if let Ok(v) = std::env::var("OTEL_TRACES_SAMPLER_ARG") {
            if let Ok(r) = v.parse::<f64>() {
                cfg.sample_ratio = r;
            }
        }
        cfg
    }

    /// Whether [`protocol`](Self::protocol) is one this crate can export with.
    ///
    /// Checked at init, which refuses to start and says why rather than
    /// exporting over HTTP against an operator's explicit `grpc` setting.
    pub fn protocol_is_supported(&self) -> bool {
        self.protocol.eq_ignore_ascii_case(SUPPORTED_PROTOCOL)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_headers() {
        let h = parse_headers("api-key=secret,x-tenant=acme");
        assert_eq!(h.get("api-key").map(String::as_str), Some("secret"));
        assert_eq!(h.get("x-tenant").map(String::as_str), Some("acme"));
    }

    /// Base64 credentials end in `=` padding. Splitting on every `=` instead of
    /// the first would silently truncate the token and produce a 401 that looks
    /// like a server-side problem.
    #[test]
    fn splits_on_the_first_equals_only() {
        let h = parse_headers("authorization=Bearer YWJjZA==");
        assert_eq!(
            h.get("authorization").map(String::as_str),
            Some("Bearer YWJjZA==")
        );
    }

    #[test]
    fn percent_decodes_values() {
        let h = parse_headers("authorization=Bearer%20abc%2Cdef");
        assert_eq!(
            h.get("authorization").map(String::as_str),
            Some("Bearer abc,def")
        );
    }

    #[test]
    fn skips_malformed_entries() {
        let h = parse_headers("good=1,nonsense,=novalue,also-good=2");
        assert_eq!(h.len(), 2);
        assert!(h.contains_key("good"));
        assert!(h.contains_key("also-good"));
    }

    /// A malformed escape must pass through rather than be mangled or dropped.
    #[test]
    fn leaves_invalid_escapes_verbatim() {
        let h = parse_headers("k=100%off");
        assert_eq!(h.get("k").map(String::as_str), Some("100%off"));
    }

    /// The whole point of the hand-written `Debug`: a credential must not reach
    /// a log through a diagnostic print.
    #[test]
    fn debug_redacts_header_values_but_keeps_names() {
        let cfg = TelemetryConfig {
            headers: parse_headers("authorization=Bearer super-secret-token"),
            ..Default::default()
        };
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("super-secret-token"),
            "header value leaked into Debug output: {rendered}"
        );
        assert!(
            rendered.contains("authorization"),
            "header name should survive for diagnosis: {rendered}"
        );
    }

    #[test]
    fn default_protocol_is_supported() {
        assert!(TelemetryConfig::default().protocol_is_supported());
    }

    #[test]
    fn grpc_protocol_is_rejected() {
        let cfg = TelemetryConfig {
            protocol: "grpc".to_string(),
            ..Default::default()
        };
        assert!(!cfg.protocol_is_supported());
    }

    #[test]
    fn protocol_match_is_case_insensitive() {
        let cfg = TelemetryConfig {
            protocol: "HTTP/PROTOBUF".to_string(),
            ..Default::default()
        };
        assert!(cfg.protocol_is_supported());
    }
}
