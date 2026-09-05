//! The six telemetry doctor checks (Task 25).
//!
//! Each check answers one question a person asks when telemetry looks wrong:
//! did the subscriber install, is the settings store usable, can the
//! collector be reached, is there a policy managing this install, what
//! attribution mode is active, and did an environment variable typo silently
//! do nothing. Six fixed ids, so a bug report can name one exactly:
//! `telemetry.subscriber`, `telemetry.store`, `telemetry.endpoint`,
//! `telemetry.policy`, `telemetry.identity`, `telemetry.env`.
//!
//! **A check whose subject is absent reports [`CheckSeverity::Skipped`], not
//! `Ok`.** "No policy client is configured" and "the policy client says
//! everything is fine" are different states, and a person debugging needs to
//! tell them apart — so [`PolicyCheck`] (no policy-client wiring exists yet
//! in this crate) and an unconfigured [`EndpointCheck`] both report
//! `Skipped` rather than a falsely reassuring `Ok`.
//!
//! **No finding ever names the install or session identifier.** Doctor
//! output gets pasted into bug reports; a check that wants to talk about
//! identity reports the *mode* (anonymous / pseudonymous / identified), never
//! the value.
//!
//! **Every `Warning`/`Error` finding carries a remediation.** A finding that
//! says something is wrong without saying what to do about it is half a
//! finding.
//!
//! Each struct holds only the `Arc<TelemetryPolicy>` and/or
//! `Arc<StartupReport>` fields it actually reads, rather than both
//! unconditionally: a stored field nothing ever reads is a `dead_code`
//! warning under this workspace's `-D warnings`, and there is nothing for,
//! say, [`PolicyCheck`] to do with either one while it has no policy client
//! to consult.
//!
//! `run` never captures the borrowed `ctx: &dyn AppContext` it is handed —
//! [`DoctorFuture`] is `'static`, so, exactly like
//! [`crate::doctor::builtin::EnvRequiredCheck`], each check clones its own
//! owned state into the `async move` block and ignores `ctx` entirely.

use crate::app::context::AppContext;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use crate::telemetry::axes::Attribution;
use crate::telemetry::policy::TelemetryPolicy;
use crate::telemetry::startup::StartupReport;
use crate::telemetry::store::StoreState;
use crate::telemetry::subscriber::SubscriberOutcome;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

/// Reachability of the configured OTLP endpoint.
///
/// The only check here that does I/O, bounded at two seconds. A doctor
/// command that hangs on a firewalled collector is a doctor command people
/// stop running, and an unreachable collector on a laptop is normal — so a
/// timeout is a `Warning` and never an `Error`.
const ENDPOINT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Split `host:port`, tolerating a leading `scheme://` and a trailing path —
/// `TelemetryPolicy::endpoint` is usually a full OTLP URL
/// (`http://collector:4318`), and the question this check asks is about the
/// socket, not the path.
fn host_port(endpoint: &str) -> Option<(String, u16)> {
    let without_scheme = match endpoint.split_once("://") {
        Some((_, rest)) => rest,
        None => endpoint,
    };
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let (host, port) = authority.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    Some((host.to_string(), port))
}

// ── telemetry.subscriber ────────────────────────────────────────────────────

pub struct SubscriberCheck {
    report: Arc<StartupReport>,
}

impl DoctorCheck for SubscriberCheck {
    fn id(&self) -> &'static str {
        "telemetry.subscriber"
    }

    fn title(&self) -> &'static str {
        "Tracing subscriber"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Whether the framework's tracing subscriber was installed")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let report = self.report.clone();
        Box::pin(async move {
            let (severity, message, remediation) = match report.subscriber {
                SubscriberOutcome::Installed => (
                    CheckSeverity::Ok,
                    "the framework installed the tracing subscriber; spans and events reach \
                     the OTel bridge"
                        .to_string(),
                    None,
                ),
                SubscriberOutcome::ForeignSubscriber => (
                    CheckSeverity::Warning,
                    "another crate already installed a tracing subscriber; traces and logs \
                     are not exported, but metrics are unaffected"
                        .to_string(),
                    Some(
                        "compose cli_framework::telemetry::init::otel_layer into your own \
                         subscriber instead of calling with_telemetry()"
                            .to_string(),
                    ),
                ),
            };
            DoctorFinding {
                check_id: "telemetry.subscriber".to_string(),
                title: "Tracing subscriber".to_string(),
                severity,
                message,
                detail: None,
                remediation,
            }
        })
    }
}

