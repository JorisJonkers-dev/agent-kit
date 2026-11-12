#!/usr/bin/env bash
# Builds the Rust model gateway from this kit's cargo workspace and installs it
# where the `model-gateway` command on PATH looks for it
# (~/.local/share/agent-kit/model-gateway/model-gateway). Until it is installed
# that command runs the node runner, so nothing breaks mid-way; a build that
# fails leaves the installed binary (or the node runner) as it was.
#
#   --check                    report what would happen; build nothing.
#   AGENT_KIT_MODEL_GATEWAY_DIR  where the binary goes (default as above).
#   KIT_ROOT                   the kit (default: this script's parent).
#
# Exit status: 0 installed, already current, or skipped (no cargo, no workspace,
# or the node runner is pinned); 1 the build or the smoke check failed.
# Rollback: docs/RUST.md ("Swapping the binary in").
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
KIT=${KIT_ROOT:-$(dirname "$HERE")}
DEST_DIR=${AGENT_KIT_MODEL_GATEWAY_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/model-gateway}
PIN=${XDG_CONFIG_HOME:-$HOME/.config}/agent-kit/model-gateway-node
MANIFEST=$KIT/scripts/Cargo.toml
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

say() { printf 'model-gateway (rust): %s\n' "$*"; }

if [ -e "$PIN" ]; then say "skipped, the node runner is pinned by $PIN"; exit 0; fi
if [ ! -f "$MANIFEST" ]; then say "skipped, no cargo workspace at $MANIFEST"; exit 0; fi
if ! command -v cargo >/dev/null 2>&1; then say "skipped, cargo is not on PATH; the node runner stays"; exit 0; fi
if [ "$CHECK" = 1 ]; then say "would build and install $DEST_DIR/model-gateway"; exit 0; fi

target=${CARGO_TARGET_DIR:-$KIT/scripts/target}
if ! cargo build --release --locked -p model-gateway --manifest-path "$MANIFEST" >&2; then
  say "the build failed; the installed gateway is unchanged"
  exit 1
fi
built=$target/release/model-gateway
[ -x "$built" ] || { say "the build left no $built"; exit 1; }
# The smoke check: the binary starts and answers --help before it replaces anything.
"$built" --help >/dev/null 2>&1 || { say "the new binary does not answer --help; the installed gateway is unchanged"; exit 1; }
if [ -x "$DEST_DIR/model-gateway" ] && cmp -s "$built" "$DEST_DIR/model-gateway"; then
  say "already current: $DEST_DIR/model-gateway"
  exit 0
fi
mkdir -p "$DEST_DIR"
tmp=$(mktemp "$DEST_DIR/.model-gateway.XXXXXX")
cp "$built" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$DEST_DIR/model-gateway" \
  || { rm -f "$tmp"; say "could not install $DEST_DIR/model-gateway"; exit 1; }
say "installed $DEST_DIR/model-gateway"
