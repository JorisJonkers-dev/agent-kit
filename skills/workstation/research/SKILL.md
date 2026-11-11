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

The instance is available when the headless runner is on `PATH` and the Workflow
A workflow file is present:

```sh
command -v gold-band
test -f "${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/gold-band-personal/workflows/research.json"
```

Both must succeed. When either fails, no instance is available: go to "Fall back
to direct delegation".

## Start the run

Write the research question to a requirement file under the session scratch
directory, never inside the repository, then run the headless runner from the
repository root:

```sh
scratch="${TMPDIR:-/tmp}/agent-kit-session"
mkdir -p "$scratch"
printf '%s\n' "<research question>" > "$scratch/research-requirement.md"

gold-band run \
  --workflow "${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit/gold-band-personal/workflows/research.json" \
  --requirement-file "$scratch/research-requirement.md"
```

The run's own first node, the run-start shim, re-checks the repository rule and
the family allowlist before any model node runs, and may still refuse. When it
refuses, report its refusal text as it stands (not the rule message below) and
fall back to direct delegation. Do not start the work instance, and do not retry the run on another
instance.

## Fall back to direct delegation

Fall back through the `delegate` skill, which reaches the other clients through
the model gateway. Say exactly one of these, then delegate:

- No instance available: `No Gold Band instance is available on this machine; falling back to direct delegation.`
- Repository refused by the rule: `The personal instance's repository rule (remotes: not-nedap) refuses this repository; falling back to direct delegation.`

Give the delegation the research question and the repository, and keep the
gateway's data policy: a repository with a `nedap` remote goes nowhere the policy
does not permit.

## What to report

Report which lane ran: the Gold Band run, with its run id and where its result
landed, or direct delegation, with the delegations that answered. For a fallback,
quote the exact message above; for a refused run, quote the shim's refusal text.
Then say what the research found and what remains unverified.
