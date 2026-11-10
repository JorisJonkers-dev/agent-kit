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
python3 <skill-dir>/scripts/audit.py <repo-dir> --latest-sha <release-sha>
```

The script prints one line per gap, `<file>: <gap> (source: <doc>)`, or `no gaps`. It exits 2 for a missing directory. It checks:

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

## 4. Improve

For a gap in the repository, fix it on a branch in a worktree under the projects-layout rule, and only when the owner asks. For a gap in a template or library, draft the issue in the report with a title, the file path, the live source that says otherwise and the command output. Look up labels with `gh label list --repo JorisJonkers-dev/REPO` before you pass any `--label`, because the estate uses prefixed labels such as `type:` and `component:`. Once the owner says yes, file it with `gh issue create`. Never open a pull request against a template or library repository.

## 5. Where the docs and reality disagree

Verified live on 2026-11-10. The local clones were older than the live repositories, so they were not used as evidence. Each bullet says which side this procedure follows and whether the audit script reports it.

- `repo-template` is not a GitHub template (`is_template` is false), though REPO_SETUP says to use it as one. Followed: the live repository setting, so step 2 of section 2 stops and asks. The audit reports it: no, the script is offline; the `gh api` check in section 2 finds it.
- Workflow pins across the org (found by partial code search) range from old tags to `main`, and the github-workflows README example is older than the latest release. Followed: the latest github-workflows release, resolved live in section 1. The audit reports it: yes, every pin that is `main`, not a 40-hex SHA, or behind `--latest-sha`.
- `template-go-vue` migrates with goose (`pressly/goose` in `go.mod`, `internal/platform/pg/store.go`), while `ADR-0021` says templates move to liquibase-runner. Followed: `ADR-0021`. The audit reports it: no; the Migrations check in section 3 is by hand.
- `template-go` and `template-go-vue` run several CI jobs, while `ADR-0018` asks for one job. `Pipeline Complete` is still the required check in both. Followed: `ADR-0018`. The audit reports it: only the count of `Pipeline Complete` jobs; the CI shape check in section 3 is by hand.
- `workspace` CLAUDE.md names recovery scripts, and only `scripts/ops/recover-wedged-apiserver-cache.sh` exists in `fleet-infra`; the others it names are absent. Its `platform/fleet-infra` path is correct. Followed: what exists in `fleet-infra`, read live. The audit reports it: no; that repository is not audited here.
- `workspace` CLAUDE.md deploy-config-schema trap: `deploy-config-schema` is archived, its README still shows `validate deployment`, and the current validator is deploy-kit. Followed: deploy-kit. The audit reports it: yes, any use of `deploy-config-schema` in the scoped files.
- Coverage: REPO_SETUP and `testing-standards` section 6 with `ADR-0022` state different unit coverage gates, and the template coverage scripts count uncovered functions and report branches and conditions. Followed: `testing-standards` section 6 and `ADR-0022`, not REPO_SETUP. The audit reports it: no; the Testing check in section 3 is by hand.
- `go-commons`, `template-go` and `template-go-vue` are placed under `inbox/` in `workspace/.gitmodules`, which means they have no group topic. Followed: REPO_SETUP, which requires a group topic. The audit reports it: no.
- `fleet.yaml` is flagged by the audit as retired, but no live document says so. Deploy-kit's examples use `<name>.project.yml`. Followed: no document; treat it as unverified. The audit reports it: yes, with the source `none (unverified)`.

## When a doc cannot be read

If `gh` is missing, `gh auth status` shows signed out, or a repository is unreachable, stop. Name the document (repository and path) and say what it was needed for. Do not fall back to remembered content, to an older clone's copy, or to a guess. A clone may confirm a live finding, but it is never the source.
