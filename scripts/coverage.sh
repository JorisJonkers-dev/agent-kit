#!/usr/bin/env bash
# Line coverage for the cargo workspace, failing below 100 percent.
#
#   scripts/coverage.sh     # rustup toolchain with the llvm-tools component, jq
#
# NixOS: see docs/RUST.md (rustup, gcc and jq from nix shell). The nixpkgs
# rustc cannot be used: its instrumented binaries crash writing the profile.
#
# No new crate: rustc's own `-C instrument-coverage` writes the profiles and
# llvm-profdata / llvm-cov, taken from the same toolchain's llvm-tools
# component, read them. A mismatched LLVM is refused here, since its error
# otherwise is a profile-format version mismatch. src/main.rs files (the one exit-wrapper
# line each crate documents) are excluded. Resolution goes through the
# crate-gate as for every cargo command here (.cargo/config.toml).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest=$root/scripts/Cargo.toml
target=${COVERAGE_TARGET_DIR:-$root/scripts/target/coverage}
min=${COVERAGE_MIN_LINES:-100}

command -v rustc >/dev/null && PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin:$PATH"

for tool in cargo rustc jq llvm-profdata llvm-cov; do
    command -v "$tool" >/dev/null || { echo "coverage: $tool not on PATH (docs/RUST.md)" >&2; exit 2; }
done

rust_llvm=$(rustc -vV | sed -n 's/^LLVM version: \([0-9]*\)\..*/\1/p')
tool_llvm=$(llvm-cov --version | sed -n 's/.*LLVM version \([0-9]*\)\..*/\1/p')
if [ "$rust_llvm" != "$tool_llvm" ]; then
    echo "coverage: rustc uses LLVM $rust_llvm but llvm-cov is LLVM $tool_llvm; profiles would not match" >&2
    exit 2
fi

rm -rf "$target"
mkdir -p "$target/profiles"
export CARGO_TARGET_DIR=$target
export RUSTFLAGS="${RUSTFLAGS:-} -C instrument-coverage"
export LLVM_PROFILE_FILE=$target/profiles/cov-%p-%m.profraw

cargo test --locked --workspace --manifest-path "$manifest"

bins=()
while IFS= read -r line; do bins+=("$line"); done < <(cargo test --locked --workspace --no-run --message-format=json \
    --manifest-path "$manifest" 2>/dev/null \
    | jq -r 'select(.reason=="compiler-artifact" and .profile.test==true and .executable!=null) | .executable')
[ "${#bins[@]}" -gt 0 ] || { echo "coverage: no test binaries found" >&2; exit 2; }

llvm-profdata merge -sparse "$target"/profiles/*.profraw -o "$target/merged.profdata"
args=()
for b in "${bins[@]}"; do args+=(-object "$b"); done
llvm-cov report --instr-profile="$target/merged.profdata" \
    --ignore-filename-regex='(/\.cargo/|/rustc/|/nix/store/|src/main\.rs$)' \
    --summary-only "${args[@]}" | tee "$target/report.txt"
pct=$(llvm-cov export --instr-profile="$target/merged.profdata" --summary-only \
    --ignore-filename-regex='(/\.cargo/|/rustc/|/nix/store/|src/main\.rs$)' "${args[@]}" \
    | jq -r '.data[0].totals.lines.percent')
echo "coverage: workspace line coverage ${pct}% (required ${min}%)"
awk -v p="$pct" -v m="$min" 'BEGIN { exit !(p + 0 >= m + 0) }' \
    || { echo "coverage: below ${min}% line coverage" >&2; exit 1; }
