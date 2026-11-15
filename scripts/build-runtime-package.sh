#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: build-runtime-package.sh vX.Y.Z [out-dir]}"
out_dir="${2:-dist}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python_bin="${PYTHON:-python3}"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

mkdir -p "$out_dir" "$stage/rendered" "$stage/package/home"

"$python_bin" "$root/scripts/render_agent_kit.py" --output "$stage/rendered"

cp -a "$stage/rendered/.claude" "$stage/package/home/.claude"
# No repo skill renders to .codex now that the hooks are gone, so the
# renderer emits no .codex directory at all and `cp -a` of it fails with
# `cannot stat`, so create the directory and copy only if the renderer produced one.
mkdir -p "$stage/package/home/.codex"
if [ -d "$stage/rendered/.codex" ]; then
  cp -a "$stage/rendered/.codex/." "$stage/package/home/.codex/"
fi
cp -a "$stage/rendered/.agents" "$stage/package/home/.agents"
cp -a "$root/runner/manifests/runtime/mcp" "$stage/package/mcp"
cp -a "$root/runner/manifests/runtime/bin" "$stage/package/bin"
cp "$root/runner/manifests/runtime/runtime-package.yaml" "$stage/package/runtime-package.yaml"
cp "$root/runner/manifests/runtime-package.schema.json" "$stage/package/runtime-package.schema.json"
cp "$root/manifest.yaml" "$stage/package/manifest.yaml"

(
  cd "$stage/package"
  find . -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
)

tarball="$out_dir/agent-kit-runtime-home-${version}.tar.gz"
tar -C "$stage/package" -czf "$tarball" .
sha256sum "$tarball" > "$tarball.sha256"
