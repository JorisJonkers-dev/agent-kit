import { describe, expect, it } from 'vitest'

import { parseDefaults, type GatewayConfig } from './config.js'
import {
  catalogueStaleness,
  judgeListing,
  namesOf,
  notUsableReason,
  parseLiteLLMCatalogue,
  probeFor,
  type LiteLLMCatalogue,
} from './litellm-catalogue.js'
import type { ListedModel, ModelListing } from './models.js'

const model = (id: string, status: string, extra: Record<string, unknown> = {}): Record<string, unknown> => ({
  id,
  metadata: { free: false, reason: '', status, ...extra },
})

const RAW = {
  providers: {
    'opencode-go': {
      models: [
        model('opencode-go/kimi-k2.7-code', 'usable'),
        model('opencode-go/iceman-contributor-free', 'blocked', { reason: 'This OpenCode Go model trains on request data.' }),
        model('opencode-go/beam-free', 'usable', { free: true }),
      ],
    },
    vibe: {
      models: [
        model('vibe/zai-glm-5-3', 'usable'),
        model('vibe/glm-5-3', 'error', { reason: 'timeout' }),
        model('vibe/mistral-small-latest', 'unprobed'),
        { id: 7 },
        'not a model',
        { id: 'vibe/bare' },
      ],
    },
  },
  updated_at: '2026-11-03T11:17:01Z',
}

function catalogue(raw: unknown = RAW): LiteLLMCatalogue {
  const parsed = parseLiteLLMCatalogue(raw)
  if ('problem' in parsed) throw new Error(parsed.problem)
  return parsed
}

const CONFIG: GatewayConfig = parseDefaults({
  clients: {
    claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
    opencode: { binary: 'opencode', cost_class: 'metered-personal', litellm_route: 'opencode-go', model_source: 'live' },
    vibe: { binary: 'vibe', cost_class: 'subsidised', litellm_route: 'vibe', model_source: 'live' },
  },
  models: [
    { client: 'vibe', family: 'zhipu', model: 'glm-5-3', names: ['zai-glm-5-3'], notes: 'n', tier: 'hard' },
    { client: 'claude', family: 'anthropic', model: 'claude-x', notes: 'n', tier: 'hard' },
  ],
  settings: {
    availability_window_minutes: 60,
    max_batch_delegations: 50,
    max_parallel_per_client: 3,
    metered_personal_caps: { per_day_usd: 5, per_workflow_usd: 2 },
    output_allowance_tokens: { frontier: 4, hard: 3, routine: 2, trivial: 1 },
    plan_warning_ratio: 0.8,
    summary_max_chars: 1500,
  },
  version: 1,
})

const listed = (client: string, id: string): ListedModel => ({
  client,
  cost_class: 'subsidised',
  family: 'zhipu',
  model: id,
  notes: '',
  source: 'live',
  tier: null,
})

describe('parseLiteLLMCatalogue', () => {
  it('reads each route’s models by id without the route prefix', () => {
    const read = catalogue()
    expect(read.updated_at).toBe('2026-11-03T11:17:01Z')
    expect(read.routes.get('opencode-go')?.get('beam-free')).toEqual({ free: true, reason: '', status: 'usable' })
    expect(read.routes.get('vibe')?.get('bare')).toEqual({ free: false, reason: '', status: 'unknown' })
    expect([...(read.routes.get('vibe')?.keys() ?? [])]).toEqual(['zai-glm-5-3', 'glm-5-3', 'mistral-small-latest', 'bare'])
  })

  it('keeps an id that does not carry its route prefix as it is', () => {
    expect(catalogue({ providers: { vibe: { models: [model('other', 'usable')] } }, updated_at: 'x' }).routes.get('vibe')?.has('other')).toBe(true)
  })

  it('says why a file is not a catalogue', () => {
    expect(parseLiteLLMCatalogue([])).toEqual({ problem: 'not a JSON object' })
    expect(parseLiteLLMCatalogue({ providers: {} })).toEqual({ problem: 'no updated_at' })
    expect(parseLiteLLMCatalogue({ providers: [], updated_at: 'x' })).toEqual({ problem: 'no providers object' })
    expect(parseLiteLLMCatalogue({ providers: { vibe: {} }, updated_at: 'x' })).toEqual({ problem: 'provider vibe has no models list' })
    expect(parseLiteLLMCatalogue({ providers: { vibe: 3 }, updated_at: 'x' })).toEqual({ problem: 'provider vibe has no models list' })
  })
})

