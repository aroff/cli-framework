# `with_ask`

Demonstrates **`deploy`**, **`status`**, **`logs`**, and optional natural-language **`ask`** when an LLM is configured.

When `cli-framework` is built with `--features chat`, `ask` emits an `ASK_DEPRECATED` warning and `chat` is the preferred interface.

## Run

From the repository root:

```bash
export OPENAI_API_KEY=...   # or ANTHROPIC_API_KEY
export LLM_PROVIDER=openai # or anthropic
cargo run --example with_ask
```

Without API keys, built-in commands still work; **`ask`** is unavailable until `with_llm_from_env` succeeds (see program output at startup).

Try at the prompt:

- `deploy --env prod`
- `ask show me the system status` (when LLM is enabled)

## See also

README [AI Ask Command](../../README.md#ai-ask-command) and [Security](../../README.md#security).
