//! Startup banner for `mcp serve`.
//!
//! On `mcp serve` startup the framework prints a banner that shows the two
//! things a user needs to connect: the **server URL** (HTTP transport) or the
//! transport mode (stdio), and the **list of registered MCP tools**.
//!
//! The rendering functions in this module are pure and deterministic so they
//! can be unit-tested without binding a port. Emission (which stream, color vs.
//! plain ASCII, `--quiet`/`--json` handling) lives in [`emit_banner`].

use crate::mcp::McpToolRegistry;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

const TITLE: &str = "MCP server running";
const MIN_WIDTH: usize = 58;
/// Descriptions longer than this are truncated to a single, bounded line.
const MAX_DESC_WIDTH: usize = 60;

/// A single registered tool, as shown in the banner's `Tools` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub name: String,
    /// Short description; `None` renders the name alone.
    pub description: Option<String>,
}

/// What the banner's first field shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerTransport {
    /// HTTP transport: the full client-connectable URL plus an
    /// `all_interfaces` hint when bound to a wildcard host.
    Http { url: String, all_interfaces: bool },
    /// stdio transport: no URL, banner goes to stderr.
    Stdio,
}

/// Everything needed to render the startup banner.
#[derive(Debug, Clone)]
pub struct BannerData {
    pub transport: BannerTransport,
    pub tools: Vec<ToolLine>,
}

impl BannerData {
    /// Build banner data for an HTTP server bound to `host:port` serving `path`.
    pub fn http(host: &str, port: u16, path: &str, registry: &McpToolRegistry) -> Self {
        let all_interfaces = host == "0.0.0.0" || host == "::";
        let url = format!("http://{}:{}{}", host, port, path);
        Self {
            transport: BannerTransport::Http {
                url,
                all_interfaces,
            },
            tools: tools_from_registry(registry),
        }
    }

    /// Build banner data for a stdio server.
    pub fn stdio(registry: &McpToolRegistry) -> Self {
        Self {
            transport: BannerTransport::Stdio,
            tools: tools_from_registry(registry),
        }
    }

    /// Connectable URL, if this is an HTTP banner.
    fn url(&self) -> Option<&str> {
        match &self.transport {
            BannerTransport::Http { url, .. } => Some(url),
            BannerTransport::Stdio => None,
        }
    }

    fn transport_name(&self) -> &'static str {
        match self.transport {
            BannerTransport::Http { .. } => "http",
            BannerTransport::Stdio => "stdio",
        }
    }
}

/// Derive the tool list from the actually-registered MCP tools, sorted by name
/// for a stable, deterministic banner (the registry is an unordered map).
fn tools_from_registry(registry: &McpToolRegistry) -> Vec<ToolLine> {
    let mut tools: Vec<ToolLine> = registry
        .list_tools()
        .into_iter()
        .map(|d| ToolLine {
            name: d.name,
            description: short_description(&d.description),
        })
        .collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

/// Reduce a description to a single bounded line (first line, trimmed,
/// truncated with an ellipsis). Returns `None` for an empty description.
fn short_description(raw: &str) -> Option<String> {
    let first = raw.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return None;
    }
    Some(truncate(first, MAX_DESC_WIDTH))
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1).max(1);
    let mut out: String = chars[..keep].iter().collect();
    out.push('…');
    out
}

/// Box-drawing glyphs, selectable between Unicode and plain ASCII.
struct Glyphs {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    bullet: char,
    dash: &'static str,
}

impl Glyphs {
    fn unicode() -> Self {
        Self {
            top_left: '┌',
            top_right: '┐',
            bottom_left: '└',
            bottom_right: '┘',
            horizontal: '─',
            bullet: '•',
            dash: "—",
        }
    }

    fn ascii() -> Self {
        Self {
            top_left: '+',
            top_right: '+',
            bottom_left: '+',
            bottom_right: '+',
            horizontal: '-',
            bullet: '*',
            dash: "-",
        }
    }
}

/// Rendering style. `ascii` degrades box-drawing/Unicode glyphs to plain ASCII.
#[derive(Debug, Clone, Copy)]
pub struct RenderStyle {
    pub ascii: bool,
}