describe('catalogueStaleness', () => {
  const at = Date.parse('2026-11-03T11:17:01Z')

  it('is quiet for two days, then names the age', () => {
    expect(catalogueStaleness(catalogue(), '/c.json', at + 48 * 3_600_000)).toBeUndefined()
    expect(catalogueStaleness(catalogue(), '/c.json', at + 49 * 3_600_000)).toBe(
      'the LiteLLM catalogue at /c.json is stale: updated 2026-11-03T11:17:01Z, 49 hours ago; setup refreshes it',
    )
  })

  it('warns when the catalogue names no instant', () => {
    expect(catalogueStaleness(catalogue({ ...RAW, updated_at: 'yesterday' }), '/c.json', at)).toBe(
      'the LiteLLM catalogue at /c.json names no updated_at instant, so its age is unknown',
    )
  })
})

describe('probeFor', () => {
  it('prefers a usable probe under any name', () => {
    expect(probeFor(catalogue(), 'vibe', ['glm-5-3', 'zai-glm-5-3'])?.status).toBe('usable')
    expect(probeFor(catalogue(), 'vibe', ['glm-5-3'])?.status).toBe('error')
  })

  it('strips the route prefix, and looks up no id of another provider', () => {
    expect(probeFor(catalogue(), 'opencode-go', ['opencode-go/kimi-k2.7-code'])?.status).toBe('usable')
    expect(probeFor(catalogue(), 'opencode-go', ['openrouter/kimi-k2.7-code'])).toBeUndefined()
  })

  it('knows nothing of a route or a model the catalogue does not list', () => {
    expect(probeFor(catalogue(), 'nowhere', ['x'])).toBeUndefined()
    expect(probeFor(catalogue(), 'vibe', ['x'])).toBeUndefined()
  })
})

describe('notUsableReason', () => {
  it('gives a blocked model the provider’s reason, and another status with or without one', () => {
    expect(notUsableReason('vibe', { free: false, reason: 'Labs model.', status: 'blocked' })).toBe(
      'blocked by the provider on LiteLLM route vibe: Labs model.',
    )
    expect(notUsableReason('vibe', { free: false, reason: 'timeout', status: 'error' })).toBe(
      'the LiteLLM catalogue marks it error on route vibe: timeout',
    )
    expect(notUsableReason('vibe', { free: false, reason: '', status: 'unprobed' })).toBe(
      'the LiteLLM catalogue marks it unprobed on route vibe',
    )
  })
})

describe('namesOf', () => {
  it('adds the defaults entry’s id and names, once each', () => {
    expect(namesOf(CONFIG, 'vibe', 'zai-glm-5-3')).toEqual(['zai-glm-5-3', 'glm-5-3'])
    expect(namesOf(CONFIG, 'vibe', 'glm-5-3')).toEqual(['glm-5-3', 'zai-glm-5-3'])
    expect(namesOf(CONFIG, 'vibe', 'other')).toEqual(['other'])
  })
})

describe('judgeListing', () => {
  const listing: ModelListing = {
    missing: [{ client: 'agy', reason: 'not installed' }],
    models: [
      listed('claude', 'claude-x'),
      listed('opencode', 'opencode-go/beam-free'),
      listed('opencode', 'opencode-go/iceman-contributor-free'),
      listed('opencode', 'opencode-go/kimi-k2.7-code'),
      listed('opencode', 'opencode-go/new-model'),
      listed('vibe', 'glm-5-3'),
      listed('vibe', 'mistral-small-latest'),
    ],
    skipped: [{ client: 'vibe', model: 'zzz', reason: 'earlier' }],
  }

  it('leaves the listing alone without a catalogue', () => {
    expect(judgeListing(CONFIG, listing, undefined)).toBe(listing)
  })

  it('offers only usable models, marks free ones, and skips the rest with the reason', () => {
    const judged = judgeListing(CONFIG, listing, catalogue())
    expect(judged.missing).toBe(listing.missing)
    expect(judged.models.map((entry) => [entry.client, entry.model, entry.free])).toEqual([
      ['claude', 'claude-x', undefined],
      ['opencode', 'opencode-go/beam-free', true],
      ['opencode', 'opencode-go/kimi-k2.7-code', undefined],
      ['opencode', 'opencode-go/new-model', undefined],
      ['vibe', 'glm-5-3', undefined],
    ])
    expect(judged.skipped).toEqual([
      {
        client: 'opencode',
        model: 'opencode-go/iceman-contributor-free',
        reason: 'blocked by the provider on LiteLLM route opencode-go: This OpenCode Go model trains on request data.',
      },
      { client: 'vibe', model: 'mistral-small-latest', reason: 'the LiteLLM catalogue marks it unprobed on route vibe' },
      { client: 'vibe', model: 'zzz', reason: 'earlier' },
    ])
  })

  it('judges nothing for a client the configuration no longer names', () => {
    expect(judgeListing(CONFIG, { missing: [], models: [listed('ghost', 'x')], skipped: [] }, catalogue()).models).toHaveLength(1)
  })
})
