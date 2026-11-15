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
#   GOLD_BAND_SHARE    the data home for the desktop entry, icon and runtime.nix
#                      (default ${XDG_DATA_HOME:-~/.local/share}).
#   GOLD_BAND_CACHE    build area (default ~/.cache/agent-kit/gold-band).
#   GOLD_BAND_STATE    the build record (default
#                      ~/.local/state/agent-kit/gold-band/build.json).
#   CRATE_AGE          the crate-age executable (default: crate-age on PATH).
#   PACKAGES_CARGO_INDEX  the packages proxy's cargo index (default
#                      sparse+https://packages.jorisjonkers.dev/cargo/).
#   GOLD_BAND_FORCE=1  rebuild even when the record already matches the pin.
#   --check            report what would happen; build nothing.
#   --retire [--check] remove the launcher, runtime.nix, desktop entry and icons
#                      this script installs (the binaries and the clone stay).
#
# Besides the two binaries it installs, every run (built, already built, or
# --check as a report only): the `gold-band-launch` wrapper beside them, the
# `gold-band/runtime.nix` library shell, `applications/gold-band.desktop` and the
# icon set (32, 64, 128, 256, 512 PNG and the SVG) from the pinned source's src-tauri/icons, so Gold Band is
# findable in GNOME and starts on NixOS. Files already there, hand-made or not,
# are replaced when they differ.
#
# Exit status: 0 built (or already built, or skipped for want of a clone);
# 3 a locked crate is younger than 7 days (or its age is unprovable beyond the
# two known exceptions): the build is stopped and the crate reported; 1 any
# other failure.
#
# The clone is read with `git archive`, never touched. The build copy has
# Gold Band's own `.cargo/config.toml` replaced (it points crates.io at a
# third-party mirror, which would bypass the cooldown) by one that resolves
# only through the packages proxy, which hides versions younger than 7 days,
# with cargo's own 7-day cooldown on top (docs/RUST.md).
# The desktop binary is built with tauri/custom-protocol so it embeds web/dist instead of loading a dev server.
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
INDEX=${PACKAGES_CARGO_INDEX:-sparse+https://packages.jorisjonkers.dev/cargo/}
CREDENTIALS=${CARGO_HOME:-$HOME/.cargo}/credentials.toml
SHARE=${GOLD_BAND_SHARE:-${XDG_DATA_HOME:-$HOME/.local/share}}
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1
RETIRE=0
if [ "${1:-}" = "--retire" ] || [ "${2:-}" = "--retire" ]; then RETIRE=1; [ "${2:-}" = "--check" ] && CHECK=1; fi

say() { printf 'gold-band: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "this builder is for Linux; the Mac build is its own ticket"
[ "$(uname -m)" = x86_64 ] || die "only Linux x86_64 is supported here (this host is $(uname -m))"

recorded() {
  [ -f "$STATE" ] && grep -q "\"commit\": \"$commit\"" "$STATE" \
    && [ -x "$BIN_DIR/gold-band" ] && [ -x "$BIN_DIR/gold-band-desktop" ]
}
# The icons of the pinned source's src-tauri/icons, installed as gold-band.* into
# hicolor under the directory of their real size (the SVG under scalable).
ICON_SOURCES="32x32.png 64x64.png 128x128.png 128x128@2x.png icon.png logo-source.svg"
CHANGED=0

# Where icon $1 (file $2) goes: PNGs by the width and height in their header.
icon_dest() {
  local dim w h
  case $1 in
    *.svg) printf '%s/icons/hicolor/scalable/apps/gold-band.svg\n' "$SHARE"; return ;;
  esac
  dim=$(od -An -tu1 -j16 -N8 "$2")
  # shellcheck disable=SC2086
  set -- $dim
  w=$(( $1 * 16777216 + $2 * 65536 + $3 * 256 + $4 ))
  h=$(( $5 * 16777216 + $6 * 65536 + $7 * 256 + $8 ))
  printf '%s/icons/hicolor/%sx%s/apps/gold-band.png\n' "$SHARE" "$w" "$h"
}

# Every file this surface owns besides the two binaries; the retire rule removes them all.
retire_desktop_integration() {
  local f
  for f in "$BIN_DIR/gold-band-launch" "$SHARE/gold-band/runtime.nix" "$SHARE/applications/gold-band.desktop" \
    "$SHARE"/icons/hicolor/*/apps/gold-band.png "$SHARE/icons/hicolor/scalable/apps/gold-band.svg"; do
    [ -e "$f" ] || continue
    if [ "$CHECK" = 1 ]; then say "would remove $f"; else rm -f "$f" && say "removed $f"; CHANGED=1; fi
  done
  [ "$CHECK" = 1 ] || rmdir "$SHARE/gold-band" 2>/dev/null || true
  refresh_icon_cache
}

# Best effort: gtk-update-icon-cache where it exists; elsewhere (NixOS) a stale
# icon-theme.cache would hide the new files, so it is removed and rebuilt by the shell.
refresh_icon_cache() {
  local theme="$SHARE/icons/hicolor"
  [ "$CHANGED" = 1 ] && [ "$CHECK" != 1 ] && [ -d "$theme" ] || return 0
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$theme" >/dev/null 2>&1 || say "gtk-update-icon-cache failed; continuing"
  else
    rm -f "$theme/icon-theme.cache"
  fi
}

# Writes $2 (a file) to $1 when it differs; --check only reports.
place() {
  if [ -f "$1" ] && cmp -s "$2" "$1"; then
    say "up to date: $1"
  elif [ "$CHECK" = 1 ]; then
    say "would install $1"
  else
    mkdir -p "$(dirname "$1")"
    cp "$2" "$1.agent-kit-tmp" && chmod "$3" "$1.agent-kit-tmp" && mv -f "$1.agent-kit-tmp" "$1"
    CHANGED=1
    say "installed $1"
  fi
}

# The launcher, runtime.nix, the .desktop entry and the icon. The launcher runs
# the binary directly where pkg-config finds WebKitGTK (or there is no nix-shell);
# otherwise it enters runtime.nix, which puts the libraries on the loader path.
desktop_integration() {
  local tmp launcher="$BIN_DIR/gold-band-launch" runtime="$SHARE/gold-band/runtime.nix"
  tmp=$(mktemp -d)
  cat > "$tmp/launcher" <<'LAUNCH'
#!/usr/bin/env bash
# managed by agent-kit scripts/gold-band/build-linux.sh
# Starts the Gold Band desktop app; on NixOS its WebKit/GTK libraries come from runtime.nix.
GB_BIN=@BIN@
GB_RUNTIME=@RUNTIME@
export GB_BIN
if pkg-config --exists webkit2gtk-4.1 2>/dev/null || ! command -v nix-shell >/dev/null 2>&1; then
  exec "$GB_BIN" "$@"
fi
args=$(printf '%q ' "$@")
exec nix-shell -I nixpkgs=flake:nixpkgs "$GB_RUNTIME" --run 'export LD_LIBRARY_PATH="$GB_LIBS"; export XDG_DATA_DIRS="$GB_DATA:${XDG_DATA_DIRS:-}"; exec "$GB_BIN" '"$args"
LAUNCH
  sed -i -e "s|@BIN@|'$BIN_DIR/gold-band-desktop'|" -e "s|@RUNTIME@|'$runtime'|" "$tmp/launcher"
  cat > "$tmp/desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Gold Band
Comment=Gold Band desktop
Exec=$launcher
Icon=gold-band
Terminal=false
Categories=Development;
StartupWMClass=gold-band-desktop
DESKTOP
  place "$launcher" "$tmp/launcher" 0755
  place "$runtime" "$HERE/runtime.nix" 0644
  place "$SHARE/applications/gold-band.desktop" "$tmp/desktop" 0644
  for src in $ICON_SOURCES; do
    if ! git -C "$CLONE" show "$commit:src-tauri/icons/$src" > "$tmp/icon" 2>/dev/null || [ ! -s "$tmp/icon" ]; then
      say "no src-tauri/icons/$src at $commit; that icon was not installed"
      continue
    fi
    place "$(icon_dest "$src" "$tmp/icon")" "$tmp/icon" 0644
  done
  refresh_icon_cache
  rm -rf "$tmp"
}

if [ "$RETIRE" = 1 ]; then
  retire_desktop_integration
  exit 0
fi

if [ ! -e "$CLONE/.git" ]; then
  say "skipped: no Gold Band clone at $CLONE (git clone https://github.com/diodeme/Gold-Band there to enable it)"
  exit 0
fi
commit=$(git -C "$CLONE" rev-parse --verify "$REF^{commit}" 2>/dev/null) \
  || die "commit $REF is not in $CLONE (git -C $CLONE fetch origin)"

if [ "${GOLD_BAND_FORCE:-0}" != 1 ] && recorded; then
  say "already built: $(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$STATE") at ${commit} ($STATE)"
  desktop_integration
  exit 0
fi
if [ "$CHECK" = 1 ]; then
  say "would build $commit ($PIN_TAG) from $CLONE: crate-age over its Cargo.lock, then the desktop app and gold-band through the packages proxy, into $BIN_DIR"
  desktop_integration
  exit 0
fi

command -v "$CRATE_AGE" >/dev/null 2>&1 || die "no crate-age ($CRATE_AGE); scripts/install-crate-age.sh installs it"
grep -q '^\[registries\.jorisjonkers-packages\]' "$CREDENTIALS" 2>/dev/null \
  || die "no packages proxy token in $CREDENTIALS; run setup, which writes it"

work="$CACHE/$commit"
src="$work/src"
# The previous build's target directory is kept across the re-extract, so a rebuild is incremental.
rm -rf "$work/target.keep"
[ -d "$src/target" ] && mv "$src/target" "$work/target.keep"
rm -rf "$src"
mkdir -p "$src" "$(dirname "$STATE")"
git -C "$CLONE" archive "$commit" >"$work/src.tar"
tar -x -C "$src" -f "$work/src.tar"
[ -d "$work/target.keep" ] && mv "$work/target.keep" "$src/target"
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

# 2. The packages proxy replaces Gold Band's mirror config in the build copy
# only. The token comes from the credentials setup wrote.
mkdir -p "$src/.cargo"
cat > "$src/.cargo/config.toml" <<TOML
[source.crates-io]
replace-with = "jorisjonkers-packages"

[source.jorisjonkers-packages]
registry = "$INDEX"

[registries.jorisjonkers-packages]
index = "$INDEX"

[registry]
global-credential-providers = ["cargo:token"]
global-min-publish-age = "7 days"
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
# tauri/custom-protocol is what `tauri build` (upstream's release) turns on: without it the
# binary is a dev build that loads devUrl http://localhost:1420 instead of the embedded web/dist.
cargo build --release -p gold-band-desktop --features tauri/custom-protocol
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
desktop_integration
