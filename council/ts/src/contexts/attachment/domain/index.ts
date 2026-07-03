export const SKILL_CARD_RISKS = ['low', 'medium', 'high'] as const

export * from './resolver.js'

export type SkillCardRisk = (typeof SKILL_CARD_RISKS)[number]

export interface SkillCardConfig {
  readonly name: string
  readonly purpose: string
  readonly positiveTriggers: readonly string[]
  readonly negativeTriggers: readonly string[]
  readonly requiredMcpProfile: string
  readonly risk: string
  readonly expectedOutputs: readonly string[]
}

export interface AttachmentProfileConfig {
  readonly name: string
  readonly mcpProfile: string
  readonly skillCards: readonly SkillCardConfig[]
  readonly fullSkills: readonly string[]
}

export interface AttachmentProfilesActiveConfig {
  readonly profiles?: readonly string[]
  readonly skillCards?: readonly string[]
  readonly fullSkills?: readonly string[]
}

export interface AttachmentProfilesConfig {
  readonly active?: AttachmentProfilesActiveConfig
  readonly profiles: readonly AttachmentProfileConfig[]
}

export interface SkillCard {
  readonly name: string
  readonly purpose: string
  readonly positiveTriggers: readonly string[]
  readonly negativeTriggers: readonly string[]
  readonly requiredMcpProfile: string
  readonly risk: SkillCardRisk
  readonly expectedOutputs: readonly string[]
}

export interface AttachmentProfile {
  readonly name: string
  readonly mcpProfile: string
  readonly skillCards: readonly SkillCard[]
  readonly fullSkills: readonly string[]
}

export interface AttachmentProfilesActiveSet {
  readonly profiles: readonly string[]
  readonly skillCards: readonly string[]
  readonly fullSkills: readonly string[]
}

export interface AttachmentProfileCatalog {
  readonly active: AttachmentProfilesActiveSet
  readonly profiles: readonly AttachmentProfile[]
  readonly profileByName: ReadonlyMap<string, AttachmentProfile>
  readonly skillCardByName: ReadonlyMap<string, SkillCard>
}

export interface ParseAttachmentProfilesConfigOptions {
  readonly knownMcpProfiles: ReadonlySet<string> | readonly string[]
  readonly maxActiveProfiles?: number
  readonly maxActiveSkillCards?: number
  readonly maxActiveFullSkills?: number
}

export function parseAttachmentProfilesConfig(
  config: unknown,
  options: ParseAttachmentProfilesConfigOptions,
): AttachmentProfileCatalog {
  const root = readAttachmentProfilesRoot(config)
  const record = expectRecord(root, 'attachmentProfiles')
  const knownMcpProfiles = new Set(options.knownMcpProfiles)
  const profileByName = new Map<string, AttachmentProfile>()
  const skillCardByName = new Map<string, SkillCard>()
  const profiles = readProfiles(record.profiles, knownMcpProfiles, profileByName, skillCardByName)
  const fullSkills = new Set(profiles.flatMap((profile) => profile.fullSkills))
  const active = readActiveSet(
    record.active,
    options,
    profileByName,
    skillCardByName,
    fullSkills,
  )

  return { active, profiles, profileByName, skillCardByName }
}

function readAttachmentProfilesRoot(config: unknown): unknown {
  if (config === undefined) {
    return config
  }
  if (isRecord(config) && 'attachmentProfiles' in config) {
    return config.attachmentProfiles
  }
  return config
}

function readProfiles(
  value: unknown,
  knownMcpProfiles: ReadonlySet<string>,
  profileByName: Map<string, AttachmentProfile>,
  skillCardByName: Map<string, SkillCard>,
): readonly AttachmentProfile[] {
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error('attachmentProfiles.profiles must be a non-empty array')
  }

  return value.map((profileValue, index) =>
    readProfile(
      profileValue,
      `attachmentProfiles.profiles[${String(index)}]`,
      knownMcpProfiles,
      profileByName,
      skillCardByName,
    ),
  )
}

function readProfile(
  value: unknown,
  path: string,
  knownMcpProfiles: ReadonlySet<string>,
  profileByName: Map<string, AttachmentProfile>,
  skillCardByName: Map<string, SkillCard>,
): AttachmentProfile {
  const record = expectRecord(value, path)
  const name = readRequiredString(record.name, `${path}.name`)
  if (profileByName.has(name)) {
    throw new Error(`${path}.name duplicates profile ${JSON.stringify(name)}`)
  }
  const mcpProfile = readMcpProfile(record.mcpProfile, `${path}.mcpProfile`, knownMcpProfiles)
  const skillCards = readSkillCards(
    record.skillCards,
    `${path}.skillCards`,
    knownMcpProfiles,
    skillCardByName,
  )
  const fullSkills = readStringArray(record.fullSkills, `${path}.fullSkills`, false)
  const profile = { name, mcpProfile, skillCards, fullSkills }
  profileByName.set(name, profile)
  return profile
}

