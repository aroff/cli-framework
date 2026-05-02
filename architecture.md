# Architecture

Macro view of **cli-framework**: what the library is, how **`src/`** modules group responsibility, and how request flow moves through the runtime. For how to use the library, see [README.md](README.md). For contributing and running checks, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Role in the stack

**cli-framework** is a Rust library: command registration and dispatch, optional LLM **`ask`**, plugin loading, **ailoop-core** integration, output sanitization and risk policy, HTTP retry helpers, and terminal-oriented output modes. Host binaries implement **`AppContext`**, construct **`AppBuilder`**, and register **`Command`** values.

## Major components

| Area | Module(s) under `src/` | Responsibility |
|------|-------------------------|----------------|
| Application shell | `app` | **`AppBuilder`** composes **`CommandRegistry`**, optional LLM, risk policy, app meta; **`App::run`** / argument dispatch. |
| Commands | `command` | **`Command`** (metadata, optional **`CommandSpec`**, **`validator`**, **`execute`**), registry, **`ask`** wiring. |
| Parsing and specs | `parser`, `spec` | CLI string to structured args; **`CommandPath`** tree, **`CommandSpec`** / **`ArgSpec`** when enabled. |
| LLM | `llm`, `command::ask` | **`LlmProvider`** implementations, resolution, confirmation and risk gates before dispatch. |
| Plugins | `plugin` | Registry TOML / manifests, constrained paths, command loading. |
| Human-in-the-loop | `ailoop` | **ailoop-core** client for confirmations and prompts outside **`ask`**. |
| Security | `security` | Sanitization of untrusted terminal output; **`CommandRiskPolicy`** for **`ask`**. |
| HTTP | `http_retry`, `retry` | Retry policies, **`RetryableHttpClient`**, **`secure_reqwest_client`**. |
| Presentation | `cli_output`, `cli_mode`, `message` | Help, tables, JSON, progress (feature-gated), message types. |

Supporting: **`auth`**, **`retry`**, **`data_source`**, **`message`**, **`observability`** (feature-gated), **`testkit`** (feature-gated, for tests).

## Execution flow

1. **Build time:** **`AppBuilder`** registers commands (and optional groups at **`CommandPath`**). Optional LLM configures **`create_ask_command`** and provider handle.
2. **Run time:** Parse argv (or repl input) to a command id and **`CommandArgs`**, look up **`execute`**, **`await`** the future with **`&mut dyn AppContext`**.
3. **`ask` path:** Parse query → **`LlmProvider::resolve_command`** against registry-derived metadata → enforce risk → confirm if required → dispatch same as (2).

## Security model (behavioral summary)

- Untrusted strings (LLM, plugins, HTTP) are sanitized before emission to the terminal.
- **`ask`** maps resolved commands to tiers; destructive tier requires policy and interactive confirmation as documented in README.
- Plugin manifest paths are rooted and validated to limit directory escape.

## Crate dependencies (external, high level)

Declared in **`Cargo.toml`**: **`tokio`**, **`async-trait`**, **`anyhow`**, **`thiserror`**, **`serde`**, **`reqwest`**, **`clap`**, **`regex`**, **`toml`**, **`log`**, **`crossterm`**, **ailoop-core** (git pin), **async-openai**, **anthropic-sdk**; optional **comfy-table**, **indicatif** behind features.

## Cargo features (compile-time surface)

Default is empty. Common flags: **`table-advanced`**, **`progress`**, **`clap-dispatch`**, **`strict-args`**, **`strict-types`**, **`legacy-arg-coercion`**, **`observability`**, **`testkit`**. Full matrix is authoritative in **`Cargo.toml`**.