/// Render the full startup banner box as a multi-line string (no trailing newline).
pub fn render(data: &BannerData, style: &RenderStyle) -> String {
    let glyphs = if style.ascii {
        Glyphs::ascii()
    } else {
        Glyphs::unicode()
    };

    // Build the indented body lines (logical content; empty strings = blank rows).
    let mut body: Vec<String> = Vec::new();

    // Blank row beneath the top border (matches the spec layout).
    body.push(String::new());

    // First field: URL (HTTP) or transport mode (stdio).
    let label_width = "transport".len();
    match &data.transport {
        BannerTransport::Http {
            url,
            all_interfaces,
        } => {
            let mut url_line = format!("{:<width$}  {}", "URL", url, width = label_width);
            if *all_interfaces {
                url_line.push_str("  (reachable on all interfaces)");
            }
            body.push(url_line);
            body.push(format!(
                "{:<width$}  {}",
                "transport",
                "http (Streamable HTTP)",
                width = label_width
            ));
        }
        BannerTransport::Stdio => {
            body.push(format!(
                "{:<width$}  {}",
                "transport",
                "stdio (stdin/stdout JSON-RPC)",
                width = label_width
            ));
        }
    }

    body.push(String::new());

    // Tools section.
    if data.tools.is_empty() {
        body.push(format!("Tools (0)  {} no tools registered", glyphs.dash));
    } else {
        body.push(format!("Tools ({})", data.tools.len()));
        let name_width = data
            .tools
            .iter()
            .map(|t| t.name.chars().count())
            .max()
            .unwrap_or(0);
        for tool in &data.tools {
            match &tool.description {
                Some(desc) => body.push(format!(
                    "  {} {:<width$}  {}",
                    glyphs.bullet,
                    tool.name,
                    desc,
                    width = name_width
                )),
                None => body.push(format!("  {} {}", glyphs.bullet, tool.name)),
            }
        }
    }

    body.push(String::new());

    // Footer.
    match &data.transport {
        BannerTransport::Http { .. } => body.push("Press Ctrl-C to stop.".to_string()),
        BannerTransport::Stdio => body.push(format!(
            "(reading JSON-RPC on stdin {} banner on stderr)",
            glyphs.dash
        )),
    }

    // Compute the box width: wide enough for the title header, every indented
    // body line, and a minimum.
    let indent = "  ";
    let title_min = 4 + TITLE.chars().count() + 2; // "┌─ " + title + " " + "─" + corner
    let body_max = body
        .iter()
        .map(|line| indent.len() + line.chars().count() + 2)
        .max()
        .unwrap_or(0);
    let width = MIN_WIDTH.max(title_min).max(body_max);

    let mut out = String::new();

    // Top border: corner, "─ TITLE ", filler dashes, right corner.
    let header_prefix_len = 3 + TITLE.chars().count() + 1; // corner + "─ " + title + " "
    let fill = width.saturating_sub(header_prefix_len + 1);
    out.push(glyphs.top_left);
    out.push(glyphs.horizontal);
    out.push(' ');
    out.push_str(TITLE);
    out.push(' ');
    for _ in 0..fill {
        out.push(glyphs.horizontal);
    }
    out.push(glyphs.top_right);
    out.push('\n');

    // Body (open sides — no vertical borders, matching the spec layout).
    for line in &body {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(indent);
            out.push_str(line);
            out.push('\n');
        }
    }

    // Bottom border.
    out.push(glyphs.bottom_left);
    for _ in 0..width.saturating_sub(2) {
        out.push(glyphs.horizontal);
    }
    out.push(glyphs.bottom_right);

    out
}

/// Render the machine-readable startup object emitted with `--json`.
///
/// HTTP: `{"event":"mcp_started","url":"…","transport":"http","tools":[…]}`.
/// stdio omits the `url` field.
pub fn startup_json(data: &BannerData) -> String {
    let tools: Vec<&str> = data.tools.iter().map(|t| t.name.as_str()).collect();
    let value = match data.url() {
        Some(url) => serde_json::json!({
            "event": "mcp_started",
            "url": url,
            "transport": data.transport_name(),
            "tools": tools,
        }),
        None => serde_json::json!({
            "event": "mcp_started",
            "transport": data.transport_name(),
            "tools": tools,
        }),
    };
    value.to_string()
}

/// How the banner should be emitted, resolved from `--quiet` / `--json`
/// conventions and the environment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BannerSettings {
    /// Suppress the banner entirely.
    pub quiet: bool,
    /// Emit the machine-readable startup object instead of the box.
    pub json: bool,
}

impl BannerSettings {
    /// Resolve settings from the environment only (`QUIET`, `OUTPUT_FORMAT=json`).
    pub fn from_env() -> Self {
        Self {
            quiet: crate::cli_mode::is_quiet(),
            json: matches!(
                crate::cli_mode::read_env_var("OUTPUT_FORMAT").as_deref(),
                Some("json")
            ),
        }
    }

