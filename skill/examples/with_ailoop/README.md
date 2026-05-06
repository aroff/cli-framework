# `with_ailoop`

Uses **ailoop-core** (via **cli-framework**'s `AiloopClient`) for confirmations and choice prompts inside command handlers.

## Prerequisites: start `ailoop serve`

All HITL operations route over WebSocket to a paired `ailoop serve` process. Start one before running the example:

```bash
# Default: listens on ws://localhost:8080
ailoop serve --port 8080
```

Or point to an existing server via environment variable:

```bash
export AILOOP_SERVER=ws://your-ailoop-host:8080
```

The framework will use `ws://localhost:8080` if `AILOOP_SERVER` is unset.

## Run

From the repository root (in a second terminal, after `ailoop serve` is running):

```bash
cargo run --example with_ailoop
```

Configure **`AILOOP_SERVER`** / **`AILOOP_CHANNEL`** if your ailoop setup differs from defaults (see crate README environment section).

## See also

README [ailoop integration](../../README.md#ailoop-integration) and `examples/with_ailoop/src/main.rs`.
