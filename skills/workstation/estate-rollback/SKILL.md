---
name: estate-rollback
description: Use when a person asks to pause, roll back or resume one Project in the JorisJonkers-dev estate, or asks which earlier release a Project should go back to.
---

# Estate rollback, pause and resume

This skill plans a hold on one Project and hands the owner the exact command to run. It never dispatches a workflow, never pushes to the estate repository, and never copies doc content. The canonical docs are read live with `gh` every time. Verifying against a local clone at `~/Projects/JorisJonkers-dev/estate` is fine, but the answer comes from `gh`.

## Read first, live

1. deploy-kit `spec/v1/55-delivery.md`, sections "Pause and Rollback" and "Failure and undo". Fetch with `gh api -H 'Accept: application/vnd.github.raw' repos/JorisJonkers-dev/deploy-kit/contents/spec/v1/55-delivery.md`.
2. The estate `README.md`, section "Holding a Project".
3. The estate `docs/runbooks/` directory. List it with `gh api repos/JorisJonkers-dev/estate/contents/docs/runbooks --jq '.[].path'` and read any file with a Rollback section.
4. The estate workflows `.github/workflows/rollback.yml`, `pause.yml` and `resume.yml`. Read rollback.yml in full on every run, because it is the file that runs and it changes.
5. `participants.yml`, to confirm the Project's name as the list spells it, and deploy-kit `CONTEXT.md` for the words Pause and Rollback.

## Procedure

1. Establish the facts. The pin is the `event.toolkit.fluxcd.io/version` annotation in `projects/<project>/source.yaml`. For a rollback, get from the owner the target release (X.Y.Z, as the Project's repository tagged it) and the reason. Also get from the owner whether the pinned release carries a Migration Proof. If it does, get the release the owner has confirmed the proof names and pass it as `--proof-names`; if it carries none, pass `--no-proof`. A rollback states exactly one of the two, and plan.py refuses otherwise. The target must be earlier than the pin, and where a proof exists it must be the release the owner confirmed.
2. Run `python3 <skill dir>/scripts/plan.py --project P --action pause|rollback|resume [--target V] [--reason R] [--proof-names V | --no-proof]`. The script reads the pin with one GET, applies the refusals, and prints the plan with the exact command. If it refuses, report the refusal and stop. Do not edit the command to get around it. Live use never passes `--pin`: it is a by-hand override for offline use and the worked examples, it skips the pin-file read so a recorded rollback is not checked, and the plan says so (`pin given by hand: ...`).
3. Explain the flow as the workflows run it, taken from rollback.yml and the README (see the disagreement section below for what the docs say against the files). A rollback refuses an unproven target before anything runs, records the pause and the target in one commit to the pin file, starts composition, and leaves the Project paused. A pause records who paused and why in one commit and moves no pin. A resume removes the record and starts composition, which moves the pin to the Project's newest fragment.
4. Hand the owner the command. The owner runs it from a shell where `gh` is signed in to GitHub with access to JorisJonkers-dev/estate. A rollback plan gives one command now and a resume command for later, to run only once the fix has landed.
5. Verify the value, not the exit code. After the owner runs it, rerun `plan.py --project P --action resume` for the same project. Its `state:` lines show whether the pin file now records the pause and the rolled-back version.

## Worked examples

### Worked example: target later than the pin (refused)

Project `grimoire`, pinned at 1.5.0. The owner asks for 1.6.0 because of a bad deploy.

```
python3 scripts/plan.py --project grimoire --action rollback --target 1.6.0 --no-proof --reason "bad deploy" --pin 1.5.0
REFUSED: target 1.6.0 is not earlier than the pin 1.5.0; a rollback goes back, never forward.
```

Exit status 1, and no command is printed. Tell the owner that a rollback needs an earlier release, and ask again.

### Worked example: target with no Migration Proof (refused)

Project `grimoire`, pinned at 1.5.0. The owner asks for 1.3.0 and has not said whether the pinned release carries a Migration Proof.

```
python3 scripts/plan.py --project grimoire --action rollback --target 1.3.0 --reason "bad deploy" --pin 1.5.0
REFUSED: Migration Proof not stated: pass --proof-names <release the proof names> or --no-proof when the pinned release carries none
```

Exit status 1, and no command is printed. Ask the owner whether a proof exists. If it does, rerun with `--proof-names` set to the release the owner has confirmed the proof names; if not, rerun with `--no-proof`.

Another refusal: with `--proof-names 1.4.2` (the release the owner confirmed the proof names) and the target 1.3.0, plan.py refuses with `target 1.3.0 is not the release the pinned release 1.5.0's Migration Proof names (1.4.2); only that one runs against the current schema.` Tell the owner that 1.4.2 is the only target the proof allows.

## Where the docs and reality disagree

Read live on 2026-11-10:

- Spec 55-delivery.md, Pause and Rollback: one commit moves the pin, the rollback takes no backup of its own, and the Project is left paused.
- rollback.yml: its header says the same. Its record step makes one commit and then dispatches composition. It has no backup job and no wait.
- estate README, "Holding a Project": says a rollback is two commits. The first pauses the Project and asks for a backup. Composition renders one-shot backup Jobs, and the run waits up to 45 minutes for `cluster-state.yml` to report them succeeded. Only then does the second commit record the release. The README also says the Project's database dump is not rendered yet (deploy-kit#423).
- Finding: the README is out of date against both the spec and the workflow. The 45-minute backup wait is not in rollback.yml. The skill follows the estate workflows, the files that run, and the agent re-reads rollback.yml each time. No workflow in the estate holds the backup wait the README describes.
- Finding: the README gives deploy-kit#423 as the reason a rollback has no backup. rollback.yml gives deploy-kit 0.31.0 and #557, the scheduled backups of each Durability Class. Neither citation was followed here.
- Finding: a Migration Proof names a revision (`testedAgainst`), not a release version. plan.py compares only the owner's `--proof-names` against the target. The owner must confirm that the release named is the one the proof means. rollback.yml does the real check, by walking the composition locks.
- Finding: the pin annotation sits on the Kustomization, while the pause and rollback annotations sit on the OCIRepository in the same file. plan.py reads both from that file.

## When a doc cannot be read

If `gh` is missing, signed out, or the repository is unreachable, stop. Name the doc and the command that failed. Do not fall back to remembered content, and do not guess the pin or the proof. plan.py stops the same way.
