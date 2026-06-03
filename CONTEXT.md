# cli-framework

Domain glossary for the `cli-framework` Rust library: a CLI application
framework with a central command registry, optional LLM-assisted resolution
(`ask`, `chat`), ailoop-backed human-in-the-loop prompts, plugin loading, and
optional MCP exposure.

This file is a glossary, not a spec. Implementation details belong in
`README.md`, `CONTRIBUTING.md`, `docs/`, and `specs/`.

## Language

**Command**:
The canonical unit of work a user can invoke. Registered with `AppBuilder`,
identified by `id`, executed against an `AppContext`.
_Avoid_: Action, operation, handler.

**Tool**:
An MCP-surface projection of a **Command**. Not a distinct type — when a
binary runs in MCP mode (or under `chat`), each registered Command is exposed
as a tool named `<app_name>_<command_id>` (underscores; nested paths flatten
`/`→`_`, e.g. `myapp_cluster_get`) with a JSON Schema derived from its
`CommandSpec`. Underscores are used for OpenAI tool-name compatibility
(`src/mcp/mod.rs`). "Tool" is only used at the MCP/`chat` boundary.
_Avoid_ using "tool" to mean Command in any other context.

**Resolution**:
The phase that turns some input — argv, a natural-language `ask` query, an
MCP tool call, or a `chat` tool call — into a concrete `(Command,
ArgValue map)` pair. Different entry paths have different Resolution
strategies but all share the same downstream **Dispatch**.
_Avoid_: routing, lookup, matching.

**Ask resolution**:
The LLM-backed flavor of **Resolution**: a natural-language query is sent to
a provider, which returns a Command id, args, confidence, and reasoning. Not
a separate concept from Resolution — just one strategy.

**Dispatch**:
The phase that executes a resolved **Command** against the `AppContext`. The
per-Command callback is named `execute` in code; do not use "execute" as a
noun for the whole phase.
_Avoid_: invoke, run, handle.

**Risk tier**:
The static safety classification of a **Command**: `Safe`, `Sensitive`, or
`Destructive`. A property of the Command (via policy), not the call site.

**Risk policy**:
The configurable mapping from Command id / category to **Risk tier**
(`CommandRiskPolicy`). Defaulted by category; overridable per-Command via
`AppBuilder::with_risk_policy`.

**Risk gate**:
The phase between **Resolution** and **Dispatch** in the `ask` / `chat`
paths that enforces the **Risk policy**: `Sensitive` requires a
**Confirmation**; `Destructive` is blocked unless `ALLOW_DESTRUCTIVE_COMMANDS`
is set and a Confirmation channel is available.
_Avoid_: risk check, safety check.

**MCP tool gate**:
A peer of the **Risk gate** specific to the MCP entry path
(`AppBuilder::with_mcp_tool_gate`). MCP calls do **not** flow through the
Risk gate or trigger a Confirmation; operators must wire an MCP tool gate
explicitly if they want allowlisting or confirmation for MCP.

**Confirmation**:
A single y/N-style **HITL** interaction requested before Dispatch when the
Risk gate requires one. Not specific to risk — any command may request one
ad-hoc via `AiloopClient`.
_Avoid_: prompt, approval (overloaded).

**CommandSpec**:
A Command's typed argument declaration (`src/spec/`). **Mandatory** — every
Command has one (see ADR 0065); generated from the command's annotated struct
by `#[derive(CommandSpec)]` (ADR 0064). Drives the parser, generates help,
derives MCP JSON Schemas, feeds the Spec validator, and produces the typed
extractor. Use "spec" only as shorthand for CommandSpec; never as a generic
word for any declaration.

**ArgSpec**:
The per-argument piece inside a **CommandSpec** (name, kind, value type,
required-ness, etc.). Declaration-time, not runtime.

**ArgValue map**:
The runtime, typed parsed-args value (`HashMap<String, ArgValue>`) — the
**single erased intermediate** every entry path converges on. CLI argv produces
it via the clap mapper; MCP/`chat` JSON produces it via `json_value_to_arg_value`.
Dispatch carries it to the leaf, where the typed wrapper deserializes it into the
Command's typed args struct. "Args" alone is ambiguous — qualify `ArgValue map`
(runtime) vs `ArgSpec` (declaration). See `docs/adr/0061-typed-handlers-argvalue-backbone.md`.

