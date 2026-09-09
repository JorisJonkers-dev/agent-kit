#!/usr/bin/env bash
# Collect everything needed to compose a PR into an output directory.
# Read-only: this script never mutates local or remote state.
#
# Usage: bash collect.sh [outdir]   (default: /tmp/pr-ctx)

set -uo pipefail
OUT="${1:-/tmp/pr-ctx}"
mkdir -p "$OUT"
: > "$OUT/notes.txt"

note() { echo "$*" >> "$OUT/notes.txt"; }
have() { command -v "$1" >/dev/null 2>&1; }

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  note "FATAL: not inside a git repository"
  cat "$OUT/notes.txt"; exit 1
fi

# ---------- branch / base ----------
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
BASE=""
for cand in $(git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|origin/||') main master develop trunk; do
  [ -n "$cand" ] || continue
  if git show-ref --verify --quiet "refs/remotes/origin/$cand" || git show-ref --verify --quiet "refs/heads/$cand"; then
    BASE="$cand"; break
  fi
done
BASE="${BASE:-main}"
REF="origin/$BASE"
git show-ref --verify --quiet "refs/remotes/$REF" || REF="$BASE"

MERGE_BASE="$(git merge-base "$REF" HEAD 2>/dev/null || echo "")"
RANGE="${MERGE_BASE:+$MERGE_BASE..HEAD}"

{
  echo "branch=$BRANCH"
  echo "base=$BASE"
  echo "merge_base=${MERGE_BASE:-unknown}"
  echo "commits_ahead=$([ -n "$RANGE" ] && git rev-list --count "$RANGE" || echo '?')"
  if git rev-parse --abbrev-ref '@{upstream}' >/dev/null 2>&1; then
    echo "unpushed=$(git log --oneline '@{upstream}..HEAD' 2>/dev/null | wc -l | tr -d ' ')"
  else
    echo "unpushed=all (no upstream branch — push will need -u)"
  fi
  echo "upstream_set=$(git rev-parse --abbrev-ref '@{upstream}' 2>/dev/null || echo 'no')"
} > "$OUT/branch.txt"

# ---------- commits & diff ----------
if [ -n "$RANGE" ]; then
  git log --pretty=format:'%h%x09%an%x09%s' "$RANGE" > "$OUT/commits.txt"
  git log --pretty=format:'--- %h%n%B' "$RANGE" > "$OUT/commit_bodies.txt"
  git diff --stat "$RANGE" > "$OUT/diffstat.txt"
  git diff --name-only "$RANGE" > "$OUT/files.txt"
  git diff --name-status "$RANGE" > "$OUT/files_status.txt"
  # Bounded patch: useful for intent, capped so it can't blow up context.
  git diff "$RANGE" -- . ':(exclude)*.lock' ':(exclude)*-lock.json' ':(exclude)*.svg' \
    | head -c 200000 > "$OUT/diff_head.patch"
else
  note "WARN: no merge-base against $BASE; commit/diff context unavailable"
fi

# ---------- PR template discovery ----------
ROOT="$(git rev-parse --show-toplevel)"
: > "$OUT/template_candidates.txt"
while IFS= read -r f; do
  echo "$f" >> "$OUT/template_candidates.txt"
done < <(
  find "$ROOT/.github" "$ROOT/docs" "$ROOT" -maxdepth 2 \
    \( -iname 'pull_request_template*.md' -o -iname 'PULL_REQUEST_TEMPLATE*.md' \
       -o -iname 'merge_request_template*.md' \) 2>/dev/null | sort -u
  find "$ROOT/.github/PULL_REQUEST_TEMPLATE" "$ROOT/.gitlab/merge_request_templates" \
    -maxdepth 1 -iname '*.md' 2>/dev/null | sort -u
)
TCOUNT=$(wc -l < "$OUT/template_candidates.txt" | tr -d ' ')
note "templates_found=$TCOUNT"
if [ "$TCOUNT" = "1" ]; then
  cp "$(head -1 "$OUT/template_candidates.txt")" "$OUT/template.md" 2>/dev/null \
    && note "template copied to $OUT/template.md"
elif [ "$TCOUNT" = "0" ]; then
  note "NO TEMPLATE: synthesize headings from merged-PR history instead"
else
  note "MULTIPLE TEMPLATES: ask the user which one (see template_candidates.txt)"
fi

# ---------- CODEOWNERS for touched paths ----------
for co in "$ROOT/.github/CODEOWNERS" "$ROOT/CODEOWNERS" "$ROOT/docs/CODEOWNERS"; do
  [ -f "$co" ] && cp "$co" "$OUT/CODEOWNERS" && note "codeowners=$co" && break
done

# ---------- GitHub metadata ----------
if have gh && gh auth status >/dev/null 2>&1; then
  gh repo view --json nameWithOwner,visibility,isPrivate,defaultBranchRef,url \
    > "$OUT/repo.json" 2>/dev/null || note "WARN: gh repo view failed"

  gh pr list --head "$BRANCH" --state all --limit 5 \
    --json number,title,url,state,isDraft,body \
    > "$OUT/existing_pr.json" 2>/dev/null || echo '[]' > "$OUT/existing_pr.json"

  gh pr list --state merged --limit 100 --json \
number,title,body,url,mergedAt,author,labels,assignees,reviews,reviewRequests,baseRefName,headRefName,files,additions,deletions,changedFiles \
    > "$OUT/merged_prs.json" 2>/dev/null || { echo '[]' > "$OUT/merged_prs.json"; note "WARN: merged PR dump failed"; }

  gh label list --limit 200 --json name,description > "$OUT/labels.json" 2>/dev/null || echo '[]' > "$OUT/labels.json"

  OWNER="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["nameWithOwner"].split("/")[0])' "$OUT/repo.json" 2>/dev/null || echo '')"
  if [ -n "$OWNER" ]; then
    gh project list --owner "$OWNER" --format json > "$OUT/projects.json" 2>/dev/null \
      || note "WARN: gh project list failed (likely missing 'project' scope: gh auth refresh -s project)"
  fi
else
  note "FATAL-ish: gh unavailable or unauthenticated — no template mining, no reviewer history, no boards"
  echo '[]' > "$OUT/merged_prs.json"
fi

echo "== collect.sh done -> $OUT =="
cat "$OUT/notes.txt"
