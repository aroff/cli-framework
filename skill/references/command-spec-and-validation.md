# CommandSpec and validation

Typed argument specification with `CommandSpec`, `ArgSpec`, and `SpecValidator`. See also [`docs/migration-typed-spec.md`](../../docs/migration-typed-spec.md).

## `CommandSpec`

Attaching a `CommandSpec` enables:
- Type-checked argument parsing
- JSON Schema generation for MCP tools
- `SpecValidator` automatic checks (required args, type conformance, conflicts)

```rust
use cli_framework::prelude::*;

let spec = CommandSpec {
    args: vec![
        ArgSpec {
            name: "env",
            long: Some("env"),
            help: Some("Target environment name"),
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            ..Default::default()
        },
        ArgSpec {
            name: "replicas",
            long: Some("replicas"),
            help: Some("Number of replicas"),
            value_type: ArgValueType::Int,
            cardinality: Cardinality::Optional,
            ..Default::default()
        },
        ArgSpec {
            name: "dry_run",
            long: Some("dry-run"),
            help: Some("Simulate without applying changes"),
            value_type: ArgValueType::Bool,
            cardinality: Cardinality::Optional,
            ..Default::default()
        },
    ],
};
```

## `ArgValueType` variants

| Variant | Maps to JSON Schema |
|---------|-------------------|
| `ArgValueType::String` | `{ "type": "string" }` |
| `ArgValueType::Int` | `{ "type": "integer" }` |
| `ArgValueType::Float` | `{ "type": "number" }` |
| `ArgValueType::Bool` | `{ "type": "boolean" }` |

## `Cardinality` variants

| Variant | Meaning |
|---------|---------|
| `Cardinality::Required` | Must be present; appears in `inputSchema.required` |
| `Cardinality::Optional` | May be omitted |
| `Cardinality::Repeated` | Accepts multiple values; produces a list (or, for a flag, an occurrence count) |

`Repeated` alone says nothing about whether a value is *mandatory* — `--header`
(zero or more) and a `<skill-ids>...` positional (one or more) are both
`Repeated`. Set `min_occurs` to say which:

```rust
ArgSpec {
    name: "skill-ids",
    kind: ArgKind::Positional,
    value_type: ArgValueType::String,
    cardinality: Cardinality::Repeated,
    min_occurs: Some(1),           // one or more
    help: "Skill IDs to remove",   // becomes the property `description`
    ..Default::default()
}
```

`min_occurs: Some(1)` (or more) lists the argument in `inputSchema.required`
and adds `minItems` (arrays) or `minimum` (count flags). `None` and `Some(0)`
both mean zero-or-more. It is ignored for `Required` and `Optional`, and it is
schema-level only — CLI parsing arity is unchanged.

`help` is what an MCP client sees: it is emitted as the property's
`description` for every argument shape, so an empty `help` leaves an agent with
nothing but the argument's name and type.

## Custom `validator`

Runs after `SpecValidator`, before `execute`. Return `Err` to abort with a user-visible message:

```rust
validator: Some(Arc::new(|args| {
    let env = args.get("env").map(|s| s.as_str()).unwrap_or("");
    if env == "prod" {
        return Err(anyhow::anyhow!("Production deploys require use of deploy/create-prod"));
    }
    Ok(())
})),
```

## Fully-specced command example

```rust
Command {
    id: "create",
    summary: "Create a deployment in the target environment",
    syntax: Some("deploy create --env <env> [--replicas <n>] [--dry-run]"),
    category: Some("deploy"),
    spec: Some(CommandSpec {
        args: vec![
            ArgSpec {
                name: "env",
                long: Some("env"),
                help: Some("Target environment"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                ..Default::default()
            },
            ArgSpec {
                name: "dry_run",
                long: Some("dry-run"),
                help: Some("Simulate without side effects"),
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                ..Default::default()
            },
        ],
    }),
    validator: Some(Arc::new(|args| {
        if args.get("env").map(|v| v.is_empty()).unwrap_or(true) {
            return Err(anyhow::anyhow!("--env cannot be empty"));
        }
        Ok(())
    })),
    execute: Arc::new(|_ctx, args| Box::pin(async move {
        let env = args.get("env").unwrap_or("unknown");
        println!("deploying to {env}");
        Ok(())
    })),
}
```
