# Proposal: optional project configuration support (`cli-framework`)

## Problem

Several CLIs in this workspace reimplement the same ideas: find a project root, read a TOML (or `.conf`) file, optionally walk parent directories, and merge with environment variables. There is no shared abstraction in `cli-framework` today.

## What we see today (brief inventory)

| Project | Anchor | Config shape |
|--------|--------|----------------|
| **agwiki** | Wiki root; `agwiki.toml` at root | TOML (`AgwikiConfig`, versioned) |
| **newton** | `newton.toml` at workspace root; nested `.newton/configs/*.conf` for monitor/batch | TOML + `key=value` `.conf` files; env overrides in loader |
| **fastskill** | Walk up to `skill-project.toml`; `[tool.fastskill]` section | TOML nested under PEP 518-style project file |

Patterns repeated: upward directory walk, “marker” directory (`.newton`), parse + validate, env override, clear errors when file is missing.

## Goals

1. **Optional Cargo feature** (e.g. `project-config`) so minimal binaries pay nothing.
2. **Discovery helpers**: walk parents for a named file; optional “marker directory” (e.g. `.mytool`) with configurable semantics.
3. **Loading**: read UTF-8 file, parse TOML into caller-provided `Deserialize` type; surface path + parse errors consistently (aligned with `DiagnosticReporter` style where reasonable).
4. **Precedence** (documented, opt-in per call site): e.g. CLI flags > env > project file > defaults (apps compose this; the crate provides building blocks).
5. **No forced schema**: apps own serde types; framework does not mandate `agwiki.toml` vs `foo.toml`.

## Non-goals

- Replacing full config crates (e.g. `config`, `figment`) for complex multi-format stacks.
- Automatic migration of agwiki/newton/fastskill (tracked as separate work).

## Sketch API (illustrative)

```rust
// Discovery
pub fn find_file_upward(start: &Path, filename: &str) -> Option<PathBuf>;
pub fn find_marker_dir_upward(start: &Path, dir_name: &str) -> Option<PathBuf>;

// Load
pub fn load_toml_file<T: DeserializeOwned>(path: &Path) -> Result<T, ConfigLoadError>;
```

`ConfigLoadError` should carry `path`, `kind` (io / parse), and display-friendly messages.

Env merge can stay a small optional helper or a documented pattern copied from newton’s `apply_env_overrides` (not necessarily generic in v1).

## Feature name

Candidates: `project-config`, `config-toml`, `workspace-config`. Prefer **`project-config`** if we bundle discovery + TOML load.

## Dependencies

Likely `toml` + `serde` (already in tree); keep behind feature.

## Deliverables

1. Feature flag + module e.g. `cli_framework::project_config`.
2. Unit tests: upward walk, missing file, broken TOML.
3. README / skill reference: when to use vs hand-rolled `dirs`/`toml`.
4. Follow-up issues: adopt in newton/agwiki/fastskill incrementally.

## Risks

- Over-generalizing before second consumer; mitigate by shipping **small** API and extending after dogfooding.

## Related issue

Tracked on the cli-framework GitHub project as a dedicated “Idea” item (configuration files support).
