# cli-framework

Domain glossary for the `cli-framework` Rust library: a CLI application
framework with a central command registry, optional LLM-assisted resolution
(`ask`, `chat`), ailoop-backed human-in-the-loop prompts, plugin loading,
optional MCP exposure, and optional authentication.

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
Distinguish two things that both touch this trait: (1) **user-stored services**
— fields a consumer puts in its own context impl; (2) **framework accessors** —
defaulted methods on the trait that return `None`/a no-op and are *populated by
the wrapper* (`opt_registry`, `opt_global_args`, and `opt_token_provider`
[`auth` feature, ADR 0069] today; the planned `opt_config` / `telemetry`).
The latter are the only handle reachable through `&mut dyn
AppContext`, so framework-owned services that handlers must reach are exposed
this way — this is **not** the same as stuffing a service into user state.
One deviation worth noting: `opt_registry` / `opt_global_args` return a
borrowed `Option<&T>`, but `opt_token_provider` returns an owned
`Option<Arc<dyn TokenProvider>>` — the cheap `Arc` clone lets a handler move
the provider into a client that outlives the `&mut dyn AppContext` borrow.
_Avoid_ extension traits on the wrapper (e.g. `AiloopContext`) as the handler
access path: they are unreachable from `&dyn AppContext` (a handler must build
its own client, as the `ailoop` example does).

**DispatchEnv**:
The **framework-internal** struct (`src/app/dispatch.rs`) carrying services
the framework owns during a dispatch: the Command registry, ailoop client,
stdout capture, the **TokenProvider** when wired via
`AppBuilder::with_token_provider` (`auth` feature), etc. The provider lives
here, not in user `AppContext` state; the wrapper surfaces it through
`opt_token_provider`, exactly as it surfaces the registry through
`opt_registry`. Combined with `AppContext` at Dispatch time inside a
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

**auth feature** _(ADR 0069; shipped in 0.5.8)_:
The Cargo feature flag in `cli-framework` that enables the **TokenProvider**
trait, **AccessToken**, **Auth commands**, the `AppContext::opt_token_provider()`
accessor, and **AuthenticatedHttpClient**. Gated consistently with the existing
optional capabilities (`api-server`, `doctor`, `mcp-server`, etc.). Consumers
that never use auth pay no compile cost and get no extra subcommands.
Note the asymmetry with `cli-framework-oidc`: only that crate's **client** half
(`OidcClient`) depends on this feature. Its **server** half
(`oidc_validation_layer` / `OidcClaims`) depends on `api-server` instead and
needs neither `auth` nor a `TokenProvider` — a validate-only API server never
touches this feature.
_Avoid_: enabling auth behaviour without this flag — all auth types and
accessors are only present when `auth` is enabled.

**Interactive flow** / **Non-interactive acquisition**:
The load-bearing axis the `token()` contract pivots on. An **interactive flow**
needs a human to complete an authorization step in real time — **Device Code**
and **Auth Code + PKCE** (browser/prompt). **Non-interactive acquisition** is a
machine-to-machine token exchange with no human — a **refresh-token exchange**
or the **Client Credentials** grant. The rule: `token()` MAY perform
non-interactive acquisition on its own, but MUST NOT start an interactive flow;
interactive flows happen only under an explicit `auth login`.
_Avoid_: "automatic vs manual", "silent vs prompted" — the canonical axis is
*interactive* (needs a human now) vs *non-interactive* (machine-to-machine).
The test that decides it: does running the command pop a browser/prompt?

