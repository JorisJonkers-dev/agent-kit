# Organization Repositories

source: README.md; manifest.yaml; runner-manifests/README.md; council/README.md
built-at: 2026-07-02T22:50:34Z

This fragment is tracked seed content. Treat it as seed-if-absent and preserve
local edits during regeneration.

## Current Repository

- `agent-kit`: versioned source for checked-in agent templates, generated
  Claude/Codex surfaces, council bundle, installer artifacts, and runner runtime
  manifests.
- Repository URL: `https://github.com/JorisJonkers-dev/agent-kit`.
- Organization profile: `https://github.com/JorisJonkers-dev`.

## Adjacent Artifacts

- `runtime-home`: OCI-published runtime-home package under
  `ghcr.io/jorisjonkers-dev/agent-kit/runtime-home`.
- `platform-blueprints`: named as the future owner for cluster realization
  details in runner-manifest handoff docs.
- `github/spec-kit`: upstream source for the Spec Kit seed, pinned in
  `manifest.yaml` and `spec-kit-source.lock`.

## Council Split Support

The council driver can extract a subtree into a destination repo with
`council.mjs split --path <path> --dest <owner>/<repo>`, preserving history through
`git subtree split` and using `gh` to create and push the remote when enabled.
