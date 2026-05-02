# Contributing to CLI Framework

This document is for developers who build, test, or submit changes to this repository. For integrating the library, see [README.md](README.md); for deeper topics, [architecture.md](architecture.md) and [docs/](docs/).

## Prerequisites

- Rust stable (edition 2021; MSRV aligned with the workspace, typically 1.70 or newer)
- `cargo fmt` and `clippy` (via `rustup component add rustfmt clippy`)

## Clone and build

```bash
git clone https://github.com/aroff/cli-framework.git
cd cli-framework
cargo build
```

## Repository layout

- `src/` — library code (`app`, `command`, `parser`, `spec`, `llm`, `plugin`, `security`, …)
- `examples/` — binaries you run with `cargo run --example <name>`
- `tests/` — crates registered via `Cargo.toml` `[[test]]` (unit, integration, contract helpers)
- `docs/` — guides (`migration-typed-spec.md`, `testing.md`)
- `specs/` — design notes for in-flight work
- `tools-cli-framework/` — bundled Agent Skill (`SKILL.md` + `references/`)

## Running tests

```bash
cargo test
```

Integration and unit tests are registered in `Cargo.toml` under `[[test]]` entries; `cargo test` runs them together with library tests.

## Local CI parity

To match the GitHub Actions checks before opening a PR, run:

```bash
./scripts/run-ci-tests.sh
```

That script runs, in order: `cargo fmt --check`, `cargo clippy` with `-D warnings`, `cargo audit`, release build, full `cargo test`, and integration-focused test invocation. Install `cargo-audit` once with `cargo install cargo-audit`; the audit step is required when using this script.

## Code conventions

- Run `cargo fmt --all` before committing.
- Fix all Clippy warnings; CI treats warnings as errors (`RUSTFLAGS="-D warnings"` in `run-ci-tests.sh`).
- Prefer small, focused commits with messages that follow [Conventional Commits](https://www.conventionalcommits.org/).

## Pull requests

- Describe the change and link related issues when applicable.
- If behavior or public API changes, update [README.md](README.md), [architecture.md](architecture.md), or relevant docs under `docs/` in the same change when reasonable.

## Optional features

The crate exposes Cargo features (`table-advanced`, `progress`, `clap-dispatch`, `strict-args`, `strict-types`, `testkit`, and others documented in `Cargo.toml`). When adding behavior behind a feature, document it briefly in README or architecture as appropriate.

## Further reading

- Design and components: [architecture.md](architecture.md)
- Typed command specs and migrations: [docs/migration-typed-spec.md](docs/migration-typed-spec.md)
- Testing helpers: [docs/testing.md](docs/testing.md)
