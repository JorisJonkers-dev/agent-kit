#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: build-runtime-package.sh vX.Y.Z [out-dir]}"
out_dir="${2:-dist}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python_bin="${PYTHON:-python3}"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

mkdir -p "$out_dir" "$stage/rendered" "$stage/package/home" "$stage/package/sdd"

"$python_bin" "$root/render-agent-kit.py" --output "$stage/rendered"

cp -a "$stage/rendered/.claude" "$stage/package/home/.claude"
cp -a "$stage/rendered/.codex" "$stage/package/home/.codex"
cp -a "$stage/rendered/.agents" "$stage/package/home/.agents"
cp -a "$stage/rendered/.specify" "$stage/package/home/.specify"
cp -a "$stage/rendered/.specify/." "$stage/package/sdd/"
cp -a "$root/runner-manifests/runtime/mcp" "$stage/package/mcp"
cp -a "$root/runner-manifests/runtime/bin" "$stage/package/bin"
cp "$root/runner-manifests/runtime/runtime-package.yaml" "$stage/package/runtime-package.yaml"
cp "$root/runner-manifests/runtime-package.schema.json" "$stage/package/runtime-package.schema.json"
cp "$root/manifest.yaml" "$stage/package/manifest.yaml"

(
  cd "$stage/package"
  find . -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
)

tarball="$out_dir/agent-kit-runtime-home-${version}.tar.gz"
tar -C "$stage/package" -czf "$tarball" .
sha256sum "$tarball" > "$tarball.sha256"
