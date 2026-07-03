# GitHub Conventions

source: README.md; PORTABILITY.md; manifest.yaml; runner-manifests/README.md
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

## Repository Identity

- Organization: `JorisJonkers-dev`.
- Primary repository: `JorisJonkers-dev/agent-kit`.
- Artifact coordinate: `github:JorisJonkers-dev/agent-kit`.
- Durable source is a pinned `agent-kit` release tag, not generated client
  homes.

## Publishing And Access

- Runtime-home artifact repository:
  `ghcr.io/jorisjonkers-dev/agent-kit/runtime-home`.
- Release tag pattern: `v{version}`.
- Installer artifacts are served by a KB service and installed with bearer-token
  authentication.
- The GitHub App must be installed on a destination owner before council split
  pushes can authenticate.

## Authorship

- Do not add generated-by footers, assistant names, model names, or
  `Co-Authored-By` trailers.
- Keep PRs small, scoped, reviewable, and revertable.
