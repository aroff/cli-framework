# Global/persistent flags are ambient configuration, parsed into a typed struct and injected via context

Status: proposed

Persistent/global flags are declared **once** as a typed global-args struct `G`, parsed at the root by
the framework on any invocation, and delivered to the application via a consumer-supplied
`build_context(globals: G) -> Ctx` factory — i.e. they populate `AppContext`, they are **not**
parameters of each command's typed args struct. Guiding rule: *a value that configures the
environment/services (verbose, output format, skills-dir, server URL, state-dir) is a global and lives
in context; a value that is a genuine per-operation input — varies call-to-call, an MCP client should
legitimately set it — is not a global and stays a normal command arg.*

## Why

Distinguishing ambient configuration from command inputs is the standard pattern (cobra
`PersistentFlags`, clap `global(true)`, axum/actix middleware-populated state). Both consumers proved
the need and the classification:

- **fastskill** hand-writes `strip_global_flags` (a bespoke pre-parser with its own error handling)
  for `--skills-dir/--global/--verbose`, then injects them into `FsState` — which *is* its
  `AppContext`. These configure the (lazily constructed) service.
- **newton** re-declares cross-cutting flags on nearly every command: `workspace` 33×, `state-dir`
  11×, `verbose` 6×, `server` 5×, `format` 4×.

Three concrete reasons context-injection (not per-command flatten) is the robust choice:

1. **MCP correctness (the primary driver).** Globals in context do not appear in each MCP tool's input
   schema — correct, since an LLM calling `skillopt_run` must not choose `--skills-dir`/`--verbose`;
   those are set when the MCP server starts. Flattening globals into every command struct would put
   them in every tool schema — noisy and semantically wrong.
2. **Construction timing.** `build_context(globals)` provides the values at the exact moment shared
   services are configured (fastskill builds its service from the globals). Per-command flatten would
   force each handler to re-derive service config.
3. **Single source of truth, no signature bloat.** One `G` replaces newton's 33×/11×/… re-declarations
   and fastskill's hand-rolled parser; command structs stay focused on their own domain args.

## Considered options

- **(A, chosen) Typed `G`, parsed at root, injected via `build_context(G) -> Ctx`.**
- **(B) Flatten a shared `GlobalArgs` into every command's typed struct** (clap-native) — rejected:
  pollutes MCP tool schemas, re-couples every handler to cross-cutting flags, and complicates
  service-construction timing. Correct only if "globals" were really shared command inputs; they are
  configuration.

## Consequences

- New framework capability: persistent-flag registration + a `build_context` factory hook (today
  consumers construct the context manually before `AppBuilder`).
- fastskill deletes `strip_global_flags` and the manual `FsState` argv plumbing; newton collapses
  repeated flags into one `G`.
- The help `Options:` block enumerates the registered globals (the residual cosmetic from ADR/spec on
  grouped help).
- Per-flag edge cases (e.g. newton `--workspace`) are resolved by the guiding rule, not re-litigated:
  ambient → `G`; genuine per-call input → command arg.
