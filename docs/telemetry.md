# Telemetry

Every application built on `cli-framework` with the `telemetry` feature gets
OpenTelemetry instrumentation for free: a level a person controls, a catalogue
of named probes that says what each level sends in words, a redacting export
boundary, and a `telemetry` command group for reading and changing all of it.

This document is written for three readers, in this order: the person running
the app, the author building one, and the operator running a fleet of them.

## 1. What is sent by default

**Nothing.**

On an end-user installation — a CLI on somebody's laptop — the telemetry level
defaults to `off`. Nothing is collected, nothing is exported, no network
connection is opened, and no identifier is generated. Turning it on is a
deliberate act performed by the person at the keyboard.

The first time such an app runs interactively it prints one two-line notice to
stderr and never mentions it again:

```
demo: usage statistics are off.
Turn them on with `demo telemetry set usage`; see what would be sent with `demo telemetry info`.
```

The notice is suppressed when stderr is not a terminal, on non-interactive
surfaces, when a kill switch is set, and once the person has chosen a level
themselves. An app that declares a privacy page gets ` Details: <url>` appended
to the second line.

A **service** deployment is the other case: a long-running server started by an
operator who configured a collector endpoint. There, the level defaults to
`diagnostic` when an endpoint exists, and there is no notice — the operator
already knows, because they configured it.

Even with a level set, **nothing leaves the process without an endpoint.**
Export requires a level above `off` *and* a configured OTLP endpoint.

## 2. Telemetry levels and the probe catalogue

There are four telemetry levels, ordered:

| Level | Meaning |
| --- | --- |
| `off` | Collect nothing, export nothing. The end-user default. |
| `usage` | That the app ran and what was asked of it. No arguments, no messages, no addresses. |
| `diagnostic` | Adds error kinds, argument *names*, configuration and secret-store activity, outbound request shape. |
| `debug` | Adds the text of things: panic messages, the token that failed to parse, allowlisted argument values. Meant for a support session, not for a fleet. |

A **probe** is a named unit of instrumentation with a minimum level, a one-line
summary, and a plain sentence saying what it sends. A probe is effective when
the telemetry level is at least its minimum **and** the probe and all of its
ancestors are enabled. Dotted ids are hierarchical: `cli.command.args` is a
child of `cli.command`, and disabling the parent disables the child.

The framework ships twenty built-in probes:

| Probe | Minimum level | What it sends |
| --- | --- | --- |
| `cli.process` | usage | That the app ran, its version, and the exit status class |
| `cli.command` | usage | The registered command path, the invocation surface, duration and status |
| `cli.command.args` | diagnostic | Argument names and how many there were, never their values |
| `cli.command.arg_values` | debug | Values of arguments the author explicitly allowlisted, and no others |
| `cli.usage_error` | usage | The kind of mistake: unknown command, unknown flag, missing argument, invalid value or failed validation |
| `cli.usage_error.token` | debug | The offending token itself |
| `cli.panic` | usage | That the app panicked and the source location, never the message |
| `cli.panic.message` | debug | The panic message text |
| `cli.help` | usage | Which command's help was asked for |
| `cli.feature` | usage | The name of a feature the author registered, and nothing else |
| `cli.auth` | usage | That a login, logout, refresh or failure happened, never a credential |
| `cli.config` | diagnostic | Which setting was touched and whether it succeeded, never the value |
| `cli.secrets` | diagnostic | Which backend was used and whether it succeeded, never a secret |
| `cli.doctor` | usage | Which checks ran and how severe their findings were |
| `cli.plugin` | diagnostic | Which plugin loaded or failed to load |
| `cli.chat` | usage | That a chat session ran and how long it lasted, never prompt text |
| `http.client` | diagnostic | Method, status and duration, never the URL path or query |
| `http.client.server_address` | diagnostic | The destination host name, on the span only |
| `http.server` | usage | The matched route template, method, status and duration |
| `mcp.session` | usage | That an MCP session ran, which tools it called and how long it lasted |

