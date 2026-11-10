#!/usr/bin/env bash
# Build the pinned Gold Band (github.com/diodeme/Gold-Band, AGPL, third party)
# on a Linux workstation and install it: the desktop app and the headless
# `gold-band` binary. Nothing of Gold Band's source is copied into this
# repository and Gold Band's code is never changed. Upstream packages are not
# used: both binaries are built from the pinned commit.
#
#   GOLD_BAND_DIR      the existing clone (default ~/Projects/diodeme/Gold-Band).
#                      No clone: the surface is skipped with a message, exit 0.
#   GOLD_BAND_REF      the commit to build (default: the pin below, v0.19.1).
#   GOLD_BAND_BIN_DIR  where the binaries go (default ~/.local/bin).
#   GOLD_BAND_CACHE    build area (default ~/.cache/agent-kit/gold-band).
#   GOLD_BAND_STATE    the build record (default
#                      ~/.local/state/agent-kit/gold-band/build.json).
#   CRATE_AGE          the crate-age executable (default: crate-age on PATH).
#   CRATE_GATE_PORT    the gate's port (default 17342).
#   GOLD_BAND_FORCE=1  rebuild even when the record already matches the pin.
#   --check            report what would happen; build nothing.
#
# Exit status: 0 built (or already built, or skipped for want of a clone);
# 3 a locked crate is younger than 7 days (or its age is unprovable beyond the
# two known exceptions): the build is stopped and the crate reported; 1 any
# other failure.
#
# The clone is read with `git archive`, never touched. The build copy has
# Gold Band's own `.cargo/config.toml` replaced (it points crates.io at a
# third-party mirror, which would bypass the crate gate) by one that resolves
# only through the gate, the way this repository's `.cargo/config.toml` does.
# The web UI is built with npm through the packages proxy. On NixOS the system
# libraries a Tauri app links against come from `shell.nix` beside this file.
# docs/GOLD-BAND.md has the details.
set -eu

PIN=987fc3e298b15d725a7f0e9b609b0f77dba63a74
PIN_TAG=v0.19.1
HERE=$(cd "$(dirname "$0")" && pwd)
CLONE=${GOLD_BAND_DIR:-$HOME/Projects/diodeme/Gold-Band}
REF=${GOLD_BAND_REF:-$PIN}
BIN_DIR=${GOLD_BAND_BIN_DIR:-$HOME/.local/bin}
CACHE=${GOLD_BAND_CACHE:-$HOME/.cache/agent-kit/gold-band}
STATE=${GOLD_BAND_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/agent-kit/gold-band/build.json}
CRATE_AGE=${CRATE_AGE:-crate-age}
PORT=${CRATE_GATE_PORT:-17342}
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

say() { printf 'gold-band: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "this builder is for Linux; the Mac build is its own ticket"
[ "$(uname -m)" = x86_64 ] || die "only Linux x86_64 is supported here (this host is $(uname -m))"

if [ ! -e "$CLONE/.git" ]; then
  say "skipped: no Gold Band clone at $CLONE (git clone https://github.com/diodeme/Gold-Band there to enable it)"
  exit 0
fi
commit=$(git -C "$CLONE" rev-parse --verify "$REF^{commit}" 2>/dev/null) \
  || die "commit $REF is not in $CLONE (git -C $CLONE fetch origin)"

recorded() {
  [ -f "$STATE" ] && grep -q "\"commit\": \"$commit\"" "$STATE" \
    && [ -x "$BIN_DIR/gold-band" ] && [ -x "$BIN_DIR/gold-band-desktop" ]
}
if [ "${GOLD_BAND_FORCE:-0}" != 1 ] && recorded; then
  say "already built: $(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$STATE") at ${commit} ($STATE)"
  exit 0
fi
if [ "$CHECK" = 1 ]; then
  say "would build $commit ($PIN_TAG) from $CLONE: crate-age over its Cargo.lock, then the desktop app and gold-band through the crate gate, into $BIN_DIR"
  exit 0
fi

command -v "$CRATE_AGE" >/dev/null 2>&1 || die "no crate-age ($CRATE_AGE); scripts/gate/install-linux.sh installs it"
curl -fsS -m 3 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 \
  || die "the crate gate does not answer on 127.0.0.1:$PORT; scripts/gate/install-linux.sh installs it"

work="$CACHE/$commit"
src="$work/src"
rm -rf "$src"
mkdir -p "$src" "$(dirname "$STATE")"
git -C "$CLONE" archive "$commit" >"$work/src.tar"
tar -x -C "$src" -f "$work/src.tar"
version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$src/Cargo.toml" | head -1)
say "building Gold Band $version at $commit"

# 1. The age check over Gold Band's own lockfile. Exit 1 (a version younger
# than 7 days) stops the build. Exit 2 fails closed too, except for the two
# entries that have no publish date by nature: Gold Band's own path crate and
# the git patch of tao pinned to one revision.
age_out="$work/crate-age.txt"
set +e
"$CRATE_AGE" --root "$src" >"$age_out" 2>&1
age=$?
set -e
cat "$age_out"
# A young crate stops the build whatever the exit code: the lock always holds
# the tao git patch, so crate-age exits 2 even when it also found a young one.
if grep -q '^TOO NEW ' "$age_out"; then
  say "STOPPED: a crate in Gold Band's Cargo.lock is younger than 7 days (see above); nothing was built"
  exit 3