// ── telemetry.store ─────────────────────────────────────────────────────────

pub struct StoreCheck {
    report: Arc<StartupReport>,
}

impl DoctorCheck for StoreCheck {
    fn id(&self) -> &'static str {
        "telemetry.store"
    }

    fn title(&self) -> &'static str {
        "Settings store"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Whether the telemetry settings file could be opened")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let report = self.report.clone();
        Box::pin(async move {
            let (severity, message, remediation) = match &report.store {
                StoreState::Ready(path) => (
                    CheckSeverity::Ok,
                    format!(
                        "the telemetry settings file is available at {}",
                        path.display()
                    ),
                    None,
                ),
                StoreState::Unavailable(reason) => (
                    CheckSeverity::Warning,
                    format!("the telemetry settings file is unavailable: {reason}"),
                    Some(
                        "check that the configuration directory exists and is writable by \
                         this user"
                            .to_string(),
                    ),
                ),
            };
            DoctorFinding {
                check_id: "telemetry.store".to_string(),
                title: "Settings store".to_string(),
                severity,
                message,
                detail: None,
                remediation,
            }
        })
    }
}

// ── telemetry.endpoint ──────────────────────────────────────────────────────

pub struct EndpointCheck {
    policy: Arc<TelemetryPolicy>,
}

impl DoctorCheck for EndpointCheck {
    fn id(&self) -> &'static str {
        "telemetry.endpoint"
    }

    fn title(&self) -> &'static str {
        "Collector reachability"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Whether the configured OTLP collector can be reached, bounded at 2 seconds")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let policy = self.policy.clone();
        Box::pin(async move {
            let Some(endpoint) = policy.endpoint.clone() else {
                return DoctorFinding {
                    check_id: "telemetry.endpoint".to_string(),
                    title: "Collector reachability".to_string(),
                    severity: CheckSeverity::Skipped,
                    message: "no OTLP endpoint is configured".to_string(),
                    detail: None,
                    remediation: None,
                };
            };

            let remediation = format!(
                "confirm the collector at {endpoint} is reachable from this host, or unset \
                 the configured endpoint if this app should not export telemetry"
            );

            // Resolution *and* connection both happen inside the blocking
            // task: `TcpStream::connect_timeout` only bounds the handshake,
            // not the DNS lookup that has to happen first to produce the
            // `SocketAddr` it takes, and an unbounded lookup would defeat
            // the whole "bounded at two seconds" point of this check. The
            // outer `tokio::time::timeout` bounds what the caller waits for
            // regardless of which half was slow.
            let target = endpoint.clone();
            let handle = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let (host, port) = host_port(&target).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "endpoint has no parseable host:port",
                    )
                })?;
                let addr = (host.as_str(), port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "endpoint did not resolve to any address",
                        )
                    })?;
                std::net::TcpStream::connect_timeout(&addr, ENDPOINT_PROBE_TIMEOUT)?;
                Ok(())
            });

            match tokio::time::timeout(ENDPOINT_PROBE_TIMEOUT, handle).await {
                Ok(Ok(Ok(()))) => DoctorFinding {
                    check_id: "telemetry.endpoint".to_string(),
                    title: "Collector reachability".to_string(),
                    severity: CheckSeverity::Ok,
                    message: format!("reached the configured collector at {endpoint}"),
                    detail: None,
                    remediation: None,
                },
                Ok(Ok(Err(e))) => DoctorFinding {
                    check_id: "telemetry.endpoint".to_string(),
                    title: "Collector reachability".to_string(),
                    severity: CheckSeverity::Warning,
                    message: format!("could not reach the configured collector at {endpoint}: {e}"),
                    detail: None,
                    remediation: Some(remediation),
                },
                Ok(Err(_join_error)) => DoctorFinding {
                    check_id: "telemetry.endpoint".to_string(),
                    title: "Collector reachability".to_string(),
                    severity: CheckSeverity::Warning,
                    message: format!(
                        "could not check reachability of the configured collector at {endpoint}"
                    ),
                    detail: None,
                    remediation: Some(remediation),
                },
                Err(_elapsed) => DoctorFinding {
                    check_id: "telemetry.endpoint".to_string(),
                    title: "Collector reachability".to_string(),
                    severity: CheckSeverity::Warning,
                    message: format!(
                        "timed out after 2s reaching the configured collector at {endpoint}"
                    ),
                    detail: None,
                    remediation: Some(remediation),
                },
            }
        })
    }
}

// ── telemetry.policy ────────────────────────────────────────────────────────