Applications register probes of their own with
[`with_telemetry_ops`](#6-for-authors); those appear in the same catalogue and
get the same switches.

## 3. Turning it on, checking it, turning it off

The `telemetry` command group is present on end-user deployments. The output
below was captured from a real application called `demo`; only the settings-file
path is substituted, because the capture ran against a temporary directory and
the real one is platform-dependent.

### `telemetry status` — what is happening right now

```
$ demo telemetry status
telemetry level: off (source: default)
endpoint: none configured; nothing exports
attribution: pseudonymous
install id: none (anonymous)
organisation policy: not managed
settings file: /home/you/.config/demo/telemetry.json
probes:
  cli.auth (min level usage): enabled, not currently effective -- Sign-in activity
  cli.chat (min level usage): enabled, not currently effective -- Chat sessions
  cli.command (min level usage): enabled, not currently effective -- Which command ran
  ... one line per probe, alphabetically ...
```

`source:` names the layer the level came from — `default`, `config_file`,
`environment`, `flags`, or an organisation policy. `install id: none
(anonymous)` is literal: at level `off` no identifier exists to send.

Add `--json` for a machine-readable form with the same information. The keys
are `level`, `level_source`, `attribution`, `install_id`, `endpoint`,
`endpoint_source`, `policy`, `kill_switch`, `probes` and `store`:

```
$ demo telemetry status --json
{"level":"usage","level_source":"config_file","attribution":"pseudonymous","install_id":"b1e111de-41ec-493e-90fb-f1f3285b941a","endpoint":null,"endpoint_source":null,"policy":"organisation policy: not managed","kill_switch":null,"probes":[{"id":"cli.auth","min_level":"usage","enabled":true,"effective":true,"summary":"Sign-in activity"}, ...],"store":"/home/you/.config/demo/telemetry.json"}
```

### `telemetry set <level>` — choose a level

```
$ demo telemetry set usage
telemetry level set to usage
```

Accepts `off`, `usage`, `diagnostic` and `debug`. The choice is written to the
settings file and takes effect on the next run. Raising the level above `off`
for the first time is also what generates the install id.

### `telemetry info` — what each probe would send

`status` says what is on. `info` says what that *means*, probe by probe,
including the `sends:` sentence:

```
$ demo telemetry info
telemetry probe catalog:
  cli.auth (min level usage): effective -- Sign-in activity
    sends: That a login, logout, refresh or failure happened, never a credential
  cli.chat (min level usage): effective -- Chat sessions
    sends: That a chat session ran and how long it lasted, never prompt text
  cli.command (min level usage): effective -- Which command ran
    sends: The registered command path, the invocation surface, duration and status
  ... one entry per probe ...
```

### `telemetry disable <probe>` / `telemetry enable <probe>` — one probe at a time

```
$ demo telemetry disable cli.command.args
probe cli.command.args disabled
$ demo telemetry enable cli.command.args
probe cli.command.args enabled
```

Disabling a parent disables its children. This is how to keep a level while
excluding one specific thing.

### `telemetry reset` — start over

```
$ demo telemetry reset
telemetry settings deleted; the next run starts over as a new install, with a new id
```

`reset` deletes the framework's telemetry file outright: level, install id,
first-run notice marker and probe switches go together. That is deliberate —
see [ADR 0077](adr/0077-consent-lives-in-telemetry-level-only.md). The next run
behaves exactly like a fresh installation, including the notice.

### Where the settings live

The framework owns one file, separate from the application's own configuration:

```
<config dir>/<app>/telemetry.json     # or telemetry.toml, matching the app's config format
```

It holds the level, the install id, the notice marker and the probe switches,
and it never roams to another machine: consent is granted on the machine it
applies to. If the configuration directory cannot be found, telemetry does not
fail the app — it degrades to anonymous, reports a doctor finding, and carries
on.

## 4. Kill switches

Three environment variables disable telemetry completely, before any
configuration is read. They are checked in this order:

| Variable | Value | Scope |
| --- | --- | --- |
| `<APP>_TELEMETRY_DISABLED` | `1` | This application. `<APP>` is the app name upper-cased, with `-` and `.` replaced by `_`. |
| `OTEL_SDK_DISABLED` | `true` | Every OpenTelemetry SDK in the process. |
| `DO_NOT_TRACK` | `1` | Every tool on the machine that honours the convention. |

`DO_NOT_TRACK` and `OTEL_SDK_DISABLED` are honoured **because other tools
honour them.** Someone who sets `DO_NOT_TRACK=1` in their shell profile has
made a general statement about their machine, and an app that quietly exempted
itself would be defeating the point. `OTEL_SDK_DISABLED` is the OpenTelemetry
specification's own switch, and a process where the SDK is disabled must not
have a second telemetry pipeline running beside it.

A kill switch is absolute: no provider is built, no exporter is created, and
`telemetry status` reports which switch fired. Nothing in configuration, no
organisation policy, and no command-line flag can re-enable it.

## 5. What is never sent, at any level

Some values are excluded by construction, not by policy — no telemetry level,
no allowlist and no application code can bring them back.

**Never, at any level:**

- host names, user names, the current directory, `$HOME`
- environment variable values
- IP addresses
- raw URLs, URL paths and query strings
- file paths and file names taken from arguments
- prompt text and message content

**Attribute keys that are always dropped:** `url.full`, `url.path`,
`url.query`, `host.name`, `process.command_line`.

**Key fragments that are always dropped**, wherever they appear in a key:
`password`, `secret`, `token`, `authorization`, `cookie`, `api_key`. An
application can add more of its own with
[`with_telemetry_never`](#6-for-authors); it cannot remove any of these.

**Metric labels are allowlisted, not filtered.** Only `command`, `surface`,
`status`, `kind`, `feature`, `check`, `severity`, `tool`, `plugin`,
`http.route`, `http.request.method` and `http.response.status_code` may appear
on a metric, and `command` only for command paths the application registered —
never a string a person typed. This bounds metric cardinality as much as it
protects privacy.

`exception.message` is a `debug`-only attribute; `error.type` — the *kind* of
error, not its text — appears from `diagnostic`.

## 6. For authors

Telemetry is on the `telemetry` feature, which implies `observability`,
`config` and `doctor`:

```toml
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["telemetry"] }
```

An app that does nothing else already gets the whole catalogue, the command
group, the notice and the doctor checks. The builder API is for the parts only
the author knows.

### Declaring the deployment

```rust
use cli_framework::app::AppBuilder;
use cli_framework::Deployment;

let builder = AppBuilder::new()
    .with_version("demo", "0.1.0")
    .with_deployment(Deployment::EndUser {
        privacy_url: Some("https://example.com/privacy".into()),
    });
```

`Deployment::EndUser` is the default and is the one that gets the `off`
default, the notice, the `telemetry` command group, and the end-user clamp —
environment variables and flags may lower the telemetry level on an
installation but never raise it above what the person chose.
`Deployment::Service` is the fleet case; a standalone `ApiServerBuilder`
defaults to it.

### Where it sends, and how much

```rust
use cli_framework::TelemetryDefaults;

let builder = builder.with_telemetry_defaults(TelemetryDefaults {
    endpoint: Some("http://collector:4318".into()),
    headers: Some("api-key=...".to_string().into()),
    arg_value_allowlist: vec!["region".into()],
    sample_ratio: Some(0.1),
});
```

These are defaults, not decisions: `OTEL_EXPORTER_OTLP_ENDPOINT`,
`OTEL_EXPORTER_OTLP_HEADERS` and `OTEL_TRACES_SAMPLER_ARG` each win over the
corresponding field, because where a fleet sends and how much it samples are
the operator's call. `headers` is a `SecretString` and lives off the
configuration tree entirely — it is never written to a file, never roamed, and
never printed. `arg_value_allowlist` names the arguments whose *values* may be
recorded at `debug`, under the `cli.command.arg_values` probe; every other
argument records its name only.

### Registering probes of your own

```rust
use cli_framework::telemetry::{ProbeSpec, TelemetryLevel};

static OPS_PROBES: &[ProbeSpec] = &[ProbeSpec {
    id: "sync.batch",
    min_level: TelemetryLevel::Diagnostic,
    summary: "Batch synchronisation",
    sends: "How many records were synchronised and whether the batch succeeded",
}];

let builder = builder.with_telemetry_ops(OPS_PROBES);
```

The slice is `'static` because a probe is a declaration: `telemetry info` lists
it, the configuration manifest grows a `telemetry.sync.batch.enabled` switch
for it, and the summary has to outlive any one invocation. Ids are validated at
build time — a malformed id, a collision with a reserved first segment
(`level`, `attribution`, `install_id`, `notice_shown`, `endpoint`, `traces`,
`metrics`, `logs`), or a duplicate fails `build()` rather than disappearing
quietly. A probe may not use `enabled` as any segment after the first either:
the framework owns `telemetry.<probe>.enabled` as every probe's switch.

Write the `sends` sentence for the person who will read it in
`telemetry info`. It is the app's promise about that probe.

### Attributes

Framework attributes carry their own minimum level. An application attribute
has none, so it is dropped at the export boundary unless its exact key is
allowlisted:

```rust
let builder = builder
    .with_telemetry_attrs(vec!["app.tenant_kind".into()])
    .with_telemetry_never(vec!["employee_id".into()]);
```

The never-list always wins: allowlisting `app.api_key` does not bring it back.

### Identity

```rust
use std::sync::Arc;
use cli_framework::Identity;

let builder = builder.with_telemetry_identity(Arc::new(|_ctx| {
    Some(Identity { enduser_id: Some("u-123".into()), tenant: Some("acme".into()) })
}));
```

A closure, not a value: identity is not known at build time — a CLI resolves it
after authentication, a service per request. The framework calls it only when
the resolved attribution permits attaching an identity at all. The three
attribution modes are `anonymous` (no identifier), `pseudonymous` (a random
per-install UUID; the default) and `identified` (the resolver's answer).

### Logging

An application that wants its own logging alongside telemetry calls
`install_default_logging()` and holds the returned guard:

```rust
let _guard = cli_framework::telemetry::install_default_logging();
```

The guard reserves a slot for the OpenTelemetry layer, which the framework
attaches during startup once the policy is resolved. Hold it for the life of
the process. If the application installs an unrelated global subscriber
instead, the framework cannot attach the layer: it prints one warning, records
a `telemetry.subscriber` doctor finding, and exports metrics only.

### Servers and distributed tracing

A long-running server declares `Deployment::Service`; a standalone
`ApiServerBuilder` defaults to it. With an endpoint configured a `Service`
starts at `diagnostic` (an end-user install starts at `off`), and the operator
sets the level through the environment or the configuration file — there is no
`telemetry` command group and no clamp. Every HTTP request is wrapped in an
`http.request` server span named from the *matched route pattern*, never the
concrete path, so one resource id does not become one operation name; a
request that matched nothing reports the method alone. Do not open a request
span of your own in a handler: it nests inside the framework's instead of
replacing it. Attach detail with `tracing::info!`; the event lands on the
enclosing span.

Inbound `traceparent`/`tracestate` headers are extracted automatically, so a
call from another cli-framework service continues that trace. A request with
no header gets a fresh root. Outbound propagation is explicit, because the
framework does not own your HTTP client:

```rust
use cli_framework::telemetry::propagation::TracedRequestBuilder as _;

let resp = client.get(url).with_trace_context().send().await?;
```

Without that call on every outbound request, `A → B → C` is three traces, not
one — and nothing errors when it is missing. Baggage is never propagated; it
would carry caller-supplied attributes into every downstream service's
telemetry.

### Emitting from a handler

`ctx.telemetry()` is the app-level handle. It exists on every context, in
every build, and is a no-op when telemetry is off:

```rust
ctx.telemetry().counter("myapp.widgets_created").add(1, &[]);
ctx.telemetry().histogram("myapp.render_ms").record(elapsed_ms, &[]);
```

Instruments emitted this way ride the same pipeline as the built-in metrics.
`SpanHandle::set_attr` records only keys declared at the span's callsite —
`tracing` fixes a span's fieldset at compile time, so an undeclared key is
dropped — and `record_error` sets the span's OpenTelemetry status to `Error`.

### Testing

Two knobs exist for tests and nothing else:

- `AppBuilder::with_telemetry_config_dir(dir)` points the settings file at a
  temporary directory, so a test never reads or writes the developer's real
  consent file and never has to touch `XDG_CONFIG_HOME`.
- `CliTestHarness::with_interactive_stderr(bool)` declares whether stderr is a
  terminal, which is what decides whether the first-run notice prints. It
  defaults to `false`, so a notice test cannot pass in a terminal and fail in
  CI.

Set `DO_NOT_TRACK=1` in a test that must be sure nothing is sent; it is
honoured before any file is read or any id minted. A test that exercises real
export must be its own `[[test]]` binary: the tracer and meter providers are
process-global, so a second export test in the same binary reports into the
first one's collector. `tests/integration/telemetry_end_to_end.rs` is the
shape — a `wiremock` server stubbing `POST /v1/traces` and `/v1/metrics`, a
flush, then assertions on the bytes the collector received. Go through
`AppBuilder` for real: a subscriber assembled by hand in a test can pass while
the binary never installs one.

### Migrating from `with_telemetry`

`AppBuilder::with_telemetry(TelemetryConfig)` and `TelemetryConfig::from_env()`
are deprecated in v0.6.0 and removed in v0.8.0. Replace them with
`with_deployment(Deployment::Service)` plus `with_telemetry_defaults`; the
framework reads `OTEL_*` itself. Until then the old call still works, with two
things to know: an app that calls it and declares no deployment is treated as a
`Service` (every existing caller configured a collector by hand, which is a
server), and it exports through the pre-v0.6.0 pipeline, *bypassing* the
redacting boundary — nothing in section 5 applies until the app migrates.

### Diagnosing

Six doctor checks report on telemetry, and `doctor` runs them like any other:

| Check | Reports |
| --- | --- |
| `telemetry.subscriber` | Whether the framework's tracing subscriber was installed |
| `telemetry.store` | Whether the telemetry settings file could be opened |
| `telemetry.endpoint` | Whether the configured OTLP collector can be reached, bounded at 2 seconds |
| `telemetry.policy` | Whether an organisation policy client is managing this install |
| `telemetry.identity` | The attribution mode in effect, never the identifier itself |
| `telemetry.env` | Whether every set `<APP>_TELEMETRY_*` variable matched a known setting |

## 7. For operators

### Configuration keys

Telemetry is one section of the application's published configuration
manifest, generated from the probe registry. All keys are machine-scoped:

| Key | Manageable | Enforceable | Notes |
| --- | --- | --- | --- |
| `telemetry.level` | yes | **no** | Consent may be recommended, never enforced. A stored policy that tries fails validation. |
| `telemetry.attribution` | yes | no | `anonymous`, `pseudonymous`, `identified`. |
| `telemetry.endpoint` | yes | yes | Where OTLP goes. |
| `telemetry.<probe>.enabled` | yes | yes | One per probe, default `true`. |
| `telemetry.install_id` | local only | — | Never roams, never managed. |
| `telemetry.notice_shown` | local only | — | Never roams, never managed. |

There is deliberately no `telemetry.enabled` — the level *is* the switch, and
two switches would let them disagree.

### Environment variables

Every key under `telemetry.` has an environment form: the app name and the key
path, upper-cased, with `.` and `-` becoming `_`.

```
DEMO_TELEMETRY_LEVEL=diagnostic
DEMO_TELEMETRY_ENDPOINT=http://collector:4318
DEMO_TELEMETRY_CLI_COMMAND_ARGS_ENABLED=false
```

A `<APP>_TELEMETRY_*` variable that matches no key is not silently ignored: it
produces a warning and a `telemetry.env` doctor finding, because a typo in a
fleet-wide variable is otherwise invisible.

The standard OpenTelemetry variables are honoured too:
`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`,
`OTEL_TRACES_SAMPLER_ARG`, `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES` and
`OTEL_SDK_DISABLED`.

On a `Service` deployment these take effect directly. On an `EndUser`
installation the clamp applies: the effective telemetry level is the lower of
the full resolution and the resolution *without* environment variables, flags
or builder overrides — an operator can turn an installation down, never up.

### The wire

OTLP over `http/protobuf` only, default port 4318. `grpc` is rejected at
initialisation rather than failing silently later. Set headers through
`OTEL_EXPORTER_OTLP_HEADERS`; they are held as a secret and never written to
disk.

### Sampling

An `EndUser` installation always samples everything: there is one process, and
a dropped trace is the whole story. A `Service` uses
`ParentBased(TraceIdRatioBased(r))`, where `r` comes from
`OTEL_TRACES_SAMPLER_ARG`, falling back to the author's
`TelemetryDefaults::sample_ratio`. An absent or out-of-range ratio means full
sampling — never zero, so a misconfigured value cannot silently stop the
traces. Telemetry level `debug` forces `AlwaysOn` regardless.

### Shutdown

A service flushes fully on shutdown, including on `SIGTERM`. An end-user
installation gets a 500 ms budget and no more: telemetry never delays somebody's
prompt, and the exit code is never changed by an export failure.

### What the collector sees

Two resources, deliberately different:

- **Metrics** carry `service.name`, `service.version`, `cli.deployment`,
  `cli.telemetry.level`, `os.type`, `host.arch` and the `telemetry.sdk.*`
  attributes. No identifier — metrics are aggregates and must not become a
  per-install series.
- **Traces and logs** carry the same set plus `cli.install.id`, `session.id`
  and `os.version`, so a support conversation about one installation can find
  its traces.

Spans are named from a fixed set: `cli.command` is the root, with
`cli.config.load`, `cli.config.migrate`, `cli.config.policy_refresh`,
`cli.secrets.op`, `cli.plugin.load`, `http.client.request` and `http.request`
as children. Metric instruments are likewise fixed: `cli.command.invocations`,
`cli.command.duration_ms`, `cli.process.duration_ms`, `cli.usage_errors`,
`cli.panics`, `cli.help.shown`, `cli.feature.uses`, `cli.auth.events`,
`cli.doctor.findings`, `cli.plugin.loads`, `cli.chat.turns`,
`cli.chat.sessions`, `http.client.request.duration`,
`http.server.request.duration` and `mcp.tool.calls`.

If nothing arrives, run the app's `doctor`. `telemetry.endpoint` opens a TCP
connection to the configured collector with a two-second budget and says which
half failed; `telemetry.subscriber` says whether the OpenTelemetry layer could
be attached at all, which is the difference between "no traces" and "no
telemetry"; `telemetry.env` catches the misspelled `<APP>_TELEMETRY_*` variable
that would otherwise look like a setting that simply did not take.
`telemetry status` is the faster first answer: it prints the resolved level,
the layer it came from, and the endpoint in one screen.

## 8. Known limitations

- OTLP `http/protobuf` is the only wire protocol.
  `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` is rejected at initialisation and
  telemetry stays off, with the reason on stderr.
- There is no OTLP logs pipeline. `TelemetryConfig::logs_enabled`,
  `record_arg_values` and `arg_value_allowlist` are reserved; `traces_enabled`
  and `metrics_enabled` are honoured, and disabling traces still creates spans
  so propagation keeps working.
- Fewer probes emit than the catalogue declares. `telemetry info` lists every
  probe the build *can* send; some are registered but not yet wired to an
  emission site. The wired share is pinned by a test and grows per release.
