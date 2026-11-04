import { describe, expect, it } from 'vitest'

import { parseDefaults, type GatewayConfig } from './config.js'
import { declaredClients, listModels, modelKey } from './models.js'

/** Two declared clients, one configured and one live, with models for each. */
function config(): GatewayConfig {
  return parseDefaults({
    clients: {
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
      hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'live' },
      vibe: { binary: 'vibe', cost_class: 'subsidised', model_source: 'config' },
    },
    models: [
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
      { client: 'claude', family: 'anthropic', model: 'claude-haiku-5-5', notes: 'cheap', tier: 'routine' },
      { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
      { client: 'vibe', family: 'mistral', model: 'mistral-large-4-0', notes: 'broad', tier: 'hard' },
    ],
    settings: {
      availability_window_minutes: 60,
      max_parallel_per_client: 3,
      metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
      plan_warning_ratio: 0.8,
      summary_max_chars: 1500,
    },
    version: 1,
  })
}

describe('listModels', () => {
  it('lists the declared models of an installed client with its cost class and source', () => {
    const listing = listModels(config(), new Set(['claude']))

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

  it('leaves out a declared client that is not installed and names it with the reason', () => {
    const listing = listModels(config(), new Set(['claude']))

    expect(listing.missing).toEqual([{ client: 'codex', reason: 'not installed: codex is not on PATH' }])
  })

  it('lists nothing for a client whose models come from itself or its configuration, and does not report it missing', () => {
    const listing = listModels(config(), new Set())

    expect(listing.models).toEqual([])
    expect(listing.missing.map((entry) => entry.client)).toEqual(['claude', 'codex'])
  })

  it('leaves out a model the override disabled', () => {
    const disabled = { ...config(), disabled: [modelKey('claude', 'claude-haiku-5-5')] }

    expect(listModels(disabled, new Set(['claude'])).models.map((entry) => entry.model)).toEqual(['claude-sonnet-5-5'])
  })

  it('sorts by client, then model id', () => {
    const listing = listModels(config(), new Set(['claude', 'codex']))

    expect(listing.models.map((entry) => modelKey(entry.client, entry.model))).toEqual([
      'claude:claude-haiku-5-5',
      'claude:claude-sonnet-5-5',
      'codex:gpt-5.5',
    ])
  })
})

describe('declaredClients', () => {
  it('names only the clients whose models the gateway declares, in client order', () => {
    expect(declaredClients(config()).map(([id]) => id)).toEqual(['claude', 'codex'])
  })
})

describe('modelKey', () => {
  it('joins a client and a model id the way the disabled list names them', () => {
    expect(modelKey('codex', 'gpt-5.5')).toBe('codex:gpt-5.5')
  })
})
