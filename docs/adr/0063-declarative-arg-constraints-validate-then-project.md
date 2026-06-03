# Declarative arg constraints in ArgSpec; validate the ArgValue map, then project to the typed struct infallibly

Status: proposed

`ArgSpec` gains declarative value constraints (numeric min/max, allowed-values for enums, optional
pattern) in addition to its existing `value_type`/`cardinality`/`conflicts_with`/`requires`. All
user-facing validation runs in a single pass — `CommandSpec::validate_typed_args` over the runtime
**ArgValue map**, emitting the existing stable diagnostic codes (E003 missing-required, E004
invalid-value, E005 conflict, E006 requires). Deserializing the validated ArgValue map into the
command's typed args struct `T` is then an **infallible projection** (pure shape-mapping); it never
becomes a second source of user-facing errors. Cross-field/business rules that aren't declaratively
expressible remain in the Custom validator closure. `#[derive(CommandSpec)]` infers constraints from
the Rust type where possible (enum → allowed-values, `NonZeroUsize` → minimum 1, …).

## Why

Today `ArgSpec` carries only the value *type*, so domain constraints have nowhere declarative to live.
newton hand-rolls them in `TryFrom<CommandArgs>` with ad-hoc codes (e.g. `--parallel-limit must be
positive`, `--timeout non-negative` → `CLI_MIG_002`). Moving constraints into `ArgSpec` pays off twice:

1. The checks fold into the Spec validator with **uniform E-codes**, deleted from every consumer.
2. The constraints flow into the **MCP JSON Schema** (`minimum`, `enum`, …), so an LLM tool call is
   guided to valid values instead of being rejected at runtime — a correctness upgrade for the
   MCP-first goal.

The validate-then-project ordering is the robustness keystone: if validation were split between the
Spec validator and serde-style deserialization, two inconsistent error vocabularies would result.
Keeping deserialization infallible guarantees one diagnostic surface.

## Consequences

- `ArgSpec` is extended (additive to the struct; the validator and the JSON-Schema emitter both learn
  the new fields).
- A deserialization failure of a *validated* ArgValue map indicates a framework bug, not user error —
  it should panic/log, not surface as a diagnostic.
- Pairs with ADR 0061 (ArgValue map as runtime backbone) and the `#[derive(CommandSpec)]` work.
