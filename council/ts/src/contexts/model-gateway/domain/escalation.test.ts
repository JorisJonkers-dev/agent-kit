import { describe, expect, it } from 'vitest'

import type { CostClass, GatewayConfig, Tier } from './config.js'
import {
  attemptInput,
  candidatePool,
  climbOrder,
  fallThroughOrder,
  FAILURE_TAIL_CHARS,
  isCannotStart,
  isRefusal,
  retryPrompt,
  type Candidate,
} from './escalation.js'
import { decidePolicy, type DataScope } from './policy.js'
import type { RunInput } from './run.js'

function candidate(client: string, model: string, tier: Tier | null, costClass: CostClass = 'subsidised'): Candidate {
  return { client, costClass, family: 'any', model, tier }
}

const POOL: readonly Candidate[] = [
  candidate('claude', 'opus', 'frontier'),
  candidate('claude', 'sonnet', 'hard'),
  candidate('claude', 'haiku', 'routine'),
  candidate('vibe', 'glm', 'hard'),
  candidate('hermes', 'personal-hard', 'hard', 'metered-personal'),
  candidate('hermes', 'personal-frontier', 'frontier', 'metered-personal'),
  candidate('codex', 'employer-frontier', 'frontier', 'metered-employer'),
]

function keys(candidates: readonly Candidate[]): string[] {
  return candidates.map((entry) => entry.model)
}

describe('climbOrder', () => {
  it('lists the stronger models of the same cost class by tier and then by catalogue order', () => {
    expect(keys(climbOrder(candidate('vibe', 'small', 'trivial'), POOL.filter((entry) => entry.costClass === 'subsidised')))).toEqual([
      'haiku',
      'sonnet',
      'glm',
      'opus',
    ])
  })

  it('lists only subsidised models, so a metered-personal model climbs to a stronger subsidised one and never to a stronger metered-personal one', () => {
    const personal = candidate('hermes', 'personal-routine', 'routine', 'metered-personal')

    expect(keys(climbOrder(personal, POOL))).toEqual(['sonnet', 'glm', 'opus'])
    expect(keys(climbOrder(candidate('claude', 'haiku', 'routine'), POOL))).toEqual(['sonnet', 'glm', 'opus'])
  })

  it('never lists a model of no stronger tier, itself, or a model that has no tier, and has nowhere to climb from a model that has none', () => {
    const untiered = candidate('vibe', 'unknown', null)

    expect(keys(climbOrder(candidate('claude', 'opus', 'frontier'), POOL))).toEqual([])
    expect(keys(climbOrder(candidate('claude', 'sonnet', 'hard'), [...POOL, untiered]))).toEqual(['opus'])
    expect(climbOrder(untiered, POOL)).toEqual([])
  })
})

describe('fallThroughOrder', () => {
  it('lists the models of the same cost class at the wanted tier or above, the nearest tier first', () => {
    expect(keys(fallThroughOrder(candidate('vibe', 'glm', 'hard'), POOL))).toEqual(['sonnet', 'opus'])
    expect(keys(fallThroughOrder(candidate('hermes', 'personal-hard', 'hard', 'metered-personal'), POOL))).toEqual(['personal-frontier'])
  })

  it('lists nothing for a model that has no tier, since nothing is known to cover it', () => {
    expect(fallThroughOrder(candidate('vibe', 'unknown', null), POOL)).toEqual([])
  })
})

describe('failures', () => {
  it('reads refusals and launches that never started from a result’s reason', () => {
    expect(['policy-refused', 'cap-exceeded', 'no-estimate', 'workflow-ended', 'provider-not-allowed', 'unknown-client'].map(isRefusal)).toEqual([
      true,
      true,
      true,
      true,
      true,
      true,
    ])
    expect([undefined, 'timeout', 'exit-1', 'check-failed', 'parse-error'].map(isRefusal)).toEqual([false, false, false, false, false])
    expect(['auth', 'rate-limited', 'launch-failed'].map(isCannotStart)).toEqual([true, true, true])
    expect([undefined, 'timeout', 'exit-1', 'check-failed'].map(isCannotStart)).toEqual([false, false, false, false])
  })

  it('attaches the failure to the original prompt, cut to its last characters', () => {
    expect(retryPrompt('do it', 'exit-1', '  boom\n')).toBe('do it\n\nA previous attempt failed: exit-1; boom')
    expect(retryPrompt('do it', 'timeout', '  \n')).toBe('do it\n\nA previous attempt failed: timeout')
    const long = `${'a'.repeat(FAILURE_TAIL_CHARS)}${'b'.repeat(FAILURE_TAIL_CHARS)}`
    expect(retryPrompt('do it', 'exit-1', long)).toBe(`do it\n\nA previous attempt failed: exit-1; ${'b'.repeat(FAILURE_TAIL_CHARS)}`)
  })

  it('builds an attempt’s input on the model it is moved to, keeping the provider only for a client that names one', () => {
    const input: RunInput = { client: 'vibe', mode: 'read-only', model: 'glm', prompt: 'p', provider: 'stale', time_limit_s: 60, workflow: 'w' }

    expect(attemptInput(input, candidate('claude', 'opus', 'frontier'), 'again')).toEqual({
      client: 'claude',
      mode: 'read-only',
      model: 'opus',
      prompt: 'again',
      time_limit_s: 60,
      workflow: 'w',
    })
    expect(attemptInput(input, { ...candidate('hermes', 'm', 'hard'), provider: 'anthropic' }, 'again').provider).toBe('anthropic')
  })
})

