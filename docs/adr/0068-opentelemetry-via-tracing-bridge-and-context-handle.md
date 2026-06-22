# OpenTelemetry: tracing-bridge substrate, context-injected emit handle, simple-for-CLI export

Status: proposed

OpenTelemetry becomes a built-in, opt-in subsystem behind a new `telemetry` feature (implies
`observability`, off by default). Three decisions:

1. **`tracing` is the substrate.** The framework already emits `tracing` spans/events everywhere, so
   **traces and logs** are exported by bridging `tracing → OpenTelemetry` via `tracing-opentelemetry`;
   framework and app code keep using ordinary `tracing` macros. **Metrics** go through the
   `opentelemetry` Meter API directly (tracing cannot model metrics cleanly).
2. **Derived apps emit through a context-injected handle.** `AppContext::telemetry() -> &dyn Telemetry`
   (defaulted to a zero-sized `NoopTelemetry`; overridden by the dispatch wrapper with the live handle)
   exposes `event` / `counter` / `histogram` / `span`. App spans nest under the auto-created command
   span because they attach to the active `tracing` context. No app wires the OTel SDK itself.
3. **Export mode follows run shape.** `Simple` (synchronous, lossless) span/log export for one-shot CLI
   invocations; `Batch` for long-running `api-server` / `mcp serve`. A `TelemetryGuard` stored on `App`
   force-flushes on every exit path (success, error, SIGINT), with `Drop` as backstop.

All configuration flows through the layered config framework (ADR 0067) as the `[telemetry]` section,
honoring the standard `OTEL_*` env contract; with no endpoint or `OTEL_SDK_DISABLED=true` the whole
subsystem is inert.

## Why

- **Reuse the substrate, don't fork it.** Instrumentation already exists as `tracing` calls; a bridge
  turns the entire framework — and every app's existing `tracing` usage — into exportable telemetry for
  free. A parallel hand-rolled span API would duplicate and drift.
- **The handle is the "pre-defined mechanism" the request asked for.** Putting it on `AppContext`
  (mirroring `ctx.config()`, ADR 0067) means apps emit through the framework's configured pipeline with
  no SDK code, it auto-nests under command spans, it is testable, and it compiles to nothing when the
  feature is off.
- **CLIs lose batch spans.** Short-lived processes routinely exit before a batch exporter flushes —
  silent data loss. `Simple` is the correct default for one-shot runs; `Batch` only earns its keep for
  long-running serving modes, and even there we force-flush on shutdown.
- **Single auto-instrumentation seam.** `execute_command_direct` (`src/app/builder.rs:705`) is the one
  chokepoint all surfaces (CLI, chat, MCP, API) pass through, so one span + a `cli.invocation.surface`
  attribute yields usage analytics sliceable by entry point with no per-surface work.
- Strict/robust defaults: no endpoint ⇒ no egress; argument **values** are never recorded without an
  explicit opt-in allowlist (spec 013 posture); auth/OTLP headers never hit spans or logs.

## Considered options

- **(A, chosen) `tracing` bridge for traces/logs + Meter API for metrics; handle on `AppContext`;
  simple-for-CLI / batch-for-serving.**
- **(B) Native OTel span API throughout, no tracing bridge.** Rejected: discards existing `tracing`
  instrumentation, forces a second logging path, more churn for less coverage.
- **(C) Apps wire their own OTel SDK; framework only documents conventions.** Rejected: every consumer
  re-implements init/flush/propagation, defeating "built-in and configurable," and framework dispatch
  stays uninstrumented.
- **(D) Always-Batch with force-flush.** Rejected as the default: relies on the flush firing on every
  exit including signals; `Simple` is lossless by construction for the common one-shot case. Batch is
  retained only for serving modes.
- **(E) Reuse the `observability` feature for the whole OTel stack.** Rejected: conflates local logging
  with ship-to-collector export; `telemetry` implies `observability` instead (clean separation).

## Consequences

- New public surface: `telemetry` feature, `cli_framework::telemetry` module (replaces the placeholder
  `src/observability/opentelemetry.rs`), `AppBuilder::with_telemetry`, `AppContext::telemetry()`,
  `Telemetry`/`SpanHandle`/`Counter`/`Histogram`, `telemetry::{tracer, meter}()`. Minor version bump;
  note in `CHANGELOG.md`.
- `DispatchEnv` carries an `InvocationSurface`; each entry point (CLI run, chat tool call, MCP
  `tools/call`, API handler) stamps it. `CliAppContextWrapper` carries the live `Telemetry` handle.
- `with_tracing()` (api/mod.rs) upgrades to real server spans with inbound W3C extraction + HTTP
  metrics; `RetryableHttpClient` injects outbound W3C context — the CLI becomes a first-class
  participant in distributed traces.
- The `cli.*` span/metric attribute namespace is framework-reserved; app keys must not collide.
- Glossary: add **Invocation surface** (`cli|chat|mcp|api`), **Telemetry handle**, **Telemetry guard**.
- Depends on ADR 0067 (config framework) landing first; pairs with spec 014.
