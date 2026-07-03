import { describe, expect, it } from 'vitest'

import {
  parseAttachmentProfilesConfig,
  type AttachmentProfilesConfig,
  type SkillCardRisk,
} from './index.js'

const MCP_PROFILES = ['minimal', 'frontend', 'cluster', 'code-intel', 'full-diagnostic'] as const

const validCard = {
  name: 'kb-recall',
  purpose: 'Recall focused knowledge before changing durable behavior.',
  positiveTriggers: ['architecture decision', 'repo convention'],
  negativeTriggers: ['single-file typo', 'mechanical formatting'],
  requiredMcpProfile: 'minimal',
  risk: 'low',
  expectedOutputs: ['short recall summary', 'relevant source note identifiers'],
} satisfies AttachmentProfilesConfig['profiles'][number]['skillCards'][number]

const validConfig = {
  active: {
    profiles: ['implementation'],
    skillCards: ['kb-recall'],
    fullSkills: ['kb-first'],
  },
  profiles: [
    {
      name: 'implementation',
      mcpProfile: 'minimal',
      skillCards: [validCard],
      fullSkills: ['kb-first', 'run-tests'],
    },
  ],
} satisfies AttachmentProfilesConfig

describe('parseAttachmentProfilesConfig', () => {
  it('normalizes declarative attachment profiles without filesystem or YAML IO', () => {
    const catalog = parseAttachmentProfilesConfig(
      { attachmentProfiles: validConfig },
      { knownMcpProfiles: MCP_PROFILES, maxActiveProfiles: 2, maxActiveSkillCards: 2, maxActiveFullSkills: 2 },
    )

    expect(catalog.active).toEqual({
      profiles: ['implementation'],
      skillCards: ['kb-recall'],
      fullSkills: ['kb-first'],
    })
    expect(catalog.profiles).toEqual([
      {
        name: 'implementation',
        mcpProfile: 'minimal',
        skillCards: [
          {
            ...validCard,
            risk: 'low' satisfies SkillCardRisk,
          },
        ],
        fullSkills: ['kb-first', 'run-tests'],
      },
    ])
    expect(catalog.profileByName.get('implementation')?.mcpProfile).toBe('minimal')
    expect(catalog.skillCardByName.get('kb-recall')?.requiredMcpProfile).toBe('minimal')
  })

  it('also accepts the attachmentProfiles object directly for pure domain callers', () => {
    expect(
      parseAttachmentProfilesConfig(validConfig, { knownMcpProfiles: new Set(MCP_PROFILES) }).profiles,
    ).toHaveLength(1)
  })

  it('rejects duplicate profile names and duplicate skill card names', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            { ...validConfig.profiles[0], name: 'implementation' },
            { ...validConfig.profiles[0], name: 'implementation', skillCards: [] },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[1].name duplicates profile "implementation"')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            validConfig.profiles[0],
            {
              name: 'review',
              mcpProfile: 'minimal',
              skillCards: [{ ...validCard, requiredMcpProfile: 'code-intel' }],
              fullSkills: [],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[1].skillCards[0].name duplicates skill card "kb-recall"')
  })

  it('rejects unknown MCP profile references from profiles and cards', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [{ ...validConfig.profiles[0], mcpProfile: 'unknown' }],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].mcpProfile references unknown MCP profile "unknown"')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, requiredMcpProfile: 'unknown' }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow(
      'attachmentProfiles.profiles[0].skillCards[0].requiredMcpProfile references unknown MCP profile "unknown"',
    )
  })

  it('rejects empty trigger arrays and blank trigger entries', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, positiveTriggers: [] }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards[0].positiveTriggers must be a non-empty string array')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, negativeTriggers: [' '] }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow(
      'attachmentProfiles.profiles[0].skillCards[0].negativeTriggers[0] must be a non-empty string',
    )
  })

  it('rejects invalid risk and expected output shape', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, risk: 'critical' }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards[0].risk must be "low", "medium", or "high"')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, expectedOutputs: [] }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards[0].expectedOutputs must be a non-empty string array')
  })

  it('rejects oversized configured active sets', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          ...validConfig,
          active: { profiles: ['implementation'] },
        },
        { knownMcpProfiles: MCP_PROFILES, maxActiveProfiles: 0 },
      ),
    ).toThrow('attachmentProfiles.active.profiles must contain at most 0 item(s)')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          ...validConfig,
          active: { skillCards: ['kb-recall'] },
        },
        { knownMcpProfiles: MCP_PROFILES, maxActiveSkillCards: 0 },
      ),
    ).toThrow('attachmentProfiles.active.skillCards must contain at most 0 item(s)')

    expect(() =>
      parseAttachmentProfilesConfig(
        {
          ...validConfig,
          active: { fullSkills: ['kb-first'] },
        },
        { knownMcpProfiles: MCP_PROFILES, maxActiveFullSkills: 0 },
      ),
    ).toThrow('attachmentProfiles.active.fullSkills must contain at most 0 item(s)')
  })

  it('rejects malformed profile and skill card fields', () => {
    expect(() => parseAttachmentProfilesConfig(undefined, { knownMcpProfiles: MCP_PROFILES })).toThrow(
      'attachmentProfiles must be an object',
    )
    expect(() =>
      parseAttachmentProfilesConfig({ profiles: [] }, { knownMcpProfiles: MCP_PROFILES }),
    ).toThrow('attachmentProfiles.profiles must be a non-empty array')
    expect(() =>
      parseAttachmentProfilesConfig(
        { profiles: [{ ...validConfig.profiles[0], name: '' }] },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].name must be a non-empty string')
    expect(() =>
      parseAttachmentProfilesConfig(
        { profiles: [{ ...validConfig.profiles[0], skillCards: 'kb-recall' }] },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards must be an array')
    expect(() =>
      parseAttachmentProfilesConfig(
        { profiles: [{ ...validConfig.profiles[0], skillCards: [null] }] },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards[0] must be an object')
    expect(() =>
      parseAttachmentProfilesConfig(
        {
          profiles: [
            {
              ...validConfig.profiles[0],
              skillCards: [{ ...validCard, purpose: 'One line.\nSecond line.' }],
            },
          ],
        },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].skillCards[0].purpose must be one line')
    expect(() =>
      parseAttachmentProfilesConfig(
        { profiles: [{ ...validConfig.profiles[0], fullSkills: [''] }] },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.profiles[0].fullSkills[0] must be a non-empty string')
  })

  it('validates active-set references after parsing profiles and cards', () => {
    expect(() =>
      parseAttachmentProfilesConfig(
        { ...validConfig, active: { profiles: ['missing'] } },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.active.profiles[0] references unknown profile "missing"')

    expect(() =>
      parseAttachmentProfilesConfig(
        { ...validConfig, active: { skillCards: ['missing-card'] } },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.active.skillCards[0] references unknown skill card "missing-card"')

    expect(() =>
      parseAttachmentProfilesConfig(
        { ...validConfig, active: { fullSkills: ['missing-skill'] } },
        { knownMcpProfiles: MCP_PROFILES },
      ),
    ).toThrow('attachmentProfiles.active.fullSkills[0] references unknown full skill "missing-skill"')
  })
})
