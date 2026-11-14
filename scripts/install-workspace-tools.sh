#!/usr/bin/env bash
# Builds the named command-line tools from this kit's pinned cargo workspace and
# installs each one into ~/.local/bin, where the owner's scripts, the Gold Band
# workflows and the CI helpers call them by name. The registry's
# `workspace_tools:` list says which; setup passes those names.
#
#   install-workspace-tools.sh [--check] NAME...
#
#   --check                    report what would happen; build nothing.
#   KIT_ROOT                   the kit (default: this script's parent).
#   WORKSPACE_TOOLS_BIN_DIR    where the binaries go (default ~/.local/bin).
#
# One `cargo build --release --locked` covers every name, so the lockfile is the
# pin and nothing newer is resolved. The build runs from the kit so its
# .cargo/config.toml applies (registry.global-min-publish-age, the 7-day crate
# cooldown); a kit without that setting is refused, never built without the
# cooldown. Each new binary must start and print its usage before anything is
# replaced, so a bad build leaves every installed binary as it was.
#
# Exit status: 0 installed, already current, or skipped (no cargo, no workspace);
# 1 the build, a smoke check or an install failed; 2 bad usage.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
KIT=${KIT_ROOT:-$(dirname "$HERE")}
BIN_DIR=${WORKSPACE_TOOLS_BIN_DIR:-$HOME/.local/bin}
MANIFEST=$KIT/scripts/Cargo.toml
CONFIG=$KIT/.cargo/config.toml
CHECK=0
if [ "${1:-}" = "--check" ]; then CHECK=1; shift; fi
if [ "$#" = 0 ]; then echo "usage: install-workspace-tools.sh [--check] NAME..." >&2; exit 2; fi
for name in "$@"; do
  case $name in
    --*|-*|""|*[!a-z0-9-]*) echo "install-workspace-tools.sh: not a tool name: $name" >&2; exit 2 ;;
  esac
done

say() { printf 'workspace tools: %s\n' "$*"; }

if [ ! -f "$MANIFEST" ]; then say "skipped, no cargo workspace at $MANIFEST"; exit 0; fi
if ! command -v cargo >/dev/null 2>&1; then say "skipped, cargo is not on PATH"; exit 0; fi
if ! grep -q '^[[:space:]]*global-min-publish-age[[:space:]]*=' "$CONFIG" 2>/dev/null; then
  say "refused: $CONFIG does not set registry.global-min-publish-age, so the 7-day crate cooldown would not apply"
  exit 1
fi
if [ "$CHECK" = 1 ]; then
  for name in "$@"; do say "would build and install $BIN_DIR/$name"; done
  exit 0
fi

target=${CARGO_TARGET_DIR:-$KIT/scripts/target}
packages=()
for name in "$@"; do packages+=(-p "$name"); done
# From the kit, so its .cargo/config.toml (the cooldown) applies whatever the caller's directory.
cd "$KIT"
if ! cargo build --quiet --release --locked --manifest-path "$MANIFEST" "${packages[@]}" >&2; then
  say "the build failed; the installed tools are unchanged"
  exit 1
fi

# Every binary starts before any is installed. Most print their usage and exit 2
# for --help, so the check is that something ran and printed, not a status of 0.
for name in "$@"; do
  built=$target/release/$name
  [ -x "$built" ] || { say "the build left no $built; the installed tools are unchanged"; exit 1; }
  out=$("$built" --help 2>&1 </dev/null) && status=0 || status=$?
  if [ "$status" -ge 126 ] || [ -z "$out" ]; then
    say "the new $name does not start (exit $status); the installed tools are unchanged"
    exit 1
  fi
done

mkdir -p "$BIN_DIR" || { say "could not create $BIN_DIR"; exit 1; }
rc=0
for name in "$@"; do
  built=$target/release/$name
  dest=$BIN_DIR/$name
  if [ -x "$dest" ] && cmp -s "$built" "$dest"; then
    say "already current: $dest"
    continue
  fi
  tmp=$(mktemp "$BIN_DIR/.$name.XXXXXX") || { say "could not install $dest"; rc=1; continue; }
  if cp "$built" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$dest"; then
    say "installed $dest"
  else
    rm -f "$tmp"
    say "could not install $dest"
    rc=1
  fi
done
exit $rc
