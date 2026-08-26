# Agents

Entries use [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) normative keywords.

CFW-0001 - Automated or human-assisted changes MUST keep this repository scoped to [**cli-framework**](https://github.com/aroff/cli-framework): a Rust library for CLI apps (command registry, optional `ask` / LLM, plugins, ailoop, security helpers). Agents MUST NOT treat this project as an unrelated codebase (for example generic TUI or widget frameworks).

CFW-0002 - Work that affects builds, merges, or shared workflow MUST align with [CONTRIBUTING.md](CONTRIBUTING.md).

CFW-0003 - Commit messages MUST conform to [Conventional Commits](https://www.conventionalcommits.org/).

CFW-0004 - Agents MUST NOT use `git commit --no-verify` to bypass hooks.

CFW-0005 - Change sets intended for integration SHOULD pass project checks: `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` (see [CONTRIBUTING.md](CONTRIBUTING.md) and `scripts/run-ci-tests.sh` for full parity).

CFW-0006 - Behavioral or public API changes SHOULD update consumer-facing docs ([README.md](README.md), [docs/](docs/), [specs/](specs/)) when users or integrators would otherwise be misled; module or flow edits SHOULD align the **System design** section in [CONTRIBUTING.md](CONTRIBUTING.md).

CFW-0007 - Temporary or generated artifacts MUST NOT be committed (for example build output under `target/`).

CFW-0008 - The **`cli-framework`** crate and materials in this repository are licensed under **[Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0)**. The `license` field in `Cargo.toml` and license notices in documentation MUST remain consistent with that choice unless maintainers explicitly approve a change.

CFW-0009 - When opening a pull request, agents MUST arm auto-merge immediately with `gh pr merge --auto --squash --delete-branch`. Required status checks gate the merge; auto-merge never merges early.

CFW-0010 - Agents MUST NOT use `--admin`, `--force`, or any other bypass or override flag to merge a pull request. If a required check is red or a review is required, the pull request MUST be left open and reported.

CFW-0011 - Agents MUST NOT merge a pull request with failing checks, even when the failure appears unrelated to the change.

CFW-0012 - Agents MUST NOT change repository visibility.
