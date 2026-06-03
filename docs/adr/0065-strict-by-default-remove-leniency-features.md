# Strict by construction: remove the strict-args / strict-types / legacy-arg-coercion feature toggles

Status: proposed

The three opt-in leniency features are deleted; their strict behavior becomes unconditional:

- **`strict-types`** — `CommandSpec` is mandatory (non-optional in the type system, ADR 0061/0064), so
  there is nothing to gate. Removed.
- **`strict-args`** — unknown flags are always an error (E002). With the legacy trailing-var-arg
  capture path removed, there is no lenient mode to fall back to. Removed.
- **`legacy-arg-coercion`** — removed with the rest of the legacy parse path.

The framework no longer *offers* leniency; it cannot be opted back into.

## Why

Per the project's standing strict/robust stance, silently swallowing `--typo` (today's default, since
`strict-args` is off) or registering a command with no spec is exactly the quiet-wrong-state behavior
to refuse. Greenfield means we delete the toggles rather than flip their defaults — fewer code paths,
no accidental re-entry into legacy behavior.

## Consequences

- Glossary: the **Typed-spec model** term is deleted (typed-vs-untyped is no longer a distinction);
  the **CommandSpec** term drops its "optional / `spec: None`" wording.
- Pairs with the legacy-path removal in ADR 0061/0064 and spec #89 item 1.
- Consumers that previously relied on extra unrecognized args being absorbed must declare them
  (positional/variadic) explicitly — desired, not a regression.
