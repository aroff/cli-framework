# Ask, LLM integration, and security

How `ask` works, LLM environment detection, risk tiers, and output sanitization. See also `src/security/` and `src/llm/`.

## LLM setup

Auto-detect from environment:

```rust
if std::env::var("LLM_PROVIDER").is_ok() || std::env::var("OPENAI_API_KEY").is_ok() {
    builder = builder.with_llm_from_env()?;
}
```

`with_llm_from_env` reads:
- `LLM_PROVIDER` — selects backend (`openai`, `anthropic`, etc.)
- `OPENAI_API_KEY` — activates OpenAI backend when `LLM_PROVIDER` is unset
- `ANTHROPIC_API_KEY` — activates Anthropic backend

Provider implementations live in `src/llm/openai.rs` and `src/llm/anthropic.rs`.

## `resolve_command`

`ask` maps user natural language input to a registered `Command` by consulting `summary`, `syntax`, and `category` fields of each command's spec. Only commands with a `CommandSpec` and a meaningful `summary` are reliably resolvable.

```rust
// CommandSpec quality matters for ask resolution:
Command {
    id: "deploy",
    summary: "Create a new deployment in the target environment", // used by resolve_command
    syntax: Some("deploy --env <env>"),
    category: Some("deploy"),
    // ...
}
```

## Risk tiers

Every AI-resolved command is classified before execution:

| Tier | When | Behavior |
|------|------|---------|
| `Safe` | Read-only commands (list, show, health) | Execute directly |
| `Sensitive` | Config mutations, auth, writes | Prompt user for confirmation |
| `Destructive` | Data deletion, irreversible ops | Blocked; requires `ALLOW_DESTRUCTIVE_COMMANDS=1` |

Risk classification is defined in `src/security/command_risk.rs`. Commands are classified by their `category` field and execution semantics.

## `ALLOW_DESTRUCTIVE_COMMANDS`

```bash
ALLOW_DESTRUCTIVE_COMMANDS=1 my-tool ask "drop all datasets"
```

Only set this in controlled environments (CI, batch pipelines with explicit operator approval). Never enable by default in interactive tools.

## Output sanitization

All strings from LLM responses, plugin data, or external APIs pass through `src/security/output_sanitize.rs` before display. The sanitizer:
- Strips ANSI CSI/OSC escape sequences and terminal control characters
- Preserves printable ASCII, valid UTF-8 multi-byte characters, newlines, tabs, and carriage returns

This prevents terminal-injection attacks from malicious LLM output.

```rust
// Used internally; accessible as:
use cli_framework::security::sanitize_output;

let safe = sanitize_output(&untrusted_string);
println!("{safe}");
```