**TokenProvider**:
The generic trait in `cli-framework` that abstracts how an application
acquires a bearer token. Declares `token()`, `login()`, `logout()`, and
`invalidate()`. `token()` returns a currently-valid token, performing
**non-interactive acquisition** when needed — a refresh-token exchange, or a
client-credentials grant — but **never an interactive flow**. It returns
`AuthError::NotAuthenticated` only when a valid token cannot be obtained
non-interactively (i.e. the sole remaining path is an interactive login); for a
client-credentials provider with nothing cached, `token()` therefore acquires
directly rather than returning `NotAuthenticated`. Interactive login happens
only through an explicit `auth login`. `login()` and `logout()` default to returning
`AuthError::NotSupported` — a consumer using a static key or custom SSO is not
forced to implement them. The default is an **error, not a silent no-op**:
`auth login` against such a provider exits 1 (see **Auth commands**). When wired
via `AppBuilder::with_token_provider`, the framework auto-registers the built-in
**Auth commands**.
_Avoid_: calling it "auth client", "credential provider", or "identity provider"
(that's the external service, not this trait).

**Logged in**:
Canonically means **a usable or refreshable token is currently cached** — the
sense reported by `TokenStatus.logged_in`, `peek()`, and `auth status
--no-refresh`. Distinct from **"can obtain a token"**: a client-credentials
provider can always obtain one (**non-interactive acquisition**) even when
nothing is cached, so it is *not* "logged in" until it has acquired and cached.
This is why `auth status` (which calls `token()`, so it answers "could I make an
authenticated call right now?") may differ from `auth status --no-refresh`
(which calls `peek()`, answering "is a token already in the cache?") for such a
provider. The divergence is intentional.
_Avoid_: using "logged in" to mean "has credentials configured" — a provider can
be fully configured (client id + secret) and not logged in.

**AccessToken**:
The value returned by `TokenProvider::token()`. Carries the raw bearer string
and an optional expiry instant — nothing more. Any refresh token is retained
internally by the provider and **never appears in an AccessToken** (refresh is
the provider's concern). Opaque to cli-framework — the framework only injects it
as `Authorization: Bearer <raw>` and never inspects claims.
_Avoid_: calling an `AccessToken` a "JWT" — on the client side it is an opaque
bearer string the framework never parses. (The **server** side legitimately
treats incoming bearer tokens as JWTs; see `OidcClaims` /
`oidc_validation_layer()` — that qualification is intentional, not a conflict.)

**AuthError**:
The error type returned across the **TokenProvider** surface. Two variants are
load-bearing in the design and named throughout this glossary:
- `NotAuthenticated` — `token()` has no cached token and no usable refresh
  token. Surfaced (not auto-recovered) by `auth status` / `auth token` and the
  401-retry path; the cue to run `auth login`.
- `NotSupported(&str)` — the default result of `login()` / `logout()` on a
  provider that does not implement them (the `&str` names the operation, e.g.
  `"login"`); drives the `auth login` / `auth logout` exit-1 behavior.
Other variants (network, provider-specific) exist at the implementation level
but are not part of the canonical language.
_Avoid_: treating `NotAuthenticated` and `NotSupported` as interchangeable —
the first means "no token yet", the second means "this provider can't do that".

**Auth commands**:
The built-in command group (`auth login`, `auth logout`, `auth status`,
`auth token`) auto-registered by `AppBuilder` when a **TokenProvider** is
wired in. Registered in `cli-framework`; the behavior of `login` / `logout`
is delegated to the provider, and a provider returning `NotSupported` makes
`auth login` / `auth logout` exit 1. `auth status` and `auth token` work for
any provider and are **non-interactive** — they call `token()`, which MAY
refresh or perform **non-interactive acquisition** (client-credentials) over the
network and write the cache, but never triggers an interactive prompt or
browser; they surface `NotAuthenticated` as "not logged in; run `auth login`"
rather than launching a flow.
_Avoid_: "auth module", "login commands"; and prefer **non-interactive** over
"passive" — these commands are not necessarily side-effect-free (a refresh is a
network call + cache write), they merely never prompt.

**AuthenticatedHttpClient**:
A thin wrapper around `RetryableHttpClient` in `cli-framework`. It fetches
`TokenProvider::token()` **once per logical request** and injects the result as
`Authorization: Bearer <raw>`; the inner `RetryableHttpClient` may then retry
that same already-signed request up to its own transient budget (5xx / 429 /
network — note 401 is **not** in that budget). The **401 retry is a distinct,
single, auth-layer retry that wraps the inner client**: on a 401 the wrapper
calls `TokenProvider::invalidate()`, re-fetches `token()`, and re-issues the
request once. If `token()` then returns `NotAuthenticated` it surfaces that
error rather than launching a login flow (the 401 path never pops an
interactive prompt). Does not own a `TokenProvider` directly — the handler
retrieves the provider via `AppContext::opt_token_provider()` and constructs the
client itself. The framework does not pre-build or vend a shared instance.
_Avoid_: "auth client" (overloaded with the OIDC client concept).

**cli-framework-oidc**:
A companion crate in the same workspace that provides a concrete
**TokenProvider** implementation backed by OpenID Connect / OAuth 2.0. Works
with any OIDC-compliant provider (Keycloak, Azure AD, …). Split into two
Cargo features matching its two halves:
- **`client`** — `OidcClient` and the three **OIDC flows**; depends on
  `cli-framework/auth`.
- **`server`** — `oidc_validation_layer()` + `OidcClaims`; depends on
  `cli-framework/api-server`, not on `auth`.
A consumer enables only the half it needs. Consumers that need no OIDC depend
on neither.
_Avoid_: "Keycloak crate" — the crate is provider-agnostic at the OIDC level;
Keycloak is one backend.

**Token cache**:
The **client-side**, **on-disk** store where `cli-framework-oidc` (client half)
persists access and refresh tokens between CLI invocations. Holds **tokens**.
Owned entirely by the OIDC crate — not a concept in cli-framework core. Cache
directory is supplied explicitly at `OidcClient` construction time (consumer
knows their app name and XDG paths). When the config manager (ADR 0067) ships, a
convenience helper will read the path from the config layer instead. Default
format: JSON file at `<cache_dir>/oidc-token.json`, permissions 0600.
_Avoid_: "credential store", "keychain" (those imply OS secret storage, which
is not in scope for v1). _Distinguish from the **JWKS cache**_ — different side,
different contents.

**JWKS cache**:
The **server-side**, **in-memory** store where the **server** half's
`oidc_validation_layer` caches the provider's JSON Web Key Set (the public
**verification keys**) to validate incoming bearer tokens. Holds **keys, not
tokens**; 5-minute TTL, serve-stale-on-error. An unknown `kid` forces one
refetch (the cue for a rotated signing key), bounded on both axes — **single-flight**
(≤1 fetch in flight) and a **min-refetch-interval** rate-limit — so an
attacker-supplied, unsigned `kid` can't amplify into a fetch flood against the
shared IdP (ADR 0070). It
**never reads the Token cache** — the two caches share nothing. This is the
precise distinction that kills the "same binary shares one cached token"
misreading: the human logs in once on the client side (Token cache); the server
independently fetches verification keys (JWKS cache) to check *incoming* tokens.
_Avoid_: implying tokens flow from the Token cache into validation — the server
validates *received* tokens against keys, full stop.

**OidcClaims**:
The typed set of validated claims extracted from a verified JWT — the **success
type of both** `OidcValidator::validate()` and the `oidc_validation_layer`. A
plain value (`sub`, `aud`, `scopes`, `roles`, `raw`, …); axum is not required to
obtain one. It **additionally** implements the axum `FromRequestParts` extractor,
so handlers behind the layer can pull the value the layer inserted into the
request extension map — that is one optional consumption path, not the type's
identity. Entirely a `cli-framework-oidc` type — cli-framework core defines
nothing for the server-side claims surface.
_Avoid_: calling `OidcClaims` "the extractor" as if axum were required — it is a
plain value the layer and the validator both produce; the extractor is one way to
consume it.

**oidc_validation_layer()**:
The function in `cli-framework-oidc` (server half) that **returns a**
`tower::util::BoxCloneSyncServiceLayer<Router, Request<Body>, Response, Infallible>`
— the exact type `ApiServerBuilder::auth()` accepts — built from an `OidcValidationConfig`
(issuer, audience, JWKS TTL, …). The layer validates incoming
`Authorization: Bearer` tokens against the provider's JWKS. Behavior: caches
JWKS with a **5-minute TTL**; on JWT signature validation failure with cached
keys, refetches JWKS once before returning 401 (handles key rotation without
spurious rejections); a short debounce prevents thundering-herd refetches under
concurrent load.
_Avoid_: "auth middleware" — use the function name to be precise. It is a tower
`Layer`, not a bare middleware service. After spec 018 the layer is a thin
**transport adapter over `OidcValidator`** — it parses the `Authorization`
header, calls `OidcValidator::validate_authorization`, and maps the typed
`OidcValidationError` back to an HTTP response. The verification itself lives in
`OidcValidator`, not here.

**OidcValidator** _(spec 018)_:
The cloneable, callable handle for verifying a JWT **in-process**, with no axum
request — the library-facing counterpart to `oidc_validation_layer`. Built from
an `OidcValidationConfig` via `OidcValidator::new` (same construction + config
validation as the layer). `validate(token)` verifies an already-extracted bearer
token; `validate_authorization(Option<&str>)` parses an `Authorization` header
value (case-insensitive `Bearer` scheme) first. Both return
`Result<OidcClaims, OidcValidationError>`. `Clone + Send + Sync`; clones share
one `Arc`-backed `JWKS cache` / discovery cell / single-flight gate, so
concurrent calls don't duplicate fetches. The crate exposes **only this concrete
type — there is no `TokenValidator` trait** (ADR 0071): the framework runtime
never calls a validator, so the trait seam, if wanted, belongs to the consumer.
_Avoid_: "TokenValidator" (that is the *consumer's* trait, not this crate's);
"validation middleware" (that is the layer — `OidcValidator` is callable, not a
tower `Layer`).

**OidcValidationError** _(spec 018)_:
The **fully typed** outcome of a failed verification — the error half of
`OidcValidator::validate`. Four variants split by transport shape:
`MissingToken` (no credential offered), `MalformedAuthorization` (a credential
was offered but isn't a well-formed `Bearer <token>`, including non-UTF-8),
`InvalidToken(TokenRejection)` (a token was extracted and rejected), and
`JwksUnavailable` (infra: no keys). The rejection cause is the nested
**`TokenRejection`** enum (`Undecodable`, `UnsupportedAlgorithm`, `UnknownKey`,
`Malformed`, `Expired`, `NotYetValid`, `InvalidSignature`, `InvalidIssuer`,
`InvalidAudience`). Consumers match on these variants; they do **not**
string-match. The HTTP `WWW-Authenticate` `error_description` strings are
*derived* from the variant inside the layer's single `error_to_response`
mapping, not carried on the error.
_Avoid_: a stringly-typed `error_description: Option<String>` payload — the
reason is a typed variant; the wire string is a transport detail of the layer.

**OIDC flow**:
The OAuth 2.0 grant type used by `cli-framework-oidc` to acquire tokens.
Three flows ship in v1:
- **Device Code**: CLI prints a URL + code; user completes login on any
  device. Works headless and over SSH. No local server or browser required.
- **Auth Code + PKCE**: CLI opens a browser on the local machine; Keycloak
  redirects to a short-lived loopback server (`localhost:<port>/callback`)
  that captures the code. Standard workstation flow.
- **Client Credentials**: No user interaction — `client_id` + `client_secret`
  exchanged directly for a token. For CI pipelines and service accounts.

All three flows produce the same result for the framework: an **AccessToken**.
Each flow may also yield a refresh token, retained internally by `OidcClient`
and never surfaced in the AccessToken. Flow selection is a construction-time
decision on `OidcClient`, not runtime.
_Avoid_: "OAuth flow", "grant type" in user-facing text — "flow" is the
canonical term here.

**Invocation surface**:
The entry path through which a **Command** was dispatched: `cli` (argv run),
`chat` (LLM chat tool call), `mcp` (MCP `tools/call`), or `api` (HTTP API
handler). Carried on `DispatchEnv` and stamped by each entry path before
**Dispatch**. Used as a telemetry attribute (`cli.invocation.surface`) to
slice usage analytics by entry point. Lives in `cli_framework::app` — it is
a dispatch concept, not a telemetry concept.
_Avoid_: "mode", "transport" (surface is about the caller, not the wire).

**Telemetry handle**:
The `&dyn Telemetry` value returned by `AppContext::telemetry()`. Always
present — the default impl returns a zero-sized `NoopTelemetry`; the dispatch
wrapper overrides it with the live handle when the `telemetry` feature is
enabled and an endpoint is configured. App handlers call `ctx.telemetry()` to
emit spans, events, counters, and histograms through the framework's configured
OTel pipeline without wiring the SDK themselves. App spans automatically nest
under the enclosing command span because they attach to the current `tracing`
context.
_Avoid_: "OTel handle", "metrics client" — the canonical term is Telemetry handle.

**Telemetry guard**:
A RAII value created at run-entry-time (when `App::run_with_args` or
`ApiServerBuilder::serve` initialises the OTel SDK) that force-flushes and
shuts down all OTel providers when dropped. Owned as a local variable by the
run entry-point, so its lifetime matches the active run exactly. `Drop` is the
backstop; the entry-point also flushes explicitly on both success and error
return paths so Ctrl-C and SIGINT do not lose buffered spans.
_Avoid_: "shutdown hook", "flush guard" — Telemetry guard is the canonical term.

**Telemetry config** (`TelemetryConfig`):
The value passed to `AppBuilder::with_telemetry(…)` that controls the OTel
subsystem. Populated from `OTEL_*` environment variables (via
`TelemetryConfig::from_env()`) with explicit fields taking precedence.
Key fields: `enabled`, `endpoint` (absent → whole subsystem is inert),
`protocol` (`http/protobuf` default or `grpc`), `sample_ratio`,
per-signal toggles (`traces_enabled`, `metrics_enabled`, `logs_enabled`),
and `record_arg_values` + `arg_value_allowlist` (security gate — R13).
`OTEL_SDK_DISABLED=true` is a final veto evaluated at init time and cannot
be overridden by any builder field.
_Avoid_: "OTel config", "observability config" — use Telemetry config.

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
- A **TokenProvider** wired via `AppBuilder::with_token_provider` lives in
  **DispatchEnv** and is reached by handlers through
  `AppContext::opt_token_provider`, never from user `AppContext` state. Wiring
  it auto-registers the **Auth commands**. (`auth` feature.)
- `token()` refreshes silently when a refresh token is available but returns
  `NotAuthenticated` when nothing is cached — it never auto-launches an **OIDC
  flow**. Acquiring a token interactively is exclusively `auth login`'s job.
- **AuthenticatedHttpClient** injects the `token()` result as a bearer header
  and, on a 401, calls `invalidate()` then retries once; it never escalates to
  an interactive login.
- Token *acquisition* (client) and token *validation* (server) are independent
  halves of **cli-framework-oidc**: the client half depends on the `auth`
  feature, the server half (`oidc_validation_layer` / `OidcClaims`) depends on
  `api-server`. A binary that is both client and server uses both.

## Example dialogue

> **Dev:** "If a user types `myapp ask 'wipe staging'` and the LLM picks
> the `deploy` command, what stops it from running?"
>
> **Domain expert:** "Ask resolution returns a `(Command, ArgValue map)`
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

---

> **Dev:** "Our binary is both the CLI and the API server. A user runs
> `myapp some-command`, which calls our own API, but they've never logged
> in. What happens?"
>
> **Domain expert:** "The handler pulls the **TokenProvider** off the context
> via `opt_token_provider`, builds an **AuthenticatedHttpClient**, and fires
> the request. The client calls `token()` — but nothing's cached and there's
> no refresh token, so `token()` returns `NotAuthenticated`. It does **not**
> pop a browser; the command fails with a clear 'run `auth login`' message.
> Interactive login is only ever `auth login`'s job."
>
> **Dev:** "So they run `myapp auth login`. That's the same binary — where
> does the OIDC part come from?"
>
> **Domain expert:** "`cli-framework` only owns the `TokenProvider` trait and
> the `auth` commands. The actual flow lives in `cli-framework-oidc`'s
> **client** half — the consumer wired an `OidcClient` (say, **Auth Code +
> PKCE**) as the provider. `auth login` opens the browser, the loopback server
> catches the code, and the token lands in the **Token cache**. Now `token()`
> returns it and the command works."
>
> **Dev:** "And on the server side of the same binary — what checks the token
> that arrives in the `Authorization` header?"
>
> **Domain expert:** "That's the **server** half of `cli-framework-oidc`,
> which depends on `api-server`, not `auth`. The consumer passed
> `oidc_validation_layer()` to `ApiServerBuilder::auth()`. It validates the
> JWT against Keycloak's JWKS — caching keys for 5 minutes, refetching once on
> a signature miss so a key rotation doesn't cause spurious 401s — and exposes
> the claims to handlers as `OidcClaims`. Note the asymmetry: the client half
> treats the token as an opaque bearer string; the server half is the one that
> actually parses it as a JWT."
>
> **Dev:** "What if a token expires mid-session and the API returns 401?"
>
> **Domain expert:** "The `AuthenticatedHttpClient` calls `invalidate()` and
> retries once. If the provider can refresh silently, the retry succeeds. If
> not, `token()` returns `NotAuthenticated` and the error surfaces — still no
> surprise browser prompt. The 401 path never escalates to interactive login."

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
