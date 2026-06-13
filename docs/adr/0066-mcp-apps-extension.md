# MCP extension: generic `_meta` passthrough, resource serving, app-only visibility

Status: accepted (corrected boundary — supersedes the original UI-typed shape)

cli-framework is the generic MCP transport for ALL its consumers. It may know generic MCP
protocol concepts — tools can carry an opaque `_meta`, the server can serve resources, tools can
be `visibility`-tagged. It MUST NOT know UI concepts like "ui", "resourceUri", or CSP — those
belong to a consumer (e.g. `entitystore-ui`). The MCP layer therefore provides three concept-free
capabilities under the existing `mcp-server` feature; no new default dependencies.

## Rule of thumb — typed vs. opaque

- A field cli-framework **acts on** stays typed. → `visibility` STAYS: cli-framework interprets it
  for app-only dispatch behavior (the tool remains dispatchable via `tools/call` but is flagged so
  hosts can hide it from the model).
- A field cli-framework only **passes through** becomes opaque. → per-tool and per-resource
  metadata are generic `serde_json::Value`; cli-framework never inspects their contents.

## The three capabilities

- **Generic per-tool `_meta` passthrough (CF-1).** `Command` carries an opaque
  `meta: Option<serde_json::Value>`, set via `Command::with_meta(value)`, plus
  `visibility: Option<Vec<String>>` via `with_visibility(..)`. The emitted tool's `tools/list`
  entry carries that `_meta` value **verbatim** (sourced directly from `cmd.meta`, not wrapped),
  and any visibility tags. The consumer supplies the entire `_meta` shape — e.g.
  `{"ui":{"resourceUri":"ui://es/x/detail","csp":{…},"preferApp":true}}` — and cli-framework emits
  it unchanged. The framework names no `ui`/`resourceUri`/`csp`.
- **Generic resource serving (CF-2).** A `ResourceRegistry` (URI → provider closure →
  `UiResource`) is held alongside the tool registry. `CliFrameworkHandler` implements
  `list_resources` / `read_resource`; a per-resource opaque `meta` (set via
  `UiResource::with_meta(value)`) is placed **verbatim** in `contents[]._meta`. The consumer builds
  any UI-shaped object (e.g. `{"ui":{"csp":{…}}}`) and passes it in. `get_info` advertises the
  `resources` capability only when the registry is non-empty (tools-only servers are unchanged).
- **App-only export / visibility (CF-3).** `visibility:["app"]` marks a tool that remains
  dispatchable via `tools/call` but is flagged so hosts can hide it from the model. It composes
  with `McpToolExportPolicy::ExposeMcpOnly` (the tool still appears in `tools/list`). This is the
  one MCP-Apps-adjacent concept cli-framework retains, because it changes dispatch behavior.

## R1 — does `rmcp` 1.6 carry per-tool `_meta`?

**Yes.** Verified by direct read of the vendored crate
(`rmcp-1.6.0/src/model/tool.rs:41`): `Tool` has `#[serde(rename = "_meta")] pub meta: Option<Meta>`,
and `Meta(pub JsonObject)` is freely constructible. `ResourceContents` (text and blob variants)
likewise carries `_meta` with a `with_meta(Meta)` setter (`model/resource.rs:71/80/116`). So we emit
the opaque metadata through the native rmcp types — no local tool-serialization fork.

One field gap: `rmcp::model::Tool` has **no** `visibility` field. Rather than fork the tool wire
type, `visibility` tags ride inside `_meta` (`_meta.visibility`) on the live rmcp `Tool`, while the
framework's own `McpToolDescriptor` keeps a dedicated top-level `visibility` field (where the
model-visible JSON-shape contract is asserted). `Tool` and `ServerInfo` are `#[non_exhaustive]`, so
both are built via constructors + field assignment rather than struct literals.

## Why this boundary

The original Phase 0 wrongly added typed UI vocabulary (`UiToolMeta`, `UiCsp`, a `Command::ui`
field) to the core command model. That made the generic transport library know presentation
concepts it has no business knowing (CFW-0001). Greenfield + strict: cli-framework provides a
**generic `_meta` passthrough + resource serving + visibility**; all UI / MCP-Apps semantics
(`ui`, `resourceUri`, `csp`, `preferApp`) live in the consumer. The engine, descriptor, and
resource layers stay ignorant of presentation; the consumer owns the entire `_meta` shape on both
tools and resources.

## Consequences

- `Command` gains two public fields (`meta`, `visibility`); all construction sites set them
  (`None` by default). Builders `with_meta` / `with_visibility` mirror `with_expose_mcp`.
- Public surface: `mcp::resources::{ResourceRegistry, UiResource, UiResourceBody, ResourceProvider,
  ResourceListing}` with `UiResource::with_meta`; `CliFrameworkHandler::{with_resource_registry,
  list_resources_result, read_resource_uri}`. **Removed:** `command::{UiToolMeta, UiCsp}` and the
  `Command::with_ui` / `UiResource::with_csp` builders.
- The transport entry points (`serve_mcp_*`, `build_mcp_axum_router`) are unchanged; the handler
  seam is exposed and exercised in-process by `tests/unit/mcp_apps.rs`.
- **CF-6 — populated `ResourceRegistry` is now served end-to-end.** The original CF-2 wiring left a
  gap: no serve entry point ever called `with_resource_registry`, so a populated registry had no
  route to the served handler. CF-6 closes this **additively** (signatures unchanged; default is an
  empty registry → a tools-only server, as before):
  - Consumer slot: `AppBuilder::with_mcp_resource_registry(Arc<ResourceRegistry>)`. The
    auto-registered `mcp serve` command threads it into the handler via `with_resource_registry`
    for **both** stdio and HTTP transports.
  - HTTP-mount seam: `mcp::build_mcp_axum_router_with_resources(...)` (the existing
    `build_mcp_axum_router` delegates with an empty registry). Pair with
    `ApiServer::mcp_router(...)` to mount under `/mcp`.
  - Lower-level variants taking `Arc<ResourceRegistry>`: `serve_mcp_stdio_opts_with_resources`,
    `serve_mcp_with_gate_opts_with_resources`, `transport_stdio::start_stdio_with_resources`,
    `transport_http::{start_streamable_http_with_resources, mcp_axum_router_with_resources}`.
  - The binding crate (entitystore-mcp-apps) builds a `ResourceRegistry`, calls
    `register`/`register_static` to attach `ui://…` providers, wraps it in `Arc`, and hands it to
    `AppBuilder::with_mcp_resource_registry` (stdio + the auto-registered HTTP serve) or to
    `build_mcp_axum_router_with_resources` (custom Axum mount).
  - End-to-end coverage: `tests/integration/mcp_http.rs::test_resources_list_and_read_via_mcp_serve_stdio_subcommand`
    drives the real `mcp serve --transport stdio` subprocess and asserts `resources/list` +
    `resources/read` return the registered `ui://` resource.
