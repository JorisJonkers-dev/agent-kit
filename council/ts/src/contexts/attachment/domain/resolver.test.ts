import { describe, expect, it } from 'vitest'

import { parseAttachmentProfilesConfig, resolveAttachments } from './index.js'
import type { AttachmentProfilesConfig } from './index.js'
import type { TaskResolvedAttachment } from '../../../shared-kernel/index.js'

const MCP_PROFILES = ['minimal', 'frontend', 'cluster', 'code-intel', 'full-diagnostic'] as const

function card(
  name: string,
  requiredMcpProfile: (typeof MCP_PROFILES)[number],
  positiveTriggers: readonly string[],
  negativeTriggers: readonly string[] = ['do not use'],
): AttachmentProfilesConfig['profiles'][number]['skillCards'][number] {
  return {
    expectedOutputs: [`${name} output`],
    name,
    negativeTriggers,
    positiveTriggers,
    purpose: `${name} purpose.`,
    requiredMcpProfile,
    risk: 'low',
  }
}

const catalog = parseAttachmentProfilesConfig(
  {
    profiles: [
      {
        fullSkills: ['kb-first', 'run-tests'],
        mcpProfile: 'minimal',
        name: 'minimal',
        skillCards: [
          card('kb-first', 'minimal', ['repo convention', 'architecture decision'], ['single-file typo']),
          card('task-scope', 'minimal', ['bounded task', 'acceptance criteria']),
        ],
      },
      {
        fullSkills: ['frontend-build'],
        mcpProfile: 'frontend',
        name: 'frontend',
        skillCards: [
          card('design-review', 'frontend', ['ui', 'component', 'app-ui'], ['backend only']),
          card('accessibility-pass', 'frontend', ['a11y', 'keyboard navigation']),
        ],
      },
      {
        fullSkills: ['fleet-change'],
        mcpProfile: 'cluster',
        name: 'cluster',
        skillCards: [
          card('fleet-change', 'cluster', ['fleet.yaml', 'kubernetes', 'traefik'], ['documentation only']),
          card('service-registry', 'cluster', ['public service', 'sso protected']),
        ],
      },
      {
        fullSkills: ['code-intel'],
        mcpProfile: 'code-intel',
        name: 'code-intel',
        skillCards: [
          card('dependency-map', 'code-intel', ['dependency-cruiser', 'cross-context', 'barrel']),
          card('type-surface', 'code-intel', ['strict typescript', 'typecheck']),
        ],
      },
      {
        fullSkills: ['diagnostics'],
        mcpProfile: 'full-diagnostic',
        name: 'full-diagnostic',
        skillCards: [
          card('ci-diagnostic', 'full-diagnostic', ['failing ci', 'test logs', 'diagnostic']),
          card('runtime-trace', 'full-diagnostic', ['runtime failure', 'trace']),
        ],
      },
    ],
  },
  { knownMcpProfiles: MCP_PROFILES },
)

