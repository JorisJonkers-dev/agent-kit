import type { EngineDef, EngineTagged } from '../../../domain/contracts/index.js'

export type ModelTierName = 'cheap' | 'standard' | 'strong' | 'max'
export type ModelWorkload = 'single' | 'bulk' | 'strict-json'

// Council role classes drive the default (non-program) engine choice.
// Canonical serialization is kebab-case; underscore aliases are accepted only
// at the parse boundary (parseRoleClass).
export type ModelRoleClass = 'investigate' | 'cross-critique' | 'implement' | 'consolidate-lock'

export interface ModelMatrixEntry {
  readonly tier: ModelTierName
  readonly codex: EngineDef
  readonly claude: EngineDef
}

export interface ModelMatrixRequest extends EngineTagged {
  readonly workload?: ModelWorkload
  readonly strictJson?: boolean
  readonly bulk?: boolean
  readonly roleClass?: ModelRoleClass
  readonly programScale?: boolean
}

const ROLE_CLASS_ALIASES: Readonly<Record<string, ModelRoleClass>> = {
  investigate: 'investigate',
  research: 'investigate',
  'cross-critique': 'cross-critique',
  cross_critique: 'cross-critique',
  critique: 'cross-critique',
  implement: 'implement',
  implementation: 'implement',
  'consolidate-lock': 'consolidate-lock',
  consolidate_lock: 'consolidate-lock',
  consolidate: 'consolidate-lock',
  lock: 'consolidate-lock',
}

export function parseRoleClass(value: string | undefined): ModelRoleClass | undefined {
  return ROLE_CLASS_ALIASES[value?.trim() ?? '']
}

export interface ResolvedModel {
  readonly engine: EngineDef
  readonly model_tier: ModelTierName
  readonly reason: string
}

export const MODEL_MATRIX: Readonly<Record<ModelTierName, ModelMatrixEntry>> = {
  cheap: {
    tier: 'cheap',
    codex: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
    claude: { cli: 'claude', model: 'haiku', label: 'claude:haiku' },
  },
  standard: {
    tier: 'standard',
    codex: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
    claude: { cli: 'claude', model: 'haiku', label: 'claude:haiku' },
  },
  strong: {
    tier: 'strong',
    codex: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
    claude: { cli: 'claude', model: 'sonnet', label: 'claude:sonnet' },
  },
  max: {
    tier: 'max',
    codex: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
    claude: { cli: 'claude', model: 'opus', label: 'claude:opus' },
  },
}

export const MODEL_TIER_ALIASES: Readonly<Record<string, ModelTierName>> = {
  trivial: 'cheap',
  cheap: 'cheap',
  haiku: 'cheap',
  moderate: 'standard',
  standard: 'standard',
  hard: 'strong',
  strong: 'strong',
  high: 'strong',
  sonnet: 'strong',
  xhigh: 'max',
  max: 'max',
  opus: 'max',
}

// Resolution precedence (locked by the council):
//   explicit-engine
//     > strict-json-claude                       (capability constraint: JSON reliability)
//     > program-consolidate-lock -> claude:opus   (the one protected program-scale stage)
//     > program-codex-override                    (program scale: every other role -> codex)
//     > role-class-policy                          (default scale: investigate/critique->codex,
//                                                   implement->claude capped at strong,
//                                                   consolidate-lock->claude)
//     > legacy-workload                            (bulk->codex, single->claude)
export function resolveModelMatrix(request: ModelMatrixRequest = {}): ResolvedModel {
  if (request.engine) {
    return { engine: request.engine, model_tier: normalizeModelTier(request.model_tier), reason: 'explicit-engine' }
  }

  const tier = normalizeModelTier(request.model_tier)
  const entry = MODEL_MATRIX[tier]

  if (request.strictJson || request.workload === 'strict-json') {
    return { engine: entry.claude, model_tier: tier, reason: 'strict-json-claude' }
  }

  const role = request.roleClass

  if (request.programScale) {
    if (role === 'consolidate-lock') {
      return { engine: MODEL_MATRIX.max.claude, model_tier: 'max', reason: 'program-consolidate-lock-claude-opus' }
    }
    return { engine: entry.codex, model_tier: tier, reason: 'program-codex-override' }
  }

  if (role !== undefined) {
    if (role === 'investigate' || role === 'cross-critique') {
      return { engine: entry.codex, model_tier: tier, reason: 'role-class-policy' }
    }
    if (role === 'implement') {
      // Implementation stays cheap: never opus unless an engine was set explicitly.
      const capped: ModelTierName = tier === 'max' ? 'strong' : tier
      return { engine: MODEL_MATRIX[capped].claude, model_tier: capped, reason: 'role-class-policy' }
    }
    // consolidate-lock (default scale): strong claude, opus only at the max tier.
    return { engine: entry.claude, model_tier: tier, reason: 'role-class-policy' }
  }

  if (request.bulk || request.workload === 'bulk') {
    return { engine: entry.codex, model_tier: tier, reason: 'bulk-codex-first' }
  }

  return { engine: entry.claude, model_tier: tier, reason: 'single-claude-default' }
}

export function normalizeModelTier(value: string | undefined): ModelTierName {
  return MODEL_TIER_ALIASES[value?.trim() ?? ''] ?? 'standard'
}

export function parseEngineSpec(spec: string): EngineDef {
  const [cli, model] = spec.split(':', 2)
  if ((cli !== 'claude' && cli !== 'codex') || !model) {
    throw new Error(`engine must be claude:<model> or codex:<model>: ${spec}`)
  }

  return {
    cli,
    model,
    label: `${cli}:${model}`,
  }
}
