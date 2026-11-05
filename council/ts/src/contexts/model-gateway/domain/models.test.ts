import { describe, expect, it } from 'vitest'

import { parseDefaults, type ClientConfig, type GatewayConfig } from './config.js'
import {
  familyOf,
  familyOfModelId,
  hermesDefaultModel,
  listModels,
  modelKey,
  parseAntigravityModels,
  parseOpenCodeModels,
  vibeModels,
  type ClientListing,
  type ListedModel,
} from './models.js'

/** Two declared clients, one live, one configured and one live with providers, with models for each. */
function config(): GatewayConfig {
  return parseDefaults({
    clients: {
      agy: { binary: 'agy', cost_class: 'subsidised', families: ['google'], model_source: 'live' },
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
      hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live' },
      opencode: {
        binary: 'opencode',
        cost_class: 'metered-personal',
        model_source: 'live',
        provider_cost_classes: { 'opencode-go': 'subsidised' },
      },
      vibe: { binary: 'vibe', cost_class: 'subsidised', model_source: 'config' },
    },
    models: [
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
      { client: 'claude', family: 'anthropic', model: 'claude-haiku-5-5', notes: 'cheap', tier: 'routine' },
      { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
      { client: 'vibe', family: 'mistral', model: 'mistral-large-latest', names: ['mistral-large-4-0'], notes: 'broad', tier: 'hard' },
      { client: 'vibe', family: 'zhipu', model: 'glm-5-3', names: ['zai-glm-5-3'], notes: 'review', tier: 'hard' },
      { client: 'agy', family: 'google', model: 'gemini-3.8-flash-high', notes: 'long context', tier: 'hard' },
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
}

/** The listings a client's own source gave, by client id. */
function listings(entries: Readonly<Record<string, ClientListing>> = {}): ReadonlyMap<string, ClientListing> {
  return new Map(Object.entries(entries))
}

/** The configuration with one client's settings changed. */
function withClient(base: GatewayConfig, id: string, patch: Partial<ClientConfig>): GatewayConfig {
  const client = base.clients[id]
  if (client === undefined) throw new Error(`no client ${id}`)
  return { ...base, clients: { ...base.clients, [id]: { ...client, ...patch } } }
}

/** The Vibe models its configuration declared, leaving the catalogue's built-ins aside. */
function fromConfig(gateway: GatewayConfig, listed: ReadonlyMap<string, ClientListing>): readonly ListedModel[] {
  return listModels(gateway, new Set(['vibe']), listed).models.filter((entry) => entry.source === 'config')
}

describe('listModels', () => {
  it('lists a model a listing names twice once', () => {
    const listed = listings({ opencode: { models: [{ model: 'opencode-go/glm-5.3', provider: 'opencode-go' }, { model: 'opencode-go/glm-5.3', provider: 'opencode-go' }] } })

    expect(listModels(config(), new Set(['opencode']), listed).models).toHaveLength(1)
  })

  it('lists a configured Vibe GLM model the catalogue does not know', () => {
    const listed = listings({ vibe: { models: [{ model: 'zai-glm-6' }] } })

    expect(fromConfig(config(), listed).map((entry) => [entry.model, entry.family])).toEqual([['zai-glm-6', 'zhipu']])
  })

  it('lists the declared models of an installed client with its cost class and source', () => {
    const listing = listModels(config(), new Set(['claude']), listings())

    expect(listing.models).toEqual([
      {
        client: 'claude',
        cost_class: 'subsidised',
        family: 'anthropic',
        model: 'claude-haiku-5-5',
        notes: 'cheap',
        source: 'declared',
        tier: 'routine',
      },
      {
        client: 'claude',
        cost_class: 'subsidised',
        family: 'anthropic',
        model: 'claude-sonnet-5-5',
        notes: 'general',
        source: 'declared',
        tier: 'hard',
      },
    ])
  })

  it('leaves out a client that is not installed and names it with the reason, whatever its source', () => {
    const listing = listModels(config(), new Set(['claude']), listings())

    expect(listing.missing).toEqual([
      { client: 'agy', reason: 'not installed: agy is not on PATH' },
      { client: 'codex', reason: 'not installed: codex is not on PATH' },
      { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
      { client: 'opencode', reason: 'not installed: opencode is not on PATH' },
      { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
    ])
  })

  it('lists nothing for an installed client whose listing the gateway does not have, and does not report it missing', () => {
    const listing = listModels(config(), new Set(['hermes']), listings())

    expect(listing.models).toEqual([])
    expect(listing.missing.map((entry) => entry.client)).toEqual(['agy', 'claude', 'codex', 'opencode', 'vibe'])
  })

  it('joins a live listing to the catalogue, and marks it live', () => {
    const listing = listModels(config(), new Set(['agy']), listings({ agy: { models: [{ model: 'gemini-3.8-flash-high' }] } }))

    expect(listing.models).toEqual([
      {
        client: 'agy',
        cost_class: 'subsidised',
        family: 'google',
        model: 'gemini-3.8-flash-high',
        notes: 'long context',
        source: 'live',
        tier: 'hard',
      },
    ])
  })

  it('lists a model the catalogue does not know with no tier and no notes', () => {
    const listing = listModels(config(), new Set(['agy']), listings({ agy: { models: [{ model: 'gemini-4-pro' }] } }))

    expect(listing.models).toEqual([
      { client: 'agy', cost_class: 'subsidised', family: 'google', model: 'gemini-4-pro', notes: '', source: 'live', tier: null },
    ])
  })

  it('leaves out a model of a family the client may not reach', () => {
    const listed = listings({
      agy: { models: [{ model: 'gemini-4-pro' }, { model: 'claude-sonnet-4-6' }, { model: 'gpt-oss-120b-medium' }] },
    })

    expect(listModels(config(), new Set(['agy']), listed).models.map((entry) => entry.model)).toEqual(['gemini-4-pro'])
  })

  it('reports every model whose family it cannot work out in skipped, with the reason', () => {
    const listed = listings({
      opencode: {
        models: [
          { model: 'opencode-go/longcat-2.0', provider: 'opencode-go' },
          { model: 'opencode-go/hy3', provider: 'opencode-go' },
        ],
      },
    })

    const listing = listModels(config(), new Set(['opencode']), listed)

    expect(listing.models).toEqual([])
    expect(listing.skipped).toEqual([
      {
        client: 'opencode',
        model: 'opencode-go/hy3',
        reason: 'no catalogue entry with a family and a tier, and the id names no known family: add one to defaults.json or the machine override',
      },
      {
        client: 'opencode',
        model: 'opencode-go/longcat-2.0',
        reason: 'no catalogue entry with a family and a tier, and the id names no known family: add one to defaults.json or the machine override',
      },
    ])
  })

  it('takes the cost class of the provider a model runs on, and the client cost class for the rest', () => {
    const listed = listings({
      opencode: {
        models: [
          { model: 'opencode-go/glm-5.3', provider: 'opencode-go' },
          { model: 'other/gemini-4-argon', provider: 'other' },
        ],
      },
    })
    const named = withClient(config(), 'opencode', { providers: ['opencode-go', 'other'] })

    expect(listModels(named, new Set(['opencode']), listed).models.map((entry) => [entry.model, entry.cost_class])).toEqual([
      ['opencode-go/glm-5.3', 'subsidised'],
      ['other/gemini-4-argon', 'metered-personal'],
    ])
  })

  it('lists a configured client by the aliases its configuration declares, marked config', () => {
    const listed = listings({ vibe: { models: [{ model: 'mistral-large-latest' }, { model: 'mistral-medium-4-1' }] } })

    expect(fromConfig(config(), listed)).toEqual([
      {
        client: 'vibe',
        cost_class: 'subsidised',
        family: 'mistral',
        model: 'mistral-large-latest',
        notes: 'broad',
        source: 'config',
        tier: 'hard',
      },
      {
        client: 'vibe',
        cost_class: 'subsidised',
        family: 'mistral',
        model: 'mistral-medium-4-1',
        notes: '',
        source: 'config',
        tier: null,
      },
    ])
  })

  it('joins a Vibe model to the catalogue on its config name and lists it once, by its alias', () => {
    const listed = listings({ vibe: { models: [{ model: 'glm-5-3', names: ['zai-glm-5-3'] }, { model: 'glm-5-3', names: ['zai-glm-5-3'] }] } })

    expect(fromConfig(config(), listed)).toEqual([
      {
        client: 'vibe',
        cost_class: 'subsidised',
        family: 'zhipu',
        model: 'glm-5-3',
        notes: 'review',
        source: 'config',
        tier: 'hard',
      },
    ])
  })

  it('joins a Vibe model on its name when its alias is not the catalogue id', () => {
    const listed = listings({ vibe: { models: [{ model: 'large', names: ['mistral-large-4-0'] }] } })

    expect(fromConfig(config(), listed).map((entry) => [entry.model, entry.tier, entry.notes])).toEqual([
      ['large', 'hard', 'broad'],
    ])
  })

  it('joins a Vibe model on an alias the catalogue lists among its names', () => {
    const listed = listings({ vibe: { models: [{ model: 'zai-glm-5-3' }] } })

    expect(fromConfig(config(), listed).map((entry) => [entry.model, entry.tier])).toEqual([['zai-glm-5-3', 'hard']])
  })

  it('lists a Vibe model whose alias and names both miss the catalogue with no tier, its family from its name', () => {
    const listed = listings({ vibe: { models: [{ model: 'fast', names: ['mistral-medium-4-1'] }] } })

    expect(fromConfig(config(), listed).map((entry) => [entry.model, entry.family, entry.tier, entry.notes])).toEqual([
      ['fast', 'mistral', null, ''],
    ])
  })

  it('reports a Vibe model whose alias and names name no family', () => {
    const listed = listings({ vibe: { models: [{ model: 'fast', names: ['mystery'] }] } })

    expect(listModels(config(), new Set(['vibe']), listed).skipped.map((entry) => entry.model)).toEqual(['fast'])
  })

  it('leaves out a Vibe model whose alias or catalogue id the override disabled', () => {
    const listed = listings({ vibe: { models: [{ model: 'glm-5-3', names: ['zai-glm-5-3'] }, { model: 'mistral-large-latest' }] } })
    const byAlias = { ...config(), disabled: [modelKey('vibe', 'glm-5-3')] }
    const byName = { ...config(), disabled: [modelKey('vibe', 'mistral-large-latest')] }

    expect(fromConfig(byAlias, listed).map((entry) => entry.model)).toEqual(['mistral-large-latest'])
    expect(fromConfig(byName, listed).map((entry) => entry.model)).toEqual(['glm-5-3'])
  })

  it('leaves out a Vibe model whose recorded catalogue name the override disabled', () => {
    const listed = listings({ vibe: { models: [{ model: 'glm-5-3', names: ['zai-glm-5-3'] }] } })
    const disabled = { ...config(), disabled: [modelKey('vibe', 'zai-glm-5-3')] }

    expect(fromConfig(disabled, listed)).toEqual([])
  })

  it('leaves out a model the override disabled', () => {
    const disabled = { ...config(), disabled: [modelKey('claude', 'claude-haiku-5-5'), modelKey('agy', 'gemini-4-pro')] }
    const listed = listings({ agy: { models: [{ model: 'gemini-4-pro' }] } })

    expect(listModels(disabled, new Set(['agy', 'claude']), listed).models.map((entry) => entry.model)).toEqual([
      'claude-sonnet-5-5',
    ])
  })

  it('reports a listing that failed in missing, with its reason', () => {
    const installed = new Set(['agy', 'claude', 'codex', 'hermes', 'opencode', 'vibe'])
    const listing = listModels(config(), installed, listings({ agy: { reason: 'listing failed: agy models exited 1' } }))

    expect(listing.missing).toEqual([{ client: 'agy', reason: 'listing failed: agy models exited 1' }])
    expect(listing.models.map((entry) => entry.model)).toEqual(['claude-haiku-5-5', 'claude-sonnet-5-5', 'gpt-5.5'])
  })

  it('sorts by client, then model id', () => {
    const listed = listings({
      agy: { models: [{ model: 'gemini-4-pro' }, { model: 'gemini-3.8-flash-high' }] },
      vibe: { models: [{ model: 'glm-5-3' }, { model: 'mistral-large-latest' }] },
    })
    const listing = listModels(config(), new Set(['agy', 'claude', 'codex', 'vibe']), listed)

    expect(listing.models.map((entry) => modelKey(entry.client, entry.model))).toEqual([
      'agy:gemini-3.8-flash-high',
      'agy:gemini-4-pro',
      'claude:claude-haiku-5-5',
      'claude:claude-sonnet-5-5',
      'codex:gpt-5.5',
      'vibe:glm-5-3',
      'vibe:mistral-large-latest',
    ])
  })
})

describe('familyOf', () => {
  it('names the family a model id starts with', () => {
    expect(familyOf('gemini-3.8-flash-high')).toBe('google')
    expect(familyOf('claude-sonnet-4-6')).toBe('anthropic')
    expect(familyOf('gpt-oss-120b-medium')).toBe('openai')
    expect(familyOf('deepseek-v4.1-flash')).toBe('deepseek')
    expect(familyOf('glm-5.3')).toBe('zhipu')
    expect(familyOf('zai-glm-5-3')).toBe('zhipu')
    expect(familyOf('zai-glm-6')).toBe('zhipu')
    expect(familyOf('kimi-k3')).toBe('moonshot')
    expect(familyOf('minimax-m2')).toBe('minimax')
    expect(familyOf('grok-4.7')).toBe('xai')
    expect(familyOf('qwen3-coder')).toBe('alibaba')
    expect(familyOf('mistral-large-4-0')).toBe('mistral')
    expect(familyOf('codestral-2508')).toBe('mistral')
  })

  it('says nothing for a model id it cannot place', () => {
    expect(familyOf('hy3')).toBeUndefined()
    expect(familyOf('longcat-2.0')).toBeUndefined()
  })
})

describe('familyOfModelId', () => {
  it('names the family of a model id a client writes as <provider>/<model>', () => {
    expect(familyOfModelId('anthropic/claude-sonnet-4.6')).toBe('anthropic')
    expect(familyOfModelId('google/gemini-3.8-flash-high')).toBe('google')
  })

  it('names the family of a bare model id, as a client that writes no provider does', () => {
    expect(familyOfModelId('glm-5-3')).toBe('zhipu')
  })

  it('says nothing for a model id it cannot place', () => {
    expect(familyOfModelId('anthropic/hy3')).toBeUndefined()
  })
})

describe('hermesDefaultModel', () => {
  it('reads the model and provider Hermes’ own configuration names as its default', () => {
    expect(hermesDefaultModel('{"default":"anthropic/claude-sonnet-4.6","provider":"anthropic"}')).toEqual({
      model: 'anthropic/claude-sonnet-4.6',
      provider: 'anthropic',
    })
  })

  it('reads a default model whose provider the configuration does not name', () => {
    expect(hermesDefaultModel('{"default":"glm-5.3"}')).toEqual({ model: 'glm-5.3' })
  })

  it('reads a configuration that answers with the model id alone', () => {
    expect(hermesDefaultModel('"glm-5.3"')).toEqual({ model: 'glm-5.3' })
  })

  it('says nothing when the configuration names no model, or answers with something else', () => {
    expect(hermesDefaultModel('{}')).toBeUndefined()
    expect(hermesDefaultModel('{"default":""}')).toBeUndefined()
    expect(hermesDefaultModel('""')).toBeUndefined()
    expect(hermesDefaultModel('not json')).toBeUndefined()
    expect(hermesDefaultModel('[]')).toBeUndefined()
  })
})

describe('parseAntigravityModels', () => {
  it('reads one model per line after the fetching line, by its id', () => {
    const stdout = 'Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6\n'

    expect(parseAntigravityModels(stdout)).toEqual(['gemini-3.8-flash-high', 'claude-sonnet-4-6'])
  })

  it('reads a listing that ends without a newline, and skips lines that name no model', () => {
    expect(parseAntigravityModels('Fetching available models...\ngemini-4-pro\tGemini 4 Pro')).toEqual(['gemini-4-pro'])
    expect(parseAntigravityModels('Fetching available models...\n\n\t\n')).toEqual([])
  })
})

describe('parseOpenCodeModels', () => {
  it('reads one provider model per line, with the provider it runs on', () => {
    expect(parseOpenCodeModels('opencode-go/glm-5.3\nother/gemini-4-argon\n')).toEqual([
      { model: 'opencode-go/glm-5.3', provider: 'opencode-go' },
      { model: 'other/gemini-4-argon', provider: 'other' },
    ])
  })

  it('skips a line that names no provider or no model', () => {
    expect(parseOpenCodeModels('\nno-provider\n/provider-only\nprovider/\n')).toEqual([])
  })
})

describe('vibeModels', () => {
  it('reads no model from an entry that is not a table', () => {
    expect(vibeModels('models = { oops = "x" }\n')).toEqual([])
  })

  it('reads the alias and name of every model the configuration declares, in file order', () => {
    const toml = `[models.mistral-large-4-0]
name = "mistral-large-4-0"
provider = "mistral"
alias = "mistral-large-latest"

[models.zai-glm-5-3]
name = "zai-glm-5-3"
provider = "mistral"
alias = "glm-5-3"
`

    expect(vibeModels(toml)).toEqual([
      { model: 'mistral-large-latest', names: ['mistral-large-4-0'] },
      { model: 'glm-5-3', names: ['zai-glm-5-3'] },
    ])
  })

  it('names a model by its alias alone when the alias is its name', () => {
    expect(vibeModels('models = { small = { name = "small", alias = "small", provider = "mistral" } }\n')).toEqual([{ model: 'small', names: [] }])
  })

  it('reads an inline models table, and names a model without an alias by its key', () => {
    expect(vibeModels('models = { small = { name = "small-1", provider = "mistral" } }\n')).toEqual([{ model: 'small', names: ['small-1'] }])
    expect(vibeModels('models = { small = { provider = "mistral" } }\n')).toEqual([{ model: 'small', names: [] }])
  })

  it('reads no model from a configuration that declares none', () => {
    expect(vibeModels('active_model = "mistral-large-4-0"\n')).toEqual([])
    expect(vibeModels('models = "not a table"\n')).toEqual([])
  })

  it('throws on a file that is not TOML, so the caller never acts on half a file', () => {
    expect(() => vibeModels('this is not TOML\n')).toThrow()
  })
})

describe('modelKey', () => {
  it('joins a client and a model id the way the disabled list names them', () => {
    expect(modelKey('codex', 'gpt-5.5')).toBe('codex:gpt-5.5')
  })
})

describe('listModels: Vibe built-in models', () => {
  const installed = new Set(['vibe'])

  it('lists the catalogue entries, marked declared, when the configuration declares nothing', () => {
    const listing = listModels(config(), installed, listings({ vibe: { models: [] } }))

    expect(listing.models.map((entry) => [entry.model, entry.source, entry.tier])).toEqual([
      ['glm-5-3', 'declared', 'hard'],
      ['mistral-large-latest', 'declared', 'hard'],
    ])
    expect(listing.models[0]).toMatchObject({ client: 'vibe', cost_class: 'subsidised', family: 'zhipu', notes: 'review' })
  })

  it('lists a model the configuration declares once, marked config, and the rest declared', () => {
    const listed = listings({ vibe: { models: [{ model: 'glm-5-3', names: ['zai-glm-5-3'] }] } })

    expect(listModels(config(), installed, listed).models.map((entry) => [entry.model, entry.source])).toEqual([
      ['glm-5-3', 'config'],
      ['mistral-large-latest', 'declared'],
    ])
  })

  it('lists a configured alias that names a catalogue entry once, under the alias', () => {
    const listed = listings({ vibe: { models: [{ model: 'large', names: ['mistral-large-4-0'] }] } })

    expect(listModels(config(), installed, listed).models.map((entry) => [entry.model, entry.source])).toEqual([
      ['glm-5-3', 'declared'],
      ['large', 'config'],
    ])
  })

  it('does not bring back a catalogue entry whose configured model the override disabled', () => {
    const listed = listings({ vibe: { models: [{ model: 'large', names: ['mistral-large-4-0'] }] } })
    const disabled = { ...config(), disabled: [modelKey('vibe', 'large')] }

    expect(listModels(disabled, installed, listed).models.map((entry) => entry.model)).toEqual(['glm-5-3'])
  })

  it('leaves out a catalogue entry the override disabled', () => {
    const disabled = { ...config(), disabled: [modelKey('vibe', 'zai-glm-5-3')] }

    expect(listModels(disabled, installed, listings({ vibe: { models: [] } })).models.map((entry) => entry.model)).toEqual([
      'mistral-large-latest',
    ])
  })

  it('lists the catalogue entries for Vibe when its configuration cannot be read, and still reports the reason', () => {
    const listing = listModels(config(), installed, listings({ vibe: { reason: 'no configuration' } }))

    expect(listing.models.map((entry) => [entry.model, entry.source])).toEqual([
      ['glm-5-3', 'declared'],
      ['mistral-large-latest', 'declared'],
    ])
    expect(listing.missing.filter((entry) => entry.client === 'vibe')).toEqual([{ client: 'vibe', reason: 'no configuration' }])
  })

  it('lists no model for a client without built-in models when its listing cannot be read', () => {
    const listing = listModels(config(), new Set(['agy']), listings({ agy: { reason: 'listing failed' } }))

    expect(listing.models).toEqual([])
    expect(listing.missing.filter((entry) => entry.client === 'agy')).toEqual([{ client: 'agy', reason: 'listing failed' }])
  })

  it('applies the client family restriction to the built-in models', () => {
    const restricted = withClient(config(), 'vibe', { families: ['mistral'] })

    expect(listModels(restricted, installed, listings({ vibe: { models: [] } })).models.map((entry) => entry.model)).toEqual([
      'mistral-large-latest',
    ])
  })

  it('leaves out a configured model of a barred family without hiding or leaking its catalogue entry', () => {
    const restricted = withClient(config(), 'vibe', { families: ['mistral'] })
    const gateway = {
      ...restricted,
      models: [...restricted.models, { client: 'vibe', family: 'zhipu', model: 'glm-5-3-extra', notes: 'x', tier: 'hard' as const }],
    }
    const listing = listModels(gateway, installed, listings({ vibe: { models: [{ model: 'glm', names: ['zai-glm-5-3'] }] } }))

    expect(listing.models.map((entry) => [entry.model, entry.source])).toEqual([['mistral-large-latest', 'declared']])
    expect(listing.skipped).toEqual([])
  })

  it('lists no catalogue entry for a configured client that has no built-in models, Hermes included', () => {
    const base = withClient(config(), 'hermes', { model_source: 'config' })
    const hermes = {
      ...base,
      models: [...base.models, { client: 'hermes', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'x', tier: 'hard' as const }],
    }

    const listing = listModels(hermes, new Set(['hermes']), listings({ hermes: { models: [{ model: 'gpt-5.5' }] } }))

    expect(listing.models.map((entry) => [entry.model, entry.source])).toEqual([['gpt-5.5', 'config']])
  })
})

describe('listModels: OpenCode providers', () => {
  const installed = new Set(['opencode'])
  const mixed = {
    models: [
      { model: 'opencode-go/glm-5.3', provider: 'opencode-go' },
      { model: 'opencode/glm-5.3', provider: 'opencode' },
      { model: 'other/gemini-4-argon', provider: 'other' },
      { model: 'openrouter/hy3', provider: 'openrouter' },
    ],
  }

  it('lists only the models of the default opencode-go provider, and drops the rest without a trace', () => {
    const listing = listModels(config(), installed, listings({ opencode: mixed }))

    expect(listing.models.map((entry) => entry.model)).toEqual(['opencode-go/glm-5.3'])
    expect(listing.skipped).toEqual([])
    expect(listing.missing.filter((entry) => entry.client === 'opencode')).toEqual([])
  })

  it('lists the models of every provider the client names, but never the pay-per-token opencode provider', () => {
    const named = withClient(config(), 'opencode', { providers: ['opencode-go', 'other', 'opencode'] })

    expect(listModels(named, installed, listings({ opencode: mixed })).models.map((entry) => entry.model)).toEqual([
      'opencode-go/glm-5.3',
      'other/gemini-4-argon',
    ])
  })

  it('drops an OpenCode model that names no provider', () => {
    const listed = listings({ opencode: { models: [{ model: 'glm-5.3' }] } })

    expect(listModels(config(), installed, listed).models).toEqual([])
  })

  it('lists an OpenCode model whose provider only its id names, as a run accepts it', () => {
    const listed = listings({ opencode: { models: [{ model: 'opencode-go/glm-5.3' }] } })

    expect(listModels(config(), installed, listed).models.map((entry) => [entry.model, entry.cost_class])).toEqual([
      ['opencode-go/glm-5.3', 'subsidised'],
    ])
  })
})