describe('resolveAttachments', () => {
  it('returns deterministic frontend attachments for identical task input', () => {
    const input = {
      paths: ['services/app-ui/src/components/NavTabs.vue'],
      signals: ['keyboard navigation', 'component polish'],
      taskKind: 'ui-tweak',
    }

    const first = resolveAttachments(input, catalog)
    const second = resolveAttachments(input, catalog)
    const taskAttachment: TaskResolvedAttachment = first

    expect(first).toEqual(second)
    expect(first.mcpProfile).toBe('frontend')
    expect(taskAttachment).toMatchObject({
      activeSkills: ['design-review', 'accessibility-pass', 'frontend-build'],
      mcpProfile: 'frontend',
    })
    expect(first.activeSkills).toEqual(['design-review', 'accessibility-pass', 'frontend-build'])
    expect(first.selectedSkillCards.map((skill) => skill.name)).toEqual(['design-review', 'accessibility-pass'])
    expect(first.contextProfile.profile).toBe('implementer')
    expect(first.lensIds.length).toBeGreaterThan(0)
  })

  it('selects cluster attachments from fleet and ingress paths', () => {
    const resolved = resolveAttachments(
      {
        paths: ['platform/inventory/fleet.yaml', 'platform/rendered/traefik/apps.yaml'],
        signals: ['public service needs SSO protected routing'],
        taskKind: 'maintenance',
      },
      catalog,
    )

    expect(resolved.mcpProfile).toBe('cluster')
    expect(resolved.activeSkills).toEqual(['fleet-change', 'service-registry'])
    expect(resolved.selectedProfiles).toEqual(['cluster'])
  })

  it('escalates to code-intel only when selected skill cards require it', () => {
    const resolved = resolveAttachments(
      {
        paths: ['council/ts/src/contexts/attachment/domain/resolver.ts'],
        signals: ['strict TypeScript', 'cross-context barrel dependency-cruiser'],
        taskKind: 'refactor',
      },
      catalog,
    )

    expect(resolved.mcpProfile).toBe('code-intel')
    expect(resolved.activeSkills).toEqual(['dependency-map', 'type-surface', 'code-intel'])
    expect(resolved.activeSkills).not.toContain('ci-diagnostic')
  })

  it('uses the full-diagnostic profile for diagnostic signals and logs', () => {
    const resolved = resolveAttachments(
      {
        paths: ['.github/workflows/ci.yml'],
        signals: ['failing CI', 'test logs', 'runtime failure trace'],
        taskKind: 'bugfix',
      },
      catalog,
    )

    expect(resolved.mcpProfile).toBe('full-diagnostic')
    expect(resolved.activeSkills).toEqual(['ci-diagnostic', 'runtime-trace', 'diagnostics'])
    expect(resolved.selectedProfiles).toEqual(['full-diagnostic'])
  })

  it('keeps minimal attachments for low-fit tasks and rejects unrelated skills', () => {
    const resolved = resolveAttachments(
      {
        contextProfile: 'minimal',
        paths: ['README.md'],
        signals: ['single-file typo'],
        taskKind: 'maintenance',
      },
      catalog,
    )

    expect(resolved.mcpProfile).toBe('minimal')
    expect(resolved.activeSkills).toEqual(['kb-first', 'run-tests'])
    expect(resolved.selectedSkillCards).toEqual([])
    expect(resolved.rejectedSkillCards).toContainEqual({
      name: 'kb-first',
      reason: 'negative-trigger',
      score: -28,
    })
  })

  it('lets conflicting positive and negative triggers suppress high-profile escalation', () => {
    const resolved = resolveAttachments(
      {
        paths: ['platform/inventory/fleet.yaml'],
        signals: ['kubernetes documentation only'],
        taskKind: 'maintenance',
      },
      catalog,
    )

    expect(resolved.mcpProfile).toBe('minimal')
    expect(resolved.activeSkills).toEqual(['kb-first', 'run-tests'])
    expect(resolved.rejectedSkillCards).toContainEqual({
      name: 'fleet-change',
      reason: 'negative-trigger',
      score: 14,
    })
  })

  it('caps the active skill set at twelve items after deterministic ordering', () => {
    const manySkillsCatalog = parseAttachmentProfilesConfig(
      {
        profiles: [
          {
            fullSkills: ['z-full', 'a-full'],
            mcpProfile: 'code-intel',
            name: 'wide',
            skillCards: Array.from({ length: 14 }, (_, index) =>
              card(`skill-${String(index).padStart(2, '0')}`, 'code-intel', ['shared trigger']),
            ),
          },
        ],
      },
      { knownMcpProfiles: MCP_PROFILES },
    )

    const resolved = resolveAttachments(
      { signals: ['shared trigger'], taskKind: 'refactor' },
      manySkillsCatalog,
    )

    expect(resolved.mcpProfile).toBe('code-intel')
    expect(resolved.activeSkills).toHaveLength(12)
    expect(resolved.activeSkills).toEqual([
      'skill-00',
      'skill-01',
      'skill-02',
      'skill-03',
      'skill-04',
      'skill-05',
      'skill-06',
      'skill-07',
      'skill-08',
      'skill-09',
      'skill-10',
      'skill-11',
    ])
  })

  it('breaks equal skill and profile scores by stable names', () => {
    const tieCatalog = parseAttachmentProfilesConfig(
      {
        profiles: [
          {
            fullSkills: [],
            mcpProfile: 'frontend',
            name: 'zeta',
            skillCards: [card('z-card', 'frontend', ['same trigger'])],
          },
          {
            fullSkills: [],
            mcpProfile: 'frontend',
            name: 'alpha',
            skillCards: [card('a-card', 'frontend', ['same trigger'])],
          },
        ],
      },
      { knownMcpProfiles: MCP_PROFILES },
    )

    const resolved = resolveAttachments({ signals: ['same trigger'], taskKind: 'feature' }, tieCatalog)

    expect(resolved.selectedProfiles).toEqual(['alpha'])
    expect(resolved.activeSkills).toEqual(['a-card', 'z-card'])
  })
})
