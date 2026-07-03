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

stage_council_skill() {
  local skill_src="$1"
  local dest="$2"
  mkdir -p "$dest"
  cp "$skill_src" "$dest/SKILL.md"
  chmod 0644 "$dest/SKILL.md"
  "$python_bin" - "$root" "$dest" <<'PY'
import importlib.util
import shutil
import sys
from pathlib import Path

root = Path(sys.argv[1])
dest = Path(sys.argv[2])
spec = importlib.util.spec_from_file_location("render_agent_kit", root / "render-agent-kit.py")
if spec is None or spec.loader is None:
    raise SystemExit("cannot load render-agent-kit.py")
renderer = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = renderer
spec.loader.exec_module(renderer)
for rel, mode in renderer.council_toolkit_files():
    source = root / "council" / rel
    target = dest / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(int(mode, 8))
PY
}

cp -a "$stage/rendered/.claude" "$stage/package/home/.claude"
cp -a "$stage/rendered/.codex" "$stage/package/home/.codex"
cp -a "$stage/rendered/.agents" "$stage/package/home/.agents"
stage_council_skill "$root/templates/repo/.claude/skills/council/SKILL.md" "$stage/package/home/.claude/skills/council"
stage_council_skill "$root/templates/repo/.agents/skills/council/SKILL.md" "$stage/package/home/.codex/skills/council"
cp -a "$root/templates/repo/.specify" "$stage/package/home/.specify"
cp -a "$root/templates/repo/.specify/." "$stage/package/sdd/"
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
