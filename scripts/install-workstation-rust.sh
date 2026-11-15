#!/usr/bin/env bash
# Builds the workstation's Rust tools from this kit's cargo workspace and
# installs them in ~/.local/share/agent-kit/bin: agent-kit-keys (tokens and
# keys), litellm-harness (LiteLLM provider entries and the agent-kit-litellm
# launcher), measure-litellm-budget and workstation-kube. Setup runs it before
# the tokens-and-keys step, which needs agent-kit-keys.
#
#   --check                    report what would happen; build nothing.
#   KIT_ROOT                   the kit (default: this script's parent).
#
# Exit status: 0 installed, already current, or --check; 1 the build or the
# install failed (what was installed stays as it was); 2 bad usage; 3 cargo is
# not on PATH.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
KIT=${KIT_ROOT:-$(dirname "$HERE")}
DEST_DIR=${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/bin
MANIFEST=$KIT/scripts/Cargo.toml
PACKAGES=(agent-kit-keys litellm-harness litellm-budget workstation-kube)
BINARIES=(agent-kit-keys litellm-harness measure-litellm-budget workstation-kube)
CHECK=0
case "${1:-}" in
  "") ;;
  --check) CHECK=1 ;;
  *) echo "usage: install-workstation-rust.sh [--check]" >&2; exit 2 ;;
esac

say() { printf 'workstation tools (rust): %s\n' "$*"; }

if [ "$CHECK" = 1 ]; then say "would build and install ${BINARIES[*]} in $DEST_DIR"; exit 0; fi
command -v cargo >/dev/null 2>&1 || { say "cargo is not on PATH; the tools are not built"; exit 3; }

target=${CARGO_TARGET_DIR:-$KIT/scripts/target}
args=()
for package in "${PACKAGES[@]}"; do args+=(-p "$package"); done
cd "$KIT"
cargo build --release --locked "${args[@]}" --manifest-path "$MANIFEST" >&2 \
  || { say "the build failed; the installed tools are unchanged"; exit 1; }
for binary in "${BINARIES[@]}"; do
  [ -x "$target/release/$binary" ] || { say "the build left no $target/release/$binary"; exit 1; }
done
mkdir -p "$DEST_DIR" || { say "could not create $DEST_DIR"; exit 1; }
for binary in "${BINARIES[@]}"; do
  built=$target/release/$binary
  if [ -x "$DEST_DIR/$binary" ] && cmp -s "$built" "$DEST_DIR/$binary"; then
    say "already current: $DEST_DIR/$binary"
    continue
  fi
  tmp=$(mktemp "$DEST_DIR/.$binary.XXXXXX") || { say "could not install $DEST_DIR/$binary"; exit 1; }
  if cp "$built" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$DEST_DIR/$binary"; then
    say "installed $DEST_DIR/$binary"
  else
    rm -f "$tmp"
    say "could not install $DEST_DIR/$binary"
    exit 1
  fi
done
