# Telemetry (OpenTelemetry)

`cli_framework::telemetry`, behind the `telemetry` feature (implies
`observability`, `config`, `doctor`). Full reference: `docs/telemetry.md` in
the cli-framework source. This file is the consumer's short version.

## What you get with no code

Every dispatch (CLI, chat, MCP, `version`) opens a `cli.command` span and
records `cli.command.invocations` / `cli.command.duration_ms` tagged
`{command, surface, status}`. A catalogue of named probes is gated by a
telemetry level — `off` < `usage` < `diagnostic` < `debug` — and everything
crosses a redacting export boundary before it leaves the process.

Resolution order: author defaults → recommended policy → settings file →
`<APP>_TELEMETRY_*` env → flags → builder overrides → enforced policy. Kill
switches beat all of it: `<APP>_TELEMETRY_DISABLED=1`, `OTEL_SDK_DISABLED=true`,
`DO_NOT_TRACK=1`. There is no `telemetry.enabled` key; the level is the switch.
Root `--help` lists all of these under *Environment Variables* (probe switches
as one `<APP>_TELEMETRY_<PROBE>_ENABLED` row); an app's own `register_env_var`
wording wins on a shared name.

## Declare the deployment — the one decision that matters

```rust
use cli_framework::app::AppBuilder;
use cli_framework::{Deployment, TelemetryDefaults};

let app = AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    .with_deployment(Deployment::Service)          // or EndUser { privacy_url: Some(..) }
    .with_telemetry_defaults(TelemetryDefaults {
        endpoint: Some("http://collector:4318".into()),
        ..Default::default()
    })
    .build(ctx)?;
```

| | `EndUser` (default) | `Service` |
|---|---|---|
| Default level | `off`; first-run notice on stderr | `diagnostic` once an endpoint is set, else `off`; no notice |
| Who sets the level | the person: `myapp telemetry set <level>` | the operator: env / config file |
| Clamp | env, flags and builder can lower the level, never raise it | none |
| `telemetry` command group | `status` / `info` / `set` / `enable` / `disable` / `reset` | not registered |
| Sampling | always full | `OTEL_TRACES_SAMPLER_ARG`, else `TelemetryDefaults::sample_ratio` |
| Shutdown flush | 500 ms budget; exit code never changed | full, including `SIGTERM` |

A standalone `ApiServerBuilder` defaults to `Service`. The framework reads
`OTEL_EXPORTER_OTLP_ENDPOINT` / `_HEADERS` / `OTEL_TRACES_SAMPLER_ARG` itself;
`TelemetryDefaults` fields are defaults those variables override. `headers` is
a `SecretString`: never written to disk, never printed.

**Deprecated (v0.6.0, removed v0.8.0):** `with_telemetry(TelemetryConfig)` and
`TelemetryConfig::from_env()`. The shim still works, is treated as `Service`,
and bypasses the redacting boundary — migrate.

## Author API — only what the author knows

```rust
use std::sync::Arc;
use cli_framework::Identity;
use cli_framework::telemetry::{ProbeSpec, TelemetryLevel};

static OPS: &[ProbeSpec] = &[ProbeSpec {
    id: "sync.batch",
    min_level: TelemetryLevel::Diagnostic,
    summary: "Batch synchronisation",
    sends: "Record count and whether the batch succeeded",   // shown by `telemetry info`
}];

let builder = builder
    .with_telemetry_ops(OPS)                                  // validated at build(); bad or duplicate id = build error
    .with_telemetry_attrs(vec!["app.tenant_kind".into()])     // app attrs are dropped unless allowlisted
    .with_telemetry_never(vec!["employee_id".into()])         // never-list beats the allowlist
    .with_telemetry_identity(Arc::new(|_ctx| {                // called only when attribution = identified
        Some(Identity { enduser_id: Some("u-123".into()), tenant: Some("acme".into()) })
    }));
```

In a handler: `ctx.telemetry().counter("myapp.x").add(1, &[])` /
`.histogram("myapp.ms").record(v, &[])` — a no-op when telemetry is off.
`SpanHandle::set_attr` only records keys declared at the span's callsite;
`record_error` works.

## Logging: hold the guard

```rust
let _guard = cli_framework::telemetry::install_default_logging();   // in main(), before build
```

The framework attaches the OTel layer to that subscriber once the policy is
resolved. If your app installs an unrelated global subscriber instead, traces
cannot be exported: one stderr warning, a `telemetry.subscriber` doctor
finding, metrics only. Never call `tracing_subscriber::...::init()` yourself.

## Distributed tracing

Inbound `traceparent` on `ApiServerBuilder` is extracted automatically; every
request gets an `http.request` span named from the matched route pattern (do
not add your own). Outbound is one explicit call per request:

```rust
use cli_framework::telemetry::propagation::TracedRequestBuilder as _;
let resp = client.get(url).with_trace_context().send().await?;
```

Skip it and `A → B → C` is three traces. Baggage is never propagated.

## Testing

- `AppBuilder::with_telemetry_config_dir(tmp)` — isolate the consent file;
  never touch `XDG_CONFIG_HOME`.
- `CliTestHarness::with_interactive_stderr(true)` — make the first-run notice
  print; the default is `false` so tests are not tty-dependent.
- `DO_NOT_TRACK=1` — guarantee nothing is sent.
- Real-export tests: one `[[test]]` binary each (providers are process-global);
  `wiremock` on `POST /v1/traces` + `/v1/metrics`, flush, assert the received
  bytes. Always go through `AppBuilder`, never a hand-built subscriber.

## Diagnosing

`myapp telemetry status` (resolved level, which layer set it, endpoint), then
`myapp doctor`: `telemetry.subscriber`, `.store`, `.endpoint` (2 s TCP probe),
`.policy`, `.identity`, `.env` (a misspelled `<APP>_TELEMETRY_*` variable).

## Limits

OTLP `http/protobuf` only (`grpc` is rejected at init and telemetry stays off).
No OTLP logs pipeline. Not every catalogued probe is wired to an emission site
yet — `telemetry info` lists what the build *can* send.
