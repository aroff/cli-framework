# Two parallel LLM stacks: `ask` vs `chat`

Status: **realized**

The `ask` command and its in-tree LLM provider layer (`src/llm/`, driven by
`LLM_PROVIDER` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`) have been removed.
The `chat` command (now a default feature, backed by `aikit-agent`) is the
sole natural-language command surface. The two-stack duplication described
below no longer exists.

## Historical context

The `ask` command used an in-tree LLM provider layer (`src/llm/`, driven by
`LLM_PROVIDER` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`). The `chat` command
(feature `chat`) instead delegated to the external `aikit-agent` crate
(driven by `AIKIT_LLM_URL` / `AIKIT_MODEL`). The two stacks shared no code,
provider abstractions, HTTP client, or env-var surface.

We accepted this duplication deliberately rather than retrofitting `ask`
onto `aikit-agent`. `ask` predates `aikit-agent` and was a thin single-turn
classifier; `chat` was a multi-turn tool-calling agent and was easiest to
build by adopting `aikit-agent` whole.

The removal was executed in a single breaking-change commit: `ask`, `src/llm/`,
`CommandResolution`, `CommandMetadata`, `LlmProvider`, `enforce_risk_gate`,
and the `async-openai`/`anthropic-sdk` dependencies were deleted. The `chat`
feature was promoted to `default`. Migration: `myapp ask "..."` becomes
`myapp chat -p "..."`.
