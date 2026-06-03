# Typed command handlers; the ArgValue map is the runtime backbone; CommandArgs removed

Status: proposed

Consumer `execute` handlers receive a **typed args struct** (deserialized from the runtime
**ArgValue map**) instead of the stringly `CommandArgs`. `CommandArgs` is removed entirely. A single
annotated struct, via `#[derive(CommandSpec)]`, generates both the `CommandSpec` (for help, MCP JSON
Schema, and validation) and the extractor that turns the ArgValue map into that struct. Commands are
registered with a typed `register::<T>(path, handler)`; the registry's stored callback erases `T` and
deserializes from the ArgValue map at the leaf.

## Why

Both downstream consumers independently rebuilt the typed layer the framework should own, on top of a
representation the framework was throwing away:

- The framework already produces a typed **ArgValue map** on every entry path — CLI argv via the clap
  mapper, MCP/`chat` JSON via `json_value_to_arg_value` — then **flattened it down to stringly
  `CommandArgs`** for `execute`.
- **newton** then un-flattened it back: `impl TryFrom<CommandArgs> for RunArgs` adapters plus a
  `framework_setup` module of getter helpers (`get_bool`, `get_opt_path`, `.parse::<usize>()` with
  ad-hoc `CLI_MIG_002` error codes), re-validating integers and ranges the `CommandSpec` already knew.
- **fastskill** un-flattened it differently: re-parsing argv through clap (`parse_from_args`) inside a
  bridge closure, threading raw argv through `FsState.raw_remaining_args`.

So the live data flow was typed → string → typed: wasteful and a correctness hazard (per-consumer
re-validation). Making the ArgValue map the backbone and the typed extractor framework-generated
deletes newton's TryFrom/getter layer and fastskill's entire bridge, collapses each command to a
single declaration (newton previously maintained the spec *and* the adapter), and upgrades MCP tool
quality (a `spec`-less command exports only an opaque trailing var-arg).

## Considered options

- **Keep `CommandArgs` consumer-facing, add opt-in `.parse::<T>()`** — rejected: every handler keeps
  an unpacking line; the typed→string→typed round-trip survives; leanness goal half-met.
- **Replace the internal `ArgValue` representation with `serde_json::Value`** — rejected as the
  intermediate: redundant with the framework's existing `ArgValue`, which is already CommandSpec-aligned
  and produced by both paths. (serde may still be used *inside* the derive's extractor — see below.)
- **Erase to `clap::ArgMatches`** — rejected: cannot represent the MCP/`chat` JSON entry paths.

## Open implementation fork (settle by prototyping skillopt, not by debate)

How the derive produces the extractor: (A) emit a bespoke `from_arg_value_map`, or (B) lean on serde
(`ArgValue` map → `serde_json::Value` → `T: DeserializeOwned`), attractive because the MCP path is
already JSON so serde unifies CLI + MCP and the derive only emits the spec. Build skillopt both ways
behind the same `register::<T>` API; keep the smaller one.

## Consequences

- Greenfield, no backward compatibility: this is a breaking change to the public `Command`/handler
  API. All consumers migrate (fastskill + product-cli are bridge users; aikit/newton/agwiki are
  already spec-native but still adopt the typed handler signature). Each consumer adopts by bumping
  its pinned cli-framework rev.
- `#[derive(CommandSpec)]` reuses clap's `#[arg(...)]`/`#[command(...)]` attribute vocabulary (clap
  remains the internal CLI engine), minimizing migration churn.
- Per-consumer arg validation (e.g. newton's `CLI_MIG_002`) folds into `CommandSpec` value-types and
  constraints, validated once by the framework's Spec validator with stable E-codes.
- Glossary updated: `CommandArgs` removed; **ArgValue map** named as the runtime backbone; Resolution
  now yields `(Command, ArgValue map)`.
