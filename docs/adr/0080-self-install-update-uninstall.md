# Self-install, update and uninstall as opt-in framework commands

Status: accepted (2026-09-16)

Derived apps are single static binaries and each solves "get it onto a laptop,
put it on PATH, keep it current" alone: fastskill ships a shell script, cogni
ships paired scripts, aikit documents a manual tarball, newton relies on
Homebrew and Scoop, and none share vocabulary, checksum naming or update
handling. The only `install` in the framework is `mcp install`, which
registers the app in an agent's MCP config and never touches the binary. This
ADR adds one opt-in feature, `self-install`, so every app inherits the same
commands, layout, platform handling and installer scripts, in the shape Claude
Code and rustup use: a thin installer script fetches and verifies, the binary
places itself.

## Decisions

### Scope and gating

- Cargo feature `self-install`, off by default. Implies `doctor` and `config`
  (the receipt guard and policy keys read through the config layer). Adds
  `flate2`, `zip`, `sha2`, `semver`, and `self-replace` on Windows. `tar`,
  `winreg`, `dirs` and `reqwest` are already present.
- The `self` command group registers only for `Deployment::EndUser`, as the
  `telemetry` group does. A `Service` binary is placed by its operator.
- The group follows the app's built-in command namespace
  (`with_builtin_command_namespace`), so fastskill gets `fastskill cli self
  install`. The framework registers no root aliases; an app that wants
  `myapp install` registers the exported command itself and it appears in
  help like any other command. aikit already owns root `install` and
  `update`, which is why the group exists.

### Commands

```
self install   [--bin-dir DIR] [--system] [--no-modify-path] [--unmanaged]
               [--from ARCHIVE] [--force] [--from-bootstrap]
self update    [stable|latest|<semver>] [--check] [--from ARCHIVE] [--force]
self uninstall [--purge]
self status    [--json]
```

- Fully non-interactive. The framework's HITL channel is ailoop, which does
  not exist on a fresh machine, and no TTY prompt is added.
- `self install` refuses to run as root or under `sudo` unless `--system` is
  given or `<APP>_INSTALL_ALLOW_SUDO=1` is set, because it would otherwise
  write into root's home. `--system` never elevates; it fails with the exact
  command to run.
- `--unmanaged` installs to the given directory with no PATH edit and no
  receipt (golden images, CI). A later `self update` then refuses, as
  intended.
- `--from ARCHIVE` installs or updates from a local archive verified against
  a local `SHA256SUMS` beside it, with no network (air-gapped sites).
- `--from-bootstrap` is passed only by the installer scripts: the binary
  moves itself into place instead of copying, so the download leaves nothing
  behind. Any other invocation copies and tells the user the source can be
  deleted.