function readSkillCards(
  value: unknown,
  path: string,
  knownMcpProfiles: ReadonlySet<string>,
  skillCardByName: Map<string, SkillCard>,
): readonly SkillCard[] {
  if (!Array.isArray(value)) {
    throw new Error(`${path} must be an array`)
  }

  return value.map((cardValue, index) =>
    readSkillCard(
      cardValue,
      `${path}[${String(index)}]`,
      knownMcpProfiles,
      skillCardByName,
    ),
  )
}

function readSkillCard(
  value: unknown,
  path: string,
  knownMcpProfiles: ReadonlySet<string>,
  skillCardByName: Map<string, SkillCard>,
): SkillCard {
  const record = expectRecord(value, path)
  const name = readRequiredString(record.name, `${path}.name`)
  if (skillCardByName.has(name)) {
    throw new Error(`${path}.name duplicates skill card ${JSON.stringify(name)}`)
  }
  const purpose = readPurpose(record.purpose, `${path}.purpose`)
  const positiveTriggers = readStringArray(record.positiveTriggers, `${path}.positiveTriggers`, true)
  const negativeTriggers = readStringArray(record.negativeTriggers, `${path}.negativeTriggers`, true)
  const requiredMcpProfile = readMcpProfile(
    record.requiredMcpProfile,
    `${path}.requiredMcpProfile`,
    knownMcpProfiles,
  )
  const risk = readRisk(record.risk, `${path}.risk`)
  const expectedOutputs = readStringArray(record.expectedOutputs, `${path}.expectedOutputs`, true)
  const card = {
    name,
    purpose,
    positiveTriggers,
    negativeTriggers,
    requiredMcpProfile,
    risk,
    expectedOutputs,
  }
  skillCardByName.set(name, card)
  return card
}

function readActiveSet(
  value: unknown,
  options: ParseAttachmentProfilesConfigOptions,
  profileByName: ReadonlyMap<string, AttachmentProfile>,
  skillCardByName: ReadonlyMap<string, SkillCard>,
  fullSkills: ReadonlySet<string>,
): AttachmentProfilesActiveSet {
  const record = value === undefined ? {} : expectRecord(value, 'attachmentProfiles.active')
  const profiles = readActiveReferences(
    record.profiles,
    'attachmentProfiles.active.profiles',
    options.maxActiveProfiles,
    'profile',
    profileByName,
  )
  const skillCards = readActiveReferences(
    record.skillCards,
    'attachmentProfiles.active.skillCards',
    options.maxActiveSkillCards,
    'skill card',
    skillCardByName,
  )
  const activeFullSkills = readActiveReferences(
    record.fullSkills,
    'attachmentProfiles.active.fullSkills',
    options.maxActiveFullSkills,
    'full skill',
    fullSkills,
  )

  return { profiles, skillCards, fullSkills: activeFullSkills }
}

function readActiveReferences(
  value: unknown,
  path: string,
  maxItems: number | undefined,
  label: string,
  knownValues: ReadonlySet<string> | ReadonlyMap<string, unknown>,
): readonly string[] {
  const refs = value === undefined ? [] : readStringArray(value, path, false)
  if (maxItems !== undefined && refs.length > maxItems) {
    throw new Error(`${path} must contain at most ${String(maxItems)} item(s)`)
  }
  refs.forEach((ref, index) => {
    if (!knownValues.has(ref)) {
      throw new Error(`${path}[${String(index)}] references unknown ${label} ${JSON.stringify(ref)}`)
    }
  })
  return refs
}

function readMcpProfile(
  value: unknown,
  path: string,
  knownMcpProfiles: ReadonlySet<string>,
): string {
  const profile = readRequiredString(value, path)
  if (!knownMcpProfiles.has(profile)) {
    throw new Error(`${path} references unknown MCP profile ${JSON.stringify(profile)}`)
  }
  return profile
}

function readPurpose(value: unknown, path: string): string {
  const purpose = readRequiredString(value, path)
  if (purpose.includes('\n') || purpose.includes('\r')) {
    throw new Error(`${path} must be one line`)
  }
  return purpose
}

function readRisk(value: unknown, path: string): SkillCardRisk {
  if (value === 'low' || value === 'medium' || value === 'high') {
    return value
  }
  throw new Error(`${path} must be "low", "medium", or "high"`)
}

function readStringArray(value: unknown, path: string, requireItems: boolean): readonly string[] {
  if (!Array.isArray(value) || (requireItems && value.length === 0)) {
    throw new Error(`${path} must be a non-empty string array`)
  }
  return value.map((item, index) => readRequiredString(item, `${path}[${String(index)}]`))
}

function readRequiredString(value: unknown, path: string): string {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`${path} must be a non-empty string`)
  }
  return value
}

function expectRecord(value: unknown, path: string): Readonly<Record<string, unknown>> {
  if (!isRecord(value)) {
    throw new Error(`${path} must be an object`)
  }
  return value
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
