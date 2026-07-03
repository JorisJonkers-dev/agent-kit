export type ContextArchetype =
  | 'implementer'
  | 'researcher'
  | 'reviewer'
  | 'surveyor'
  | 'designer-lens'
  | 'consolidator-judge'

export type WorkspaceAxis = 'none' | 'read' | 'write'
export type RepoContextAxis = 'none' | 'summary' | 'targeted' | 'full'
export type SkillsAxis = 'none' | 'named' | 'relevant' | 'all'
export type McpAxis = 'none' | 'read' | 'write'
export type NetworkAxis = 'off' | 'on'

export interface ContextAxes {
  readonly workspace: WorkspaceAxis
  readonly repo_context: RepoContextAxis
  readonly skills: SkillsAxis
  readonly mcp: McpAxis
  readonly network: NetworkAxis
}

export interface ContextProfileRequest {
  readonly archetype?: string
  readonly context_profile?: string
  readonly overrides?: Partial<ContextAxes>
}

export interface ResolvedContextProfile extends ContextAxes {
  readonly archetype: ContextArchetype
  readonly profile: string
}

const IMPLEMENTER_AXES: ContextAxes = {
  workspace: 'write',
  repo_context: 'targeted',
  skills: 'relevant',
  mcp: 'read',
  network: 'off',
}

const MINIMAL_AXES: ContextAxes = {
  workspace: 'read',
  repo_context: 'summary',
  skills: 'named',
  mcp: 'none',
  network: 'off',
}

const FULL_AXES: ContextAxes = {
  workspace: 'read',
  repo_context: 'full',
  skills: 'relevant',
  mcp: 'read',
  network: 'off',
}

export const ARCHETYPE_CONTEXT_DEFAULTS: Readonly<Record<ContextArchetype, ContextAxes>> = {
  implementer: IMPLEMENTER_AXES,
  researcher: {
    workspace: 'read',
    repo_context: 'summary',
    skills: 'relevant',
    mcp: 'read',
    network: 'on',
  },
  reviewer: {
    workspace: 'read',
    repo_context: 'targeted',
    skills: 'relevant',
    mcp: 'read',
    network: 'off',
  },
  surveyor: {
    workspace: 'read',
    repo_context: 'full',
    skills: 'relevant',
    mcp: 'read',
    network: 'on',
  },
  'designer-lens': {
    workspace: 'read',
    repo_context: 'targeted',
    skills: 'relevant',
    mcp: 'none',
    network: 'off',
  },
  'consolidator-judge': {
    workspace: 'read',
    repo_context: 'full',
    skills: 'named',
    mcp: 'none',
    network: 'off',
  },
}

export const NAMED_CONTEXT_PROFILES: Readonly<Record<string, ContextAxes>> = {
  minimal: MINIMAL_AXES,
  focused: IMPLEMENTER_AXES,
  full: FULL_AXES,
  networked: {
    ...FULL_AXES,
    network: 'on',
  },
  'seed-if-absent': MINIMAL_AXES,
}

const DEFAULT_ARCHETYPE: ContextArchetype = 'implementer'

export function resolveContextProfile(
  request: ContextProfileRequest = {},
): ResolvedContextProfile {
  const archetype = normalizeArchetype(request.archetype)
  const profileAxes = parseContextProfile(request.context_profile)
  const base = profileAxes ?? ARCHETYPE_CONTEXT_DEFAULTS[archetype]
  const trimmedProfile = request.context_profile?.trim()
  const profile = trimmedProfile === undefined || trimmedProfile.length === 0 ? archetype : trimmedProfile

  return {
    ...base,
    ...request.overrides,
    archetype,
    profile,
  }
}

export function normalizeArchetype(value: string | undefined): ContextArchetype {
  if (isContextArchetype(value)) {
    return value
  }

  return DEFAULT_ARCHETYPE
}

export function parseContextProfile(value: string | undefined): ContextAxes | undefined {
  const profile = value?.trim()
  if (!profile) {
    return undefined
  }

  const named = NAMED_CONTEXT_PROFILES[profile]
  if (named) {
    return named
  }

  const entries = profile
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)

  if (entries.length === 0 || entries.every((entry) => !entry.includes('='))) {
    return undefined
  }

  let axes: Partial<ContextAxes> = {}
  for (const entry of entries) {
    const [rawKey, rawValue] = entry.split('=', 2)
    axes = {
      ...axes,
      ...parseAxisEntry(rawKey?.trim(), rawValue?.trim()),
    }
  }

  return {
    ...IMPLEMENTER_AXES,
    ...axes,
  }
}

function isContextArchetype(value: string | undefined): value is ContextArchetype {
  return (
    value === 'implementer' ||
    value === 'researcher' ||
    value === 'reviewer' ||
    value === 'surveyor' ||
    value === 'designer-lens' ||
    value === 'consolidator-judge'
  )
}

function parseAxisEntry(
  key: string | undefined,
  value: string | undefined,
): Partial<ContextAxes> {
  switch (key) {
    case 'workspace':
      if (value === 'none' || value === 'read' || value === 'write') {
        return { workspace: value }
      }
      break
    case 'repo_context':
      if (
        value === 'none' ||
        value === 'summary' ||
        value === 'targeted' ||
        value === 'full'
      ) {
        return { repo_context: value }
      }
      break
    case 'skills':
      if (value === 'none' || value === 'named' || value === 'relevant' || value === 'all') {
        return { skills: value }
      }
      break
    case 'mcp':
      if (value === 'none' || value === 'read' || value === 'write') {
        return { mcp: value }
      }
      break
    case 'network':
      if (value === 'off' || value === 'on') {
        return { network: value }
      }
      break
  }

  return {}
}
