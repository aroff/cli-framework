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
| `command`, `command::ask` | `Command`, registry, `ask` |
| `parser`, `spec` | argv → args; `CommandPath`, `CommandSpec` |
| `llm` | providers, resolution |
| `plugin` | registry TOML / manifests |
| `ailoop` | ailoop-core client |
| `security` | output sanitize, `ask` risk policy |
| `http_retry`, `retry` | HTTP retry, `secure_reqwest_client` |
| `cli_output`, `cli_mode`, `message` | help, tables, JSON, modes |

Also: `auth`, `data_source`; `observability`, `testkit` behind features.

**Flow:** `AppBuilder` registers commands → `run` resolves id + `CommandArgs` → `await` `execute` on `AppContext`. `ask`: query → `resolve_command` → risk gate → optional confirm → same dispatch.

**Externals (summary):** `Cargo.toml` — e.g. `tokio`, `reqwest`, `clap`, `serde`, `ailoop-core`, `async-openai`, `anthropic-sdk`; optional `comfy-table`, `indicatif`.

**Security (summary):** sanitize untrusted terminal output; `ask` tier policy; plugin paths rooted (no traversal).

**Features:** default `[]`; see `[features]` in `Cargo.toml`. User-visible behavior → update README in the same PR.

## Tests

```bash
cargo test
```

## Local CI parity

```bash
./scripts/run-ci-tests.sh
```

Requires `cargo install cargo-audit`. Matches fmt, clippy `-D warnings`, audit, release build, tests.

## Conventions

- `cargo fmt --all`
- Clippy clean (CI uses `RUSTFLAGS="-D warnings"`)
- [Conventional Commits](https://www.conventionalcommits.org/)

## Pull requests

- Describe change; link issues.
- API or behavior change → README and/or `docs/` in the same PR when it affects integrators.

## References

- [docs/migration-typed-spec.md](docs/migration-typed-spec.md)
- [docs/testing.md](docs/testing.md)
