# Third-Party Notices

This directory vendors or adapts material from the following sources. All are
MIT-licensed except where an entry says otherwise.

Sources listed under "Registry skill sources" are not vendored into this
repository. They are pinned in `registry/estate-tooling.yaml` and cloned at
runtime by the Hermes gateway's `sync-skills` init container, which verifies
each pinned commit and refuses the source on a mismatch. The notice is recorded
here because the material executes inside agent context.

- Project: Spec Kit
- Repository: https://github.com/github/spec-kit
- Pinned tag: v0.9.5
- License: MIT
- Copyright notice: Copyright (c) GitHub and contributors

- Project: BMAD-METHOD
- Repository: https://github.com/bmad-code-org/BMAD-METHOD.git
- Pinned tag: v6.9.0
- Resolved source commit: 67f4499e
- License: MIT
- Copyright notice: Copyright (c) 2025 BMad Code, LLC
- Trademark caveat: BMad, BMad Method, BMAD-METHOD, and related names/marks
  are trademarks of BMad Code, LLC and are not licensed under MIT.

- Project: spec-compare
- Repository: https://github.com/cameronsjo/spec-compare
- Source ref: main
- License: MIT
- Copyright notice: upstream repository authors and contributors
- Extracted paths: `docs/use-case-scoring.md`, `docs/beads.md`,
  `cheatsheet-beads-openspec.md`

- Project: Skills
- Repository: https://github.com/mattpocock/skills
- Pinned tag: v1.1.0
- Resolved source commit: d574778f94cf620fcc8ce741584093bc650a61d3
- Installed skills: grill-me (via `skills` CLI, pinned; was installed by the
  now-retired install.sh -- kept here as a provenance record, agent-kit#40)
- License: MIT
- Copyright notice: Copyright (c) 2026 Matt Pocock

- Project: Superpowers
- Repository: https://github.com/obra/superpowers
- Source ref: main
- License: MIT
- Copyright notice: Copyright (c) 2025 Jesse Vincent

## Registry skill sources

- Project: Skills
- Repository: https://github.com/mattpocock/skills
- Pinned tag: v1.2.3
- Resolved source commit: 6acc160e4e0cd062dbbbd7a1b26ae92855edf07e
- Selector: `engineering/*,productivity/*,misc/*`
- License: MIT
- Copyright notice: Copyright (c) 2026 Matt Pocock

- Project: Superpowers
- Repository: https://github.com/obra/superpowers
- Pinned tag: v6.3.0
- Resolved source commit: b36e0829c6d0140e93cfef2ca599b1b07d4a7797
- License: MIT
- Copyright notice: Copyright (c) 2025 Jesse Vincent

- Project: Caveman
- Repository: https://github.com/JuliusBrussee/caveman
- Pinned tag: v2.6.0
- Resolved source commit: b82c0ad42c2bedc1f2cd78e414dadfaffbaaeec3
- License: MIT

- Project: kubernetes-skill
- Repository: https://github.com/LukasNiessen/kubernetes-skill
- Source ref: main
- Resolved source commit: f85547fb3a1ec909b2cbe4dc68f831590ac385ea
- License: MIT

- Project: drawio-mcp
- Repository: https://github.com/jgraph/drawio-mcp
- Source ref: main
- Resolved source commit: 14b318b19cc37b159f841227b9d11fbd18ce18ea
- Selector: `plugins/claude-code/skills/drawio`
- License: Apache-2.0 (NOT MIT; see the upstream LICENSE)

- Project: olcli
- Repository: https://github.com/aloth/olcli
- Source ref: main
- Resolved source commit: 0f8085d90ea69eb0f3886d0ceba50bc9953a9c9e
- License: MIT
- Note: also installed as a CLI from npm as `@aloth/olcli`.

## MIT License

Copyright (c) the upstream authors and contributors listed above

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
