#!/usr/bin/env python3
"""Plan a pause, rollback or resume of one Project in the JorisJonkers-dev estate.

Prints the plan and the exact `gh workflow run` command for the owner to run.
This script never runs a workflow and never writes anything: its only gh call
is a GET of the Project's pin file (gh api). Stdlib python3 only.

Exit status: 0 plan printed; 1 refused (reason on stderr, no command printed).
"""

import argparse
import re
import shlex
import subprocess
import sys

REPO = "JorisJonkers-dev/estate"
PROJECT_RE = re.compile(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$")
NUMBER = r"(?:0|[1-9][0-9]*)"
VERSION_RE = re.compile(rf"^{NUMBER}\.{NUMBER}\.{NUMBER}$")
PIN_RE = re.compile(
    r"^\s*event\.toolkit\.fluxcd\.io/version:[ \t]*[\"']?([^\"'\s#]+)[\"']?[ \t]*(?:#.*)?$",
    re.M,
)
ANNOTATION_RE = re.compile(
    r"^\s*estate\.jorisjonkers\.dev/([a-z-]+):[ \t]*[\"']?(.*?)[\"']?[ \t]*$", re.M
)
CONTROL_RE = re.compile("[\x00-\x1f\x7f\x85\u2028\u2029]")


class Refusal(Exception):
    """The plan is refused; the message says why."""


def refuse(message):
    raise Refusal(message)


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def read_pin_file(project):
    """GET projects/<project>/source.yaml from the estate repository, read-only."""
    cmd = [
        "gh", "api",
        f"repos/{REPO}/contents/projects/{project}/source.yaml",
        "-H", "Accept: application/vnd.github.raw",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    except FileNotFoundError:
        refuse("gh is not on PATH, so the pin cannot be read. Stop; do not "
               "fill the pin in from memory. Install gh or pass --pin from a "
               "reading you made yourself.")
    except subprocess.TimeoutExpired:
        refuse("gh did not answer within 120 seconds reading "
               f"projects/{project}/source.yaml. Stop and retry.")
    if result.returncode != 0:
        detail = result.stderr.strip() or "no detail"
        refuse(f"could not read projects/{project}/source.yaml from {REPO} "
               f"(gh exit {result.returncode}: {detail}). Check `gh auth status` "
               "and that the repository is reachable. Stop.")
    return result.stdout


def parse_pin_file(text, project):
    """Return (pin version, annotations dict) from a source.yaml text."""
    pins = PIN_RE.findall(text)
    if len(pins) != 1:
        refuse(f"expected exactly one event.toolkit.fluxcd.io/version annotation "
               f"in projects/{project}/source.yaml, found {len(pins)}.")
    pin = pins[0]
    if not VERSION_RE.match(pin):
        refuse(f"the pin '{pin}' in projects/{project}/source.yaml is not a "
               "release version X.Y.Z.")
    annotations = {name: value for name, value in ANNOTATION_RE.findall(text)}
    return pin, annotations


def gh_run_command(workflow, pairs):
    parts = ["gh", "workflow", "run", workflow, "--repo", REPO, "--ref", "main"]
    for key, value in pairs:
        parts += ["-f", f"{key}={value}"]
    return " ".join(shlex.quote(part) for part in parts)


def check_reason(reason):
    if reason is None or not reason.strip():
        refuse("this action needs --reason (non-blank); the workflow records it on the pin file.")
    if CONTROL_RE.search(reason):
        refuse("--reason must be one line with no control characters.")


def build(args):
    project = args.project
    if not PROJECT_RE.match(project):
        refuse(f"'{project}' is not a Project's name (lowercase letters, digits and hyphens).")

    if args.action != "rollback" and (args.target or args.proof_names or args.no_proof):
        refuse("--target, --proof-names and --no-proof only apply to --action rollback.")
    if args.action in ("pause", "rollback"):
        check_reason(args.reason)

    if args.pin is not None:
        pin = args.pin
        if not VERSION_RE.match(pin):
            refuse(f"--pin '{pin}' is not a release version X.Y.Z.")
        annotations = {}
        pin_source = "given with --pin (file not read)"
        pin_note = ("pin given by hand: the pin file was not read, so a recorded "
                    "rollback is not checked")
    else:
        pin, annotations = parse_pin_file(read_pin_file(project), project)
        pin_source = f"projects/{project}/source.yaml, event.toolkit.fluxcd.io/version"
        pin_note = None

    lines = [
        "estate rollback plan (this script runs nothing; the owner runs the command)",
        f"project:  {project}",
        f"action:   {args.action}",
        f"pin:      {pin} ({pin_source})",
    ]
    if pin_note:
        lines.append(pin_note)
    if "paused-by" in annotations:
        lines.append(f"state:    paused by {annotations['paused-by']} "
                     f"at {annotations.get('paused-at', '?')}: "
                     f"{annotations.get('paused-reason', '')}")
    if "rollback-version" in annotations:
        lines.append(f"state:    rolled back to {annotations['rollback-version']}")

    if args.action == "pause":
        command = gh_run_command("pause.yml", [("project", project), ("reason", args.reason)])
        lines += [
            "",
            "The pin stays at " + pin + " until a resume. Composition still checks",
            "the newest fragment and reports what it finds.",
            "",
            "Step 1, run by the owner:",
            "  " + command,
        ]
    elif args.action == "resume":
        if "rollback-fragment" in annotations:
            lines.append("WARNING: this Project is rolled back. Resuming moves the pin to "
                         "its NEWEST fragment, so the rolled-back release returns unless "
                         "the fix has landed. Only resume once it has.")
        command = gh_run_command("resume.yml", [("project", project)])
        lines += [
            "",
            "Step 1, run by the owner (starts a composition that moves the pin):",
            "  " + command,
        ]
    else:  # rollback
        if not args.target:
            refuse("--action rollback needs --target VERSION (the release to go back to).")
        target = args.target
        if not VERSION_RE.match(target):
            refuse(f"--target '{target}' is not a release version X.Y.Z.")
        if "rollback-fragment" in annotations:
            refuse(f"{project} is already rolled back to {annotations.get('rollback-version', '?')}; "
                   "resume it first (rollback.yml refuses this too).")
        if version_key(target) >= version_key(pin):
            refuse(f"target {target} is not earlier than the pin {pin}; "
                   "a rollback goes back, never forward.")
        if (args.proof_names is not None) == bool(args.no_proof):
            refuse("Migration Proof not stated: pass --proof-names <release the proof names> "
                   "or --no-proof when the pinned release carries none"
                   + (" (not both)" if args.no_proof else ""))
        if args.proof_names is not None:
            if not VERSION_RE.match(args.proof_names):
                refuse(f"--proof-names '{args.proof_names}' is not a release version X.Y.Z.")
            if target != args.proof_names:
                refuse(f"target {target} is not the release the pinned release {pin}'s "
                       f"Migration Proof names ({args.proof_names}); only that one runs "
                       "against the current schema.")
            proof_line = f"Migration Proof: target {target} is the release --proof-names gives: ok"
        else:
            proof_line = (f"Migration Proof: --no-proof says release {pin} carries none; "
                          "rollback.yml still checks the real lock.")
        command = gh_run_command("rollback.yml", [
            ("project", project), ("version", target), ("reason", args.reason)])
        lines += [
            f"target:   {target} (earlier than the pin {pin}: ok)",
            proof_line,
            "",
            "Step 1, run by the owner (rollback.yml refuses an unproven target before anything runs):",
            "  " + command,
            "",
            "rollback.yml then records the pause and the target in its own commit, starts",
            "composition, and leaves the Project paused. Its live steps are in rollback.yml.",
            "",
            "Later, only once the fix has landed (resume moves the pin to the newest fragment):",
            "  " + gh_run_command("resume.yml", [("project", project)]),
        ]
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--project", required=True, help="the Project, as participants.yml names it")
    parser.add_argument("--action", required=True, choices=["pause", "rollback", "resume"])
    parser.add_argument("--target", help="rollback: the release version to go back to")
    parser.add_argument("--reason", help="pause and rollback: why, one line")
    parser.add_argument("--pin", help="the pinned version, instead of reading the pin file")
    parser.add_argument("--proof-names", dest="proof_names",
                        help="rollback: the release the pinned release's Migration Proof names")
    parser.add_argument("--no-proof", dest="no_proof", action="store_true",
                        help="rollback: state that the pinned release carries no Migration Proof")
    args = parser.parse_args(argv)
    try:
        print(build(args))
    except Refusal as exc:
        print(f"REFUSED: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
