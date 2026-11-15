#!/usr/bin/env bash
# Builds the user-facing tools that setup installs from source today
# (registry/estate-tooling.yaml workspace_tools, plus critique, model-gateway
# and build-tracker) for one target and packs them as one tarball:
#
#   release-binaries.sh build TAG TARGET [OUT_DIR]   -> OUT_DIR/agent-kit-tools-TAG-TARGET.tar.gz (+ .sha256)
#   release-binaries.sh sums OUT_DIR                 -> OUT_DIR/SHA256SUMS over every *.tar.gz in it
#   release-binaries.sh stamp TAG DIR               -> DIR/bootstrap.sh: scripts/bootstrap.sh with TAG and DIR/SHA256SUMS stamped in
#   release-binaries.sh smoke TAG TARGET DIR         -> verify DIR against SHA256SUMS, unpack, run every binary
#
# Build from the repository root's pinned workspace with --locked, so the
# lockfile is what is built, and from the repository. Linux
# targets are musl and must come out statically linked. docs/RUST.md,
# "Release binaries".
set -euo pipefail

TOOLS="agent-launcher gold-band-gate gold-band-run-start gold-band-findings gold-band-proof research-layout ticket-publish workflow-lint coverage-gate ci-wait litellm-gate estate critique model-gateway build-tracker"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
sha_check() { if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$@"; else shasum -a 256 -c "$@"; fi; }

is_static() {
  case "$1" in
    *-linux-musl)
      local out
      out="$(file -L "$2")"
      case "$out" in *"statically linked"*|*"static-pie linked"*) return 0 ;; esac
      echo "not statically linked: $out" >&2
      return 1 ;;
    *) return 0 ;;
  esac
}

cmd_build() {
  local tag="${1:?tag}" target="${2:?target}" out="${3:-dist}"
  local name="agent-kit-tools-${tag}-${target}"
  local pkgs=() tool
  for tool in $TOOLS; do pkgs+=(-p "$tool"); done
  mkdir -p "$out"
  out="$(cd "$out" && pwd)"
  local tdir="${CARGO_TARGET_DIR:-$ROOT/scripts/target}"
  (cd "$ROOT" && cargo build --release --locked --target "$target" --manifest-path scripts/Cargo.toml "${pkgs[@]}")
  local stage; stage="$(mktemp -d)"; trap 'rm -rf "$stage"' RETURN
  mkdir -p "$stage/$name/bin"
  for tool in $TOOLS; do
    local built="$tdir/$target/release/$tool"
    [ -x "$built" ] || { echo "the build left no $built" >&2; return 1; }
    is_static "$target" "$built"
    cp "$built" "$stage/$name/bin/$tool"
    chmod 0755 "$stage/$name/bin/$tool"
  done
  # The release tag and target, stamped like the runtime-home package's
  # file name carries the tag.
  printf '%s\n%s\n' "$tag" "$target" > "$stage/$name/VERSION"
  (cd "$stage/$name" && sha VERSION bin/* > SHA256SUMS)
  tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
  (cd "$out" && sha "$name.tar.gz" > "$name.tar.gz.sha256")
  echo "built $out/$name.tar.gz"
}

cmd_sums() {
  local dir="${1:?dir}"
  (cd "$dir" && sha agent-kit-tools-*.tar.gz > SHA256SUMS && cat SHA256SUMS)
}

cmd_stamp() {
  local tag="${1:?tag}" dir="${2:?dir}"
  [ -f "$dir/SHA256SUMS" ] || { echo "no SHA256SUMS in $dir" >&2; return 1; }
  local block; block="$(mktemp)"; trap 'rm -f "$block"' RETURN
  { echo "STAMPED_TAG='$tag'"; echo "STAMPED_SUMS='$(cat "$dir/SHA256SUMS")"; echo "'"; } > "$block"
  awk -v blockfile="$block" '
    /^# --- end stamped ---/ { skipping = 0 }
    skipping { next }
    { print }
    /^# --- stamped by/ { while ((getline line < blockfile) > 0) print line; skipping = 1 }
  ' "$ROOT/scripts/bootstrap.sh" > "$dir/bootstrap.sh"
  chmod 0755 "$dir/bootstrap.sh"
  grep -q "^STAMPED_TAG='$tag'\$" "$dir/bootstrap.sh" || { echo "stamping failed" >&2; return 1; }
  echo "stamped $dir/bootstrap.sh"
}

cmd_smoke() {
  local tag="${1:?tag}" target="${2:?target}" dir="${3:?dir}"
  local name="agent-kit-tools-${tag}-${target}"
  [ -f "$dir/SHA256SUMS" ] || { echo "no SHA256SUMS in $dir" >&2; return 1; }
  # Only this target's line: the other targets' tarballs are not here.
  (cd "$dir" && grep " ${name}.tar.gz\$" SHA256SUMS | sha_check -)
  local work; work="$(mktemp -d)"; trap 'rm -rf "$work"' RETURN
  tar -C "$work" -xzf "$dir/$name.tar.gz"
  (cd "$work/$name" && sha_check SHA256SUMS)
  [ "$(sed -n 1p "$work/$name/VERSION")" = "$tag" ] || { echo "VERSION does not carry $tag" >&2; return 1; }
  local tool out code
  for tool in $TOOLS; do
    [ -x "$work/$name/bin/$tool" ] || { echo "missing $tool" >&2; return 1; }
    is_static "$target" "$work/$name/bin/$tool"
    # Most tools print usage to stderr and exit 2 for --help or --version;
    # the check is that the binary starts and answers (exit 0 to 2 with output),
    # not that it was killed, missing a loader (126, 127) or silent.
    set +e
    out="$("$work/$name/bin/$tool" --help 2>&1)"; code=$?
    set -e
    if [ "$code" -gt 2 ] || [ -z "$out" ]; then
      echo "$tool --help: exit $code, output: ${out:-<none>}" >&2
      return 1
    fi
    echo "ok $tool (exit $code)"
  done
}

case "${1:-}" in
  build) shift; cmd_build "$@" ;;
  sums) shift; cmd_sums "$@" ;;
  stamp) shift; cmd_stamp "$@" ;;
  smoke) shift; cmd_smoke "$@" ;;
  *) echo "usage: release-binaries.sh build TAG TARGET [OUT_DIR] | sums OUT_DIR | stamp TAG DIR | smoke TAG TARGET DIR" >&2; exit 2 ;;
esac
