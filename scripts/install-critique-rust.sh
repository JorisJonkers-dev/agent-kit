#!/usr/bin/env bash
# Builds the Rust critique runner from this kit's cargo workspace and installs it
# where the critique skill's launcher (skills/workstation/critique/critique) looks
# for it (~/.local/share/agent-kit/critique/critique). Until it is installed the
# launcher runs the node runner, critique.mjs, so nothing breaks mid-way; a build
# that fails leaves the installed binary (or the node runner) as it was. A command
# the Rust runner does not port yet it hands to critique.mjs itself.
#
#   --check                    report what would happen; build nothing.
#   KIT_ROOT                   the kit (default: this script's parent).
#
# Exit status: 0 installed, already current, or skipped (no cargo, no workspace,
# or the node runner is pinned); 1 the build or the smoke check failed.
# Rollback: docs/RUST.md ("Swapping the critique runner in").
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
KIT=${KIT_ROOT:-$(dirname "$HERE")}
# Where the launcher looks (skills/workstation/critique/critique reads the same place).
DEST_DIR=${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/critique
PIN=${XDG_CONFIG_HOME:-$HOME/.config}/agent-kit/critique-node
MANIFEST=$KIT/scripts/Cargo.toml
CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  *) echo "usage: install-critique-rust.sh [--check]" >&2; exit 2 ;;
esac

say() { printf 'critique runner (rust): %s\n' "$*"; }

if [ -e "$PIN" ]; then say "skipped, the node runner is pinned by $PIN"; exit 0; fi
if [ ! -f "$MANIFEST" ]; then say "skipped, no cargo workspace at $MANIFEST"; exit 0; fi
if ! command -v cargo >/dev/null 2>&1; then say "skipped, cargo is not on PATH; the node runner stays"; exit 0; fi
if [ "$CHECK" = 1 ]; then say "would build and install $DEST_DIR/critique"; exit 0; fi

target=${CARGO_TARGET_DIR:-$KIT/scripts/target}
# From the kit, so its .cargo/config.toml (the crate gate) applies whatever the caller's directory.
cd "$KIT"
if ! cargo build --release --locked -p critique --manifest-path "$MANIFEST" >&2; then
  say "the build failed; the installed runner is unchanged"
  exit 1
fi
built=$target/release/critique
[ -x "$built" ] || { say "the build left no $built"; exit 1; }
# The smoke check: the binary starts and answers --help before it replaces anything.
"$built" --help >/dev/null 2>&1 || { say "the new binary does not answer --help; the installed runner is unchanged"; exit 1; }
if [ -x "$DEST_DIR/critique" ] && cmp -s "$built" "$DEST_DIR/critique"; then
  say "already current: $DEST_DIR/critique"
  exit 0
fi
mkdir -p "$DEST_DIR" || { say "could not create $DEST_DIR"; exit 1; }
tmp=$(mktemp "$DEST_DIR/.critique.XXXXXX") || { say "could not install $DEST_DIR/critique"; exit 1; }
trap 'rm -f "$tmp"' EXIT
if cp "$built" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$DEST_DIR/critique"; then
  say "installed $DEST_DIR/critique"
else
  say "could not install $DEST_DIR/critique"
  exit 1
fi
