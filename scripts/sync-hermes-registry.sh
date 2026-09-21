#!/usr/bin/env bash
# Push the generated Hermes artifacts into a fleet-infra checkout.
#
# The registry lives here; Flux reads fleet-infra. This copies the two
# generated bodies into the ConfigMaps that Hermes actually consumes:
#
#   registry/generated/hermes/skills-sources.conf
#     -> cluster/flux/apps/agents/hermes/skills-configmap.yaml   data.sources.conf
#   registry/generated/hermes/mcp-servers.yaml
#     -> cluster/flux/apps/agents/hermes/config-configmap.yaml    the mcp_servers: block
#
# It writes files in a git checkout and nothing else. Review and commit in
# fleet-infra; this never commits, pushes or reconciles.
#
# Usage:
#   scripts/sync-hermes-registry.sh <path-to-fleet-infra> [--check]
#
# --check exits non-zero when the checkout is out of date and changes nothing,
# which is the shape a CI job wants.

set -euo pipefail

KIT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FLEET_ROOT="${1:-}"
MODE="${2:-write}"

if [ -z "${FLEET_ROOT}" ]; then
  sed -n '2,22p' "$0"
  exit 64
fi
if [ ! -d "${FLEET_ROOT}/cluster/flux/apps/agents/hermes" ]; then
  echo "not a fleet-infra checkout: ${FLEET_ROOT}" >&2
  echo "expected ${FLEET_ROOT}/cluster/flux/apps/agents/hermes to exist" >&2
  exit 64
fi
case "${MODE}" in
  # Quoted so shellcheck does not read the bare words as commands (SC2209).
  write|--write) MODE='write' ;;
  --check) MODE='check' ;;
  *) echo "unknown option: ${MODE}" >&2; exit 64 ;;
esac

# The generated artifacts must themselves be current, or this would sync a
# stale copy of the registry into the cluster.
if ! (cd "${KIT_ROOT}" && uv run python scripts/render_registry.py --check); then
  echo "sync: generated artifacts are stale; run scripts/render_registry.py --write first" >&2
  exit 1
fi

KIT_ROOT="${KIT_ROOT}" FLEET_ROOT="${FLEET_ROOT}" MODE="${MODE}" python3 - <<'PY'
import os
import pathlib
import re
import sys

kit = pathlib.Path(os.environ["KIT_ROOT"])
fleet = pathlib.Path(os.environ["FLEET_ROOT"])
mode = os.environ["MODE"]

hermes = fleet / "cluster/flux/apps/agents/hermes"
skills_cm = hermes / "skills-configmap.yaml"
config_cm = hermes / "config-configmap.yaml"

sources = (kit / "registry/generated/hermes/skills-sources.conf").read_text()
mcp = (kit / "registry/generated/hermes/mcp-servers.yaml").read_text()

stale: list[str] = []


def indent(text: str, spaces: int) -> str:
    pad = " " * spaces
    return "".join(pad + line if line.strip() else line for line in text.splitlines(keepends=True))


# --- skills-configmap.yaml: replace everything under data.sources.conf ------
current = skills_cm.read_text()
marker = "  sources.conf: |\n"
if marker not in current:
    sys.exit(f"{skills_cm} has no `sources.conf: |` block")
head = current[: current.index(marker) + len(marker)]
desired_skills = head + indent(sources, 4)

# The banner anchor must match render_hermes_mcp's first line, or each sync prepends another copy.
current_config = config_cm.read_text()
match = re.search(
    r"\n(    # GENERATED FROM registry/estate-tooling\.yaml.*\n(?:    #.*\n|\n)*?)?    mcp_servers:\n",
    current_config,
)
if match is None:
    sys.exit(f"{config_cm} has no `    mcp_servers:` block")
desired_config = current_config[: match.start()] + "\n" + indent(mcp, 4)

for path, desired in ((skills_cm, desired_skills), (config_cm, desired_config)):
    if path.read_text() == desired:
        print(f"sync: {path.relative_to(fleet)} already current")
        continue
    if mode == "check":
        stale.append(str(path.relative_to(fleet)))
        continue
    path.write_text(desired)
    print(f"sync: wrote {path.relative_to(fleet)}")

# Verify the VALUE: read the files back and confirm every pinned commit and
# every server key from the registry is present in what is now on disk.
back_skills = skills_cm.read_text()
back_config = config_cm.read_text()
missing: list[str] = []
for line in sources.splitlines():
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    commit = line.split()[2]
    if commit not in back_skills:
        missing.append(f"commit {commit} absent from {skills_cm.name}")
for line in mcp.splitlines():
    server = re.fullmatch(r"  ([a-z0-9_-]+):", line)
    if server and f"      {server.group(1)}:" not in back_config:
        missing.append(f"server {server.group(1)} absent from {config_cm.name}")

if mode == "check" and stale:
    print("sync: OUT OF DATE: " + ", ".join(stale), file=sys.stderr)
    sys.exit(1)
if missing and mode != "check":
    for item in missing:
        print(f"sync: FAILED {item}", file=sys.stderr)
    sys.exit(1)
print("sync: verified every pinned commit and server key landed")
PY

if [ "${MODE}" = 'write' ]; then
  cat <<EOF

Next, in ${FLEET_ROOT}:

  git -C "${FLEET_ROOT}" diff -- cluster/flux/apps/agents/hermes
  # commit, open a PR, merge

Flux reads deploy/production, which a workflow publishes from main. Wait for
that publish to land before reconciling -- reconciling early silently applies
the previous revision. Then:

  kubectl -n agents-system rollout restart deploy/hermes
  kubectl -n agents-system logs deploy/hermes -c sync-skills
  kubectl -n agents-system exec deploy/hermes -- hermes skills list
  kubectl -n agents-system exec deploy/hermes -- hermes doctor

Read sync-skills' output rather than trusting Ready: it counts skills from
disk and REJECTS any source whose commit does not match its pin.
EOF
fi
