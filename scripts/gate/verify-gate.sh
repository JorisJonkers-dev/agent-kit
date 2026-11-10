#!/bin/sh
# Prove the gate wiring of .cargo/config.toml on this machine (docs/RUST.md).
# Fetches a throwaway crate with one pinned registry dependency, under the
# repository's cargo config, in three states:
#
#   1. config as committed, gate up      -> resolves, and only through the gate
#   2. [source.crate-gate] removed       -> fails to resolve
#   3. gate unreachable                  -> fails (connection refused)
#
# Exit 0 only when all three behave. The throwaway crate and a private
# CARGO_HOME live in a temp directory, so neither the user-level cargo config
# nor a cached crate can make a state pass for the wrong reason.
set -eu

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
config="$repo_root/.cargo/config.toml"
port=${CRATE_GATE_PORT:-17342}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

command -v cargo >/dev/null 2>&1 || { echo "verify-gate: cargo is not on PATH" >&2; exit 2; }
curl -fsS -m 2 "http://127.0.0.1:$port/health" >/dev/null 2>&1 \
  || { echo "verify-gate: the gate is not answering on 127.0.0.1:$port (scripts/gate/install-linux.sh)" >&2; exit 2; }

mkdir -p "$work/probe/src" "$work/probe/.cargo"
cat > "$work/probe/Cargo.toml" <<'TOML'
[package]
name = "probe"
version = "0.0.0"
edition = "2021"

[dependencies]
itoa = "=1.0.11"
TOML
echo 'fn main() { println!("{}", itoa::Buffer::new().format(7u8)); }' > "$work/probe/src/main.rs"

# run <config-file>: a fresh CARGO_HOME and lockfile, then cargo fetch.
run() {
  cp "$1" "$work/probe/.cargo/config.toml"
  rm -rf "$work/home" "$work/probe/Cargo.lock"
  mkdir -p "$work/home"
  (cd "$work/probe" && CARGO_HOME="$work/home" cargo fetch 2>&1)
}

fail=0

echo "== 1. committed config, gate up"
if out=$(run "$config"); then
  if ls "$work"/home/registry/index/127.0.0.1-*/ >/dev/null 2>&1 \
     && ! ls "$work"/home/registry/index 2>/dev/null | grep -q 'crates.io\|github.com'; then
    echo "ok: resolved, and the only registry used is the gate"
  else
    echo "FAIL: resolved but not only through the gate"; fail=1
  fi
else
  echo "$out"; echo "FAIL: did not resolve with the gate wiring present"; fail=1
fi

echo "== 2. [source.crate-gate] removed"
awk '/^\[source.crate-gate\]/{skip=1;next} /^\[/{skip=0} !skip' "$config" > "$work/no-source.toml"
if out=$(run "$work/no-source.toml"); then
  echo "$out"; echo "FAIL: resolved with the replacement source removed"; fail=1
else
  echo "$out" | grep -i 'crate-gate' | head -2
  echo "ok: refused to resolve"
fi

echo "== 3. gate unreachable"
sed "s#127.0.0.1:$port#127.0.0.1:1#" "$config" > "$work/dead.toml"
if out=$(run "$work/dead.toml"); then
  echo "$out"; echo "FAIL: resolved with nothing listening"; fail=1
else
  echo "ok: failed closed with the gate unreachable"
fi

if [ "$fail" -eq 0 ]; then echo "verify-gate: all three states behave"; else echo "verify-gate: FAILED" >&2; fi
exit "$fail"