/** A catalogue with one subsidised and one metered-employer model, both permitted for the test repository. */
const CONFIG: GatewayConfig = {
  clients: {
    claude: {
      binary: 'claude',
      cost_class: 'subsidised',
      families: undefined,
      model_source: 'declared',
      provider_cost_classes: undefined,
      providers: undefined,
    },
    codex: {
      binary: 'codex',
      cost_class: 'metered-employer',
      families: undefined,
      model_source: 'declared',
      provider_cost_classes: undefined,
      providers: undefined,
    },
  },
  disabled: [],
  models: [
    { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'n', tier: 'hard' },
    { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'n', tier: 'frontier' },
  ],
  settings: {
    availability_window_minutes: 60,
    max_parallel_per_client: 3,
    metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
    output_allowance_tokens: { frontier: 16000, hard: 6000, routine: 2000, trivial: 500 },
    plan_warning_ratio: 0.8,
    summary_max_chars: 1500,
  },
  version: 1,
}

const SCOPE: DataScope = {
  entry: { code: true, families: ['anthropic', 'openai'], history: true },
  key: 'github.com/acme/widgets',
  kind: 'repository',
}

const RUN: RunInput = { client: 'claude', mode: 'read-only', model: 'claude-sonnet-5-5', prompt: 'p', repo: '/work', time_limit_s: 60, workflow: 'w' }

/** The clients a live grant covers in a test: none, or the metered-employer client the catalogue declares. */
const NO_GRANT: ReadonlySet<string> = new Set()
const CODEX_GRANT: ReadonlySet<string> = new Set(['codex'])

function pool(granted: ReadonlySet<string>): string[] {
  return candidatePool(CONFIG, RUN, decidePolicy(CONFIG, SCOPE, CONFIG.models).permissions, granted).map(
    (candidate) => `${candidate.client}:${candidate.model}`,
  )
}

describe('candidatePool', () => {
  it('leaves a metered-employer model out while no live grant covers the workflow', () => {
    expect(pool(NO_GRANT)).toEqual(['claude:claude-sonnet-5-5'])
  })

  it('lets a metered-employer model join the pool while a live grant covers the workflow', () => {
    expect(pool(CODEX_GRANT)).toEqual(['claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
  })

  it('lets a metered-employer model in only for the client its grant names', () => {
    expect(pool(new Set(['opencode']))).toEqual(['claude:claude-sonnet-5-5'])
  })

  it('leaves out a model whose client the configuration does not declare', () => {
    const orphaned: GatewayConfig = {
      ...CONFIG,
      models: [...CONFIG.models, { client: 'nobody', family: 'openai', model: 'ghost', notes: 'n', tier: 'frontier' }],
    }

    expect(
      candidatePool(orphaned, RUN, decidePolicy(orphaned, SCOPE, orphaned.models).permissions, CODEX_GRANT).map(
        (candidate) => `${candidate.client}:${candidate.model}`,
      ),
    ).toEqual(['claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
  })

  it('never lets a metered-employer model be a climb target, whatever the grant', () => {
    const granted = candidatePool(CONFIG, RUN, decidePolicy(CONFIG, SCOPE, CONFIG.models).permissions, CODEX_GRANT)

    expect(climbOrder(candidate('claude', 'claude-sonnet-5-5', 'hard'), granted)).toEqual([])
  })

  it('lets a client that cannot start fall through to a metered-employer model of the same cost class only under a grant', () => {
    const employer = candidate('codex', 'gpt-5.5', 'frontier', 'metered-employer')
    const granted = candidatePool(CONFIG, RUN, decidePolicy(CONFIG, SCOPE, CONFIG.models).permissions, CODEX_GRANT)

    expect(fallThroughOrder(employer, granted)).toEqual([])
    expect(fallThroughOrder(employer, [...granted, candidate('codex', 'gpt-5.6', 'frontier', 'metered-employer')])).toEqual([
      candidate('codex', 'gpt-5.6', 'frontier', 'metered-employer'),
    ])
  })
})
