import { describe, expect, it } from 'vitest'

import { applyOverride, ModelGatewayError, parseDefaults, type GatewayConfig } from './config.js'

/** A small defaults file: two declared clients, one configured, two models. */
function defaults(): Record<string, unknown> {
  return {
    clients: {
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
      vibe: { binary: 'vibe', cost_class: 'subsidised', model_source: 'config' },
    },
    models: [
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
      { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
    ],
    settings: {
      availability_window_minutes: 60,
      max_batch_delegations: 50,
      max_parallel_per_client: 3,
      metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
      output_allowance_tokens: { frontier: 16000, hard: 6000, routine: 2000, trivial: 500 },
      plan_warning_ratio: 0.8,
      summary_max_chars: 1500,
    },
    version: 1,
  }
}

function shipped(): GatewayConfig {
  return parseDefaults(defaults())
}

/** The message a call throws, for the tests that pin what an error says. */
function messageOf(run: () => unknown): string {
  try {
    run()
  } catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
  throw new Error('the call did not throw')
}

/** The code a call throws, for the tests that pin how an error is reported. */
function codeOf(run: () => unknown): string | undefined {
  try {
    run()
  } catch (error) {
    return error instanceof ModelGatewayError ? error.code : undefined
  }
  throw new Error('the call did not throw')
}

describe('parseDefaults', () => {
  it('reads the shipped defaults, with no model disabled', () => {
    expect(shipped()).toEqual({
      clients: {
        claude: {
          binary: 'claude',
          cost_class: 'subsidised',
          families: undefined,
          model_source: 'declared',
          provider_cost_classes: undefined,
        },
        codex: {
          binary: 'codex',
          cost_class: 'metered-employer',
          families: undefined,
          model_source: 'declared',
          provider_cost_classes: undefined,
        },
        vibe: {
          binary: 'vibe',
          cost_class: 'subsidised',
          families: undefined,
          model_source: 'config',
          provider_cost_classes: undefined,
        },
      },
      disabled: [],
      models: [
        { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
        { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
      ],
      settings: {
        availability_window_minutes: 60,
        max_batch_delegations: 50,
        max_parallel_per_client: 3,
        metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
        output_allowance_tokens: { frontier: 16000, hard: 6000, routine: 2000, trivial: 500 },
        plan_warning_ratio: 0.8,
        summary_max_chars: 1500,
      },
      version: 1,
    })
  })

  it('keeps the families a client is restricted to', () => {
    const raw = defaults()
    raw.clients = { agy: { binary: 'agy', cost_class: 'subsidised', families: ['google'], model_source: 'live' } }

    expect(parseDefaults(raw).clients.agy?.families).toEqual(['google'])
  })

  it('keeps the cost class a client sets for each of its providers', () => {
    const raw = defaults()
    raw.clients = {
      opencode: {
        binary: 'opencode',
        cost_class: 'metered-personal',
        model_source: 'live',
        provider_cost_classes: { 'opencode-go': 'subsidised' },
      },
    }

    expect(parseDefaults(raw).clients.opencode?.provider_cost_classes).toEqual({ 'opencode-go': 'subsidised' })
  })

  it('keeps the providers a client may reach, and leaves them out when it names none', () => {
    const raw = defaults()
    raw.clients = {
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      opencode: { binary: 'opencode', cost_class: 'metered-personal', model_source: 'live', providers: ['opencode-go'] },
    }

    const { clients } = parseDefaults(raw)

    expect(clients.opencode?.providers).toEqual(['opencode-go'])
    expect(clients.claude?.providers).toBeUndefined()
  })

  it('keeps the prices a model entry carries, and leaves them out when it carries none', () => {
    const raw = defaults()
    raw.models = [
      { client: 'vibe', family: 'zhipu', model: 'zai-glm-5-3', notes: 'review', prices: { cached_input: 0.14, input: 1.4, output: 4.4 }, tier: 'hard' },
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
    ]

    expect(parseDefaults(raw).models.map((entry) => entry.prices)).toEqual([
      { cached_input: 0.14, input: 1.4, output: 4.4 },
      undefined,
    ])
  })

  const broken: [string, (raw: Record<string, unknown>) => unknown][] = [
    ['not an object', () => []],
    ['a version other than 1', (raw) => ({ ...raw, version: 2 })],
    ['clients that are not an object', (raw) => ({ ...raw, clients: [] })],
    ['a client that is not an object', (raw) => ({ ...raw, clients: { claude: 'claude' } })],
    ['a client without a binary', (raw) => ({ ...raw, clients: { claude: { cost_class: 'subsidised', model_source: 'declared' } } })],
    ['an unknown cost class', (raw) => ({ ...raw, clients: { claude: { binary: 'claude', cost_class: 'free', model_source: 'declared' } } })],
    ['an unknown model source', (raw) => ({ ...raw, clients: { claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'guess' } } })],
    ['families that are not strings', (raw) => ({ ...raw, clients: { claude: { binary: 'claude', cost_class: 'subsidised', families: [1], model_source: 'declared' } } })],
    [
      'provider cost classes that are not an object',
      (raw) => ({ ...raw, clients: { claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared', provider_cost_classes: [] } } }),
    ],
    [
      'a provider cost class that is not a cost class',
      (raw) => ({
        ...raw,
        clients: { claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared', provider_cost_classes: { p: 'free' } } },
      }),
    ],
    [
      'providers that are not strings',
      (raw) => ({ ...raw, clients: { opencode: { binary: 'opencode', cost_class: 'subsidised', model_source: 'live', providers: [1] } } }),
    ],
    ['models that are not a list', (raw) => ({ ...raw, models: {} })],
    ['a model that is not an object', (raw) => ({ ...raw, models: ['claude'] })],
    ['a model without a family', (raw) => ({ ...raw, models: [{ client: 'claude', model: 'x', notes: 'n', tier: 'hard' }] })],
    ['an unknown tier', (raw) => ({ ...raw, models: [{ client: 'claude', family: 'anthropic', model: 'x', notes: 'n', tier: 'epic' }] })],
    [
      'prices that are not an object',
      (raw) => ({ ...raw, models: [{ client: 'claude', family: 'anthropic', model: 'x', notes: 'n', prices: 1, tier: 'hard' }] }),
    ],
    [
      'a price that is missing',
      (raw) => ({
        ...raw,
        models: [{ client: 'claude', family: 'anthropic', model: 'x', notes: 'n', prices: { input: 1, output: 2 }, tier: 'hard' }],
      }),
    ],
    [
      'an input price of 0',
      (raw) => ({
        ...raw,
        models: [{ client: 'claude', family: 'anthropic', model: 'x', notes: 'n', prices: { cached_input: 0, input: 0, output: 2 }, tier: 'hard' }],
      }),
    ],
    [
      'a cached input price below 0',
      (raw) => ({
        ...raw,
        models: [{ client: 'claude', family: 'anthropic', model: 'x', notes: 'n', prices: { cached_input: -1, input: 1, output: 2 }, tier: 'hard' }],
      }),
    ],
    ['settings that are not an object', (raw) => ({ ...raw, settings: [] })],
    ['a setting that is not a number', (raw) => ({ ...raw, settings: { ...settings(), plan_warning_ratio: 'high' } })],
    ['a plan warning ratio above 1', (raw) => ({ ...raw, settings: { ...settings(), plan_warning_ratio: 80 } })],
    ['a parallel limit of 0', (raw) => ({ ...raw, settings: { ...settings(), max_parallel_per_client: 0 } })],
    ['a parallel limit that is not whole', (raw) => ({ ...raw, settings: { ...settings(), max_parallel_per_client: 1.5 } })],
    ['a batch size limit of 0', (raw) => ({ ...raw, settings: { ...settings(), max_batch_delegations: 0 } })],
    ['a batch size limit that is not whole', (raw) => ({ ...raw, settings: { ...settings(), max_batch_delegations: 2.5 } })],
    ['no batch size limit', (raw) => ({ ...raw, settings: { ...settings(), max_batch_delegations: undefined } })],
    ['caps that are not an object', (raw) => ({ ...raw, settings: { ...settings(), metered_personal_caps: 5 } })],
    ['a cap that is missing', (raw) => ({ ...raw, settings: { ...settings(), metered_personal_caps: { per_day_usd: 5 } } })],
    [
      'allowances that are not an object',
      (raw) => ({ ...raw, settings: { ...settings(), output_allowance_tokens: 500 } }),
    ],
    [
      'an allowance that is missing',
      (raw) => ({ ...raw, settings: { ...settings(), output_allowance_tokens: { trivial: 500 } } }),
    ],
    [
      'an allowance that is not a whole number of tokens',
      (raw) => ({ ...raw, settings: { ...settings(), output_allowance_tokens: { ...allowances(), hard: 1.5 } } }),
    ],
    [
      'an allowance of 0',
      (raw) => ({ ...raw, settings: { ...settings(), output_allowance_tokens: { ...allowances(), hard: 0 } } }),
    ],
    ['a disabled list that is not strings', (raw) => ({ ...raw, disabled: [1] })],
  ]

  it.each(broken)('rejects defaults that are %s', (_name, change) => {
    expect(() => parseDefaults(change(defaults()))).toThrow(ModelGatewayError)
  })

  it('says which field of the defaults is wrong', () => {
    const raw = defaults()
    raw.clients = { claude: { binary: 'claude', cost_class: 'free', model_source: 'declared' } }

    expect(messageOf(() => parseDefaults(raw))).toBe(
      'clients.claude.cost_class must be one of subsidised, metered-personal, metered-employer',
    )
  })

  it('reports a broken defaults file as invalid-config', () => {
    expect(codeOf(() => parseDefaults(null))).toBe('invalid-config')
  })
})

function settings(): Record<string, unknown> {
  return defaults().settings as Record<string, unknown>
}

/** The output allowances the fixture ships, as a test changes one of them. */
function allowances(): Record<string, number> {
  return settings().output_allowance_tokens as Record<string, number>
}

describe('applyOverride', () => {
  it('keeps the defaults when the override names nothing', () => {
    expect(applyOverride(shipped(), {})).toEqual(shipped())
  })

  it('overrides one field of a client and keeps the rest', () => {
    const merged = applyOverride(shipped(), { clients: { codex: { cost_class: 'subsidised' } } })

    expect(merged.clients.codex).toEqual({
      binary: 'codex',
      cost_class: 'subsidised',
      families: undefined,
      model_source: 'declared',
      provider_cost_classes: undefined,
    })
  })

  it('sets the cost class of one provider and keeps the others', () => {
    const base = applyOverride(shipped(), {
      clients: { opencode: { binary: 'opencode', cost_class: 'metered-personal', model_source: 'live', provider_cost_classes: { a: 'subsidised', b: 'metered-employer' } } },
    })

    const merged = applyOverride(base, { clients: { opencode: { provider_cost_classes: { b: 'subsidised' } } } })

    expect(merged.clients.opencode?.provider_cost_classes).toEqual({ a: 'subsidised', b: 'subsidised' })
  })

  it('replaces the providers a client may reach', () => {
    const base = applyOverride(shipped(), {
      clients: { opencode: { binary: 'opencode', cost_class: 'metered-personal', model_source: 'live', providers: ['a'] } },
    })

    const merged = applyOverride(base, { clients: { opencode: { providers: ['b', 'c'] } } })

    expect(merged.clients.opencode?.providers).toEqual(['b', 'c'])
    expect(applyOverride(merged, { clients: { opencode: { cost_class: 'subsidised' } } }).clients.opencode?.providers).toEqual(['b', 'c'])
  })

  it('adds a client the defaults do not name', () => {
    const merged = applyOverride(shipped(), {
      clients: { hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live' } },
    })

    expect(merged.clients.hermes).toEqual({
      binary: 'hermes',
      cost_class: 'metered-personal',
      families: undefined,
      model_source: 'live',
      provider_cost_classes: undefined,
    })
  })

  it('reads the providers a machine lets a client run on', () => {
    const merged = applyOverride(shipped(), {
      clients: { hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live', providers: ['anthropic', 'google'] } },
    })

    expect(merged.clients.hermes).toMatchObject({ providers: ['anthropic', 'google'] })
  })

  it('keeps the providers a client override does not name', () => {
    const base = applyOverride(shipped(), {
      clients: { hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live', providers: ['anthropic'] } },
    })

    const merged = applyOverride(base, { clients: { hermes: { cost_class: 'subsidised' } } })

    expect(merged.clients.hermes).toMatchObject({ cost_class: 'subsidised', providers: ['anthropic'] })
  })

  it('lets no provider when the machine names none', () => {
    expect(shipped().clients.vibe?.providers).toBeUndefined()
  })

  it('refuses a provider list that is not one', () => {
    expect(messageOf(() => applyOverride(shipped(), { clients: { vibe: { providers: 'anthropic' } } }))).toBe(
      'clients.vibe.providers must be a list of non-empty strings',
    )
  })

  it('refuses a client field it does not know, rather than quietly ignoring it', () => {
    expect(messageOf(() => applyOverride(shipped(), { clients: { vibe: { read_only_proof: 'trusted' } } }))).toBe(
      'clients.vibe.read_only_proof is not a field a client has; choose from binary, cost_class, model_source, families, provider_cost_classes, providers, plan_limits',
    )
  })

  it('refuses a top-level key it does not know, rather than applying nothing and saying nothing', () => {
    expect(messageOf(() => applyOverride(shipped(), { client: { codex: { cost_class: 'subsidised' } } }))).toBe(
      'the override.client is not a key the override has; choose from version, clients, disabled, models, settings',
    )
  })

  it('overrides the tier, notes and family of a model it names', () => {
    const merged = applyOverride(shipped(), {
      models: [{ client: 'codex', family: 'moonshot', model: 'gpt-5.5', notes: 'grant needed', tier: 'hard' }],
    })

    expect(merged.models).toEqual([
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
      { client: 'codex', family: 'moonshot', model: 'gpt-5.5', notes: 'grant needed', tier: 'hard' },
    ])
  })

  it('keeps the fields a model override does not name', () => {
    const merged = applyOverride(shipped(), { models: [{ client: 'claude', model: 'claude-sonnet-5-5', tier: 'frontier' }] })

    expect(merged.models[0]).toEqual({
      client: 'claude',
      family: 'anthropic',
      model: 'claude-sonnet-5-5',
      notes: 'general',
      tier: 'frontier',
    })
  })

  it('reads the config names of a catalogue entry and keeps them when an override names the model without them', () => {
    const base = parseDefaults({
      ...defaults(),
      models: [{ client: 'vibe', family: 'zhipu', model: 'glm-5-3', names: ['zai-glm-5-3'], notes: 'review', tier: 'hard' }],
    })
    const merged = applyOverride(base, { models: [{ client: 'vibe', model: 'glm-5-3', tier: 'frontier' }] })
    const renamed = applyOverride(base, { models: [{ client: 'vibe', model: 'glm-5-3', names: ['zai-glm-6'] }] })

    expect(base.models[0]?.names).toEqual(['zai-glm-5-3'])
    expect(merged.models[0]).toMatchObject({ names: ['zai-glm-5-3'], tier: 'frontier' })
    expect(renamed.models[0]?.names).toEqual(['zai-glm-6'])
  })

  it('rejects config names that are not a list of non-empty strings', () => {
    expect(() => applyOverride(shipped(), { models: [{ client: 'claude', model: 'claude-sonnet-5-5', names: [''] }] })).toThrow(
      'models[0].names must be a list of non-empty strings',
    )
  })

  it('overrides the entry a model is named by one of its config names, keeping its alias', () => {
    const base = parseDefaults({
      ...defaults(),
      models: [{ client: 'vibe', family: 'zhipu', model: 'glm-5-3', names: ['zai-glm-5-3'], notes: 'review', tier: 'hard' }],
    })

    const merged = applyOverride(base, { models: [{ client: 'vibe', model: 'zai-glm-5-3', tier: 'frontier' }] })

    expect(merged.models).toEqual([
      { client: 'vibe', family: 'zhipu', model: 'glm-5-3', names: ['zai-glm-5-3'], notes: 'review', tier: 'frontier' },
    ])
  })

  it('adds a declared model the defaults do not name', () => {
    const merged = applyOverride(shipped(), {
      models: [{ client: 'claude', family: 'anthropic', model: 'claude-opus-5-5', notes: 'frontier', tier: 'frontier' }],
    })

    expect(merged.models.map((entry) => entry.model)).toEqual(['claude-sonnet-5-5', 'gpt-5.5', 'claude-opus-5-5'])
  })

  it('overrides one setting and deep-merges the caps', () => {
    const merged = applyOverride(shipped(), {
      settings: { metered_personal_caps: { per_workflow_usd: 1 }, summary_max_chars: 500 },
    })

    expect(merged.settings).toEqual({
      availability_window_minutes: 60,
      max_batch_delegations: 50,
      max_parallel_per_client: 3,
      metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 1 },
      output_allowance_tokens: { frontier: 16000, hard: 6000, routine: 2000, trivial: 500 },
      plan_warning_ratio: 0.8,
      summary_max_chars: 500,
    })
  })

  it('overrides the batch size limit and the parallel limit', () => {
    const merged = applyOverride(shipped(), { settings: { max_batch_delegations: 10, max_parallel_per_client: 2 } })

    expect(merged.settings).toMatchObject({ max_batch_delegations: 10, max_parallel_per_client: 2 })
  })

  it('overrides the output allowance of one tier and keeps the others', () => {
    const merged = applyOverride(shipped(), { settings: { output_allowance_tokens: { hard: 1000 } } })

    expect(merged.settings.output_allowance_tokens).toEqual({ frontier: 16000, hard: 1000, routine: 2000, trivial: 500 })
  })

  it('replaces the disabled list', () => {
    expect(applyOverride(shipped(), { disabled: ['codex:gpt-5.5'] }).disabled).toEqual(['codex:gpt-5.5'])
  })

  it('keeps the disabled list when the override does not name one', () => {
    const base = { ...shipped(), disabled: ['codex:gpt-5.5'] }

    expect(applyOverride(base, { settings: { summary_max_chars: 500 } }).disabled).toEqual(['codex:gpt-5.5'])
  })

  it('refuses a plan warning ratio outside 0..1, naming the setting', () => {
    expect(messageOf(() => applyOverride(shipped(), { settings: { plan_warning_ratio: 80 } }))).toBe(
      'settings.plan_warning_ratio must be between 0 and 1',
    )
  })

  const broken: [string, unknown][] = [
    ['not an object', []],
    ['a version other than 1', { version: 2 }],
    ['clients that are not an object', { clients: [] }],
    ['a client that is not an object', { clients: { claude: 'claude' } }],
    ['a new client without a cost class', { clients: { hermes: { binary: 'hermes', model_source: 'live' } } }],
    ['an unknown cost class', { clients: { claude: { cost_class: 'free' } } }],
    ['an unknown model source', { clients: { claude: { model_source: 'guess' } } }],
    ['families that are not strings', { clients: { claude: { families: 'google' } } }],
    ['provider cost classes that are not an object', { clients: { claude: { provider_cost_classes: 'free' } } }],
    ['a provider cost class that is not a cost class', { clients: { claude: { provider_cost_classes: { p: 'free' } } } }],
    ['models that are not a list', { models: {} }],
    ['a model that is not an object', { models: ['claude'] }],
    ['a model without a client', { models: [{ model: 'gpt-5.5' }] }],
    ['a model without a model id', { models: [{ client: 'codex' }] }],
    ['a new model without notes', { models: [{ client: 'claude', family: 'anthropic', model: 'new', tier: 'hard' }] }],
    ['an unknown tier', { models: [{ client: 'codex', model: 'gpt-5.5', tier: 'epic' }] }],
    ['settings that are not an object', { settings: [] }],
    ['a setting that is not a number', { settings: { summary_max_chars: 'long' } }],
    ['a parallel limit of 0', { settings: { max_parallel_per_client: 0 } }],
    ['a parallel limit that is not whole', { settings: { max_parallel_per_client: 1.5 } }],
    ['a batch size limit of 0', { settings: { max_batch_delegations: 0 } }],
    ['a batch size limit that is not whole', { settings: { max_batch_delegations: 2.5 } }],
    ['caps that are not an object', { settings: { metered_personal_caps: 5 } }],
    ['a cap that is not a number', { settings: { metered_personal_caps: { per_day_usd: 'five' } } }],
    ['allowances that are not an object', { settings: { output_allowance_tokens: 500 } }],
    ['an allowance that is not a whole number of tokens', { settings: { output_allowance_tokens: { hard: 1.5 } } }],
    ['a disabled list that is not strings', { disabled: 'codex:gpt-5.5' }],
  ]

  it.each(broken)('rejects an override that is %s', (_name, override) => {
    expect(() => applyOverride(shipped(), override)).toThrow(ModelGatewayError)
  })

  it('says which field of the override is wrong', () => {
    expect(messageOf(() => applyOverride(shipped(), { models: [{ client: 'codex', model: 'gpt-5.5', tier: 'epic' }] }))).toBe(
      'models[0].tier must be one of trivial, routine, hard, frontier',
    )
  })

  it('says a new model entry needs its family, tier and notes', () => {
    expect(messageOf(() => applyOverride(shipped(), { models: [{ client: 'claude', model: 'new' }] }))).toBe(
      'models[0] (claude:new) is new, so it needs family, tier and notes',
    )
  })
})