**CommandArgs** _(removed — see ADR 0061)_:
Formerly a stringly parsed-args value (`.positional`/`.named`) handed to `execute`.
The framework flattened the typed **ArgValue map** down to this and consumers
un-flattened it back to typed structs (newton's `TryFrom<CommandArgs>` adapters,
fastskill's clap re-parse). Removed: the **ArgValue map** is now the only runtime
arg form, and `execute` receives a **typed args struct** directly.

**CommandPath**:
The hierarchical identifier of a Command, e.g. `cluster/get`. Rendered with
slashes in identifiers and flattened to underscores at the MCP boundary
(`<app>_cluster_get`, see `src/mcp/mod.rs`).

**Spec validator**:
The framework-provided validation pass (`SpecValidator`) derived
automatically from a Command's **CommandSpec**. Runs at Stage 2 of the
validation pipeline.

**Custom validator**:
The user-supplied closure on the `Command.validator` field. Runs *in
addition to* the Spec validator (not as a fallback); the two diagnostic
lists are concatenated. "Validator" alone is ambiguous — always qualify
"Spec validator" or "Custom validator".

**AppContext**:
The **user-supplied** trait carrying application state and services (API
clients, config, …). The Command's `execute` callback receives it. Anything
specific to the consuming binary lives here.

**DispatchEnv**:
The **framework-internal** struct (`src/app/dispatch.rs`) carrying services
the framework owns during a dispatch: the Command registry, ailoop client,
stdout capture, etc. Combined with `AppContext` at Dispatch time inside a
wrapper context. Not part of the public API — but the user/framework split
is a real architectural concept and the right mental model when reading the
code.
_Avoid_ stuffing framework-owned services into user `AppContext`, or
user state into `DispatchEnv`.

**AiloopContext**:
A narrow trait the wrapper implements to hand the ailoop client to code
that needs HITL. Conceptually a slice of `DispatchEnv`, not of `AppContext`.

**Plugin**:
A third-party bundle of declarative command metadata loaded from a
**Plugin manifest**. Today plugins are **metadata-only** — registering a
plugin does **not** add a Command to the in-process registry and there is
no Dispatch path for plugin commands. See
`docs/adr/0002-plugins-metadata-only.md`.

**Plugin registry**:
The top-level TOML config (`plugin-registry.toml`) that lists available
plugins by name and points at their manifests.
_Avoid_: confusing with the in-process Command registry (`AppBuilder`).

**Plugin manifest**:
A per-plugin JSON file (pointed at by `manifest_path` in the Plugin
registry) declaring the plugin's commands and their (currently unused)
`CommandExecution`.

**PluginCommand**:
A declarative entry inside a Plugin manifest. **Distinct from Command** —
different type, no Dispatch path, surfaces only for discovery (e.g. by the
Ask resolver). _Avoid_ treating a PluginCommand as a Command.

**Plugin root**:
The filesystem boundary that `manifest_path` may not escape. Traversal is
rejected with `PLUGIN_PATH_ESCAPE`.

**Ask LLM stack**:
The in-tree LLM providers under `src/llm/` (OpenAI, Anthropic) driven by
`LLM_PROVIDER`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `LLM_MODEL`. Used
**only** by the `ask` command. Slated for removal alongside `ask` (see
`docs/adr/0001-two-llm-stacks.md`).

**Chat agent stack**:
The external `aikit-agent`-based stack used **only** by the `chat` command,
driven by `AIKIT_LLM_URL`, `AIKIT_MODEL`, `OPENAI_API_KEY`. Intended
long-term replacement for the Ask LLM stack.

> "LLM" alone is ambiguous in this repo — always qualify which stack.

**HITL** (human-in-the-loop):
Umbrella term for any user interaction routed through the paired
`ailoop serve` process (Confirmation, questions, notifications,
authorization). The framework has no stdin fallback — ailoop is the HITL
channel.

## Relationships

- A **Command** is registered exactly once with `AppBuilder`.
- In MCP / `chat` mode, every registered **Command** is automatically exposed
  as a **Tool**; there is no Tool that is not backed by a Command.
- Every entry path (argv, `ask`, `chat`, MCP) performs **Resolution** then
  **Dispatch**; only the Resolution strategy differs.
- The `ask` and `chat` paths insert a **Risk gate** between Resolution and
  Dispatch; the MCP path inserts an **MCP tool gate** instead.
- A **Confirmation** is one kind of **HITL** interaction; the Risk gate
  requests a Confirmation when the **Risk tier** requires one.
- A **Command**'s `execute` receives both `AppContext` (user state) and
  framework services from `DispatchEnv` via the wrapper.
- The **Ask LLM stack** powers Ask resolution; the **Chat agent stack**
  powers `chat`. They are independent today; `chat` is intended to replace
  `ask` (see ADR 0001).
- A **Plugin** contributes **PluginCommand** metadata only — no Command is
  added to the registry and no Dispatch path exists (see ADR 0002).

## Example dialogue

> **Dev:** "If a user types `myapp ask 'wipe staging'` and the LLM picks
> the `deploy` command, what stops it from running?"
>
> **Domain expert:** "Ask resolution returns a `(Command, CommandArgs)`
> pair like any other Resolution. But before Dispatch, the Risk gate looks
> up `deploy` in the Risk policy — `deployment` is Destructive by default,
> so the gate blocks unless `ALLOW_DESTRUCTIVE_COMMANDS=1`, and even then
> it requires a Confirmation routed through ailoop (HITL). Only then does
> Dispatch invoke `execute`."
>
> **Dev:** "And if the same command is called through MCP?"
>
> **Domain expert:** "MCP skips the Risk gate entirely — the MCP entry
> path goes through the MCP tool gate instead, and that's opt-in via
> `with_mcp_tool_gate`. A Command exposed as a Tool over MCP has no
> automatic Confirmation. That's deliberate: MCP clients aren't humans."
>
> **Dev:** "What about a `PluginCommand` named `deploy` in some manifest?"
>
> **Domain expert:** "It can't be dispatched at all. Plugins are metadata
> only today — Ask resolution can *see* a PluginCommand for discovery, but
> there's no execution path. If the LLM picks one, Dispatch fails."

## Flagged ambiguities

- **"Tool"** is sometimes used loosely to mean Command — restrict it to the
  MCP/chat surface only.
- **"LLM"** is ambiguous because two independent stacks exist (Ask vs
  Chat) — always qualify which.
- **"Validator"** is ambiguous — Spec validator and Custom validator both
  run; the lists are concatenated, not fallbacks.
- **"Args"** is ambiguous — the **ArgValue map** is the runtime erased form,
  the Command's typed args struct is what `execute` receives, `ArgSpec` is
  declaration-time. (`CommandArgs` removed — see ADR 0061.)
- **"Load a plugin"** does *not* register a Command. The README's
  "load third-party commands" phrasing is `[PLANNED]` (see ADR 0002).
- **"Account" / "User" / "Project"** — not part of this domain; if any
  consumer crate uses these, they belong in *that* crate's CONTEXT.md, not
  here.