/// Always [`CheckSeverity::Skipped`]: this crate has no organizational
/// policy-client wiring yet (that is a later PR's scope), and "no policy
/// client is configured" must not be reported as the falsely reassuring `Ok`
/// that "the policy client says everything is fine" would look like.
pub struct PolicyCheck;

impl DoctorCheck for PolicyCheck {
    fn id(&self) -> &'static str {
        "telemetry.policy"
    }

    fn title(&self) -> &'static str {
        "Managed policy"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Whether an organizational policy client is managing this install")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        Box::pin(async move {
            DoctorFinding {
                check_id: "telemetry.policy".to_string(),
                title: "Managed policy".to_string(),
                severity: CheckSeverity::Skipped,
                message: "telemetry policy is not managed by an organizational policy client \
                          in this build"
                    .to_string(),
                detail: None,
                remediation: None,
            }
        })
    }
}

// ── telemetry.identity ──────────────────────────────────────────────────────

pub struct IdentityCheck {
    policy: Arc<TelemetryPolicy>,
}

impl DoctorCheck for IdentityCheck {
    fn id(&self) -> &'static str {
        "telemetry.identity"
    }

    fn title(&self) -> &'static str {
        "Attribution"
    }

    fn description(&self) -> Option<&'static str> {
        Some("The attribution mode in effect, never the identifier itself")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let policy = self.policy.clone();
        Box::pin(async move {
            // Names the *mode*, never `policy.install_id`'s value: doctor
            // output gets pasted into bug reports.
            let message = match policy.attribution {
                Attribution::Anonymous => {
                    "attribution is anonymous; no install or session identifier is sent".to_string()
                }
                Attribution::Pseudonymous => {
                    "attribution is pseudonymous; a locally minted install identifier is in use"
                        .to_string()
                }
                Attribution::Identified => {
                    "attribution is identified; the application's identity hook supplies a \
                     principal"
                        .to_string()
                }
            };
            DoctorFinding {
                check_id: "telemetry.identity".to_string(),
                title: "Attribution".to_string(),
                severity: CheckSeverity::Ok,
                message,
                detail: None,
                remediation: None,
            }
        })
    }
}

// ── telemetry.env ───────────────────────────────────────────────────────────

pub struct EnvCheck {
    report: Arc<StartupReport>,
}

impl DoctorCheck for EnvCheck {
    fn id(&self) -> &'static str {
        "telemetry.env"
    }

    fn title(&self) -> &'static str {
        "Environment variables"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Whether every set <APP>_TELEMETRY_* variable matched a known setting")
    }

    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        let report = self.report.clone();
        Box::pin(async move {
            if report.unmatched_env.is_empty() {
                DoctorFinding {
                    check_id: "telemetry.env".to_string(),
                    title: "Environment variables".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "every <APP>_TELEMETRY_* variable that was set matched a known \
                              setting"
                        .to_string(),
                    detail: None,
                    remediation: None,
                }
            } else {
                DoctorFinding {
                    check_id: "telemetry.env".to_string(),
                    title: "Environment variables".to_string(),
                    severity: CheckSeverity::Warning,
                    message: format!(
                        "these variables were set but matched no known telemetry setting: {}",
                        report.unmatched_env.join(", ")
                    ),
                    detail: None,
                    remediation: Some(
                        "check for a typo against the documented <APP>_TELEMETRY_* variable \
                         names"
                            .to_string(),
                    ),
                }
            }
        })
    }
}

/// The six telemetry doctor checks, ready to hand to
/// `AppBuilder::push_doctor_checks`.
///
/// `Arc`, not `Box`: `push_doctor_checks`/`register_doctor_checks`
/// (`src/app/builder.rs`) both take `Vec<Arc<dyn DoctorCheck>>`, and that
/// wiring — calling this function from `AppBuilder::build` — is later PR's
/// job (the startup wiring lands once every piece exists at once); this PR
/// only has to produce the checks in the shape that hook already expects.
pub fn telemetry_checks(
    policy: Arc<TelemetryPolicy>,
    report: Arc<StartupReport>,
) -> Vec<Arc<dyn DoctorCheck>> {
    vec![
        Arc::new(SubscriberCheck {
            report: report.clone(),
        }),
        Arc::new(StoreCheck {
            report: report.clone(),
        }),
        Arc::new(EndpointCheck {
            policy: policy.clone(),
        }),
        Arc::new(PolicyCheck),
        Arc::new(IdentityCheck { policy }),
        Arc::new(EnvCheck { report }),
    ]
}
