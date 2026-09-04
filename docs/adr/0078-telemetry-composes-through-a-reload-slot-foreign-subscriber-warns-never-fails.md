# ADR 0078: Telemetry composes through a reload slot in the framework's subscriber; a foreign subscriber warns and never fails

- Date: 2026-09-04
- Status: Accepted
- Relates to: ADR 0068 (tracing-bridge substrate), ADR 0076 (probe ids); specs 017, 020, 025

## Context

The OpenTelemetry bridge is a `tracing` layer, and `tracing` allows exactly one
global subscriber per process, set once. Whoever installs it first wins;
installing a second one fails, and a bridge layer that is not on the winning
subscriber sees nothing. Spec 020 recorded the consequence: the bridge was
dead end-to-end for months because every test built its own subscriber and the
library never installed one.

Spec 025 wires telemetry into every app by default, including eight fleet
services that already call `tracing_subscriber::fmt().init()` themselves, with
their own filters and formats. Four options were weighed.

1. **The framework owns the subscriber unconditionally.** Breaks every app's
   own logging setup and panics on the second install.
2. **Fail the build when a subscriber already exists.** Turns a default feature
   into a startup failure for services that did nothing wrong.
3. **Skip silently when a subscriber exists.** Reproduces the spec-020 defect:
   telemetry that looks on and exports nothing.
4. **Offer one framework-owned subscriber with a slot, and detect the rest.**

## Decision

Option 4.

- `init_default_logging()` installs the framework's subscriber: a registry, an
  env filter, a `fmt` layer, and a **reload slot** into which the OTel layer is
  inserted later, once the telemetry policy has been resolved. Apps that call
  it get logging exactly as before and telemetry for free.
- An app that installs no subscriber at all gets an implicit one: env filter
  plus OTel layer, **no `fmt` layer** — the framework never starts printing
  logs that the app did not ask for.
- An app that installed a **foreign subscriber** before `AppBuilder::build`
  keeps it. The framework logs one warning naming `init_default_logging()`,
  records a `doctor` finding, and continues with metrics only. It never fails
  the build and never installs a second subscriber.
- `AppBuilder`, `ApiServerBuilder` and the MCP server all resolve the same
  telemetry policy once per process and share the slot.

## Consequences

- Getting traces from an existing app is a one-line migration — replace the
  app's own `fmt().init()` with `init_default_logging()` — and the framework
  says so at runtime rather than leaving a silent gap.
- Flipping `telemetry` into the default feature set (spec 025 slice ⑤) is
  gated on migrating the fleet services first; otherwise the flip ships eight
  warnings and no traces.
- Tests assert telemetry by driving a real `AppBuilder` against a test exporter
  and **never build a subscriber or layer themselves** (spec 020's rule). A
  test that builds the bridge by hand proves nothing about the product.
- The slot is what makes a later hot change of **Telemetry level** possible
  without restarting; phase 1 still marks the level `restart_required`, but
  the mechanism is in place.
