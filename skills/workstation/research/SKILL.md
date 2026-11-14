---
name: research
description: Use when the owner asks to research a topic, survey literature or prior art, or find the latest work on a question. Starts a Gold Band research run when the personal instance is available, otherwise delegates directly.
---

# research

This is the front door for research. Start a Gold Band Workflow A run in the
personal instance when that instance is on this machine and its repository rule
allows the repository; otherwise fall back to direct delegation through the
`delegate` skill. Check the repository rule first, then the instance: a refused
repository never reaches the instance.

## Check the repository rule

The personal instance's rule is `remotes: not-nedap`, `no_remote: allow`
(ADR-0011). Read every remote, in both the ssh and the https form:

```sh
git remote -v
```

Refuse the repository when any remote's owner is `nedap`, in any case, so
`git@github.com:nedap/<repo>.git` and `https://github.com/nedap/<repo>.git` are
both refused. A repository with no remote is allowed. On a refusal, start no run
and do not start a run on the instance: go to "Fall back to direct delegation" and
report the refusal message given there.

## Check the instance

The instance is available when the headless runner and the proof check are on
`PATH` and the Workflow A workflow file is present:

```sh
command -v gold-band
command -v gold-band-proof
test -f "${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/gold-band-personal/workflows/research.json"
```

All three must succeed. When any fails, no instance is available: go to "Fall back
to direct delegation".

## Start the run

Workflow A's fetchers are Pi over LiteLLM, not Anthropic models, and they read
the plan Opus writes from the question. So the question must be fit for a
public model: no private repository content, no employer material, no secrets.
When it is not, do not start the run; delegate directly instead.

The run never works in the current repository. Make a fresh, empty git
repository with no remote under the session scratch directory, put the
question in it as the requirement file, and start the run from there through
the proof check, which refuses a workflow whose headless proof is missing,
skipped, failed or stale:

```sh
scratch="${TMPDIR:-/tmp}/agent-kit-session/research-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$scratch" && git -C "$scratch" init -q
printf '%s\n' "<research question>" > "$scratch/requirement.md"

cd "$scratch" && gold-band-proof run \
  --workflow "${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/gold-band-personal/workflows/research.json" \
  -- --requirement-file requirement.md
```

The run's own first node, the run-start shim, re-checks the repository rule and
the family allowlist before any model node runs, and may still refuse. When it
refuses, report its refusal text as it stands (not the rule message below) and
fall back to direct delegation. Do not start the work instance, and do not retry the run on another
instance.

The result lands on a branch of its own, `research/<run id>`, of the private
research repository, in the papers layout; the verifier has checked that every
quote it added is on its page as fetched. The branch is not pushed: say where
it is and leave the review and the push to the owner.

## Fall back to direct delegation

Fall back through the `delegate` skill, which reaches the other clients through
the model gateway. Say exactly one of these, then delegate:

- No instance available: `No Gold Band instance is available on this machine; falling back to direct delegation.`
- Repository refused by the rule: `The personal instance's repository rule (remotes: not-nedap) refuses this repository; falling back to direct delegation.`

Give the delegation the research question and the repository, and keep the
gateway's data policy: a repository with a `nedap` remote goes nowhere the policy
does not permit. Prefer the Pi clients (`pi-<route>`) the gateway lists: they run
confined to their working directory, and `pi-public` takes a question only when
it carries nothing private and names no repository.

## What to report

Report which lane ran: the Gold Band run, with its run id, its research branch
and the acceptance node's judgment, or direct delegation, with the delegations that answered. For a fallback,
quote the exact message above; for a refused run, quote the shim's refusal text.
Then say what the research found and what remains unverified.
