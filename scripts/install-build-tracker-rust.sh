#!/usr/bin/env bash
# Builds the Rust build tracker from this kit's cargo workspace and installs it
# on PATH as ~/.local/bin/build-tracker, where the build-tracker skill's launcher
# (skills/workstation/build-tracker/build-tracker) looks for it. Until it is
# installed the launcher runs the node bundle, build-tracker.mjs, so nothing
# breaks mid-way; a build that fails leaves the installed binary (or the node
# bundle) as it was.
#
#   --check                    report what would happen; build nothing.
#   KIT_ROOT                   the kit (default: this script's parent).
#
# Exit status: 0 installed, already current, or skipped (no cargo, no workspace,
# or the node bundle is pinned); 1 the build or the smoke check failed.
# Rollback: docs/RUST.md ("Swapping the build tracker in").
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
KIT=${KIT_ROOT:-$(dirname "$HERE")}
# Where the launcher looks (skills/workstation/build-tracker/build-tracker reads the same place).
DEST_DIR=${XDG_BIN_HOME:-$HOME/.local/bin}
PIN=${XDG_CONFIG_HOME:-$HOME/.config}/agent-kit/build-tracker-node
MANIFEST=$KIT/scripts/Cargo.toml
CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  *) echo "usage: install-build-tracker-rust.sh [--check]" >&2; exit 2 ;;
esac

say() { printf 'build-tracker (rust): %s\n' "$*"; }

if [ -e "$PIN" ]; then say "skipped, the node bundle is pinned by $PIN"; exit 0; fi
if [ ! -f "$MANIFEST" ]; then say "skipped, no cargo workspace at $MANIFEST"; exit 0; fi
if ! command -v cargo >/dev/null 2>&1; then say "skipped, cargo is not on PATH; the node bundle stays"; exit 0; fi
if [ "$CHECK" = 1 ]; then say "would build and install $DEST_DIR/build-tracker"; exit 0; fi

target=${CARGO_TARGET_DIR:-$KIT/scripts/target}
# From the kit, so cargo reads the workspace whatever the caller's directory.
cd "$KIT"
if ! cargo build --release --locked -p build-tracker --manifest-path "$MANIFEST" >&2; then
  say "the build failed; the installed build tracker is unchanged"
  exit 1
fi
built=$target/release/build-tracker
[ -x "$built" ] || { say "the build left no $built"; exit 1; }
# The smoke check: the binary starts and answers --help before it replaces anything.
"$built" --help >/dev/null 2>&1 || { say "the new binary does not answer --help; the installed build tracker is unchanged"; exit 1; }
if [ -x "$DEST_DIR/build-tracker" ] && cmp -s "$built" "$DEST_DIR/build-tracker"; then
  say "already current: $DEST_DIR/build-tracker"
  exit 0
fi
mkdir -p "$DEST_DIR" || { say "could not create $DEST_DIR"; exit 1; }
tmp=$(mktemp "$DEST_DIR/.build-tracker.XXXXXX") || { say "could not install $DEST_DIR/build-tracker"; exit 1; }
trap 'rm -f "$tmp"' EXIT
if cp "$built" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$DEST_DIR/build-tracker"; then
  say "installed $DEST_DIR/build-tracker"
else
  say "could not install $DEST_DIR/build-tracker"
  exit 1
fi
