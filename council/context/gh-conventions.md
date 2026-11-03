# GitHub Conventions

source: README.md; docs/PORTABILITY.md; manifest.yaml; runner/manifests/README.md
built-at: 2026-07-02T22:50:34Z

Edit this fragment under `council/context/`. `scripts/render_agent_kit.py --write`
copies it to `.council/context/` and overwrites any edit made there.

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
