# Telemetry (OpenTelemetry)

`cli_framework::telemetry` — traces + metrics over OTLP, behind the `telemetry`
feature (implies `observability`). Off by default; a no-op until you opt in.

## Enable it

```toml
[dependencies]
cli-framework = { version = "...", features = ["telemetry"] }
```

```rust
use cli_framework::app::AppBuilder;
use cli_framework::telemetry::TelemetryConfig;

let app = AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    .with_telemetry(TelemetryConfig::from_env())   // no-op until OTEL_EXPORTER_OTLP_ENDPOINT is set
    .build(ctx)?;
```

Servers use `ApiServerBuilder::with_telemetry(config, service_name, service_version)`.

## The one trap that matters: subscriber ownership

`with_telemetry()` works by installing a process-wide `tracing` subscriber. If
your `main()` (or any dependency) installs its own subscriber **first** —
`cli_framework::init_default_logging()`, a hand-rolled `tracing_subscriber::registry().init()`,
anything — `with_telemetry()` cannot attach the OTel bridge. It prints a
warning to **stderr** (deliberately not `tracing::warn!`; it doesn't own the
subscriber, so a `tracing` event could be filtered out and never seen) and
silently exports nothing. The tracer/meter providers are still installed
globally, so spans are *created* and just never leave the process — no error,
no crash, just an empty collector.

```
cli-framework telemetry: a global `tracing` subscriber is already installed,
so OpenTelemetry spans will NOT be exported.
```

**Fix**: let the framework own the subscriber. Call `init_default_logging()`
(or any manual subscriber setup) **only** when `TelemetryConfig::from_env().is_active()`
is `false` — no endpoint configured means the framework installs nothing, and
you'd otherwise lose all logging.

If your app must own its own subscriber (composing multiple layers of its
own), use `init_batch_without_subscriber` + `otel_layer` instead of
`with_telemetry()`:

```rust
let (handle, guard) = cli_framework::telemetry::init::init_batch_without_subscriber(
    &cfg, "svc", "1.0",
).expect("telemetry inactive");
tracing_subscriber::registry()
    .with(my_fmt_layer)
    .with(cli_framework::telemetry::init::otel_layer(&guard))
    .init();
```

## What you get automatically

Every command dispatch — CLI, chat, MCP, and the built-in `version` command —
opens a `cli.command` span (`cli.command.path`, `cli.invocation.surface`,
arg count/names) and records two metrics tagged `{command, surface, status}`:
the `cli.command.invocations` counter and `cli.command.duration_ms` histogram.
No handler code required.

`ApiServerBuilder` wraps every HTTP request in an `http.request` server span
(method, matched route pattern — never the concrete path, to avoid one
operation name per resource id). `/healthz`/`/readyz` are deliberately not
wrapped, or every kubelet probe would dominate every trace view.

**Do not add your own request span** in a handler — it nests inside the
framework's span rather than replacing it. Attach detail with `tracing::info!`
inside the handler; the event lands on the enclosing `http.request` span.

## Distributed tracing (context propagation)

Inbound `traceparent`/`tracestate` on an `ApiServerBuilder` request
automatically becomes the parent of that request's `http.request` span, so a
call arriving from another cli-framework service continues that trace instead
of rooting a new one. A request with no header still gets a fresh root — this
never welds an untraced caller onto someone else's trace.

Outbound propagation is explicit, because the framework doesn't own your HTTP
client:

```rust
use cli_framework::telemetry::propagation::TracedRequestBuilder as _;

let resp = client.get(url).with_trace_context().send().await?;
```

Without this on every outbound call your service makes, `A → B → C` is three
disconnected traces, not one — the single biggest gap in a multi-service
platform's telemetry, and the easiest to miss since nothing errors when it's
absent.

**Baggage is not propagated.** Only `traceparent`/`tracestate`. Baggage would
carry arbitrary caller-supplied attributes into every downstream service's
telemetry — a quiet cross-tenant leak on any multi-tenant platform.

## Authenticating to the collector

```rust
use std::collections::HashMap;
use cli_framework::telemetry::TelemetryConfig;

let mut headers = HashMap::new();
headers.insert("authorization".into(), "Bearer <token>".into());
let cfg = TelemetryConfig { headers, ..TelemetryConfig::from_env() };
```

Or `OTEL_EXPORTER_OTLP_HEADERS=key=value,key2=value2` (percent-encoding
supported; first `=` splits key from value, so a base64 value's `=` padding
survives). Sent with every OTLP request, traces and metrics both.
`TelemetryConfig`'s `Debug` impl redacts header **values** and keeps names —
logging a config can't leak a credential.

## App-level emission

```rust
// Inside a handler, via AppContext:
ctx.telemetry().counter("myapp.widgets_created").add(1, &[]);
ctx.telemetry().histogram("myapp.render_ms").record(elapsed_ms, &[]);
```

`SpanHandle::set_attr` only records attributes for keys declared at the span's
callsite — `tracing`'s fieldset is fixed at compile time, so an arbitrary key
is silently dropped. `record_error` works: it sets pre-declared `otel.status_*`
fields.

## Protocol and signal config

- **`http/protobuf` only.** `OTEL_EXPORTER_OTLP_PROTOCOL` set to anything else
  (e.g. `grpc`) is **rejected at init** — telemetry stays off and says why on
  stderr, rather than silently exporting over a protocol you didn't configure.
- `traces_enabled` / `metrics_enabled` are honoured independently. Disabling
  traces still creates spans (so propagation to downstream services keeps
  working) — it only suppresses the exporter.
- `logs_enabled` is reserved; no OTLP logs pipeline exists yet.

## Testing

Every test that exercises real export must be its **own `[[test]]` binary**.
`with_telemetry()`/`init_batch()` install a process-global subscriber and
tracer provider — a second test in the same binary exports into the first
test's collector and asserts against an empty one. See
`tests/integration/telemetry_end_to_end.rs` in the cli-framework source for
the canonical shape (a `wiremock::MockServer` stubbing `POST /v1/traces` +
`/v1/metrics`, `guard.flush()`, a ~750ms beat, then assert on bytes the
collector actually received).

Never assert telemetry by building a subscriber/layer by hand in the test —
go through `AppBuilder`/`ApiServerBuilder` for real. Hand-building the bridge
in a test can pass while the actual binary never installs one at all.
