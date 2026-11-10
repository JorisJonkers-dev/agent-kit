---
name: estate-project-intent
description: Use when writing or changing a Project's Intent (deploy/<project>.project.yml and its env files) for deploy-kit 1.x and getting it onto the estate delivery path.
---

# Estate Project Intent

This skill holds the procedure only. The rules live in deploy-kit's spec and in the estate repository. Read them live with `gh` every time, and never copy their content into your answer or your files from memory.

## Read first (live)

Read each file with `gh api repos/<owner>/<repo>/contents/<path> -H 'Accept: application/vnd.github.raw'`. List a directory with `--jq '.[].path'` on the same URL.

1. `JorisJonkers-dev/deploy-kit`: `spec/v1/10-project-intent.md`, `spec/v1/16-dependencies.md` (sections "Dependency edges" and "The database catalog"), `spec/v1/17-sign-in.md` (only when the Application signs people in), `spec/v1/60-setup.md` (sections "Onboarding a new Application" and "The Estate repository"), `CONTEXT.md`, and the directory `spec/v1/examples/minimal/` with its `notes.project.yml`, `env/notes-api/base.env` and `README.md`.
2. `JorisJonkers-dev/estate`: `README.md` (section "Reading a delivery"), `participants.yml`, and the runbook in `docs/runbooks/` that matches the change (list the directory first).
3. `JorisJonkers-dev/workspace`: `CLAUDE.md`, for the estate conventions such as PR labels. Run `gh label list --repo <owner>/<repo>` before passing `--label`.
4. For the workflow, `JorisJonkers-dev/n8n`: `.github/workflows/publish.yml`, a working caller.

Local clones under `~/Projects/JorisJonkers-dev/<repo>` may be read to check what you wrote. They are not the instruction source.

## Procedure

1. Write the files. Run `python3 <skill-dir>/scripts/intent.py scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE --image IMAGE --process PROCESS --port N --out <repo-root>`. It writes `deploy/<project>.project.yml` and `deploy/env/<process>/base.env`, and it refuses to overwrite. Resolve every line marked CHECK against the live spec. Add one `base.env` per Process. Use the shared `_project` or `_applications` scopes only for values every Process in that scope receives.
2. Declare what the spec requires for this Application: the exposure with its host and audience (ask the owner when the audience is not clear; never guess it for a public host); the `client` block only when the Application signs people in; `secrets[]` with each entry's path, keys, access, delivery and rotation, kept on the Application and not raised to the project header; `${secret:<granted-path>#<key>}` in env files, where the path matches a granted path exactly; `placement` with memory and cpu on every Process; probes; and durability for any volume. When the Application uses Postgres, add a `dependsOn` edge to `platform-postgres` on surface `postgres`, and take the database credential names and dependency placeholders from `16-dependencies.md` rather than inventing them.
3. Validate. Run `python3 <skill-dir>/scripts/intent.py validate deploy/<project>.project.yml deploy/env`, from a shell where the repository's pinned deploy-kit is on PATH. Exit 0 means accepted, 1 means refused, 2 means called wrongly or deploy-kit is not installed. Fix refusals and run again until exit 0.
4. Wire publishing. In the release workflow, after the image build, call `JorisJonkers-dev/github-workflows/.github/workflows/publish-fragment.yml` once per project file, as the n8n `publish.yml` does. Find the latest release live: `gh release view --repo JorisJonkers-dev/github-workflows --json tagName -q .tagName`. Find its commit: `gh api repos/JorisJonkers-dev/github-workflows/git/ref/tags/<TAG> --jq .object`. If the object type is `tag`, dereference it with `git/tags/<sha>`. Pin `uses:` to the commit SHA with the tag in a trailing comment. Read the workflow's inputs at that SHA with `gh api "repos/JorisJonkers-dev/github-workflows/contents/.github/workflows/publish-fragment.yml?ref=<sha>"`, and set them as the caller does.
5. Propose the estate registration. Read `participants.yml` live and copy the shape of its neighbouring entries (`dormant`, `owner`, `reason`). Leave the value of `dormant` to the owner. `maxAge` defaults to 7 days when omitted. Give the owner the snippet as text, with the project name as its key. Never push to `JorisJonkers-dev/estate`; the owner makes that change.
6. Commit or push the application repository only when the owner asks. Then the owner watches the estate's delivery run: the job summary in the estate's Actions tab, as its README describes.

## Where the docs and reality disagree

(a) Location. Spec chapter 10 puts Intent under `platform/`. The live application repositories checked on 2026-11-10 (n8n, auth-api, grimoire, estate-dashboard, home-portal) keep it under `deploy/`. Follow `deploy/`. The estate repository keeps `platform/` for the platform's own Projects, and that is not a reason to move an application's files.

(b) Toolkit pin. Spec chapter 60 says a repository pins no toolkit and the estate's `toolkit.yml` is the one pin. Live, n8n pins `@jorisjonkers-dev/deploy-kit` 1.0.0 in `tools/deploy-kit/package.json` and validates with it in its CI. The `publish-fragment.yml` at github-workflows v0.45.4 (commit 2814b76) reads its own `tools/deploy-kit` lockfile, which pins 1.10.0, and the estate's `toolkit.yml` also names 1.10.0. Follow the live repository: leave its pin as it is, and tell the owner both numbers, the repository's pin (1.0.0 for n8n) and the version publish runs (1.10.0). Do not bump the pin in this change.

## When a doc cannot be read

If `gh` is missing, `gh auth status` shows you signed out, or the repository is unreachable, STOP. Name the doc you could not read, by repository and path, and ask the owner. Never fall back to remembered content, and never write the Intent from memory of the spec.

## Reporting validation

Report the output of `intent.py validate` to the owner verbatim, with its exit code. Paste the deploy-kit diagnostics as they were printed. Do not summarize, reword or drop refusals.
