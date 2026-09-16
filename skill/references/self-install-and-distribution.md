# Self-install and distribution

Status: tracks ADR 0080 (accepted). Phase 1 is implemented behind the
`self-install` feature: `self install`, `self uninstall`, `self status`, the
receipt, PATH handling, the four doctor checks and both installer scripts.
`self update`, `--system` and `--from` are phase 2 and are marked below.

Every end-user cli-framework app gets the same install story: a one-line
installer script per platform, a `self` command group that installs, updates
and uninstalls the binary, an install receipt, reversible PATH handling and
doctor checks. You add one builder call and copy two scripts. Service
binaries (`Deployment::Service`) get none of this and are deployed as images
or plain files.

## 1. What the user sees

```
curl -fsSL https://github.com/OWNER/REPO/releases/latest/download/install.sh | sh
irm https://github.com/OWNER/REPO/releases/latest/download/install.ps1 | iex
```

Pin a version by changing the path: `releases/download/v1.4.2/install.sh`.

```
myapp self install   [--bin-dir DIR] [--no-modify-path] [--unmanaged] [--force] [--from-bootstrap]
myapp self uninstall [--purge]
myapp self status    [--json]
myapp doctor                       # includes the install.* checks

# phase 2
myapp self install   [--system] [--from ARCHIVE]
myapp self update    [stable|latest|1.4.2] [--check] [--from ARCHIVE] [--force]
```

- `--force` replaces a binary that no receipt records, and rewrites an
  unreadable receipt.
- `--from-bootstrap` is for the installer scripts: the downloaded binary moves
  itself into the bin dir instead of copying.
- `--purge` also removes the app's config, state and data directories. Each
  must be named after the app, must not contain the home directory and must
  not sit at or directly under a filesystem root. Keychain entries are never
  touched.

Failures exit 1 with a stable code at the start of the message:

| Code | Meaning |
|---|---|
| `SI001` | Refused to run as root or under sudo without `MYAPP_INSTALL_ALLOW_SUDO=1` |
| `SI002` | A binary no receipt records is in the way; rerun with `--force` |
| `SI003` | The receipt cannot be read, parsed or written |
| `SI004` | Nothing to uninstall, or a package manager owns the binary |
| `SI005` | `--purge` refused a directory that failed a guard |
| `SI006` | A filesystem or registry operation failed, or no home dir was found |

The group exists only for `Deployment::EndUser`, the default. If the app
already registers its own `self` command, the framework logs a warning and
leaves it alone.

If your app moves built-ins under a namespace (fastskill uses `cli`), the
group follows: `fastskill cli self install`. If you want `myapp install` at
the root, register the exported command yourself; the framework adds no
aliases.

Environment variables, all optional:

| Variable | Effect |
|---|---|
| `MYAPP_VERSION` | Channel or version for the script: `stable` (default), `latest`, `1.4.2` |
| `MYAPP_INSTALL_DIR` | Same as `--bin-dir` |
| `MYAPP_NO_MODIFY_PATH=1` | Same as `--no-modify-path` |
| `MYAPP_UNMANAGED=1` | Same as `--unmanaged`: no PATH edit, no receipt (images, CI) |
| `MYAPP_INSTALLER_BASE_URL` | Download base for a mirror. Must be `https`; plain `http` only for `127.0.0.1`, `localhost` or `[::1]` |
| `MYAPP_GITHUB_TOKEN` or `GITHUB_TOKEN` | Private GitHub releases; sent only as a header, never to a mirror |
| `MYAPP_INSTALL_ALLOW_SUDO=1` | Allow `self install` under root or sudo |
| `MYAPP_NO_UPDATE_CHECK=1` | Silence the passive update notice, if the app enabled it |

## 2. Enable it in the app

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["self-install"] }
```

```rust
use cli_framework::app::AppBuilder;
use cli_framework::self_install::SelfInstallOptions;

let app = AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    .with_self_install(SelfInstallOptions::github("OWNER/REPO"))
    .build(ctx)?;
