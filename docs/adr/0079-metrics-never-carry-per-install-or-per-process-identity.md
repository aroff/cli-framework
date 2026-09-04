# ADR 0079: Metrics never carry per-Install or per-process identity

- Date: 2026-09-04
- Status: Accepted
- Relates to: ADR 0076 (probe ids), ADR 0077 (consent and the pseudonymous id); spec 025; cp-platform-gitops `infrastructure/observability/collector/values.yaml`

## Context

Support must correlate telemetry to one **Install**, so a pseudonymous
`cli.install.id` and a per-process `session.id` have to travel with exported
data. The natural OpenTelemetry home for both is the SDK **Resource**, which
every signal from the process shares.

The platform's collector exports metrics to Prometheus with
`resource_to_telemetry_conversion: enabled: true`: every Resource attribute
becomes a label on every series. With the ids on a shared Resource, each CLI
invocation would mint a new `session.id`, and `cli_command_invocations_total`
would gain **one series per invocation, per Install** — unbounded cardinality
that would take the metrics backend down as adoption grew. Three fixes were
considered.

1. **Strip the attributes in the collector.** Works for this collector; fails
   silently for any other, and puts the framework's safety in a file it does not
   own.
2. **Keep one Resource and drop the ids from it entirely.** Removes the
   correlation support needs.
3. **Two Resources: identity on the tracer (and, in phase 2, the logger), never
   on the meter.**

## Decision

Option 3. The framework builds **two Resources**.

- The **meter provider** Resource is a closed set: `service.name`,
  `service.version`, `cli.deployment`, `cli.telemetry.level`, `os.type`,
  `host.arch`. Every value is drawn from a small enumeration or from a release
  string.
- The **tracer provider** Resource (and the logger provider's, in phase 2) adds
  `cli.install.id`, `session.id` and `os.version`. The install id also rides on
  the root `cli.command` span, which the `usage` level exports, so
  correlation works at the lowest non-off level.

Metric **labels** are closed sets by construction: `command` is a registered
command path, `feature` a registered feature name, `surface` and `status` are
enums, `http.route` is the matched route template (never the raw path) and
`http.request.method` is a known method or `_OTHER`. A metric View drops any
key outside each instrument's allowlist.

## Consequences

- "How many Installs used feature X" is answered from traces or logs, never
  from metrics. Metrics answer "how often" and "how slow".
- No spanmetrics or metrics-generator dimension may ever include
  `cli.install.id` or `session.id`. That is a rule for cp-platform-gitops, not
  code; the platform's querying guide correlates by `resource.cli.install.id`
  on traces.
- The export-boundary test at `debug` fails on **any** attribute outside the
  allowlist on a metric or on the meter Resource. Its negative check puts
  `session.id` on the meter Resource and must go red.
- If an operator sets `OTEL_RESOURCE_ATTRIBUTES`, the framework honours it on
  both Resources; its cardinality is the operator's responsibility, and the
  platform chart does not inject it.
