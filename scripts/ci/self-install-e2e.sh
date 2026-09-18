#!/usr/bin/env bash
# End-to-end check of skill/references/install-templates/install.sh against a
# local release served on loopback (ADR 0080). Linux and macOS; the Windows
# twin is self-install-e2e.ps1.
#
#   cargo build --features self-install --bin cfw-self-install-demo
#   scripts/ci/self-install-e2e.sh [path/to/cfw-self-install-demo] [shell] [newer-demo]
#
# It renders the template for the demo app, builds the release layout the
# script expects (<base>/latest.json, <base>/v<ver>/<asset>, SHA256SUMS),
# runs the rendered script with a temporary HOME, and checks that the binary,
# the receipt and a clean uninstall all behave. `shell` defaults to sh; CI
# also runs it under dash where available.
#
# `newer-demo` is the demo built with CFW_DEMO_VERSION set higher. When it is
# given, the installed binary also runs `self update` against the same
# mirror, then `self rollback` three times.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
demo=${1:-${CARGO_TARGET_DIR:-$root/target}/debug/cfw-self-install-demo}
run_shell=${2:-sh}
newer=${3:-}
app=cfw-self-install-demo
[ -x "$demo" ] || { echo "demo binary not found: $demo" >&2; exit 1; }

version=$("$demo" --version | awk '{print $2}')
case "$(uname -s)" in
  Linux) os_part=unknown-linux-musl ;;
  Darwin) os_part=apple-darwin ;;
  *) echo "unsupported OS" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch_part=x86_64 ;;
  aarch64|arm64) arch_part=aarch64 ;;
esac
asset="${app}-${arch_part}-${os_part}.tar.gz"

work=$(mktemp -d)
server_pid=""
cleanup() {
  [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# Release layout.
rel="$work/release/v${version}"
mkdir -p "$rel" "$work/stage"
cp "$demo" "$work/stage/$app"
tar -czf "$rel/$asset" -C "$work/stage" "$app"
(cd "$rel" && { command -v sha256sum >/dev/null && sha256sum "$asset" || shasum -a 256 "$asset"; } > SHA256SUMS)
printf '{"version": "%s"}\n' "$version" > "$work/release/latest.json"

# Rendered script.
sed -e "s/__APP_ENV__/CFW_SELF_INSTALL_DEMO/g" \
    -e "s/__APP__/${app}/g" \
    -e "s#__REPO__#aroff/cli-framework#g" \
    -e "s/__TAG_PREFIX__/v/g" \
    -e "s/__SELF_CMD__/self/g" \
    "$root/skill/references/install-templates/install.sh" > "$work/install.sh"
if grep -n '__[A-Z_]*__' "$work/install.sh"; then
  echo "unrendered placeholder left in install.sh" >&2; exit 1
fi

# Loopback server on a free port.
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work/release" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do
  curl -fsS "http://127.0.0.1:${port}/latest.json" >/dev/null 2>&1 && break
  sleep 0.1
done

home="$work/home"
mkdir -p "$home"
touch "$home/.profile"
export HOME="$home" XDG_DATA_HOME="$home/.local/share" XDG_CONFIG_HOME="$home/.config" CI=1
unset XDG_BIN_HOME
export CFW_SELF_INSTALL_DEMO_INSTALLER_BASE_URL="http://127.0.0.1:${port}"

echo "== install ($run_shell)"
"$run_shell" "$work/install.sh"
bin="$home/.local/bin/$app"
[ -x "$bin" ] || { echo "binary not installed at $bin" >&2; exit 1; }
if ls -A "$home/.local/bin" | grep -q "^\.${app}-install\."; then
  echo "installer temp dir left behind" >&2; exit 1
fi
[ ! -s "$home/.profile" ] || { echo ".profile was edited although CI=1" >&2; exit 1; }

echo "== status"
status=$("$bin" self status --json)
echo "$status"
echo "$status" | grep -q '"method":"script"' || { echo "method is not script" >&2; exit 1; }

if [ -n "$newer" ]; then
  [ -x "$newer" ] || { echo "newer demo binary not found: $newer" >&2; exit 1; }
  new_version=$("$newer" --version | awk '{print $2}')
  [ "$new_version" != "$version" ] || { echo "newer demo reports $version too" >&2; exit 1; }
  new_rel="$work/release/v${new_version}"
  mkdir -p "$new_rel" "$work/stage-new"
  cp "$newer" "$work/stage-new/$app"
  # install.sh fetches musl; a gnu build updates to its own target.
  for part in unknown-linux-musl unknown-linux-gnu apple-darwin; do
    tar -czf "$new_rel/${app}-${arch_part}-${part}.tar.gz" -C "$work/stage-new" "$app"
  done
  (cd "$new_rel" && { command -v sha256sum >/dev/null && sha256sum ./*.tar.gz || shasum -a 256 ./*.tar.gz; } \
    | sed 's#  \./#  #' > SHA256SUMS)
  printf '{"version": "%s"}\n' "$new_version" > "$work/release/latest.json"

  echo "== update --check"
  "$bin" self update --check | tee "$work/out"
  grep -q "$new_version is available" "$work/out" || { echo "no update offered" >&2; exit 1; }
  [ "$("$bin" --version | awk '{print $2}')" = "$version" ] || { echo "--check changed the binary" >&2; exit 1; }

  echo "== update"
  "$bin" self update
  [ "$("$bin" --version | awk '{print $2}')" = "$new_version" ] || { echo "not updated" >&2; exit 1; }
  [ -f "$bin.prev" ] || { echo "previous binary not kept" >&2; exit 1; }
  "$bin" self status --json | grep -q "\"version\":\"$new_version\"" \
    || { echo "receipt not updated" >&2; exit 1; }

  echo "== rollback, and back again"
  "$bin" self rollback
  [ "$("$bin" --version | awk '{print $2}')" = "$version" ] || { echo "rollback failed" >&2; exit 1; }
  "$bin" self rollback
  [ "$("$bin" --version | awk '{print $2}')" = "$new_version" ] || { echo "second rollback failed" >&2; exit 1; }
  "$bin" self rollback
  printf '{"version": "%s"}\n' "$version" > "$work/release/latest.json"
fi

echo "== checksum mismatch is refused"
echo "0000000000000000000000000000000000000000000000000000000000000000  $asset" > "$rel/SHA256SUMS"
if "$run_shell" "$work/install.sh" 2>"$work/err"; then
  echo "install succeeded with a bad checksum" >&2; exit 1
fi
grep -q "checksum mismatch" "$work/err" || { cat "$work/err" >&2; exit 1; }

echo "== plain http to a non-loopback mirror is refused"
if CFW_SELF_INSTALL_DEMO_INSTALLER_BASE_URL="http://example.com" "$run_shell" "$work/install.sh" 2>"$work/err"; then
  echo "install accepted a plain http mirror" >&2; exit 1
fi
grep -q "must be https" "$work/err" || { cat "$work/err" >&2; exit 1; }

echo "== uninstall"
"$bin" self uninstall
[ ! -e "$bin" ] || { echo "binary still present" >&2; exit 1; }
[ ! -e "$bin.prev" ] || { echo "previous binary still present" >&2; exit 1; }
[ ! -e "$XDG_DATA_HOME/$app/install-receipt.json" ] || { echo "receipt still present" >&2; exit 1; }
echo "self-install e2e passed"
