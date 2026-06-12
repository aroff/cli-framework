# MCP-Apps extension: per-command UI metadata, resource serving, app-only visibility

Status: proposed

The MCP layer gains three capabilities so a consumer (e.g. `entitystore-ui`) can render
HTML views inside an MCP-Apps host without leaving the `cli-framework` command model. All of
it lives under the existing `mcp-server` feature; no new default dependencies.

- **Per-command UI metadata (CF-1).** `Command` carries an optional `ui: Option<UiToolMeta>`
  (`{ resource_uri, csp, prefer_app }`) and `visibility: Option<Vec<String>>`, set via
  `Command::with_ui(..)` / `with_visibility(..)`. The emitted tool advertises `_meta.ui`
  (`resourceUri`, optional `csp`, `preferApp`) on its `tools/list` entry, and any visibility
  tags (e.g. `["app"]`).
- **Resource serving (CF-2).** A `ResourceRegistry` (URI → provider closure → `UiResource`)
  is held alongside the tool registry. `CliFrameworkHandler` implements `list_resources` /
  `read_resource`; a per-resource `UiCsp` is placed in `contents[]._meta.ui.csp`. `get_info`
  advertises the `resources` capability only when the registry is non-empty (tools-only servers
  are unchanged).
- **App-only export (CF-3).** `visibility:["app"]` marks a tool that remains dispatchable via
  `tools/call` but is flagged so hosts can hide it from the model. It composes with
  `McpToolExportPolicy::ExposeMcpOnly` (the tool still appears in `tools/list`).

## R1 — does `rmcp` 1.6 carry per-tool `_meta`?

**Yes.** Verified by direct read of the vendored crate
(`rmcp-1.6.0/src/model/tool.rs:41`): `Tool` has `#[serde(rename = "_meta")] pub meta: Option<Meta>`,
and `Meta(pub JsonObject)` is freely constructible. `ResourceContents` (text and blob variants)
likewise carries `_meta` with a `with_meta(Meta)` setter (`model/resource.rs:71/80/116`). So the
spike's primary path holds: we emit UI metadata through the native rmcp types — no local
tool-serialization fork.

One field gap: `rmcp::model::Tool` has **no** `visibility` field. Rather than fork the tool wire
type, `visibility` tags ride inside `_meta` (`_meta.visibility`) on the live rmcp `Tool`, while the
framework's own `McpToolDescriptor` keeps a dedicated top-level `visibility` field (where the
model-visible JSON-shape contract is asserted). `Tool` and `ServerInfo` are `#[non_exhaustive]`, so
both are built via constructors + field assignment rather than struct literals.

## Why

Greenfield + strict: extend the command model and the existing rmcp handler rather than bolting on a
parallel transport. UI state lives on the `Command` (not in field metadata), keeping the engine /
descriptor layers ignorant of presentation. CSP travels with each resource so the host can sandbox
per-view.

## Consequences

- `Command` gains two public fields (`ui`, `visibility`); all construction sites set them
  (`None` by default). Builders `with_ui` / `with_visibility` mirror `with_expose_mcp`.
- New public surface: `command::{UiToolMeta, UiCsp}`, `mcp::resources::{ResourceRegistry,
  UiResource, UiResourceBody, ResourceProvider, ResourceListing}`,
  `CliFrameworkHandler::{with_resource_registry, list_resources_result, read_resource_uri}`.
- The transport entry points (`serve_mcp_*`, `build_mcp_axum_router`) are unchanged; wiring a
  populated `ResourceRegistry` into them is deferred to the consumer (Phase 2) — Phase 0 exposes
  the handler seam and is exercised in-process by `tests/unit/mcp_apps.rs`.
