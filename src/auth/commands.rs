//! Built-in authentication commands: `auth login`, `auth logout`, `auth status`, `auth token`.

use crate::app::diagnostic_reporter::DiagnosticReporter;
use crate::auth::AuthError;
use crate::command::Command;
use crate::command::CommandRegistry;
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{AUTH001, AUTH002, AUTH003};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use crate::spec::value::ArgValue;
use std::sync::Arc;

/// Register the `auth` group and its four leaf commands.
pub(crate) fn register_auth_commands(
    registry: &mut CommandRegistry,
    app_name: &'static str,
) -> anyhow::Result<()> {
    // Register the group node.
    let group_path = CommandPath::root_for("auth");
    registry
        .register_group(
            &group_path,
            GroupMetadata {
                summary: "Authentication commands",
                hidden: false,
            },
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // ── auth login ───────────────────────────────────────────────────────────
    let login_cmd = Command {
        id: Arc::from("login"),
        spec: Arc::new(CommandSpec {
            summary: "Authenticate and cache a token",
            category: Some("Authentication"),
            exit_codes: vec![
                crate::spec::command_tree::ExitCodeEntry {
                    code: 0,
                    description: "Login succeeded",
                },
                crate::spec::command_tree::ExitCodeEntry {
                    code: 1,
                    description: "Authentication error",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, _args| {
            Box::pin(async move {
                let provider = match ctx.opt_token_provider() {
                    Some(p) => p,
                    None => {
                        DiagnosticReporter::write_plain("auth: no token provider configured\n");
                        return Err(anyhow::anyhow!("AUTH_NO_PROVIDER"));
                    }
                };
                match provider.login().await {
                    Ok(()) => {
                        DiagnosticReporter::write_plain("Logged in.\n");
                        Ok(())
                    }
                    Err(AuthError::NotSupported(op)) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH001,
                            category: DiagnosticCategory::Validation,
                            message: format!("operation not supported by this provider: {op}"),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH001"))
                    }
                    Err(AuthError::Provider { message, .. }) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH002,
                            category: DiagnosticCategory::Validation,
                            message: message.clone(),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH002"))
                    }
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            })
        }),
    };

    // ── auth logout ──────────────────────────────────────────────────────────
    let logout_cmd = Command {
        id: Arc::from("logout"),
        spec: Arc::new(CommandSpec {
            summary: "Clear the cached authentication token",
            category: Some("Authentication"),
            exit_codes: vec![
                crate::spec::command_tree::ExitCodeEntry {
                    code: 0,
                    description: "Logout succeeded",
                },
                crate::spec::command_tree::ExitCodeEntry {
                    code: 1,
                    description: "Authentication error",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, _args| {
            Box::pin(async move {
                let provider = match ctx.opt_token_provider() {
                    Some(p) => p,
                    None => {
                        DiagnosticReporter::write_plain("auth: no token provider configured\n");
                        return Err(anyhow::anyhow!("AUTH_NO_PROVIDER"));
                    }
                };
                match provider.logout().await {
                    Ok(()) => Ok(()),
                    Err(AuthError::NotSupported(op)) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH001,
                            category: DiagnosticCategory::Validation,
                            message: format!("operation not supported by this provider: {op}"),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH001"))
                    }
                    Err(AuthError::Provider { message, .. }) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH002,
                            category: DiagnosticCategory::Validation,
                            message: message.clone(),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH002"))
                    }
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            })
        }),
    };

    // ── auth status ──────────────────────────────────────────────────────────
    let status_cmd = Command {
        id: Arc::from("status"),
        spec: Arc::new(CommandSpec {
            summary: "Show authentication status",
            category: Some("Authentication"),
            args: vec![
                ArgSpec {
                    name: "json",
                    kind: ArgKind::Flag,
                    short: None,
                    long: Some("json"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Bool(false)),
                    help: "Output status as JSON",
                    ..Default::default()
                },
                ArgSpec {
                    name: "no-refresh",
                    kind: ArgKind::Flag,
                    short: None,
                    long: Some("no-refresh"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Bool(false)),
                    help: "Check status without refreshing the token",
                    ..Default::default()
                },
            ],
            exit_codes: vec![
                crate::spec::command_tree::ExitCodeEntry {
                    code: 0,
                    description: "Status shown successfully",
                },
                crate::spec::command_tree::ExitCodeEntry {
                    code: 1,
                    description: "Provider error",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            Box::pin(async move {
                let as_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
                let no_refresh = matches!(args.get("no-refresh"), Some(ArgValue::Bool(true)));

                let provider = match ctx.opt_token_provider() {
                    Some(p) => p,
                    None => {
                        if as_json {
                            ctx.framework_println(
                                r#"{"logged_in":false,"expires_at":null,"expires_in_seconds":null}"#,
                            );
                        } else {
                            ctx.framework_println(&format!(
                                "Not logged in. Run `{app_name} auth login`."
                            ));
                        }
                        return Ok(());
                    }
                };

                if no_refresh {
                    let status = provider.peek().await;
                    if as_json {
                        match status {
                            Some(s) => {
                                let (expires_at_str, expires_in_secs) =
                                    format_expiry_json(s.expires_at);
                                ctx.framework_println(&format!(
                                    r#"{{"logged_in":{logged_in},"expires_at":{expires_at},"expires_in_seconds":{expires_in}}}"#,
                                    logged_in = s.logged_in,
                                    expires_at = expires_at_str,
                                    expires_in = expires_in_secs,
                                ));
                            }
                            None => {
                                ctx.framework_println(
                                    r#"{"logged_in":false,"expires_at":null,"expires_in_seconds":null}"#,
                                );
                            }
                        }
                    } else {
                        match status {
                            Some(s) => {
                                if s.logged_in {
                                    let expiry_line = format_expiry_human(s.expires_at);
                                    ctx.framework_println(&format!("Logged in{expiry_line}."));
                                } else {
                                    ctx.framework_println(&format!(
                                        "Not logged in. Run `{app_name} auth login`."
                                    ));
                                }
                            }
                            None => {
                                DiagnosticReporter::write_plain(
                                    "status unavailable in read-only mode (provider has no peek support); re-run without --no-refresh\n",
                                );
                            }
                        }
                    }
                    return Ok(());
                }

                // Default: call token() to get current auth state.
                match provider.token().await {
                    Ok(token) => {
                        if as_json {
                            let (expires_at_str, expires_in_secs) =
                                format_expiry_json(token.expires_at());
                            ctx.framework_println(&format!(
                                r#"{{"logged_in":true,"expires_at":{expires_at},"expires_in_seconds":{expires_in}}}"#,
                                expires_at = expires_at_str,
                                expires_in = expires_in_secs,
                            ));
                        } else {
                            let expiry_line = format_expiry_human(token.expires_at());
                            ctx.framework_println(&format!("Logged in{expiry_line}."));
                        }
                        Ok(())
                    }
                    Err(AuthError::NotAuthenticated) => {
                        if as_json {
                            ctx.framework_println(
                                r#"{"logged_in":false,"expires_at":null,"expires_in_seconds":null}"#,
                            );
                        } else {
                            ctx.framework_println(&format!(
                                "Not logged in. Run `{app_name} auth login`."
                            ));
                        }
                        Ok(())
                    }
                    Err(AuthError::Provider { message, .. }) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH002,
                            category: DiagnosticCategory::Validation,
                            message: message.clone(),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH002"))
                    }
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            })
        }),
    };

    // ── auth token ───────────────────────────────────────────────────────────
    let token_cmd = Command {
        id: Arc::from("token"),
        spec: Arc::new(CommandSpec {
            summary: "Print the current bearer token",
            category: Some("Authentication"),
            exit_codes: vec![
                crate::spec::command_tree::ExitCodeEntry {
                    code: 0,
                    description: "Token printed",
                },
                crate::spec::command_tree::ExitCodeEntry {
                    code: 1,
                    description: "Not authenticated or provider error",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, _args| {
            Box::pin(async move {
                let provider = match ctx.opt_token_provider() {
                    Some(p) => p,
                    None => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH003,
                            category: DiagnosticCategory::Validation,
                            message: format!("not authenticated; run `{app_name} auth login`"),
                            suggestion: None,
                            span: None,
                        });
                        return Err(anyhow::anyhow!("AUTH003"));
                    }
                };
                match provider.token().await {
                    Ok(token) => {
                        ctx.framework_println(token.as_bearer());
                        Ok(())
                    }
                    Err(AuthError::NotAuthenticated) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH003,
                            category: DiagnosticCategory::Validation,
                            message: format!("not authenticated; run `{app_name} auth login`"),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH003"))
                    }
                    Err(AuthError::Provider { message, .. }) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: AUTH002,
                            category: DiagnosticCategory::Validation,
                            message: message.clone(),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("AUTH002"))
                    }
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            })
        }),
    };

    // Register all four leaves.
    let login_path = CommandPath::new(&["auth", "login"]).unwrap();
    let logout_path = CommandPath::new(&["auth", "logout"]).unwrap();
    let status_path = CommandPath::new(&["auth", "status"]).unwrap();
    let token_path = CommandPath::new(&["auth", "token"]).unwrap();

    registry
        .register_at(&login_path, login_cmd)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&logout_path, logout_cmd)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&status_path, status_cmd)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&token_path, token_cmd)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

// ── expiry helpers ────────────────────────────────────────────────────────────

/// Format expiry as a human-readable suffix, e.g. ` (expires in 42m)`.
fn format_expiry_human(expires_at: Option<std::time::SystemTime>) -> String {
    let Some(exp) = expires_at else {
        return " (expiry unknown)".to_string();
    };
    match exp.duration_since(std::time::SystemTime::now()) {
        Ok(dur) => {
            let total_secs = dur.as_secs();
            if total_secs == 0 {
                " (expired)".to_string()
            } else {
                let hours = total_secs / 3600;
                let mins = (total_secs % 3600) / 60;
                if hours > 0 {
                    format!(" (expires in {hours}h {mins:02}m)")
                } else {
                    format!(" (expires in {mins}m)")
                }
            }
        }
        Err(_) => " (expired)".to_string(),
    }
}

/// Return `(expires_at_json_str, expires_in_json_str)` for JSON output.
///
/// `expires_at_json_str` is either a quoted ISO-8601 string or `"null"`.
/// `expires_in_json_str` is either a number (seconds) or `"null"`.
fn format_expiry_json(expires_at: Option<std::time::SystemTime>) -> (String, String) {
    let Some(exp) = expires_at else {
        return ("null".to_string(), "null".to_string());
    };
    use std::time::{Duration, UNIX_EPOCH};

    let unix_secs = exp
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    // Simple ISO-8601 UTC representation: "1970-01-01T00:00:00Z" format via seconds
    // (no external chrono dep — format manually).
    let ts_str = format_unix_as_iso8601(unix_secs);

    let expires_in = match exp.duration_since(std::time::SystemTime::now()) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    };

    (format!(r#""{ts_str}""#), expires_in)
}

/// Minimal ISO-8601 UTC formatter (yyyy-mm-ddThh:mm:ssZ) from a Unix timestamp.
fn format_unix_as_iso8601(unix_secs: u64) -> String {
    // Days since epoch
    let days = unix_secs / 86400;
    let time_of_day = unix_secs % 86400;

    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Convert days-since-Unix-epoch to (year, month, day). Handles leap years.
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Gregorian calendar calculation
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
