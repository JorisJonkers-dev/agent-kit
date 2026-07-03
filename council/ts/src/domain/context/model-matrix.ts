import type { EngineDef, EngineTagged } from '../contracts/index.js'

export type ModelTierName = 'cheap' | 'standard' | 'strong' | 'max'
export type ModelWorkload = 'single' | 'bulk' | 'strict-json'

export interface ModelMatrixEntry {
  readonly tier: ModelTierName
  readonly codex: EngineDef
  readonly claude: EngineDef
}

export interface ModelMatrixRequest extends EngineTagged {
  readonly workload?: ModelWorkload
  readonly strictJson?: boolean
  readonly bulk?: boolean
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
  moderate: 'standard',
  standard: 'standard',
  hard: 'strong',
  strong: 'strong',
  high: 'strong',
  xhigh: 'max',
  max: 'max',
  opus: 'max',
}

export function resolveModelMatrix(request: ModelMatrixRequest = {}): ResolvedModel {
  if (request.engine) {
    return {
      engine: request.engine,
      model_tier: normalizeModelTier(request.model_tier),
      reason: 'explicit-engine',
    }
  }

  const tier = normalizeModelTier(request.model_tier)
  const entry = MODEL_MATRIX[tier]

  if (request.strictJson || request.workload === 'strict-json') {
    return {
      engine: entry.claude,
      model_tier: tier,
      reason: 'strict-json-claude',
    }
  }

  if (request.bulk || request.workload === 'bulk') {
    return {
      engine: entry.codex,
      model_tier: tier,
      reason: 'bulk-codex-first',
    }
  }

  return {
    engine: entry.claude,
    model_tier: tier,
    reason: 'single-claude-default',
  }
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
