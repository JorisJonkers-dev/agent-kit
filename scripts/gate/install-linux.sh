#!/bin/sh
# Install the crate-gate and crate-age on a Linux workstation, and run the gate
# as a systemd user service on 127.0.0.1:17342 (the Mac runs the same binary
# under launchd; this is the Linux equivalent, docs/RUST.md). Idempotent.
#
# The sources live in the standards repo (ADR-0014) and are never copied into
# this repository. They are read from an existing clone with `git archive` at a
# pinned commit, so the clone's working tree is never touched:
#
#   STANDARDS_DIR   the clone (default ~/Projects/JorisJonkers-dev/standards)
#   STANDARDS_REF   the commit to build (default: the pin below; move it the day
#                   the standards repo tags a release)
#   CRATE_GATE_PORT / CRATE_GATE_MIN_DAYS   17342 / 7
#
# Binaries go to ~/.local/bin, the unit to ~/.config/systemd/user. cargo comes
# from PATH, else from `nix shell` on NixOS. Building needs a registry once,
# which the packages-proxy block in ~/.cargo/config.toml already provides; the
# gate itself is only used by this repository's .cargo/config.toml.
set -eu

STANDARDS_DIR=${STANDARDS_DIR:-$HOME/Projects/JorisJonkers-dev/standards}
STANDARDS_REF=${STANDARDS_REF:-7d0673432f10f4a515a5dc700f78daae8e389cd5}
PORT=${CRATE_GATE_PORT:-17342}
MIN_DAYS=${CRATE_GATE_MIN_DAYS:-7}
BIN_DIR=${CRATE_GATE_BIN_DIR:-$HOME/.local/bin}
UNIT_DIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
UNIT=crate-gate.service

say() { printf 'crate-gate: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "this installer is for Linux; the Mac uses launchd (standards scripts/gate/install-macos.sh)"
[ -e "$STANDARDS_DIR/.git" ] || [ -f "$STANDARDS_DIR/HEAD" ] || die "no standards clone at $STANDARDS_DIR (set STANDARDS_DIR)"
git -C "$STANDARDS_DIR" cat-file -e "$STANDARDS_REF^{commit}" 2>/dev/null \
  || git -C "$STANDARDS_DIR" fetch -q origin "$STANDARDS_REF" \
  || die "commit $STANDARDS_REF is not in $STANDARDS_DIR and could not be fetched"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$STANDARDS_DIR" archive "$STANDARDS_REF" scripts | tar -x -C "$work"

if command -v cargo >/dev/null 2>&1; then
  build() { (cd "$work/scripts" && cargo build --release --locked --bin crate-gate --bin crate-age); }
elif command -v nix >/dev/null 2>&1; then
  build() { (cd "$work/scripts" && nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc -c \
    cargo build --release --locked --bin crate-gate --bin crate-age); }
else
  die "no cargo on PATH and no nix to borrow one from"
fi
say "building crate-gate and crate-age at ${STANDARDS_REF}..."
build

mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 0755 "$work/scripts/target/release/crate-gate" "$BIN_DIR/crate-gate"
install -m 0755 "$work/scripts/target/release/crate-age" "$BIN_DIR/crate-age"
say "installed $BIN_DIR/crate-gate and $BIN_DIR/crate-age"

cat > "$UNIT_DIR/$UNIT" <<UNITFILE
[Unit]
Description=crate-gate: loopback crates.io mirror that hides versions younger than ${MIN_DAYS} days
After=network-online.target

[Service]
ExecStart=$BIN_DIR/crate-gate --addr 127.0.0.1 --port $PORT --min-days $MIN_DAYS
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
UNITFILE
say "wrote $UNIT_DIR/$UNIT"

systemctl --user daemon-reload
systemctl --user enable "$UNIT" >/dev/null 2>&1 || true
systemctl --user restart "$UNIT"

i=0
until curl -fsS -m 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -lt 50 ] || die "the gate did not answer on 127.0.0.1:$PORT within 10s (journalctl --user -u $UNIT)"
  sleep 0.2
done
say "gate is up on 127.0.0.1:$PORT"
say "next: git config core.hooksPath .githooks   (in each clone), then scripts/gate/verify-gate.sh"
