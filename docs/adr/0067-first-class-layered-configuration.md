# First-class layered configuration: typed sections resolved once, injected via context

Status: proposed

The framework gains a `cli_framework::config` resolver that merges configuration from a fixed
precedence of sources — **built-in defaults → config file(s) → environment → CLI global flags →
explicit builder overrides** (highest wins) — into typed sections, resolved **once** during
`AppBuilder::build`, frozen on `App`, and reachable from any handler via `AppContext::config()`.
Framework-owned sections (`[telemetry]` first; `[http]`, `[ailoop]` reserved) carry their own
defaults; an app registers its own `serde::Deserialize` section with
`AppBuilder::with_config_section::<T>(name)` and gets it resolved through the same layers. Merge is
per-field (deep), not whole-document replace; malformed input fails `build` with a source/field
diagnostic; unknown framework-level keys are rejected.

## Why

cli-framework had no configuration story — only the `project-config` TOML loader and global-flags-as-
context (ADR 0062). Every consumer (`fastskill` hand-rolls env+argv parsing; `newton` re-declares
cross-cutting flags) re-implements the defaults→file→env merge, each with its own precedence and error
handling. Building telemetry forced the issue: a `[telemetry]` block needs a real layered resolver,
and shipping a bespoke one-off parser for it would entrench the exact fragmentation we keep paying for.
Elevating configuration to a first-class subsystem fixes it once for all consumers and gives telemetry
a principled home as its first section.

Per the standing strict/robust stance: malformed config is a loud `build` failure, not a silent
fallback; unknown keys are rejected so typos surface (consistent with ADR 0065). Resolution is a
build-time, not runtime, operation — one frozen snapshot, no re-resolution races.

## Considered options

- **(A, chosen) Layered resolver, typed sections, resolved once, injected via `ctx.config()`.**
  Composes with ADR 0062: global flags are simply the highest-but-one precedence layer feeding the
  same resolver, and ambient flags can override file/env values.
- **(B) Keep `project-config` + per-app parsing.** Rejected: no shared precedence, every consumer
  reinvents merge/validation, telemetry config would be a special case bolted on.
- **(C) A config *trait* each app implements.** Rejected: pushes the precedence/merge burden back onto
  apps — the very duplication we are removing — and gives framework sections no canonical defaults.

## Consequences

- New public surface: `cli_framework::config`, `AppBuilder::{with_config_file, with_config_defaults,
  with_config_section}`, `AppContext::config()`. Minor version bump.
- `project-config` becomes the file-discovery layer underneath the resolver (reusing the hardened
  bounded/ownership-checked upward search from spec 013/R6), not a parallel mechanism; existing
  `find_and_load` callers keep working during migration.
- Framework sections own their defaults in one place; telemetry (spec 014) is the first consumer and
  the proving ground.
- Consumers migrate incrementally: an app can adopt `ctx.config()` for new sections while leaving
  existing bespoke parsing in place until it chooses to collapse onto the resolver.
- Glossary: add **Config section** (typed, framework- or app-owned) and **Config layer / precedence**.
