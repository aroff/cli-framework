## with_chat

Minimal example showing the built-in `chat` command (feature-gated).

### Run

From the repo root:

`cargo run --example with_chat --features chat -- chat --help`

To actually use `chat`, configure an LLM provider via the same environment variables as `ask`
(`OPENAI_API_KEY` or `ANTHROPIC_API_KEY`). This example also configures an ailoop channel to satisfy
the current `AppBuilder` requirement when an LLM provider is set.

Examples:

- One-shot: `cargo run --example with_chat --features chat -- chat -p \"show status\"`
- REPL: `cargo run --example with_chat --features chat -- chat`