```

Options on `SelfInstallOptions`:

| Option | Default | Use |
|---|---|---|
| `github("OWNER/REPO")` or `http("https://dl.example.com/myapp/")` | required | Where releases come from; `http` serves the same asset names plus `latest.json` (`{"version":"1.4.2"}`) |
| `tag_prefix("myapp-v")` | `"v"` | Monorepos that tag `myapp-v1.4.2` |
| `asset_template("{app}-{version}-{target}.{ext}")` | `"{app}-{target}.{ext}"` | Only if you already publish versioned names |
| `completions(true)` | off | Also install shell completions and list them in the receipt |
| `public_key(...)` | none | Reserved for signature verification (phase 3) |
| `update_notice(true)` | off | Passive once-a-day check (phase 3) |

## 3. Release contract

Tag `v1.4.2` (or `<tag_prefix>1.4.2`) publishes:

```
myapp-x86_64-unknown-linux-musl.tar.gz
myapp-aarch64-unknown-linux-musl.tar.gz
myapp-x86_64-apple-darwin.tar.gz
myapp-aarch64-apple-darwin.tar.gz
myapp-x86_64-pc-windows-msvc.zip
myapp-aarch64-pc-windows-msvc.zip
SHA256SUMS
install.sh
install.ps1
```

- Each archive holds the binary (`myapp` or `myapp.exe`) and `LICENSE` at the
  top level, no directory prefix. This is what the existing cli-rust-dev
  release workflow, the Homebrew formula generator and the Scoop manifest
  generator already expect.
- `SHA256SUMS` is `sha256sum` format with bare filenames, generated after all
  archives exist from inside the directory that holds them:
  `sha256sum *.tar.gz *.zip > SHA256SUMS`. It replaces `checksums.txt`; the
  generators' lookup matches either name.
- Linux targets are musl so one binary runs on every distribution and in
  Alpine containers. Substitute `-gnu` only if a dependency forces it.
- macOS ships two per-arch archives, not a universal binary.
- Upload `install.sh` and `install.ps1` on every release so the `latest`
  and version-pinned one-liners work with no hosting.
- Mark prereleases as such on GitHub: `stable` skips them, `latest` includes
  them.
- Decide your tier (section 7). Tier 2 means the macOS binaries are signed
  and notarised and the Windows binaries are Authenticode-signed before the
  archives are built.

## 4. Copy the installer scripts

Copy `install-templates/install.sh` and `install-templates/install.ps1` into
`scripts/`, replace the placeholders, commit, and add the upload step to the
release workflow.

| Placeholder | Example |
|---|---|
| `__APP__` | `myapp` (binary name) |
| `__APP_ENV__` | `MYAPP` (uppercase, `-` and `.` become `_`, the framework's own prefix rule) |
| `__REPO__` | `OWNER/REPO` |
| `__TAG_PREFIX__` | `v` (or `myapp-v` for a monorepo) |
| `__SELF_CMD__` | `self`, or `cli self` when built-ins live under the `cli` namespace |

Replace `__APP_ENV__` before `__APP__`, because the first contains the second:

```bash
sed -i 's/__APP_ENV__/MYAPP/g; s/__APP__/myapp/g; s#__REPO__#OWNER/REPO#g; s/__TAG_PREFIX__/v/g; s/__SELF_CMD__/self/g' scripts/install.sh scripts/install.ps1
grep -n '__[A-Z_]*__' scripts/install.sh scripts/install.ps1 && echo "placeholder left"
```

cli-framework tests the templates on every pull request with
`scripts/ci/self-install-e2e.sh` (sh, dash) and `scripts/ci/self-install-e2e.ps1`,
which serve a release on loopback and run install, status, a checksum
mismatch, a refused plain-http mirror and uninstall. Copy them if you change
the scripts.

The scripts do four things: detect OS and arch, download the archive and
`SHA256SUMS` into a temporary directory inside the bin dir, verify and
extract, and run `self install --from-bootstrap`. In bootstrap mode the
binary moves itself into place, and the script removes the empty directory,
so nothing is left behind. Placement, PATH and receipt logic live in the
binary; the scripts do not change when that logic does.

## 5. What `self install` does on each platform

| | Linux | macOS | Windows |
|---|---|---|---|
| Per-user bin dir | `$XDG_BIN_HOME` (absolute only) or `~/.local/bin` | same | `%USERPROFILE%\.local\bin` |
| `--system` bin dir (phase 2) | `/usr/local/bin` | `/usr/local/bin` | `C:\Program Files\myapp\bin` |
| PATH edit | one shared line `. "$HOME/.local/bin/env"` appended to rc files that already exist: `.profile`, `.bash_profile`, `.bash_login`, `.bashrc`, and `$ZDOTDIR/.zshenv` or else `$ZDOTDIR/.zshrc`; per-app `~/.config/fish/conf.d/myapp.fish` | same; many Macs only have `.zshrc` | user `Path` in `HKCU\Environment`, prepended, then `WM_SETTINGCHANGE`; removed on uninstall only if no other `.exe` lives in the dir |
| Env file | `~/.local/bin/env`, `env.fish`, shared by every framework app | same | none |
| Receipt | `~/.local/share/myapp/install-receipt.json` | `~/Library/Application Support/myapp/install-receipt.json` | `%LOCALAPPDATA%\myapp\install-receipt.json` |
| Post-copy fix-up | `chmod 755` | `chmod 755`, drop `com.apple.quarantine` | drop `Zone.Identifier` stream |
| Replace running binary | atomic rename | atomic rename | rename to `.exe.old`, cleanup at next start |

Rules that hold everywhere: no prompts; no rc file is created or rewritten;
nothing is edited when `CI` is set or stdout is not a terminal, the line is
printed instead; the shared PATH line is never removed on uninstall; temp
files live inside the bin dir, never `/tmp`; `self install` refuses root and
sudo unless the allow variable is set; `--system` (phase 2) never elevates
and prints the command to run instead.

## 6. Platform tricks worth knowing

Windows
- A running `.exe` cannot be overwritten or deleted, only renamed. Seeing
  `myapp.exe.old` once after an update is expected; the next run removes it.
- New PATH values reach new terminals only. The installer prints
  `$env:Path = "...;$env:Path"` for the current session.
- `Invoke-WebRequest` attaches the mark-of-the-web; the script unblocks the
  download and the binary strips the stream from the installed copy.
- SmartScreen warns on first run from Explorer for unsigned binaries, not
  from a terminal. Tier 2 signs.
- Windows PowerShell 5.1 on older Windows 10 builds does not default to
  TLS 1.2; the script sets it. Where `RuntimeInformation.OSArchitecture` is
  missing, it falls back to `PROCESSOR_ARCHITEW6432` and
  `PROCESSOR_ARCHITECTURE`.
- `XDG_BIN_HOME` is ignored on Windows by both the script and the binary.
- The receipt path comes from the known-folder API, which ignores a
  `LOCALAPPDATA` override, so tests that set it still write the real one.
- Defender may hold a freshly written exe for a few seconds; a rare
  `Access is denied` right after install is that scan.
- `%APPDATA%` roams between machines, `%LOCALAPPDATA%` does not, which is why
  the receipt lives in the latter.
- AppLocker and WDAC default rules block executables in user-writable
  directories, including `~/.local/bin`. That is tier 3: IT deploys through
  winget or an MSI into Program Files, and `self update` is disabled by
  policy. No per-user installer works there, ours included.

macOS
- `curl` downloads carry no quarantine flag, so tier 1 runs unsigned binaries
  fine. A browser download is quarantined and Gatekeeper blocks an unsigned,
  un-notarised binary before `self install` can run. MDM-managed Macs
  enforce this for everything: tier 2 notarises.
- zsh is the default shell and Terminal opens login shells; `.zshenv` under
  `$ZDOTDIR` is read by every zsh invocation, which is why it is the file
  edited.
- Apple Silicon reports `arm64`; the script maps it to `aarch64-apple-darwin`.
  Under Rosetta a terminal reports `x86_64`; the script checks
  `sysctl.proc_translated` and still installs the native build.

Linux
- musl static binaries avoid glibc-version complaints and run in Alpine and
  distroless images.
- `~/.local/bin` is on PATH by default on Debian, Ubuntu and Fedora only if it
  exists at login, so the rc line is still written.
- `sudo` resets PATH to `secure_path`; tools that must run under `sudo` need
  `--system` or a symlink in `/usr/local/bin`.
- Containers and provisioning: `COPY` the binary, or run the script with
  `MYAPP_UNMANAGED=1 MYAPP_INSTALL_DIR=/usr/local/bin`. Do not call
  `self install` in a Dockerfile.
- `/tmp` mounted `noexec` does not matter because temp files live in the bin
  dir.
- The script needs only POSIX sh, `tar`, `curl` or `wget`, and `sha256sum`
  or `shasum`. It is tested under dash and BusyBox sh; BusyBox `wget` lacks
  `--https-only`, so use `curl` where redirect protocol matters.

## 7. Distribution tiers

| Tier | Machines | What you promise |
|---|---|---|
| 1 | Unmanaged laptops | Scripts and `self` commands; unsigned allowed |
| 2 | Managed laptops | Tier 1 plus signed and notarised binaries; signing is required |
| 3 | Locked-down fleets | winget or MSI and Homebrew deployed by IT; `self update` disabled by policy |

Enterprise controls the framework provides: company CAs are trusted through
native TLS roots; `MYAPP_INSTALLER_BASE_URL` points at a mirror;
`GITHUB_TOKEN` handles private releases; `--from ARCHIVE` handles air-gapped
sites; the managed-configuration keys `self_update.enabled`,
`self_update.channel`, `self_update.base_url` and
`self_update.minimum_version` let an organisation pin or disable updates
fleet-wide, and the refusal names the key.

## 8. Coexistence with package managers

`self update` reads the receipt and refuses to touch a binary installed by
Homebrew, Scoop, winget or cargo, printing that manager's upgrade command.
Without a receipt it infers the manager from the executable path. Keep
publishing to your tap and bucket via the homebrew-project and
scoop-bucket-project skills; the routes do not conflict, and the shared env
file is the same one uv and other cargo-dist tools write.

## 9. Checklist for a new app

1. Add the `self-install` feature and the `with_self_install` builder call.
   It implies `doctor` and `config`.
2. Extend the release workflow: six targets, top-level archive layout,
   `SHA256SUMS`, upload `install.sh` and `install.ps1`.
3. Copy the two templates, replace the placeholders, commit under `scripts/`.
4. Pick a tier and, for tier 2, add signing and notarisation to the workflow.
5. README install section: the two one-liners first, then package managers,
   then "download an archive" last.
6. Run `myapp doctor` after a fresh install on each platform and confirm the
   four `install.*` checks pass.
