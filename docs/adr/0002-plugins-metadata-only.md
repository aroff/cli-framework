# Plugins are metadata-only (no Dispatch path)

Status: accepted

The plugin module loads `plugin-registry.toml` and per-plugin manifests
(`PluginManifest`, `PluginCommand`, `CommandExecution::Subprocess`), but no
code in `src/` actually executes a `PluginCommand`. Plugin support is, in
practice, **purely declarative**: it exposes command metadata so that the
`ask` resolver can route natural-language queries to plugin-described
commands, but it cannot dispatch them. A `PluginCommand` is a different
type from a `Command` and is not added to the in-process Command registry.

We accepted this gap rather than ship a half-implemented subprocess
dispatcher. Spawning third-party binaries from inside the framework drags
in significant surface area — argv translation, stdin/stdout/stderr
handling under MCP's stdio-reserved transport, signal/cancellation
propagation, exit-code mapping, sandboxing — and none of that work has been
prioritized against in-process Commands or the `chat` agent.

**Trajectory:** unresolved. Options on the table are (a) implement
subprocess dispatch as a non-default feature, (b) restrict the plugin
system to LLM-discovery metadata permanently, or (c) replace it with an MCP
client (let plugins be MCP servers the framework calls into). Until one is
picked, the README and `CONTEXT.md` mark plugin commands as
`[PLANNED]` for execution and document them as metadata-only today.
