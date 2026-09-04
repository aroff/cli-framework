# ADR 0076: A Probe's id is both the wire prefix of its signals and its configuration key

- Date: 2026-09-04
- Status: Accepted
- Relates to: ADR 0068 (tracing-bridge substrate), ADR 0073 (config manifest), ADR 0077 (consent), ADR 0079 (metric identity); specs 017, 025

## Context

Spec 025 turns the framework's instrumentation into a catalogue of named
**Probes** — `cli.command`, `cli.panic`, `http.client`, `mcp.session`, and so
on — that a person, or an organisation through Policy, can switch off one at a
time. Three consumers need to agree on what a Probe is called: the config
manifest that renders and validates the switch, the wire (span names, metric
names, instrumentation scopes) that a support engineer searches in Tempo and
Grafana, and the `telemetry info` command that tells a person what would be
sent.

Three shapes were considered.

1. **A single list field, `telemetry.disabled = ["cli.panic", …]`.** One field,
   trivially declared. But a list is one value: an organisation cannot recommend
   *one* entry, a person cannot switch one entry without rewriting the list, and
   there is no way to switch off a family of probes at once.
2. **Two vocabularies.** Wire names follow OpenTelemetry semantic conventions;
   configuration keys are chosen for the settings UI. Each is locally sensible,
   and they drift: the name an administrator disables and the name an engineer
   sees in a trace stop matching within a release or two.
3. **One hierarchical id doing both jobs.**

## Decision

Every Probe has one id, matching `^[a-z0-9]+(\.[a-z0-9_]+)*$`, and that id is
simultaneously:

- the **configuration key** — `telemetry.<id>.enabled`, a leaf in the app's
  published Config manifest; and
- the **wire prefix** — every span, metric and instrumentation scope the Probe
  emits is named `<id>` or `<id>.<suffix>`.

The id is hierarchical: `telemetry.cli.enabled = false` switches off every
Probe under `cli.`, and `telemetry.cli.command.args.enabled = false` switches
off only argument names. Sub-Probes carry the parts of a parent that need a
higher **Telemetry level** or a separate switch (`cli.command.args` at
`diagnostic`, `cli.command.arg_values` at `debug`).

Framework Probes and application-defined Probes (`with_telemetry_ops`) live in
one registry, `src/telemetry/ops.rs`. The manifest section, the wire names and
the `telemetry info` catalogue are all generated from that one table; nothing
else may introduce a name.

Beneath `telemetry.` the names `level`, `attribution`, `install_id`,
`notice_shown`, `endpoint`, `traces`, `metrics` and `logs` are reserved for
settings and may not be Probe ids.

## Consequences

- Renaming or removing a Probe is a **breaking change** for stored Policies and
  local config files, exactly as removing any other manifest field is (ADR
  0073). Probe names are chosen once.
- A name that is not in the registry never reaches the wire as a metric label:
  a `feature()` call with an unregistered name becomes a span event only, with
  a one-time warning. This is what keeps the `feature` label a closed set (ADR
  0079).
- `telemetry info` needs no hand-maintained list; a Probe that exists is
  documented, and one that is documented exists.
- Application authors who want a Probe switchable by Policy get it for free by
  registering the name; the manifest section is inserted by the framework.
