# `with_ailoop`

Uses **ailoop-core** (via **cli-framework**’s `AiloopClient`) for confirmations and choice prompts inside command handlers.

## Run

From the repository root:

```bash
cargo run --example with_ailoop
```

Configure **`AILOOP_SERVER_URL`** / **`AILOOP_CHANNEL`** if your ailoop setup differs from defaults (see crate README environment section).

## See also

README [ailoop integration](../../README.md#ailoop-integration) and `examples/with_ailoop/src/main.rs`.
