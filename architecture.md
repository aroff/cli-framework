# Architecture

This document describes how **cli-framework** is structured and how the main subsystems interact. It supplements the user-facing [README.md](README.md).

## Role in the stack

**cli-framework** is a Rust library for building CLIs: it centralizes command registration, parsing and dispatch, optional LLM-based natural language resolution (`ask`), plugin loading, human-in-the-loop hooks via **ailoop-core**, and shared concerns such as output sanitization and HTTP client defaults.

Applications depend on `cli_framework`, provide an `AppContext` implementation, and register `Command` values through `AppBuilder`.

## Major components

| Area | Module(s) | Responsibility |
|------|-----------|------------------|
| Application shell | `app` | `AppBuilder` composes the registry, optional LLM, risk policy, and meta; `run` and command execution entry points. |
| Commands | `command` | `Command` struct (metadata, optional `CommandSpec`, `validator`, `execute`), `CommandRegistry`, ask integration. |
| Parsing and specs | `parser`, `spec` | Maps argv to structured arguments; typed `CommandSpec` tree for validated dispatch when enabled. |
| LLM resolution | `llm`, `command::ask` | Resolves user text to a registered command; wires confirmation and risk tiers. |
| Plugins | `plugin` | Registry-backed loading from manifests; paths constrained to reduce traversal escapes. |
| Human-in-the-loop | `ailoop` | Bridges to ailoop-core for confirmations and prompts outside the `ask` flow. |
| Security | `security` | Output sanitization for untrusted strings; command risk tiers for `ask`; related policy hooks on `AppBuilder`. |
| HTTP | `http_retry`, `retry` | Retryable HTTP client patterns and `secure_reqwest_client()` defaults for callers that use reqwest. |
| CLI presentation | `cli_output`, `cli_mode`, `message` | Tables, JSON, progress (feature-gated), and message shaping for terminal output. |

## Execution flow

1. The host binary constructs `AppBuilder`, registers commands (each `Command` uses `Arc`-wrapped async `execute` closures), and optionally configures LLM, risk policy, and plugins.
2. The runtime parses input (CLI or repl-style, depending on the app), resolves to a command id and `CommandArgs`, and invokes the matching `execute` future with a mutable `AppContext`.
3. For the **`ask`** path, input is sent to the configured LLM provider; resolution is classified by risk tier, confirmation rules apply, then the resolved command runs like any other dispatch.

## Security model (summary)

- **Output sanitization**: Strings from LLMs, plugins, or externals pass through sanitization before terminal display to mitigate injection via escape sequences (see README for behavioral detail).
- **Risk tiers**: Resolved commands map to tiers (`Safe`, `Sensitive`, `Destructive`); destructive actions require explicit environment and interactive confirmation policy as documented in the README.
- **Plugins**: Manifest paths are validated relative to allowed roots to reduce path traversal from registry files.

Detailed policy tuning uses `AppBuilder::with_risk_policy()` and environment variables referenced in README.

## Optional Cargo features

Default features are minimal. Optional integrations (advanced tables, progress, clap-dispatch, strict parsing modes, testkit, etc.) are declared in `Cargo.toml` so applications opt in explicitly and keep compile graphs smaller when unused.

## Tests

Unit tests live under `tests/unit/`; integration scenarios under `tests/integration/`. Feature-gated tests use `required-features` in `Cargo.toml`. See [docs/testing.md](docs/testing.md) for harness-oriented usage.
