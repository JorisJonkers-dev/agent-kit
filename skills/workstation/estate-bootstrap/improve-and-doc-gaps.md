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
