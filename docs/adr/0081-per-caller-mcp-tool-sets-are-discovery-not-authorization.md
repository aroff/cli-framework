# Per-caller MCP tool sets are discovery, not authorization

Status: accepted (2026-09-22)

The MCP tool set has so far been fixed at build time: `McpToolRegistry` is constructed from the
command registry and every caller sees the same `tools/list`. Consumers that serve more than one
caller from one server — tenants, plan tiers, per-user saved queries — need callers to discover
different tools. ADR 0066 already established the boundary this must respect: cli-framework is the
generic MCP transport, it knows nothing about tenants, plans or users.

Two seams already existed and are reused rather than duplicated. `McpRequestAuthenticator`
(`McpToolRegistry::with_request_authenticator`) turns the HTTP request headers into an opaque
`Option<Arc<dyn Any + Send + Sync>>`; `dispatch_tool_call_with_identity` threads that value to the
command's `execute`. The identity is deliberately type-erased: the framework never inspects it.

## Decisions

**D1 — One hook, consumed by both `tools/list` and `tools/call`.** `McpDynamicToolProvider` is a
single `Arc<dyn Fn(Option<Arc<dyn Any + Send + Sync>>) -> McpDynamicToolsFuture + Send + Sync>`
installed by `McpToolRegistry::with_dynamic_tools` (or `AppBuilder::with_mcp_dynamic_tools`), and
**both** request paths derive from it. A separate list-only hook would let the advertised set and
the callable set drift, which is the failure mode this ADR exists to prevent: a tool listed but
uncallable, or callable but unlisted, is worse than no per-caller set at all.

**D2 — The callback is async, and takes exactly what `authenticate` returns.** Real providers read
a database or a policy service. The signature mirrors `McpRequestAuthenticator` so the two seams
compose without an adapter, and returns an owned `'static` boxed future so it can be awaited inside
a spawned dispatch task.

**D3 — `Vec<McpDynamicTool>`, not `HashMap<String, Command>`.** A map return reintroduces
iteration nondeterminism in the per-caller block. A `Vec` lets the provider fix the order it wants
and lets the framework preserve it, which is what makes repeated `tools/list` calls byte-stable for
one identity. The element type was `(String, Command)` as first shipped; see Amendment A, which
widened it to a struct without changing the ordering guarantee. `(String, Command)` still converts
in with `From`, so the tuple form remains the spelling for a tool a static `CommandSpec` already
describes.

**D4 — Static wins; the merge adds no ordering of its own.** A name registered at construction
always resolves to its static command, and the provider is not consulted at all on a static hit in
`tools/call`. In `tools/list` the static block is emitted exactly as `list_tools()` produced it —
the existing `HashMap` iteration order is *not* sorted as a side effect of this change, because
that would silently alter output for every consumer who never installs the hook — and the
per-caller block is appended after it in provider order. Names already in the static set, or
repeated within one provider result, are skipped, so no name is described twice. A `tools/list`
carrying two tools with one name is a protocol problem for clients, not an aesthetic one, which is
why the second case is deduped as strictly as the first. Skipping is not silent: each drop is
reported at `tracing::warn!` naming the tool and the rule that dropped it, because the dropped tool is
invisible to the provider otherwise. It is still a drop and not a rejection — the rest of the
result is listed as normal, since a provider that built a command for this caller has already made
that decision (D8).

**D5 — Recomputed per request, never cached.** The hook runs once per `tools/list`, and once per
`tools/call` that misses the static set. Caching would reintroduce drift across identities, and the
framework has no way to know when a consumer's per-caller set changes.

Two consequences the framework does not hide. The miss-path rate is chosen by the caller,
unauthenticated included — a loop of `tools/call` with invented names is one provider invocation,
typically a database read, per request, with no cache in front of it. That is a rate-limiting
question for the consumer's edge, not a performance footnote, and it is stated as such in the
rustdoc. And "never cached" is a statement about this server only: `tools/call` re-runs the hook
so revocation is prompt at dispatch, while `tools/list` is only as fresh as the client's last call
to it. We advertise `tools` without `listChanged` and never send
`notifications/tools/list_changed`, so a client that listed once at session start keeps displaying
a withdrawn tool. Enforcement lives at dispatch precisely because discovery lags.

**D6 — No identity means `None`, not "skip the hook".** Under stdio there is no HTTP request; over
HTTP there may be no authenticator installed, or the authenticator may reject the credentials. All
three cases invoke the hook with `None`. Skipping it would make the anonymous case silently differ
from an authenticated one that resolves to no tools, and would deny consumers the ability to serve
a public tool set.

**D7 — No hook installed is the byte-identical status quo.** `dynamic_tools` defaults to `None`;
with no provider, `list_tools_for_identity` returns `list_tools()` unchanged without awaiting
anything, `ServerHandler::list_tools` does not run the consumer's authenticator at all, and
dispatch resolution is the original `resolve_tool` lookup. "Byte-identical" is read to include
side effects on consumer code, not only the bytes on the wire. This is asserted by test, not
merely by inspection.

**D8 — This is a discovery surface, and the documentation must say so.** Hiding a tool from a
caller's list authorizes nothing:

- MCP clients may send `tools/call` with any name; nothing requires a prior `tools/list`.
- Tool names are derived from command ids (`{app}_{path}`) and are therefore guessable.
- Every statically registered command stays callable by every caller, whatever the hook returns.
- A tool the hook returns for an identity *is* callable by that identity, by construction (D1).
- The command risk policy applies to a per-caller tool, but only as well as the provider
  classifies it: `CommandRiskPolicy::classify` keys on `Command.id` and then the command's
  category, and a provider-built id is by construction absent from the consumer's `tiers` map. With
  no category set, the tool takes `default_tier`, which is `Safe`. "The risk policy applies" must
  not be read as "my destructive tier covers per-caller tools".
