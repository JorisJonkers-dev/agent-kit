#!/bin/sh
# Install crate-age, the after-the-fact 7-day crate cooldown check, on a Linux
# workstation (docs/RUST.md). Idempotent. It also retires the loopback
# crate-gate this script used to install: the packages proxy and cargo's own
# registry.global-min-publish-age replace it.
#
# The source lives in the standards repo (ADR-0014) and is never copied into
# this repository. It is read from an existing clone with `git archive` at a
# pinned commit, so the clone's working tree is never touched:
#
#   STANDARDS_DIR   the clone (default ~/Projects/JorisJonkers-dev/standards)
#   STANDARDS_REF   the commit to build (default: the pin below; move it with
#                   the one in .github/workflows/crate-cooldown.yml)
#
# The binary goes to ~/.local/bin. cargo comes from PATH, else from `nix shell`
# on NixOS; it resolves through the packages proxy in ~/.cargo/config.toml.
set -eu

STANDARDS_DIR=${STANDARDS_DIR:-$HOME/Projects/JorisJonkers-dev/standards}
STANDARDS_REF=${STANDARDS_REF:-7d0673432f10f4a515a5dc700f78daae8e389cd5}
BIN_DIR=${CRATE_AGE_BIN_DIR:-$HOME/.local/bin}
UNIT_DIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
OLD_UNIT=crate-gate.service

say() { printf 'crate-age: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "this installer is for Linux; the Mac installs crate-age from the standards repo"
[ -e "$STANDARDS_DIR/.git" ] || [ -f "$STANDARDS_DIR/HEAD" ] || die "no standards clone at $STANDARDS_DIR (set STANDARDS_DIR)"
git -C "$STANDARDS_DIR" cat-file -e "$STANDARDS_REF^{commit}" 2>/dev/null \
  || git -C "$STANDARDS_DIR" fetch -q origin "$STANDARDS_REF" \
  || die "commit $STANDARDS_REF is not in $STANDARDS_DIR and could not be fetched"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$STANDARDS_DIR" archive "$STANDARDS_REF" scripts | tar -x -C "$work"

if command -v cargo >/dev/null 2>&1; then
  build() { (cd "$work/scripts" && cargo build --release --locked --bin crate-age); }
elif command -v nix >/dev/null 2>&1; then
  build() { (cd "$work/scripts" && nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc -c \
    cargo build --release --locked --bin crate-age); }
else
  die "no cargo on PATH and no nix to borrow one from"
fi
say "building crate-age at ${STANDARDS_REF}..."
build

mkdir -p "$BIN_DIR"
install -m 0755 "$work/scripts/target/release/crate-age" "$BIN_DIR/crate-age"
say "installed $BIN_DIR/crate-age"

# The retired gate: stop it and remove its unit and binary, if this machine has them.
if [ -f "$UNIT_DIR/$OLD_UNIT" ]; then
  systemctl --user disable --now "$OLD_UNIT" >/dev/null 2>&1 || true
  rm -f "$UNIT_DIR/$OLD_UNIT"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
  say "removed the retired crate-gate service"
fi
[ ! -e "$BIN_DIR/crate-gate" ] || { rm -f "$BIN_DIR/crate-gate"; say "removed the retired $BIN_DIR/crate-gate"; }
say "next: git config core.hooksPath .githooks   (in each clone)"
