# Contributing to CLI Framework

Library usage: [README.md](README.md). Doc guides: [docs/](docs/).

## Prerequisites

- Rust stable (edition 2021; MSRV typically 1.70+)
- `rustfmt` + `clippy` (`rustup component add rustfmt clippy`)

## Clone and build

```bash
git clone https://github.com/aroff/cli-framework.git
cd cli-framework
cargo build
```

## Git hooks

A pre-commit hook runs `cargo fmt --check` and `cargo clippy --all-features -- -D warnings` before every commit, matching CI exactly.

```bash
cp scripts/hooks/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

## Repository layout

| Path | Role |
|------|------|
| `src/` | Library |
| `skill/` | Bundled Agent Skill (`SKILL.md`, `skill-project.toml`, `references/`) |
| `skill/examples/` | Runnable samples — `cargo run --example <name>` |
| `tests/` | `[[test]]` targets in `Cargo.toml` |
| `docs/` | `migration-typed-spec.md`, `testing.md` |
| `specs/` | In-flight design notes |
| `tools-cli-framework/` | Superseded — see `skill/` |

## System design (`src/`)

| Module(s) | Role |
|-----------|------|
| `app` | `AppBuilder`, `App::run`, dispatch |
| `command`, `command::chat` | `Command`, registry, `chat` |
| `command_surface`, `command_surface::tool_bridge` | Command→tool schemas + shared tool invocation bridge (chat / MCP) |
| `parser`, `spec` | argv → args; `CommandPath`, `CommandSpec` |
| `plugin` | registry TOML / manifests |
| `ailoop` | ailoop-core client |
| `security` | output sanitize, command risk policy |
| `http_retry`, `retry` | HTTP retry, `secure_reqwest_client` |
| `cli_output`, `cli_mode`, `message` | help, tables, JSON, modes |
| `api` (feature `api-server`) | Built-in Axum host for versioned APIs (`/api/{version}/...`) plus `/healthz` + `/readyz`; `build()` may apply a root `fallback_service` (via `root_fallback()`) as its final composition step |
| `mcp` (feature `mcp-server`) | rmcp `ServerHandler` (tools + resources). Generic, concept-free: per-command `with_meta` (opaque `serde_json::Value`) → `tools/list` top-level `_meta`; `with_visibility` (cli-framework acts on it) → `_meta.visibility`; `mcp::resources::ResourceRegistry` serves resources via `resources/list` / `resources/read` with opaque per-resource `with_meta` at `contents[]._meta`. All UI/MCP-Apps semantics live in the consumer (ADR 0066) |

Also: `auth`, `data_source`; `observability`, `testkit` behind features.

**Flow:** `AppBuilder` registers commands → `run` resolves id + `CommandArgs` → `await` `execute` on `AppContext`. Tool surfaces (chat / MCP) adapt inputs into `command_surface::tool_bridge` for shared parsing/validation/gating/dispatch.

**Externals (summary):** `Cargo.toml` — e.g. `tokio`, `reqwest`, `clap`, `serde`, `ailoop-core`, `aikit-agent`; optional `comfy-table`, `indicatif`.

**Security (summary):** sanitize untrusted terminal output; command risk tier policy; plugin paths rooted (no traversal).

**Features:** default `["clap-dispatch", "chat"]`; see `[features]` in `Cargo.toml`. The `clap-dispatch` flag is now a no-op default — all dispatch goes through the Clap path. User-visible behavior → update README in the same PR.

## Tests

```bash
cargo test
```

Tests run in parallel within a single binary and share the process environment.
Any test that mutates env vars (`std::env::set_var` / `remove_var`) MUST use a
unique, test-scoped name (e.g. `CFW_TEST_<TEST>_<PURPOSE>`) — generic names like
`TEST_VAR` cause flaky races between tests that touch the same key.

## Local CI parity

```bash
./scripts/run-ci-tests.sh
```

Requires `cargo install cargo-audit`. Matches fmt, clippy `-D warnings`, audit, release build, tests.

### `cargo audit` ignores

CI runs `cargo audit`. If an upstream dependency has no compatible patched
release available for a RustSec advisory, this repo may temporarily ignore the
advisory via `.cargo/audit.toml` and document why. Any ignores must be treated
as temporary and removed once upstream is updated.

## Conventions

- `cargo fmt --all`
- Clippy clean (CI uses `RUSTFLAGS="-D warnings"`)
- [Conventional Commits](https://www.conventionalcommits.org/)

### Documentation status tags

Use these inline tags in `README.md`, `docs/`, and `specs/` when a documented
feature does not match the shipped behavior. Tags MUST appear as the first
token of a blockquote so they stand out:

| Tag | Meaning |
|------|---------|
| `[PLANNED]` | On the roadmap, not yet implemented. Surrounding prose describes the intended shape; the code does not yet do this. |
| `[PARTIAL]` | Implemented in part. Quote which part works and which does not. |
| `[DEPRECATED]` | Still works but slated for removal. Link to the replacement. |

Every `[PLANNED]` or `[PARTIAL]` tag SHOULD link to an ADR or spec that
records *why* the gap exists.

## Pull requests

- Describe change; link issues.
- API or behavior change → README and/or `docs/` in the same PR when it affects integrators.

## References

- [docs/migration-typed-spec.md](docs/migration-typed-spec.md)
- [docs/testing.md](docs/testing.md)