- `McpToolExportPolicy` / `expose_mcp` filter the static set at construction and do **not** filter
  the hook: a command the provider returns is exported even with `expose_mcp: false`. Making the
  hook honour the export policy was considered and rejected — a provider that deliberately built a
  command for this caller has already made that decision, and a silently dropped tool would be a
  worse failure than an unexpected one. The provider is the filter for its own commands, and the
  rustdoc says so.
- What *does* still apply to a per-caller tool, unchanged: argument validation, the command risk
  policy, and any `with_gate` / `AppBuilder::with_mcp_tool_gate` execution gate, because dispatch
  builds the same `CommandAsToolBridge` regardless of where the command came from. The gate is an
  enforcement point in a way this hook is not — but a limited one:
  `ExecutionGate::before_execute(cmd, args, tier)` is not given the identity, so it can deny a
  class of calls and not a particular caller. Per-caller authorization has exactly one correct
  home: the command's own `execute`, via `ctx.request_identity::<T>()`.

Consumers MUST still enforce authorization inside the command's own `execute`. The rustdoc on
`with_dynamic_tools` and the README's MCP **Security** section both state this. The alternative — making the hook an enforcement point —
was rejected: cli-framework cannot distinguish "not yours" from "does not exist" for a consumer's
domain objects, and an authorization mechanism that silently degrades to "tool not found" is a bad
one.

## Consequences

- `serve_mcp_with_gate_opts_with_resources`, `serve_mcp_stdio_opts_with_resources` and
  `create_mcp_serve_command_with_deps` each take one additional trailing
  `Option<McpDynamicToolProvider>`. These are public but low-level; the supported entry point is
  `AppBuilder::with_mcp_dynamic_tools`.
- `ServerHandler::list_tools` now reads `context.extensions` and runs the authenticator, where it
  previously ignored the request context entirely — but only when a provider is installed. A
  consumer's authenticator is consumer code that may log, emit metrics or spend a rate-limit
  budget, so running it on a `tools/list` whose result could not be used would be a new observable
  side effect on an existing path; D7 covers that too, and a test asserts the absence of the call.

## Amendment A — a per-caller tool may own its MCP presentation (2026-09-22)

D3 as first shipped returned `Vec<(String, Command)>`, and `tools/list` derived every per-caller
tool's `description` from `Command::summary()` and its `inputSchema` from `build_input_schema`
over the command's `CommandSpec`. `CommandSpec` and `ArgSpec` are `&'static str` throughout
(`summary`, `long_about`, `ArgSpec::name`, `ArgSpec::help`, …). The motivating consumer — "the
actions of the tenant-scoped plugins installed for the authenticated user" — has exactly the shape
that cannot satisfy that: its field names and help text are database rows. The only way to
advertise them through a `&'static str` spec is `Box::leak` on every `tools/list`, an unbounded
leak on a hot path. Argument *validation* was never the problem (`Command::validator` is an owned
`Arc<dyn Fn>`, and undeclared JSON keys already pass through `json_value_to_typed_map` to
`execute`); advertising was.

**A1 — Widen the hook's element, do not add a field to `Command`.** `Command` is constructed by
struct literal in ~232 places across this repository and its consumer `entitystore`, none of them
with `..Default::default()`, so a new field breaks every one of them for a feature only the MCP
hook uses. The hook, by contrast, shipped in the same unreleased cycle and has no consumer outside
this repository's tests, so widening it costs nothing now and would be expensive later.
`McpDynamicToolsFuture` now yields `Vec<McpDynamicTool>`, where `McpDynamicTool` is `{ name:
String, command: Command, presentation: Option<McpToolPresentation> }` and `McpToolPresentation`
is `{ description: String, input_schema: serde_json::Value }`. `impl From<(String, Command)> for
McpDynamicTool` keeps the tuple spelling for tools a static spec already describes.

**A2 — A presentation is advertising, full stop.** `list_tools_for_identity` reads it;
`dispatch_tool_call_with_identity` reads only `McpDynamicTool::command`. Argument validation, risk
classification and the execution gate see a presented tool and an unpresented one identically.
Making the presented schema a validation gate was rejected: it would put an *advertising* artifact
on the enforcement path, with the framework enforcing a schema it neither authored nor
understands, and it would silently change what `execute` receives — the surface D8 already says
must be enforced in the command body.

**A3 — The presented schema replaces the derived one; it is never merged.** When
`presentation` is `Some`, `build_input_schema` is not called at all. A merged schema has two
authors, and when a call is rejected neither the provider nor cli-framework can say which half the
caller violated. One author per tool keeps that question answerable. `_meta` and `visibility`
still come from the `Command` on both paths: they describe the command, not its interface.

**A4 — De-duplication is decided before the descriptor is built.** Both drop rules in D4 key on
the name alone, so an entry that loses either race is dropped whole, presentation included. A
presentation describes a tool that is being listed; it is never a reason to list one.

### Consequences

- `McpDynamicToolsFuture`'s output type changes from `Vec<(String, Command)>` to
  `Vec<McpDynamicTool>`. Providers written against the tuple form add `.into()` (or
  `.map(McpDynamicTool::from)`); nothing else about the hook changes.
- `mcp::schema::command_to_tool_descriptor_presented` is added beside
  `command_to_tool_descriptor_full`, gated on `mcp-server`.
- Nothing changes for a consumer that installs no provider, or whose provider returns tuples.
