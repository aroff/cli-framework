use crate::app::meta::AppMeta;
use crate::command::CommandRegistry;

/// Renders formatted help text for a CLI application.
///
/// **Deprecated for the primary CLI help path.** When the `clap-dispatch` feature
/// is enabled, Clap handles `--help`/`-h` output. This renderer is preserved for
/// applications that call `render_help()` directly for custom category-grouped formatting.
pub struct HelpRenderer<'a> {
    meta: Option<&'a AppMeta>,
    commands: &'a CommandRegistry,
}

impl<'a> HelpRenderer<'a> {
    /// Create a new renderer.
    pub fn new(meta: Option<&'a AppMeta>, commands: &'a CommandRegistry) -> Self {
        Self { meta, commands }
    }

    /// Render help to an owned `String`. Infallible.
    pub fn render(&self) -> String {
        const OTHER: &str = "Other";

        let mut out = String::new();

        // 1. HEADER BLOCK
        if let Some(meta) = self.meta {
            out.push_str(meta.name);
            out.push_str(" — ");
            out.push_str(meta.description);
            out.push('\n');
            out.push('\n');

            let usage = meta.usage.unwrap_or("<name> [OPTIONS] <command>");
            out.push_str("Usage: ");
            if meta.usage.is_some() {
                out.push_str(usage);
            } else {
                out.push_str(meta.name);
                out.push_str(" [OPTIONS] <command>");
            }
            out.push('\n');
            out.push('\n');
        } else {
            let binary_name = std::env::args()
                .next()
                .unwrap_or_else(|| "<program>".to_string());
            out.push_str("Usage: ");
            out.push_str(&binary_name);
            out.push_str(" <command> [arguments]");
            out.push('\n');
            out.push('\n');
        }

        // 2. GROUP COLLECTION
        let mut groups: std::collections::HashMap<&str, Vec<&crate::command::Command>> =
            std::collections::HashMap::new();
        for cmd in self.commands.commands() {
            let key = cmd.category.unwrap_or(OTHER);
            groups.entry(key).or_default().push(cmd);
        }

        let mut group_keys: Vec<&str> = groups.keys().copied().collect();
        group_keys.sort_by(|a, b| {
            if *a == OTHER && *b == OTHER {
                std::cmp::Ordering::Equal
            } else if *a == OTHER {
                std::cmp::Ordering::Greater
            } else if *b == OTHER {
                std::cmp::Ordering::Less
            } else {
                a.to_ascii_lowercase()
                    .cmp(&b.to_ascii_lowercase())
                    .then_with(|| a.cmp(b))
            }
        });

        // 3. PER-GROUP RENDERING
        for group in group_keys {
            let Some(cmds) = groups.get_mut(group) else {
                continue;
            };

            out.push_str(&title_case(group));
            out.push_str(":\n");

            cmds.sort_by(|a, b| {
                a.id.to_ascii_lowercase()
                    .cmp(&b.id.to_ascii_lowercase())
                    .then_with(|| a.id.cmp(b.id))
            });

            let max_id_len = cmds.iter().map(|c| c.id.len()).max().unwrap_or(0);
            let col_width = max_id_len + 2;
            let indent_len = 2 + col_width;
            let indent = " ".repeat(indent_len);

            for cmd in cmds.iter() {
                out.push_str("  ");
                out.push_str(cmd.id);
                let pad = col_width.saturating_sub(cmd.id.len());
                if pad > 0 {
                    out.push_str(&" ".repeat(pad));
                }
                out.push_str(cmd.summary);
                out.push('\n');

                if let Some(syntax) = cmd.syntax {
                    out.push_str(&indent);
                    out.push_str("Usage: ");
                    out.push_str(syntax);
                    out.push('\n');
                }
            }
        }

        // 4. OPTIONS BLOCK
        out.push_str("Options:\n");
        out.push_str("  --help, -h      Print this help message.\n");
        if let Some(meta) = self.meta {
            if !meta.version.is_empty() {
                out.push_str("  --version, -V   Print version (");
                out.push_str(meta.version);
                out.push_str(").\n");
            }
        }

        out
    }

    /// Print rendered help to stdout.
    pub fn print(&self) {
        print!("{}", self.render());
    }

    /// Render help text with no dynamic content (timestamps etc.) — suitable for snapshot tests.
    pub fn render_normalized(&self) -> String {
        self.render()
    }
}

fn title_case(input: &str) -> String {
    input
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut out = String::new();
            out.extend(first.to_uppercase());
            out.extend(chars);
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}
