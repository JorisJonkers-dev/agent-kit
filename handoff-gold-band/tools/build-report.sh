#!/usr/bin/env bash
# build-report.sh: refresh the three Gold Band build trackers (specs #327,
# #350, #367) from the issue tracker and from feat/gold-band-epic.
# A ticket cited as "(#NNN" in a commit subject on the branch counts as
# landed (merged, with that commit as its note), unless its status is set
# by hand to something else afterwards. Prints each page path; the
# orchestrator republishes them with the Artifact tool.
set -euo pipefail
REPO=/Users/j.w.jonkers/IDEAProjects/agent-kit-gold-band
BT=~/.claude-personal/skills/build-tracker/build-tracker.mjs
D=~/.local/state/agent-kit/build-tracker/github.com/jorisjonkers-dev/agent-kit
cd "$REPO"
for spec in 327 350 367; do
  node "$BT" sync --spec "$spec" >/dev/null
  members=$(python3 -c "import json,sys;print(' '.join(json.load(open(sys.argv[1]))['tickets']))" "$D/$spec/state.json")
  sets=()
  while read -r sha subject; do
    for n in $(printf '%s\n' "$subject" | grep -oE '\(#[0-9]+(, #[0-9]+)*\)' | grep -oE '[0-9]+'); do
      case " $members " in *" $n "*) sets+=("$n=merged:$sha") ;; esac
    done
  done < <(git log --reverse --format='%h %s' origin/main..HEAD)
  # Hand-set statuses for tickets whose commit is only part of the work.
  # Edit this list as the build moves.
  case $spec in
    327) sets+=('345=running:roles crate landed (3362da3); setup wiring building' '328=todo:env forwarding and hooks verified from the adapter source; only app UI details left') ;;
    350) sets+=('352=merged:538168c (setup wiring still open)') ;;
  esac
  [ ${#sets[@]} -gt 0 ] && node "$BT" set --state "$D/$spec/state.json" "${sets[@]}" >/dev/null
  node "$BT" render --state "$D/$spec/state.json" --out "$D/$spec/page.html" >/dev/null
  echo "$D/$spec/page.html"
done
