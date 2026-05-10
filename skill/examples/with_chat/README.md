## with_chat

Minimal example showing the built-in `chat` command (feature-gated).

### Run

From the repo root:

`cargo run --example with_chat --features chat -- chat --help`

To actually use `chat`, configure an LLM provider via the same environment variables as `ask`
(`OPENAI_API_KEY` plus optional `AIKIT_LLM_URL` / `AIKIT_MODEL`).

Examples:

- One-shot: `cargo run --example with_chat --features chat -- chat -p \"show status\"`
- REPL: `cargo run --example with_chat --features chat -- chat`
