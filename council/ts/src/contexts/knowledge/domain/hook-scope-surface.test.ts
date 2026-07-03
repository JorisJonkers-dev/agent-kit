import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { describe, expect, it } from 'vitest'

const repoRoot = resolve(import.meta.dirname, '../../../../../..')
const hookSurfacePaths = [
  'templates/installer/partials/hooks/pre-tool-use-edit-recall.sh',
  'templates/installer/partials/hooks/pre-tool-use-git-commit-capture.sh',
  'templates/installer/partials/hooks/stop-session-digest-claude.sh',
  'templates/installer/partials/hooks/stop-session-digest-codex.sh',
  'installer/install.sh',
  ...shellFiles('templates/repo/.claude/hooks'),
  ...shellFiles('templates/repo/.codex/hooks'),
  ...shellFiles('.claude/hooks'),
  ...shellFiles('.codex/hooks'),
]

function shellFiles(relativeDir: string): readonly string[] {
  return readdirSync(resolve(repoRoot, relativeDir), { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith('.sh'))
    .map((entry) => `${relativeDir}/${entry.name}`)
    .sort()
}

function readSurface(relativePath: string): string {
  return readFileSync(resolve(repoRoot, relativePath), 'utf8')
}

describe('hook scope surfaces', () => {
  it('derive repo scope only from canonical GitHub owner/repo remotes', () => {
    const canonicalScopeHelper = 'canonical_project_scope_from_origin()'

    for (const relativePath of hookSurfacePaths) {
      const content = readSurface(relativePath)

      expect(content, relativePath).not.toContain('project:${project}')
      expect(content, relativePath).not.toContain('project:<repo-name>')
      expect(content, relativePath).not.toContain('project:<repo-basename>')
      expect(content, relativePath).not.toContain("s#.*[/:]##")

      if (content.includes('knowledge.recall') || content.includes('knowledge.capture_')) {
        expect(content, relativePath).toContain(canonicalScopeHelper)
      }
    }
  })
})
