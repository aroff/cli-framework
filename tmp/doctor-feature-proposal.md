# Proposal: built-in `doctor` framework (diagnostics + visualization)

## References (prior art)

1. **Homebrew `brew doctor`**  
   - Runs a battery of checks; documents that warnings help maintainers debug.  
   - Supports listing checks and verbose modes.  
   - Docs: [Troubleshooting](https://docs.brew.sh/Troubleshooting.html), implementation [cmd/doctor.rb](https://github.com/Homebrew/brew/blob/master/Library/Homebrew/cmd/doctor.rb).

2. **Flutter `flutter doctor`**  
   - Validator-based architecture (`DoctorValidator`), grouped output, extensible with project-aware checks.  
   - Useful model: **register validators**, run all, summarize pass/fail.  
   - Entry: [flutter_tools doctor.dart](https://github.com/flutter/flutter/blob/master/packages/flutter_tools/lib/src/doctor.dart).

3. **GitHub CLI `gh doctor`** (conceptual fit: checks for auth, tokens, git, etc.) — same UX expectation: one command, actionable messages.

## Goals

1. **`doctor` as a first-class pattern** in `cli-framework`: not one hard-coded binary behavior, but a **framework** apps extend.
2. **Built-in baseline**: minimal checks that apply to any CLI using the framework (exact list TBD in implementation; examples: “config file readable”, “cache directory writable”, “required env present”).
3. **Extension model**: each application registers **checks** (sync or async) with id, title, severity (`ok` / `warning` / `error` / `skipped`), and optional detail / remediation text.
4. **Visualization**: reuse existing CLI output facilities (`cli_output` tables, JSON lines or structured JSON when `OUTPUT_FORMAT=json`-style behavior exists) so CI and humans both get usable output.
5. **Exit codes**: convention documented — e.g. `0` if no `error`-level failures, `1` if any `error` (warnings alone may still exit 0 or 1; pick one and document).

## Non-goals

- Replacing domain-specific diagnostics (e.g. full database migrations checks) inside the framework; those stay in app code as **registered checks**.
- Shipping medical metaphors beyond the command name `doctor` (keep UX professional).

## Architecture sketch

```text
DoctorRunner
  ├── registers: Vec<Arc<dyn DoctorCheck<C>>>
  └── run(ctx: Arc<C>) -> DoctorReport

DoctorReport
  ├── items: Vec<DoctorFinding>
  └── summary: passed / warnings / errors counts

trait DoctorCheck<C: AppContext>: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn run(&self, ctx: Arc<C>) -> BoxFuture<'static, DoctorFinding>;
}
```

Apps register checks in `AppBuilder` or a `DoctorModule` alongside commands. Optional: `doctor --json`, `doctor --check <id>`.

## Integration with `cli-framework` today

- Add a **feature flag** (e.g. `doctor`) if we want to keep default builds minimal, or include in `default` if weight is low.
- Implement **`doctor` command** registration helper: `register_doctor_checks(...)` + optional `create_doctor_command()` mirroring `create_ask_command` pattern.
- **Rendering**: table output (severity column, message, hint); JSON for scripting.

## Deliverables

1. Trait + runner + default `doctor` command wiring.
2. 1–2 built-in checks that do not assume app-specific paths (or gated behind generic options).
3. Example under `skill/examples/` with custom checks.
4. Documentation in README + skill reference.

## Open questions

- Should framework ship a **default** `doctor` subcommand always, or only when the feature is enabled?
- Interaction with **risk / ask** and **MCP**: should doctor results be exposed as structured logs only (recommended v1)?

## Related issue

Tracked on the cli-framework GitHub project as a dedicated “Idea” item.
