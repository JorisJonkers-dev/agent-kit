#!/usr/bin/env python3
"""Scaffold and validate a deploy-kit 1.x Project Intent.

  intent.py scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE \
      --image IMAGE --process PROCESS --port N --out DIR
  intent.py validate PATH [EXTRA_PATH ...]

scaffold writes DIR/deploy/<project>.project.yml in the shape of the minimal
example, plus DIR/deploy/env/<process>/base.env. Values the spec leaves to the
author are marked CHECK; read the live spec before trusting them.

validate runs `deploy-kit validate PATH [EXTRA_PATH ...]` and passes its output
through unchanged. Exit status mirrors deploy-kit: 0 accepted, 1 refused,
2 called wrongly (including deploy-kit not installed).
"""
import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

AUDIENCES = ("anonymous", "authenticated", "internal", "lan")
DNS_LABEL = re.compile(r"^[a-z]([-a-z0-9]{0,61}[a-z0-9])?$")

PROJECT_TEMPLATE = """\
# Project Intent for {project}. Requirements only, no mechanisms
# (deploy-kit spec/v1/10-project-intent.md). Lines marked CHECK are defaults
# this skill chose; confirm them against the live spec before validating.

apiVersion: intent.jorisjonkers.dev/v1
kind: Project
schemaVersion: 1.0.0

project: {project}
owner: {owner}

applications:
  - id: {project}
    exposure:
      - name: public
        host: {host}
        audience: {audience}
        routes:
          - {{ path: /, match: prefix, process: {process}, surface: http }}

    processes:
      - name: {process}
        lifecycle: application
        image: {image}   # an alias; the lock resolves digest, uid and gid
        runtime: node    # CHECK: the runtime the image runs on

        provides:
          http: {port}

        # memory and cpu are required on every Process
        placement:
          memory: 256Mi  # CHECK
          cpu: 50m       # CHECK

        probes:
          readiness: {{ path: /healthz/ready, port: {port} }}  # CHECK path
          liveness: {{ path: /healthz/live, port: {port} }}    # CHECK path

        startupBudget: 20s   # CHECK
        cutover: continuous  # required; use interrupted only with a reason
"""

ENV_TEMPLATE = """\
# Environment for the {process} Process. Literal values, ${{secret:<granted-path>#<key>}}
# for a granted secret, or ${{dependency:...}} for another Application's coordinate.
"""


def scaffold(args):
    for label, value in (("--project", args.project), ("--process", args.process)):
        if not DNS_LABEL.match(value):
            print(f"intent.py: {label} must be a DNS-1123 label, got {value!r}", file=sys.stderr)
            return 2
    if args.audience not in AUDIENCES:
        print(f"intent.py: --audience must be one of {', '.join(AUDIENCES)}", file=sys.stderr)
        return 2
    if not 1 <= args.port <= 65535:
        print("intent.py: --port must be 1-65535", file=sys.stderr)
        return 2

    out = Path(args.out)
    project_file = out / "deploy" / f"{args.project}.project.yml"
    env_file = out / "deploy" / "env" / args.process / "base.env"
    existing = [p for p in (project_file, env_file) if p.exists()]
    if existing:
        for p in existing:
            print(f"intent.py: refusing to overwrite {p}", file=sys.stderr)
        return 2

    values = {
        "project": args.project, "owner": args.owner, "host": args.host,
        "audience": args.audience, "image": args.image, "process": args.process,
        "port": args.port,
    }
    project_file.parent.mkdir(parents=True, exist_ok=True)
    project_file.write_text(PROJECT_TEMPLATE.format(**values))
    env_file.parent.mkdir(parents=True, exist_ok=True)
    env_file.write_text(ENV_TEMPLATE.format(process=args.process))
    print(f"wrote {project_file}")
    print(f"wrote {env_file}")
    return 0


def validate(args):
    target = Path(args.path)
    if not target.exists():
        print(f"intent.py: no such path: {args.path}", file=sys.stderr)
        return 2
    for extra in args.extra:
        if not Path(extra).exists():
            print(f"intent.py: no such path: {extra}", file=sys.stderr)
            return 2
    tool = shutil.which("deploy-kit")
    if tool is None:
        print("intent.py: deploy-kit is not installed on PATH; install the release the "
              "repository pins and rerun. Nothing was validated.", file=sys.stderr)
        return 2

    sys.stdout.flush()
    sys.stderr.flush()
    # deploy-kit's own output is inherited, so it reaches the owner verbatim.
    code = subprocess.run([tool, "validate", args.path, *args.extra]).returncode
    if code == 0:
        print("deploy-kit validate: accepted (exit 0)")
    elif code == 1:
        print("deploy-kit validate: REFUSED (exit 1). Report the output above to the owner verbatim.")
    elif code == 2:
        print("deploy-kit validate: called wrongly (exit 2). Check the paths and arguments.")
    else:
        print(f"deploy-kit validate: unexpected exit {code}. Report it to the owner verbatim.")
    return code if code in (0, 1, 2) else 2


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    s = sub.add_parser("scaffold", help="write a minimal project file and its base.env")
    for flag in ("--project", "--owner", "--host", "--audience", "--image", "--process", "--out"):
        s.add_argument(flag, required=True)
    s.add_argument("--port", type=int, required=True)
    s.set_defaults(func=scaffold)

    v = sub.add_parser("validate", help="run deploy-kit validate and report the verdict")
    v.add_argument("path")
    v.add_argument("extra", nargs="*", help="further files or directories read together (e.g. deploy/env)")
    v.set_defaults(func=validate)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