    /// Resolve settings from parsed global flags and the command's own args,
    /// falling back to the environment. Honors `--quiet` and `--json`
    /// (plus `--output json` / `--format json`) when an app registers them.
    pub fn resolve(
        global_args: Option<&HashMap<String, ArgValue>>,
        cmd_args: &HashMap<String, ArgValue>,
    ) -> Self {
        let env = Self::from_env();
        let quiet = env.quiet
            || flag_is_true(global_args, "quiet")
            || flag_is_true(Some(cmd_args), "quiet");
        let json = env.json
            || flag_is_true(global_args, "json")
            || flag_is_true(Some(cmd_args), "json")
            || arg_equals(global_args, "output", "json")
            || arg_equals(global_args, "format", "json");
        Self { quiet, json }
    }
}

fn flag_is_true(args: Option<&HashMap<String, ArgValue>>, key: &str) -> bool {
    matches!(args.and_then(|m| m.get(key)), Some(ArgValue::Bool(true)))
}

fn arg_equals(args: Option<&HashMap<String, ArgValue>>, key: &str, expected: &str) -> bool {
    matches!(
        args.and_then(|m| m.get(key)),
        Some(ArgValue::Enum(s) | ArgValue::Str(s)) if s == expected
    )
}

/// Emit the startup banner according to `settings`.
///
/// - HTTP banners go to stdout; stdio banners go to stderr (stdout is the
///   JSON-RPC channel and MUST NOT be corrupted).
/// - `--quiet` suppresses output; `--json` emits the machine-readable object.
/// - The Unicode box degrades to plain ASCII when the target stream is not a
///   TTY or color is disabled.
pub fn emit_banner(data: &BannerData, settings: BannerSettings) {
    use std::io::Write;

    if settings.quiet {
        return;
    }

    let to_stderr = matches!(data.transport, BannerTransport::Stdio);

    let write_line = |s: &str| {
        if to_stderr {
            let _ = writeln!(std::io::stderr(), "{}", s);
        } else {
            let _ = writeln!(std::io::stdout(), "{}", s);
        }
    };

    if settings.json {
        write_line(&startup_json(data));
        return;
    }

    let color = if to_stderr {
        crate::cli_mode::should_color_stderr()
    } else {
        crate::cli_mode::should_color_output()
    };
    let style = RenderStyle { ascii: !color };
    write_line(&render(data, &style));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, desc: Option<&str>) -> ToolLine {
        ToolLine {
            name: name.to_string(),
            description: desc.map(|s| s.to_string()),
        }
    }

    fn sample_tools() -> Vec<ToolLine> {
        vec![
            tool("create_item", Some("Create a new item")),
            tool("get_item", Some("Fetch an item by id")),
            tool("search", Some("Search the catalog")),
        ]
    }

    #[test]
    fn http_banner_shows_url_first_and_tool_count() {
        let data = BannerData {
            transport: BannerTransport::Http {
                url: "http://127.0.0.1:9922/mcp".to_string(),
                all_interfaces: false,
            },
            tools: sample_tools(),
        };
        let out = render(&data, &RenderStyle { ascii: true });
        let lines: Vec<&str> = out.lines().collect();
        // First field after the top border + blank line is the URL.
        assert!(lines[0].contains("MCP server running"));
        assert!(lines[2].contains("URL"));
        assert!(lines[2].contains("http://127.0.0.1:9922/mcp"));
        // URL appears before the transport line.
        let url_idx = lines.iter().position(|l| l.contains("URL")).unwrap();
        let tr_idx = lines.iter().position(|l| l.contains("transport")).unwrap();
        assert!(url_idx < tr_idx);
        assert!(out.contains("Tools (3)"));
        assert!(out.contains("Press Ctrl-C to stop."));
    }

    #[test]
    fn http_banner_names_are_column_aligned() {
        let data = BannerData {
            transport: BannerTransport::Http {
                url: "http://127.0.0.1:9922/mcp".to_string(),
                all_interfaces: false,
            },
            tools: sample_tools(),
        };
        let out = render(&data, &RenderStyle { ascii: true });
        // Descriptions must start at the same column across tool rows.
        let cols: Vec<usize> = out
            .lines()
            .filter(|l| l.contains("Search") || l.contains("Fetch") || l.contains("Create"))
            .map(|l| {
                // column of the description's first word
                l.find("Search")
                    .or_else(|| l.find("Fetch"))
                    .or_else(|| l.find("Create"))
                    .unwrap()
            })
            .collect();
        assert_eq!(cols.len(), 3);
        assert!(cols.iter().all(|&c| c == cols[0]), "cols: {:?}", cols);
    }

    #[test]
    fn stdio_banner_has_no_url_and_mentions_stderr() {
        let data = BannerData {
            transport: BannerTransport::Stdio,
            tools: sample_tools(),
        };
        let out = render(&data, &RenderStyle { ascii: true });
        assert!(!out.contains("URL"));
        assert!(out.contains("stdio (stdin/stdout JSON-RPC)"));
        assert!(out.contains("banner on stderr"));
        assert!(out.contains("Tools (3)"));
    }

    #[test]
    fn zero_tools_renders_empty_state() {
        let data = BannerData {
            transport: BannerTransport::Stdio,
            tools: vec![],
        };
        let out = render(&data, &RenderStyle { ascii: true });
        assert!(out.contains("Tools (0)"));
        assert!(out.contains("no tools registered"));
    }

    #[test]
    fn tool_without_description_shows_name_alone() {
        let data = BannerData {
            transport: BannerTransport::Stdio,
            tools: vec![tool("bare", None)],
        };
        let out = render(&data, &RenderStyle { ascii: true });
        let line = out.lines().find(|l| l.contains("bare")).unwrap().trim_end();
        // No description appended — line ends right after the name.
        assert!(line.ends_with("bare"), "line: {:?}", line);
    }

    #[test]
    fn all_interfaces_hint_when_wildcard_host() {
        let data = BannerData {
            transport: BannerTransport::Http {
                url: "http://0.0.0.0:9922/mcp".to_string(),
                all_interfaces: true,
            },
            tools: vec![],
        };
        let out = render(&data, &RenderStyle { ascii: true });
        assert!(out.contains("http://0.0.0.0:9922/mcp"));
        assert!(out.contains("all interfaces"));
    }

    #[test]
    fn unicode_style_uses_box_drawing() {
        let data = BannerData {
            transport: BannerTransport::Stdio,
            tools: vec![],
        };
        let out = render(&data, &RenderStyle { ascii: false });
        assert!(out.contains('┌'));
        assert!(out.contains('┘'));
        assert!(out.contains('•') || out.contains("Tools (0)"));
    }

    #[test]
    fn startup_json_http_has_url_and_tools() {
        let data = BannerData {
            transport: BannerTransport::Http {
                url: "http://127.0.0.1:9922/mcp".to_string(),
                all_interfaces: false,
            },
            tools: sample_tools(),
        };
        let s = startup_json(&data);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["event"], "mcp_started");
        assert_eq!(v["url"], "http://127.0.0.1:9922/mcp");
        assert_eq!(v["transport"], "http");
        // Tools sorted by name.
        assert_eq!(
            v["tools"],
            serde_json::json!(["create_item", "get_item", "search"])
        );
    }

    #[test]
    fn startup_json_stdio_omits_url() {
        let data = BannerData {
            transport: BannerTransport::Stdio,
            tools: vec![],
        };
        let v: serde_json::Value = serde_json::from_str(&startup_json(&data)).unwrap();
        assert_eq!(v["transport"], "stdio");
        assert!(v.get("url").is_none());
        assert_eq!(v["tools"], serde_json::json!([]));
    }

    #[test]
    fn resolve_honors_quiet_and_json_flags() {
        let mut globals = HashMap::new();
        globals.insert("quiet".to_string(), ArgValue::Bool(true));
        let cmd = HashMap::new();
        let s = BannerSettings::resolve(Some(&globals), &cmd);
        assert!(s.quiet);

        let mut globals = HashMap::new();
        globals.insert("json".to_string(), ArgValue::Bool(true));
        let s = BannerSettings::resolve(Some(&globals), &cmd);
        assert!(s.json);

        let mut globals = HashMap::new();
        globals.insert("output".to_string(), ArgValue::Str("json".to_string()));
        let s = BannerSettings::resolve(Some(&globals), &cmd);
        assert!(s.json);
    }

    #[test]
    fn short_description_takes_first_line_and_truncates() {
        assert_eq!(short_description(""), None);
        assert_eq!(
            short_description("first line\nsecond"),
            Some("first line".to_string())
        );
        assert_eq!(short_description("  spaced  "), Some("spaced".to_string()));
        let long = "x".repeat(100);
        let got = short_description(&long).unwrap();
        assert!(got.chars().count() <= MAX_DESC_WIDTH);
        assert!(got.ends_with('…'));
    }
}
