# Two parallel LLM stacks: `ask` vs `chat`

Status: accepted (transitional)

The `ask` command uses an in-tree LLM provider layer (`src/llm/`, driven by
`LLM_PROVIDER` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`). The `chat` command
(feature `chat`) instead delegates to the external `aikit-agent` crate
(driven by `AIKIT_LLM_URL` / `AIKIT_MODEL`). The two stacks share no code,
provider abstractions, HTTP client, or env-var surface.

We accepted this duplication deliberately rather than retrofitting `ask`
onto `aikit-agent`. `ask` predates `aikit-agent` and is a thin single-turn
classifier; `chat` is a multi-turn tool-calling agent and was easiest to
build by adopting `aikit-agent` whole. Unifying now would require either
porting `ask` to `aikit-agent`'s message/tool model (work without a user
benefit) or extending the in-tree providers to cover `chat`'s needs
(rebuilding what `aikit-agent` already offers).

**Trajectory: `chat` replaces `ask` entirely.** `ask` already emits an
`ASK_DEPRECATED` warning when the `chat` feature is enabled. Once `chat` is
the default, `ask`, `src/llm/`, and the `LLM_*` env vars are scheduled for
removal — at which point only the Chat agent stack remains. No new features
should be added to the Ask LLM stack.