fi
case $age in
  0) age_result="all locked registry versions are at least 7 days old" ;;
  1) say "STOPPED: a crate in Gold Band's Cargo.lock is younger than 7 days (see above); nothing was built"; exit 3 ;;
  2)
    unproven=$(grep '^refused:' "$age_out" | grep -v '^refused: gold-band-desktop [0-9.]*: path dependency: no published date exists$' \
      | grep -v '^refused: tao [0-9.]*: git dependency: no published date exists$' || true)
    if printf '%s\n' "$unproven" | grep -q 'http status: 429'; then
      say "STOPPED: crates.io rate-limited the age check (HTTP 429), so ages could not be proven; nothing was built. Retry in a few minutes."
      exit 1
    fi
    if [ -n "$unproven" ]; then
      say "STOPPED: a crate whose age cannot be proven (see above); nothing was built"
      exit 3
    fi
    tao_all=$(grep -A2 '^name = "tao"$' "$src/Cargo.lock" | grep -c '^source = ' || true)
    tao_pin=$(grep -A2 '^name = "tao"$' "$src/Cargo.lock" | grep -c '^source = "git+https://github.com/tauri-apps/tao?rev=c704261c519c58cfdd0bc2d58ba24e06a0b71c92#' || true)
    [ "$tao_all" -ge 1 ] && [ "$tao_all" = "$tao_pin" ] \
      || { say "STOPPED: tao is not the pinned git revision c704261c; nothing was built"; exit 3; }
    age_result="every registry version at least 7 days old; unprovable by nature: path crates and tao at git rev c704261c"
    ;;
  *) die "crate-age failed (exit $age)" ;;
esac
say "age check: $age_result"

# 2. The gate replaces Gold Band's mirror config in the build copy only.
mkdir -p "$src/.cargo"
cat > "$src/.cargo/config.toml" <<TOML
[source.crates-io]
replace-with = "crate-gate"

[source.crate-gate]
registry = "sparse+http://127.0.0.1:$PORT/"
TOML

# 3. Build inside the system-library shell on NixOS; plain PATH elsewhere.
build_script="$work/build.sh"
cat > "$build_script" <<'BUILD'
set -eu
cd "$1"
npm ci --no-audit --no-fund
npm run web:build
# The lockfile at the pin is stale for Gold Band's own two crates, so no --locked.
cargo build --release -p gold-band --bin gold-band
cargo build --release -p gold-band-desktop
BUILD
if command -v cargo >/dev/null 2>&1 && command -v node >/dev/null 2>&1 && pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  run() { bash "$build_script" "$src"; }
elif command -v nix-shell >/dev/null 2>&1; then
  run() { nix-shell "$HERE/shell.nix" -I nixpkgs=flake:nixpkgs --run "bash '$build_script' '$src'"; }
else
  die "no Rust, Node and WebKitGTK on PATH, and no nix-shell to provide them (docs/GOLD-BAND.md lists the dependencies)"
fi
run || die "the build failed (log above); the clone and ~/.local/bin are untouched"

# 4. The lockfile may differ from the pin only in Gold Band's own two crates.
python3 -I - "$CLONE" "$commit" "$src/Cargo.lock" <<'PY' || { say "STOPPED: the build changed a third-party Cargo.lock entry, which was never age-checked; nothing was installed" >&2; exit 3; }
import re, subprocess, sys
clone, commit, new = sys.argv[1:4]
old = subprocess.run(["git", "-C", clone, "show", f"{commit}:Cargo.lock"], capture_output=True, text=True, check=True).stdout
def pkgs(text):
    return {m: m for m in re.findall(r'name = "([^"]+)"\nversion = "([^"]+)"(?:\nsource = "([^"]*)")?', text)}
own = {"gold-band", "gold-band-desktop"}
a, b = pkgs(old), pkgs(open(new).read())
def third(d):
    return {k for k, v in d.items() if v[0] not in own}
sys.exit(0 if third(a) == third(b) else 1)
PY

# 5. Install and record.
mkdir -p "$BIN_DIR"
install -m 0755 "$src/target/release/gold-band" "$BIN_DIR/gold-band"
install -m 0755 "$src/target/release/gold-band-desktop" "$BIN_DIR/gold-band-desktop"
sha_head=$(sha256sum "$BIN_DIR/gold-band" | cut -d' ' -f1)
sha_desk=$(sha256sum "$BIN_DIR/gold-band-desktop" | cut -d' ' -f1)
help_line=$("$BIN_DIR/gold-band" --help 2>&1 | head -1) || true
cat > "$STATE" <<JSON
{
  "version": "$version",
  "tag": "$PIN_TAG",
  "commit": "$commit",
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "age_check": "$age_result",
  "gold-band": {"path": "$BIN_DIR/gold-band", "sha256": "$sha_head"},
  "gold-band-desktop": {"path": "$BIN_DIR/gold-band-desktop", "sha256": "$sha_desk"}
}
JSON
say "installed $BIN_DIR/gold-band ($help_line) and $BIN_DIR/gold-band-desktop"
say "Gold Band $version built from commit $commit; record: $STATE"
