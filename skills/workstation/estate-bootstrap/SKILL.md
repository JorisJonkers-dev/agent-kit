---
name: estate-bootstrap
description: Use when starting a new JorisJonkers-dev repository from an estate template, or auditing an existing estate repository and the template it came from against the estate standards.
---

# Estate bootstrap

This skill holds the procedure for two jobs: starting a repository from an estate template, and auditing a repository against the estate's templates, shared workflows, Renovate preset, commons libraries and testing standards. It does not hold the standards. Every standard is read live with `gh` when the procedure needs it. `sources.yaml` lists each document this procedure points at, with its blob SHA, so drift can be seen later.

## Rules

- Read every document live with `gh`. Never answer from memory or copy a document's content into a repository, issue or report.
- If a document cannot be read, stop and follow "When a doc cannot be read".
- Never open a pull request against a template or library repository. A gap there becomes a drafted issue.
- File an issue only after the owner says yes in this conversation.
- A local clone at `~/Projects/JorisJonkers-dev/<repo>` may be used to verify a finding. Clones lag `main`, so when a clone and a live read disagree, the live read wins and the disagreement goes in the report.

## 1. Read first

Read these live, in order, with `gh api repos/OWNER/REPO/contents/PATH --jq .content | base64 -d`. The ADR file names are found by listing `docs/decisions` with `gh api repos/JorisJonkers-dev/workspace/contents/docs/decisions --jq '.[].name'`.

1. `JorisJonkers-dev/workspace` `CLAUDE.md`: the estate contract and its traps.
2. `workspace` `docs/testing-standards.md`.
3. `workspace` `docs/decisions/ADR-0018-*` (which template, CI shape).
4. `workspace` `docs/decisions/ADR-0021-*` (migrations) and `ADR-0022-*` (test shape), plus `ADR-0017-*` (when a shared package is extracted).
5. The README of `template-go`, `template-go-vue` and `repo-template`, and `repo-template` `docs/REPO_SETUP.md`.
6. `renovate-config` `README.md` (the preset reference string).
7. `github-workflows` `README.md`, and the latest release: `gh release view --repo JorisJonkers-dev/github-workflows --json tagName -q .tagName`. Resolve its commit SHA with `gh api repos/JorisJonkers-dev/github-workflows/git/ref/tags/TAG --jq .object.sha`, dereferencing if the object type is `tag`.
8. `deploy-kit` `README.md` (project files and the `validate` command) and `liquibase-runner` `README.md`.
9. The README of each commons library that the repository might use: `go-commons`, `python-commons`, `rust-commons`, `vue-web-commons`, `kotlin-spring-commons`.

## 2. Start a repository

1. Choose the template. A Go service or controller uses `template-go`. A contract-first Go API with a Vue SPA uses `template-go-vue`. Anything else uses `repo-template`.
2. Check the template live: `gh api repos/JorisJonkers-dev/TEMPLATE --jq .is_template`. When it prints `true`, create the repository with `gh repo create JorisJonkers-dev/NAME --private --template JorisJonkers-dev/TEMPLATE`. When it prints `false`, which `repo-template` does today, stop and ask the owner how to copy it. Its README says to use it as a template, but GitHub does not treat it as one.
3. Apply the branch ruleset so `Pipeline Complete` is the required check. `repo-template` ships `scripts/apply-ruleset.sh`. Run it, or copy it from `repo-template` if the chosen template lacks it, and confirm the result by reading the ruleset back.
4. Pin every shared workflow `uses:` to the commit SHA of the latest github-workflows release, with the tag in a trailing comment, as the templates do. A pin to `main` or to a tag name is a gap.
5. Add the Renovate preset with the exact string in the `renovate-config` README, in a root `renovate.json`.
6. Choose commons libraries from the stack, read each README, and follow `ADR-0017` for when a package is shared.
7. Follow `testing-standards` and `ADR-0022` for test names, layers, coverage and quarantine.
8. For a service with a database, follow `ADR-0021`. Do not copy goose code from a template without checking it against that ADR first, and ask the owner.
9. Replace the template placeholders. Set the `workspace-<group>` topic, which the `repo-template` REPO_SETUP document describes, because a repository without one lands in `inbox/` without any error. Rename `deploy/<name>.project.yml` and validate it with deploy-kit before merge.
10. Hand over. Once the repository deploys and its version reads back from the running system, run the `estate-project-intent` skill. That skill was not in this session's skill list, so confirm it exists before handing over.

## 3. Audit

Run the audit on the repository and on the template it came from. Each run needs a local directory, so use an existing clone under `~/Projects/JorisJonkers-dev/`, or clone it as the projects-layout skill says. Fetch the default branch before the run, or treat a clone's result as provisional.

```
estate audit <repo-dir> --latest-sha <release-sha>
```

The audit prints one line per gap, `<file>: <gap> (source: <doc>)`, or `no gaps`. It exits 2 for a missing directory. It checks:

- Pins to `main` or a non-SHA ref, and pins behind the latest release SHA. A trailing `# ...` comment is ignored. Scope: `.github/workflows/`, `.github/actions/` and `templates/**/*.tmpl` (`repo-template` carries its workflows as `.tmpl` files).
- The single `Pipeline Complete` job: only a job-level `name:` under the top-level `jobs:` key counts, in `.github/workflows/` only. `.tmpl` files are not checked for it.
- The Renovate preset: `extends` is read as a list, and an entry must equal the preset string from the `renovate-config` README, optionally followed by a `#ref`. Comments in `renovate.json` are tolerated. A file that still cannot be parsed is reported as "could not confirm the preset".
- Retired paths and names: a top-level `platform/`, `fleet.yaml`, `deploy-config-schema`, the old `packages-proxy-token` input, and `ExtraToast/`. `packages-proxy-token` is searched in every small text file outside `.git`, `node_modules`, `vendor`, `.venv`, `target` and `build`; the others only in `.github`, `deploy`, `Taskfile.yml` and `renovate.json`. Binary files and files over 1 MB are skipped.

The `platform/` check is meant for application repositories and templates. It is not meaningful for the `workspace` repository, which keeps its own `platform/` paths; ignore that line there.

Check these by hand, reading the live documents, and record each gap with its file path and the document that says otherwise:

- CI shape: one job with steps, plus `Pipeline Complete` as the only required check. A second job is allowed only when it is a reusable `uses:` job, gated from `Pipeline Complete` with `needs:` and a check that the result is `success`.
- Testing: the unit coverage gate from `testing-standards` section 6 and the coverage script; the mutation job on changed lines; the quarantine check; no `retry` setting; every skip carries a marker in the form the standards give, with a date inside the window the standards give.
- Deploy kit: a CI step runs deploy-kit `validate` on the project file before merge. None of the three templates runs it today.
- Migrations: a repository with a database changes its schema through liquibase-runner, as `ADR-0021` says. Look at `go.mod`, the build files and any `db/` directory for goose, Flyway or migrations embedded in the binary.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`improve-and-doc-gaps.md`](improve-and-doc-gaps.md): 4. Improve; 5. Where the docs and reality disagree; When a doc cannot be read