- `self update` refuses when: there is no receipt; the receipt's binary path
  differs from the running executable; the receipt or the executable path
  says a package manager installed it (`/Cellar/`, `\scoop\apps\`, `\WinGet\`,
  `/.cargo/bin/`), printing that manager's upgrade command; the target is a
  downgrade and no explicit version was given; or an enforced policy
  disables it. A lock file in the bin dir stops concurrent updates.
- `self uninstall` reverses only what the receipt lists. `--purge` also
  removes the config and data roots after printing every path, refuses if a
  root resolves to the home directory or a filesystem root, and never touches
  keychain entries.
- Shell completions are installed only when the app enables it in
  `SelfInstallOptions`; the success output otherwise prints the completion
  command and `mcp install` as hints.

### Layout

| | Linux | macOS | Windows |
|---|---|---|---|
| Bin dir (per user) | `$XDG_BIN_HOME` or `~/.local/bin` | `~/.local/bin` | `%USERPROFILE%\.local\bin` |
| Bin dir (`--system`) | `/usr/local/bin` | `/usr/local/bin` | `C:\Program Files\<app>\bin` |
| Receipt | `~/.local/share/<app>/install-receipt.json` | `~/Library/Application Support/<app>/install-receipt.json` | `%LOCALAPPDATA%\<app>\install-receipt.json` |
| Env file | `<bin dir>/env`, `env.fish` | same | none |
| PATH edit | one rc line | one rc line | user `Path` in `HKCU\Environment` |

- One bin dir on all three OSes, the uv and Claude Code convention, so every
  framework app on a machine shares one PATH entry.
- Flat binary in the bin dir. Unix update writes a temp file in the same
  directory and renames over the running binary. Windows renames the running
  exe to `<app>.exe.old`, moves the new one in, and deletes the `.old` at the
  next start (`self_install::startup_cleanup()`, called by the builder, never
  fails a command). Temp files always live inside the bin dir, never `/tmp`,
  which may be `noexec`.
- The env file is shared: every framework app writes identical content, and
  the rc line `. "$HOME/.local/bin/env"` is appended once, deduplicated by
  exact match, to rc files that already exist among `~/.profile`,
  `~/.bash_profile`, `~/.bash_login`, `~/.bashrc` and `$ZDOTDIR/.zshenv`
  (or `$ZDOTDIR/.zshrc` when there is no `.zshenv`).
  Fish gets a per-app `~/.config/fish/conf.d/<app>.fish`. No rc file is ever
  created or rewritten, nothing is edited when `CI` is set or stdout is not a
  terminal, and the shared line is never removed on uninstall because a
  sibling app may depend on it. Windows prepends to the user `Path`
  (`REG_EXPAND_SZ`) and broadcasts `WM_SETTINGCHANGE`; `setx` is not used
  because it truncates at 1024 characters. Both platforms print the export
  for the current shell.
- The receipt lives in the machine-local state root, not the config root,
  because `%APPDATA%` roams between machines and a receipt describes one
  device. It is a separate file from telemetry state so reinstall and
  uninstall never touch consent. Glossary: one **Install**, two files.
- Post-placement fix-ups: `chmod 755`; remove `com.apple.quarantine` on
  macOS; strip the `Zone.Identifier` stream on Windows.

Receipt schema (version 1): `app`, `version`, `target`, `channel`,
`bin_dir`, `binary_path`, `method` (`self-install`, `script`, `unmanaged`,
`homebrew`, `scoop`, `winget`, `cargo`, `unknown`), `modified_path` (files
edited), `completions` (files written), `source` (`{kind, repo|base_url,
tag_prefix, asset_template}`), `installed_at`.

### Release source and contract

```rust
AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    .with_self_install(
        SelfInstallOptions::github("aroff/myapp")
            .tag_prefix("myapp-v")                 // monorepos; default "v"
            .asset_template("{app}-{target}.{ext}") // default; {version} available
            .completions(true)
            .public_key(None)                      // reserved for phase 3
            .update_notice(false),                 // phase 3, opt-in
    )
```

- `ReleaseSource` has two implementations: `GitHub { repo, api_base }` and
  `Http { base_url }` serving the same asset names plus `latest.json`
  (`{"version":"1.4.2"}`). The `Http` source is also the test seam.
- Channels: `stable` is the newest non-prerelease, `latest` includes
  prereleases, a bare semver maps to `<tag_prefix><semver>`.
- Assets are archives, the cargo-dist convention the existing release
  workflow, Homebrew generator and Scoop generator already expect: `tar.gz`
  on Unix and `zip` on Windows, each holding the binary and `LICENSE`.
  Default name `<app>-<target>.tar.gz|zip`; cogni keeps its versioned names
  through the template.
- Six targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`,
  `aarch64-pc-windows-msvc`. An app that cannot build musl substitutes
  `-gnu` for itself only.
- `SHA256SUMS` in `sha256sum` format with bare filenames replaces
  `checksums.txt`; the generators' existing lookup matches it. Verification
  is mandatory in every path from phase 1. The names `<asset>.minisig` and
  the `public_key` option are reserved now so phase 3 signatures add no
  breaking change.
- Downloads use HTTPS only, the existing rustls client, and refuse redirects
  to plain HTTP. A token (`GITHUB_TOKEN` or `<APP>_GITHUB_TOKEN`) travels only
  in an `Authorization` header, never in a URL, and is never echoed.
  `<APP>_INSTALLER_BASE_URL` overrides the download base for mirrors in the
  scripts and the updater.
- The installer scripts `install.sh` and `install.ps1` are published as
  release assets, so `releases/latest/download/install.sh` and the
  version-pinned `releases/download/v1.4.2/install.sh` need no hosting. The
  scripts detect OS and arch, download the archive and `SHA256SUMS` to a
  temporary directory inside the bin dir, verify, extract, run `self install
  --from-bootstrap`, and remove the directory. The body is wrapped in a
  function called on the last line so a truncated download executes nothing.

### Distribution tiers

| Tier | Machines | What the app promises |
|---|---|---|
| 1 | Unmanaged laptops | Installer scripts and `self` commands; unsigned allowed |
| 2 | Managed laptops (MDM, SmartScreen policy) | Tier 1 plus signed and notarised binaries; signing is required, not recommended |
| 3 | Locked-down fleets (AppLocker, WDAC) | winget or MSI and Homebrew deployed by IT; `self update` disabled by policy |

The framework supports all three; signing and MSI packaging stay outside it.

### Enterprise controls

- Prerequisite PR: switch the framework's reqwest from `rustls-tls` to
  `rustls-tls-native-roots` so company CAs behind TLS-intercepting proxies
  are trusted. This also fixes managed configuration and telemetry export in
  the same environments.
- Policy keys read through managed configuration, enforced policy winning
  over flags and the refusal naming the key: `self_update.enabled`,
  `self_update.channel`, `self_update.base_url`,
  `self_update.minimum_version`.
- Passive update notice is opt-in per app, at most one check per 24 hours,
  printed to stderr after the command, silenced by telemetry level `off`,
  `CI`, and `<APP>_NO_UPDATE_CHECK=1`.

### Doctor checks

| Check id | Finding when |
|---|---|
| `install.on_path` | bin dir not on PATH, or `which <app>` resolves elsewhere |
| `install.shadowed` | another copy of the app earlier on PATH (catches pre-receipt installs) |
| `install.receipt` | no receipt, or receipt version or path differs from the running binary |
| `install.stale_files` | `.old` executable, leftover temp directory or lock file in the bin dir |

### Verification

The `Http` release source served from a local directory is the test seam.
Unit tests cover resolution, checksum, receipt and rc-file editing against
fixtures. Integration tests on a GitHub Actions matrix of the three OSes run
install, update and uninstall end to end against that local source, assert
and revert every rc-file and registry edit, and run both installer scripts
the same way. The `.ps1` gets a parser check in CI. No test touches GitHub
releases. Phase 1 is not done until the matrix is green.

## Alternatives considered

- **cargo-dist and axoupdater**: generate installers, receipts and an
  updater, but every app would adopt cargo-dist's release pipeline and the
  update UX would live outside the framework. The receipt is deliberately
  close to theirs and the shared env file is their convention, so uv and a
  framework app coexist on one machine.
- **Fat installer scripts, dumb binary** (uv, bun, deno): placement logic in
  shell and PowerShell twice, untestable from Rust, and no `update`.
- **Bare binaries instead of archives**: fewer dependencies, but would change
  the release workflow and both package-manager generators for every consumer.
- **Root `install` and `update` by default** (Claude Code): collides with
  aikit and with domain verbs generally; hidden aliases were rejected as two
  spellings with one invisible.
- **Versions directory plus launcher** (Claude Code): rollback for free, but
  symlinks on Windows need developer mode; a `.prev` file in phase 3 covers
  rollback.
- **Per-app env files and marked rc lines**: reversible, but removing a line
  could break a sibling app; the shared line is harmless and conventional.
- **Apps & Features entry** (bun): useful for IT inventory, deferred to phase
  3 with rollback as one more registry footprint to reverse.

## Phases

1. `self install`, `self uninstall`, `self status`, receipt, PATH handling,
   the four doctor checks, both installer scripts, the skill reference, the
   three-OS test matrix. Prerequisite: the native-roots TLS PR.
2. `self update` with both sources, checksum verification, the Windows
   rename and startup cleanup, policy keys, `--system`, `--from`, and the
   `SHA256SUMS` rename in the cli-rust-dev workflow and the two generators.
3. Opt-in passive notice, minisign or sigstore verification, `self rollback`
   via `.prev`, Apps & Features entry.

## Refinements made while implementing phase 1

Recorded here so the decisions above stay readable; none reverses them.

- Phase 1 ships `self install`, `self uninstall` and `self status` only.
  `--system`, `--from` and `self update` wait for phase 2. The Windows
  `.exe.old` swap and `startup_cleanup()` land in phase 1 because a reinstall
  over a running copy already needs them.
- zsh: when `$ZDOTDIR/.zshenv` does not exist, the line goes to
  `$ZDOTDIR/.zshrc` if that exists. Many macOS users only have `.zshrc`, and no
  rc file is ever created.
- `XDG_BIN_HOME` counts only when it is an absolute path, and never on
  Windows. The scripts apply the same rule as the binary.
- Windows `Path` removal on uninstall happens only when the receipt records
  that this install added the entry, and is skipped while other `.exe` files
  still live in that directory, since a sibling app may depend on it. This
  mirrors keeping the shared rc line on Unix.
- Installer scripts: `<APP>_INSTALLER_BASE_URL` must be `https`; plain `http`
  is accepted only for `127.0.0.1`, `localhost` and `[::1]`, which is what the
  end-to-end tests serve. A mirror never receives the GitHub token. Redirects
  must stay on `https`.
- The templates gain a `__SELF_CMD__` placeholder (`self`, or `cli self` when
  the app sets `builtin_command_namespace`).
- `self install --force` also rewrites an unreadable receipt, which is what
  the `install.receipt` doctor check recommends.
- Runtime failures exit 1 with a message prefixed by a stable code:
  `SI001` root refused, `SI002` foreign binary in the way, `SI003` receipt
  unreadable or unwritable, `SI004` nothing to uninstall or package-manager
  owned, `SI005` purge guard, `SI006` filesystem, registry or directory
  resolution failure.
- On Windows the receipt path comes from the known-folder API, which ignores
  `LOCALAPPDATA` overrides; tests clean the real location up.

## Consequences

- Every derived app gets identical commands, flags, layout and platform
  handling with one builder call and two copied scripts.
- Consumers on a locked-down fleet still need IT packaging; the tiers say so.
- cogni migrates from `%LOCALAPPDATA%\Programs` to `~/.local/bin`; the
  shadowed-binary check flags the old copy.
- `es` keeps `Deployment::EndUser`: a person installs it, and `es serve` in a
  container simply never calls `self install`.
- Glossary: **Deployment** widened; **Self-install**, **Install receipt** and
  **Installer script** added.
