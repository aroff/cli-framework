# `#[derive(CommandSpec)]` covers the full CommandSpec surface from one struct, via clap-compatible + `#[cfw(...)]` attributes

Status: proposed

A command is declared as a single annotated struct. `#[derive(CommandSpec)]` is the single source of
truth for the safety-critical, drift-prone surface — the `ArgSpec` list (from fields), arg-level
metadata (short/long/default/conflicts_with/requires/help-from-doc-comment), value constraints (ADR
0063), and the typed extractor (ADR 0061). It reuses clap's `#[arg(...)]`/`#[command(...)]` attribute
vocabulary where it overlaps, and adds a cli-framework `#[cfw(...)]` namespace for the metadata clap
cannot model: `deprecated`, `note`, `example`, `exit_code`, plus the registration-level `category` and
`risk` (which live on `Command`, not `CommandSpec`). No command-level metadata is supplied separately;
one struct carries everything.

## Why

Native consumers populate a wide surface — across newton/aikit/agwiki: summary 108×, notes 18,
long_about 17, examples 16, hidden 13, aliases 11, deprecated 7, env_vars 7, exit_codes 7; and at the
arg level default 185×, conflicts_with/requires 134, short 136, plus positionals. A derive that
covered only flags would regress all of them.

clap's attributes cover short/long/default/conflicts_with/requires/help/about/long_about/alias/hide/env,
but have no first-class `exit_codes`/`notes`/`examples`/`deprecated`, and no concept of the framework's
`category`/risk tier. Hence reuse-where-overlapping + a `#[cfw(...)]` extension namespace.

The rejected alternative is a **split** source of truth — derive args/extraction only, supply
command-level prose metadata via builder methods at registration. Rejected per the project's
strict/robust default: single-source is the whole point of the migration and the only way to fully
de-duplicate newton (which today maintains the spec *and* a `TryFrom` adapter). The drift risk of two
declarations outweighs the modest attribute clutter of list-heavy metadata (which is expressed as
repeatable `#[cfw(...)]` attributes).

## Consequences

- New proc-macro crate in the workspace (cli-framework has none today): emits the `CommandSpec`, the
  extractor, and registration glue.
- Refines ADR 0061's "reuse clap vocabulary" to **reuse + extend**.
- Each native consumer command collapses to one annotated struct with zero metadata loss.
